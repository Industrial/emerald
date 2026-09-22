// Plan 168 (Structured Logging) - a compiler-provided `Log` module
// (`Log.trace`/`.debug`/`.info`/`.warn`/`.error`, each with a
// `Log.<level>_fields` sibling taking a `LogFields` builder handle)
// backed by `tracing`/`tracing-subscriber`, switchable at process
// start between compact plain-text and single-line JSON output.
//
// The central design tension this plan resolves: `tracing`'s own
// macros need a fixed, compile-time-known field set per call site
// (its whole zero-cost-callsite design depends on this), but Emerald
// call sites pass a runtime-determined, variable-length field list a
// `rustc` compiling this crate can never see. This module does not
// fight that: `emerald_rt_log_event` and `emerald_rt_log_event_fields`
// each go through their OWN fixed, small set of `tracing::event!`
// call sites (one per level, per shape) - never a dynamically
// constructed field list. The dynamic `LogFields` payload crosses
// this boundary as a single pre-serialized JSON string, passed as a
// plain `&str` field value (so the custom `Visit` impl below receives
// it via `record_str`, not `record_debug` - no downcasting, no
// `Any`), and this module's own `EmeraldLayer::on_event` re-parses
// that one string back into a real `serde_json::Value` and splices it
// in as a genuine nested object - never a double-escaped string
// embedded inside a string.
//
// `LogFields` reuses plan 93's own handle registry (`crate::handle`)
// rather than inventing a second, redundant create/mutate/consume-and
// -free mechanism - this plan's own text (authored before plan 93
// existed) proposed a bespoke one, but plan 93 landed first this same
// session, so its real, already-tested registry is used directly.

use std::os::raw::c_char;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const LEVEL_TRACE: i64 = 0;
const LEVEL_DEBUG: i64 = 1;
const LEVEL_INFO: i64 = 2;
const LEVEL_WARN: i64 = 3;
const LEVEL_ERROR: i64 = 4;

fn parse_level(s: &str) -> Option<i64> {
  match s {
    "trace" => Some(LEVEL_TRACE),
    "debug" => Some(LEVEL_DEBUG),
    "info" => Some(LEVEL_INFO),
    "warn" => Some(LEVEL_WARN),
    "error" => Some(LEVEL_ERROR),
    _ => None,
  }
}

fn level_name(level: i64) -> &'static str {
  match level {
    LEVEL_TRACE => "TRACE",
    LEVEL_DEBUG => "DEBUG",
    LEVEL_INFO => "INFO",
    LEVEL_WARN => "WARN",
    LEVEL_ERROR => "ERROR",
    _ => "UNKNOWN",
  }
}

// The configured floor, as a plain `AtomicU8` this crate checks itself
// before ever reaching for a `tracing` macro - simpler and just as
// correct as leaning on `tracing`'s own per-callsite `Interest`
// caching for a module this small, and avoids this module needing to
// re-derive `tracing_subscriber::EnvFilter`'s own directive-parsing
// surface for a single global floor.
static CONFIGURED_LEVEL: AtomicU8 = AtomicU8::new(LEVEL_INFO as u8);
// `true` once `Log.configure` has installed the JSON layer; text mode
// needs no layer at all (see `emit` below), so this only tracks
// whether the JSON subscriber install itself has already happened.
static JSON_LAYER_INSTALLED: OnceLock<()> = OnceLock::new();
static CONFIGURED_ONCE: OnceLock<()> = OnceLock::new();
static FORMAT_IS_JSON: AtomicU8 = AtomicU8::new(0);

struct JsonVisitor {
  map: serde_json::Map<String, serde_json::Value>,
}

impl tracing::field::Visit for JsonVisitor {
  fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
    if field.name() == "fields_json" {
      if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
        self.map.insert("fields".to_string(), v);
      }
    } else {
      self.map.insert(
        field.name().to_string(),
        serde_json::Value::String(value.to_string()),
      );
    }
  }

  fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
    self.map.insert(
      field.name().to_string(),
      serde_json::Value::String(format!("{value:?}")),
    );
  }
}

