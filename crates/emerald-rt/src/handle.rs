// Plan 93 (Resource Handle & Lifetime Model for Native Objects) — the
// four Rust-internal primitives every future resource-holding domain
// plan (a TCP stream, a DB connection, a TLS session, a compression
// stream) builds its own typed wrapper on top of. None of these four
// are `extern "C"`/`#[no_mangle]` themselves (they're generic, and
// Emerald source never calls them directly) — only a domain plan's own
// concrete `emerald_rt_<module>_<fn>` exports (this plan's own
// `emerald_rt_handle_counter_*` trio below, proving the mechanism) are.
//
// Representation: an opaque `i64` handle, never a raw pointer smuggled
// through Emerald as an `Int64` — Emerald code can copy/store/compare
// the integer freely, but only this module's own registry can ever
// turn it into a live reference, giving every access a real validation
// point (a closed or unknown handle raises a `NativeError`, plan 92's
// exception channel, never UB and never a silent no-op).
//
// `closed: bool` on a still-present `RegistryEntry`, rather than an
// immediate `HashMap::remove` on close, is what makes "use of a
// CLOSED handle" and "use of a handle this process never issued"
// distinguishable diagnostics at all — removing the entry outright
// would make both cases see the same missing key.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

struct RegistryEntry {
  value: Option<Box<dyn Any + Send>>,
  type_tag: &'static str,
}

fn registry() -> &'static Mutex<HashMap<u64, RegistryEntry>> {
  static REGISTRY: OnceLock<Mutex<HashMap<u64, RegistryEntry>>> = OnceLock::new();
  REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_id() -> u64 {
  static COUNTER: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
  COUNTER
    .get_or_init(|| std::sync::atomic::AtomicU64::new(1))
    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// Issues a fresh, never-reused-within-this-process id for `value`,
// tagged `type_tag` (used only in diagnostics — see `handle_get`/
// `handle_get_mut` below). Cast to `i64` since Emerald has no unsigned
// integer type (plan 59's own finding); a real, disclosed cost this
// pays in exchange: the top bit of a real `u64` id is unreachable
// (see plan 93's own "Not yet decided" item 1 — wraparound is
// considered unreachable in any real process's lifetime, not formally
// proven).
pub fn handle_alloc(value: Box<dyn Any + Send>, type_tag: &'static str) -> i64 {
  let id = next_id();
  registry().lock().unwrap().insert(
    id,
    RegistryEntry {
      value: Some(value),
      type_tag,
    },
  );
  id as i64
}

// Every domain-plan-facing accessor built on this module raises
// through this exact message shape — "use of closed"/"unknown handle"
// — per `leaf-use-after-close-diagnostic`.
fn handle_error_message(id: i64, type_tag: &str, entry: Option<&RegistryEntry>) -> String {
  match entry {
    Some(e) if e.value.is_none() => format!("use of closed {type_tag} handle"),
    Some(e) => format!(
      "{type_tag} handle {id} holds a different resource type ({})",
      e.type_tag
    ),
    None => format!("unknown {type_tag} handle: this process never issued id {id}"),
  }
}

// Looks up `id`, downcasting to `&T`. `Err` carries a ready-to-raise
// message (via `crate::raise_native_error`, plan 92's channel) rather
// than a typed error enum — every call site immediately raises on
// `Err` and never inspects the failure any other way.
//
// Not yet called by any `emerald_rt_*` export in this crate (this
// plan's own `Counter` proof only ever mutates, via `handle_get_mut`
// below) — kept regardless, per plan 93's own `leaf-registry-and-
// handle-type` mandate that both accessors are real primitives future
// domain plans build on, not something this plan's own proof alone
// must exercise.
#[allow(dead_code)]
pub fn handle_get<T: 'static + Copy>(id: i64, type_tag: &'static str) -> Result<T, String> {
  let reg = registry().lock().unwrap();
  let entry = reg.get(&(id as u64));
  match entry.and_then(|e| e.value.as_ref()) {
    Some(v) => match v.downcast_ref::<T>() {
      Some(t) => Ok(*t),
      None => Err(handle_error_message(id, type_tag, entry)),
    },
    None => Err(handle_error_message(id, type_tag, entry)),
  }
}