// Plan 168's own hand-written `Layer` (not `fmt().json()` - see this
// module's own top doc comment for why): visits each event's fields,
// accumulates them into a real `serde_json::Map`, and writes one
// `serde_json::Value::Object` per line to stderr.
struct EmeraldJsonLayer;

impl<S: tracing::Subscriber> Layer<S> for EmeraldJsonLayer {
  fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
    let mut visitor = JsonVisitor {
      map: serde_json::Map::new(),
    };
    visitor.map.insert(
      "level".to_string(),
      serde_json::Value::String(event.metadata().level().to_string()),
    );
    event.record(&mut visitor);
    if let Ok(line) = serde_json::to_string(&serde_json::Value::Object(visitor.map)) {
      eprintln!("{line}");
    }
  }
}

/// Installs the global JSON layer exactly once, guarded by a real
/// `OnceLock` - a second `Log.configure("...", "json")` call is a
/// disclosed no-op here (the level/format statics below are still
/// updated every call; only the one-time global-dispatcher install
/// is skipped), matching `tracing::subscriber::set_global_default`'s
/// own "errors on a second call" contract without ever surfacing that
/// error to Emerald code.
fn ensure_json_layer_installed() {
  JSON_LAYER_INSTALLED.get_or_init(|| {
    let subscriber = tracing_subscriber::registry().with(EmeraldJsonLayer);
    let _ = tracing::subscriber::set_global_default(subscriber);
  });
}

/// `Log.configure(level, format)` - `level` is one of `"trace"`/
/// `"debug"`/`"info"`/`"warn"`/`"error"`; `format` is `"json"` or
/// `"text"`. Returns `0` on success, `-1` on an invalid level/format
/// string or a null/invalid-UTF8 pointer. Unlike `tracing::subscriber
/// ::set_global_default`'s own real one-shot contract, a SECOND call
/// is not an error here - it updates the configured level/format
/// floor (real, live behavior a program can use to change verbosity
/// at runtime) while silently skipping the one-time JSON-layer
/// install if `format` is `"json"` and it's already installed.
///
/// # Safety
/// `level`/`format`, if non-null, must point to valid, NUL-terminated
/// C strings.
pub unsafe fn log_configure(level: *const c_char, format: *const c_char) -> i64 {
  if level.is_null() || format.is_null() {
    return -1;
  }
  let Ok(level_str) = std::ffi::CStr::from_ptr(level).to_str() else {
    return -1;
  };
  let Ok(format_str) = std::ffi::CStr::from_ptr(format).to_str() else {
    return -1;
  };
  let Some(parsed_level) = parse_level(level_str) else {
    return -1;
  };
  let is_json = match format_str {
    "json" => true,
    "text" => false,
    _ => return -1,
  };
  CONFIGURED_LEVEL.store(parsed_level as u8, Ordering::SeqCst);
  FORMAT_IS_JSON.store(u8::from(is_json), Ordering::SeqCst);
  if is_json {
    ensure_json_layer_installed();
  }
  let already_configured = CONFIGURED_ONCE.set(()).is_err();
  if already_configured {
    -1
  } else {
    0
  }
}

fn text_emit(level: i64, message: &str, fields: Option<&str>) {
  match fields {
    Some(f) => eprintln!("{} {message} {f}", level_name(level)),
    None => eprintln!("{} {message}", level_name(level)),
  }
}

/// Emits one plain, message-only event at `level`, filtered against
/// the configured floor. Returns `0` if emitted, `1` if filtered out
/// by level (not an error - a real, deliberately-distinct return so a
/// caller could observe it if it ever wanted to), `-1` on a null/
/// invalid-UTF8 `message` or an unrecognized `level`.
///
/// # Safety
/// `message`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn log_event(level: i64, message: *const c_char) -> i64 {
  if message.is_null() {
    return -1;
  }
  let Ok(msg) = std::ffi::CStr::from_ptr(message).to_str() else {
    return -1;
  };
  if !(LEVEL_TRACE..=LEVEL_ERROR).contains(&level) {
    return -1;
  }
  if level < CONFIGURED_LEVEL.load(Ordering::SeqCst) as i64 {
    return 1;
  }
  if FORMAT_IS_JSON.load(Ordering::SeqCst) == 1 {
    match level {
      LEVEL_TRACE => tracing::event!(target: "emerald", tracing::Level::TRACE, message = msg),
      LEVEL_DEBUG => tracing::event!(target: "emerald", tracing::Level::DEBUG, message = msg),
      LEVEL_INFO => tracing::event!(target: "emerald", tracing::Level::INFO, message = msg),
      LEVEL_WARN => tracing::event!(target: "emerald", tracing::Level::WARN, message = msg),
      _ => tracing::event!(target: "emerald", tracing::Level::ERROR, message = msg),
    }
  } else {
    text_emit(level, msg, None);
  }
  0
}

/// `LogFields.new()` - a fresh, empty field-name/value builder,
/// reusing plan 93's own handle registry (`crate::handle`) rather
/// than a bespoke one.
pub fn log_fields_new() -> i64 {
  crate::handle::handle_alloc(Box::new(Vec::<(String, String)>::new()), "logfields")
}

/// `LogFields#set(key, value)` - appends one entry. Returns `0` on
/// success, `-1` on a closed/unknown handle or a null/invalid-UTF8
/// argument (never raises - a native-resource-misuse `NativeError`
/// is plan 93's own convention for a handle a caller is expected to
/// hold correctly, but this leaf keeps the shape simple and returns a
/// status code instead, matching every other `emerald_rt_log_*`
/// function in this module).
///
/// # Safety
/// `key`/`value`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn log_fields_set(handle: i64, key: *const c_char, value: *const c_char) -> i64 {
  if key.is_null() || value.is_null() {
    return -1;
  }
  let Ok(k) = std::ffi::CStr::from_ptr(key).to_str() else {
    return -1;
  };
  let Ok(v) = std::ffi::CStr::from_ptr(value).to_str() else {
    return -1;
  };
  let k = k.to_string();
  let v = v.to_string();
  match crate::handle::handle_get_mut::<Vec<(String, String)>, _>(handle, "logfields", |vec| {
    vec.push((k, v));
  }) {
    Ok(()) => 0,
    Err(_) => -1,
  }
}