// `handle_get`'s mutable-access sibling — `f` runs with `&mut T` while
// the registry's own lock is held, so `f` must be quick and must not
// re-enter this module (a real, disclosed constraint: no reentrancy
// guard is implemented, since no call site in this plan's own proof
// needs one).
pub fn handle_get_mut<T: 'static, R>(
  id: i64,
  type_tag: &'static str,
  f: impl FnOnce(&mut T) -> R,
) -> Result<R, String> {
  let mut reg = registry().lock().unwrap();
  let type_tag_for_err = type_tag;
  let id_for_err = id;
  let entry = reg.get_mut(&(id as u64));
  let msg =
    |entry: Option<&RegistryEntry>| handle_error_message(id_for_err, type_tag_for_err, entry);
  match entry {
    Some(e) => {
      let is_closed = e.value.is_none();
      if is_closed {
        return Err(msg(Some(e)));
      }
      let stored_tag = e.type_tag;
      match e.value.as_mut().and_then(|v| v.downcast_mut::<T>()) {
        Some(t) => Ok(f(t)),
        None => Err(format!(
          "{type_tag} handle {id} holds a different resource type ({stored_tag})"
        )),
      }
    }
    None => Err(msg(None)),
  }
}

// Marks `id` closed, dropping its boxed value in place (so no actual
// resource stays held open), and returns whether it was actually open
// beforehand. Never raises — double-close and closing an unknown id
// are both a harmless `false`, per `leaf-double-close-is-a-noop-not-
// an-error`; only a subsequent *use* raises.
pub fn handle_close(id: i64) -> bool {
  let mut reg = registry().lock().unwrap();
  match reg.get_mut(&(id as u64)) {
    Some(entry) => entry.value.take().is_some(),
    None => false,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn alloc_then_get_round_trips_the_stored_value() {
    let id = handle_alloc(Box::new(42i64), "test");
    let v: i64 = handle_get(id, "test").unwrap();
    assert_eq!(v, 42);
  }

  #[test]
  fn get_mut_sees_and_can_mutate_the_stored_value() {
    let id = handle_alloc(Box::new(0i64), "counter");
    let first = handle_get_mut::<i64, _>(id, "counter", |v| {
      *v += 1;
      *v
    })
    .unwrap();
    assert_eq!(first, 1);
    let second = handle_get_mut::<i64, _>(id, "counter", |v| {
      *v += 1;
      *v
    })
    .unwrap();
    assert_eq!(second, 2);
  }

  #[test]
  fn close_on_an_open_handle_returns_true_and_the_handle_becomes_closed() {
    let id = handle_alloc(Box::new(0i64), "counter");
    assert!(handle_close(id));
    let err = handle_get::<i64>(id, "counter").unwrap_err();
    assert_eq!(err, "use of closed counter handle");
  }

  #[test]
  fn double_close_is_a_harmless_no_op_returning_false() {
    let id = handle_alloc(Box::new(0i64), "counter");
    assert!(handle_close(id));
    assert!(!handle_close(id));
  }

  #[test]
  fn use_of_an_unknown_id_names_the_process_never_issuing_it() {
    let err = handle_get::<i64>(999_999_999, "counter").unwrap_err();
    assert_eq!(
      err,
      "unknown counter handle: this process never issued id 999999999"
    );
  }

  #[test]
  fn the_registry_is_safe_under_real_concurrent_access_from_multiple_threads() {
    let id = handle_alloc(Box::new(0i64), "concurrent");
    let handles: Vec<_> = (0..8)
      .map(|_| {
        std::thread::spawn(move || {
          for _ in 0..1000 {
            handle_get_mut::<i64, _>(id, "concurrent", |v| *v += 1).unwrap();
          }
        })
      })
      .collect();
    for h in handles {
      h.join().unwrap();
    }
    let v: i64 = handle_get(id, "concurrent").unwrap();
    assert_eq!(v, 8000);
  }
}