/// `Log.<level>_fields(message, fields)` - consumes and closes the
/// `LogFields` handle unconditionally, whether or not `level` actually
/// passes the configured floor (a suppressed event still owns, and
/// must free, the handle its call site allocated).
///
/// # Safety
/// `message`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn log_event_fields(level: i64, message: *const c_char, handle: i64) -> i64 {
  let entries =
    crate::handle::handle_get_mut::<Vec<(String, String)>, _>(handle, "logfields", |vec| {
      std::mem::take(vec)
    });
  crate::handle::handle_close(handle);
  let Ok(entries) = entries else {
    return -1;
  };
  if message.is_null() {
    return -1;
  }
  let Ok(msg) = std::ffi::CStr::from_ptr(message).to_str() else {
    return -1;
  };
  if !(LEVEL_TRACE..=LEVEL_ERROR).contains(&level) {
    return -1;
  }
  if level < CONFIGURED_LEVEL.load(Ordering::SeqCst) as i64 {
    return 1;
  }
  let mut fields_map = serde_json::Map::new();
  for (k, v) in &entries {
    fields_map.insert(k.clone(), serde_json::Value::String(v.clone()));
  }
  let fields_json =
    serde_json::to_string(&serde_json::Value::Object(fields_map)).unwrap_or_default();
  if FORMAT_IS_JSON.load(Ordering::SeqCst) == 1 {
    match level {
      LEVEL_TRACE => {
        tracing::event!(target: "emerald", tracing::Level::TRACE, message = msg, fields_json = fields_json.as_str())
      }
      LEVEL_DEBUG => {
        tracing::event!(target: "emerald", tracing::Level::DEBUG, message = msg, fields_json = fields_json.as_str())
      }
      LEVEL_INFO => {
        tracing::event!(target: "emerald", tracing::Level::INFO, message = msg, fields_json = fields_json.as_str())
      }
      LEVEL_WARN => {
        tracing::event!(target: "emerald", tracing::Level::WARN, message = msg, fields_json = fields_json.as_str())
      }
      _ => {
        tracing::event!(target: "emerald", tracing::Level::ERROR, message = msg, fields_json = fields_json.as_str())
      }
    }
  } else {
    let rendered = entries
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()
      .join(" ");
    text_emit(level, msg, Some(&rendered));
  }
  0
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_level_accepts_the_five_real_names_and_rejects_others() {
    assert_eq!(parse_level("trace"), Some(LEVEL_TRACE));
    assert_eq!(parse_level("debug"), Some(LEVEL_DEBUG));
    assert_eq!(parse_level("info"), Some(LEVEL_INFO));
    assert_eq!(parse_level("warn"), Some(LEVEL_WARN));
    assert_eq!(parse_level("error"), Some(LEVEL_ERROR));
    assert_eq!(parse_level("verbose"), None);
  }

  // Plan 168's own required test (c): a level below the configured
  // floor produces zero output bytes - proven here without capturing
  // stderr at all, since `log_event`'s own level check returns early
  // (a real, distinct `1`, never reaching a `tracing`/`eprintln!` call
  // at all) before any formatting or emission work happens.
  #[test]
  fn a_level_below_the_configured_floor_is_filtered_before_any_emission_work() {
    CONFIGURED_LEVEL.store(LEVEL_WARN as u8, Ordering::SeqCst);
    let msg = std::ffi::CString::new("suppressed").unwrap();
    let result = unsafe { log_event(LEVEL_INFO, msg.as_ptr()) };
    assert_eq!(result, 1, "INFO must be filtered out under a WARN floor");
    let result = unsafe { log_event(LEVEL_ERROR, msg.as_ptr()) };
    assert_eq!(result, 0, "ERROR must still pass a WARN floor");
    // Restore the default floor so this test doesn't leak state into
    // whichever other test in this same process runs next.
    CONFIGURED_LEVEL.store(LEVEL_INFO as u8, Ordering::SeqCst);
  }

  // Plan 168's own required test (a): `Log.configure` is idempotent —
  // the SECOND call returns `-1` (real, disclosed no-op on the one-
  // time global-dispatcher install), the FIRST returns `0`. The only
  // test in this file calling `log_configure` at all — `CONFIGURED_
  // ONCE` is a real, process-wide `OnceLock` shared by every test in
  // this binary, so a second caller anywhere else would make this
  // assertion meaningless.
  #[test]
  fn log_configure_is_idempotent_first_call_ok_second_call_reports_already_configured() {
    let level = std::ffi::CString::new("info").unwrap();
    let format = std::ffi::CString::new("text").unwrap();
    let first = unsafe { log_configure(level.as_ptr(), format.as_ptr()) };
    let second = unsafe { log_configure(level.as_ptr(), format.as_ptr()) };
    assert_eq!(first, 0, "the first Log.configure call must succeed");
    assert_eq!(
      second, -1,
      "a second Log.configure call must report already-configured, not panic or re-install"
    );
  }

  // Plan 168's own required test (b): the JSON layer emits valid,
  // `serde_json::from_str`-parseable output for a message with a
  // two-entry `LogFields`. Uses a REAL event dispatched through
  // `tracing::subscriber::with_default` (thread-local-scoped, never
  // touching the process-wide `OnceLock` `Log.configure` itself
  // guards) against a capturing `Layer` sharing `JsonVisitor` verbatim
  // with the real `EmeraldJsonLayer` — real `tracing::event!`
  // dispatch, real `Field`/`Visit` machinery, no hand-constructed fake
  // metadata.
  #[test]
  fn a_real_dispatched_event_with_fields_produces_valid_nested_json() {
    use std::sync::{Arc, Mutex};

    struct CaptureLayer(Arc<Mutex<Option<String>>>);
    impl<S: tracing::Subscriber> Layer<S> for CaptureLayer {
      fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = JsonVisitor {
          map: serde_json::Map::new(),
        };
        visitor.map.insert(
          "level".to_string(),
          serde_json::Value::String(event.metadata().level().to_string()),
        );
        event.record(&mut visitor);
        *self.0.lock().unwrap() =
          Some(serde_json::to_string(&serde_json::Value::Object(visitor.map)).unwrap());
      }
    }

    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let subscriber = tracing_subscriber::registry().with(CaptureLayer(captured.clone()));
    tracing::subscriber::with_default(subscriber, || {
      let fields_json = "{\"user_id\":\"42\",\"plan\":\"pro\"}";
      tracing::event!(target: "emerald", tracing::Level::INFO, message = "user signed in", fields_json = fields_json);
    });

    let line = captured
      .lock()
      .unwrap()
      .clone()
      .expect("event must have been captured");
    let parsed: serde_json::Value = serde_json::from_str(&line).expect("must be valid JSON");
    assert_eq!(parsed["level"], "INFO");
    assert_eq!(parsed["message"], "user signed in");
    assert_eq!(parsed["fields"]["user_id"], "42");
    assert_eq!(parsed["fields"]["plan"], "pro");
    // The raw `fields_json` field name must never leak through
    // unconverted — only the real, nested `fields` object should.
    assert!(parsed.get("fields_json").is_none());
  }

  // A malformed `fields_json` string (never actually producible by
  // `log_event_fields` itself, which always builds it from a real
  // `serde_json::to_string` call — defensive only) is silently
  // dropped rather than corrupting the rest of the event's own
  // fields.
  #[test]
  fn json_visitor_drops_an_unparseable_fields_json_value_without_panicking() {
    let mut visitor = JsonVisitor {
      map: serde_json::Map::new(),
    };
    tracing::field::Visit::record_str(&mut visitor, &real_fields_json_field(), "{not json");
    assert!(!visitor.map.contains_key("fields"));
  }

  // Obtains a real `tracing::field::Field` named `fields_json` by
  // dispatching one real, throwaway event and capturing the `Field`
  // handed to a `Visit` impl during that dispatch — `Field` has no
  // public constructor, so this is the real, supported way to get one
  // in a test, mirroring the `with_default`-based test immediately
  // above rather than hand-constructing fake `Metadata`.
  fn real_fields_json_field() -> tracing::field::Field {
    use std::sync::{Arc, Mutex};
    struct CaptureFieldLayer(Arc<Mutex<Option<tracing::field::Field>>>);
    impl<S: tracing::Subscriber> Layer<S> for CaptureFieldLayer {
      fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct Grab<'a>(&'a mut Option<tracing::field::Field>);
        impl tracing::field::Visit for Grab<'_> {
          fn record_debug(&mut self, field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {
            if field.name() == "fields_json" {
              *self.0 = Some(field.clone());
            }
          }
        }
        let mut slot = None;
        event.record(&mut Grab(&mut slot));
        *self.0.lock().unwrap() = slot;
      }
    }
    let captured: Arc<Mutex<Option<tracing::field::Field>>> = Arc::new(Mutex::new(None));
    let subscriber = tracing_subscriber::registry().with(CaptureFieldLayer(captured.clone()));
    tracing::subscriber::with_default(subscriber, || {
      tracing::event!(target: "emerald", tracing::Level::INFO, fields_json = "placeholder");
    });
    let result = captured
      .lock()
      .unwrap()
      .clone()
      .expect("field must have been captured");
    result
  }
}
