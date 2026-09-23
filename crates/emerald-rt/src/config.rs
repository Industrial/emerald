//! Plan 183 (Layered Configuration Loading) — `ConfigBuilder`/
//! `ConfigValue`, wrapping the `config` crate (chosen over `figment`
//! this session on a real, verified publish-date freshness signal —
//! see this plan's own Decision log). Two sibling plan-93 resource-
//! handle newtypes, backed by `crate::handle`'s registry directly (the
//! same shape `CliParser`/`CliParseResult` already use) — `ConfigBuilder`
//! holds a `config::ConfigBuilder<config::builder::DefaultState>` being
//! fed sources in a fixed, load-bearing call order; `ConfigValue` holds
//! this crate's own already-merged, already-frozen `config::Config`
//! output, produced once by `.build` and never mutated again.
//!
//! Named `configs`, not `config`, in `lib.rs`'s own `#[path]`
//! declaration — this crate's own `mod config` would shadow the
//! external `config` crate this module wraps, the identical collision
//! `csvs`/`tomls`/`urls` already hit and disclosed (see each of those
//! modules' own doc comments).
//!
//! Precedence order (CLI flag > environment variable > config file >
//! defaults file) is never a separate priority-number system this
//! module invents — it falls directly out of `config`'s own documented
//! last-registered-source-wins merge semantics, combined with this
//! module's own fixed method-call order (`add_defaults_file` →
//! `add_config_file` → `add_env_prefix` → `add_cli_overrides`, each
//! `add_source` call layering strictly after the previous one). See
//! this plan's own Decision log for the full reasoning.
//!
//! `ConfigBuilder`'s own builder methods (`config::ConfigBuilder`'s
//! `add_source`/`set_override`) consume `self` by value and return a
//! new value, exactly the shape `cli.rs`'s own `Command` wrapping
//! already established — mutated in place behind a stable handle via
//! `crate::handle::handle_get_mut` plus a `std::mem::take`-based swap.
//! Unlike `Command` (which has no meaningful "empty" value to swap
//! with, hence that module's own `Command::new("")` placeholder),
//! `config::ConfigBuilder<DefaultState>` derives a real `Default`, so
//! the swap here is a plain `std::mem::take`, no placeholder needed.
//!
//! `add_cli_overrides` never hands `config` a `clap::ArgMatches` to
//! parse a second time — it reads plan 182's already-parsed
//! `CliParseResult` values directly, by calling `cli::cliparseresult_
//! value` (already `pub`, already the exact per-key lookup this bridge
//! needs) and decoding its own already-built `Option[String]` block in
//! place, then layers any `Some(v)` as a `set_override`. Plan 182's own
//! parse step therefore runs exactly once per program; this module is
//! strictly a consumer of its output, never a second parser of `ARGV`.
//!
//! `.get_string`/`.get_int`/`.get_bool` collapse a genuinely-missing
//! key and a present-but-wrong-shape key to the same `None` — a real,
//! disclosed simplification, not an oversight (see this plan's own
//! Decision log): `config::Config::get::<T>`'s own `Result<T,
//! ConfigError>` distinguishes the two, but `Option[T]` is this
//! module's whole external contract, so both collapse to `Ok(None)`
//! at this module's own boundary, never surfaced as a `NativeError`
//! (a malformed VALUE is an ordinary, expected outcome a caller
//! queries for, not a native panic or a build-time failure — only a
//! malformed SOURCE, caught by `.build()` itself, raises).

use crate::handle::{handle_alloc, handle_get_mut};
use config::builder::DefaultState;
use config::{Config, ConfigBuilder as RawConfigBuilder, Environment, File};
use std::ffi::c_void;
use std::os::raw::c_char;

const BUILDER_TAG: &str = "ConfigBuilder";
const VALUE_TAG: &str = "ConfigValue";
const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

type Builder = RawConfigBuilder<DefaultState>;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

unsafe fn alloc_option_string(v: Option<&str>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(s) => {
      *ptr = OPTION_SOME;
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

unsafe fn alloc_option_i64(v: Option<i64>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(i) => {
      *ptr = OPTION_SOME;
      *ptr.add(1) = i;
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

unsafe fn alloc_option_bool(v: Option<bool>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(b) => {
      *ptr = OPTION_SOME;
      *ptr.add(1) = i64::from(b);
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

/// `ConfigBuilder.new(): ConfigBuilder`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn configbuilder_new() -> i64 {
  handle_alloc(Box::new(Config::builder()), BUILDER_TAG)
}

/// Mutates the stored builder in place, per this module's own doc
/// comment on `config::ConfigBuilder`'s consume-and-return shape.
unsafe fn mutate_builder(id: i64, f: impl FnOnce(Builder) -> Builder) {
  match handle_get_mut::<Builder, ()>(id, BUILDER_TAG, |slot| {
    let cb = std::mem::take(slot);
    *slot = f(cb);
  }) {
    Ok(()) => {}
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.add_defaults_file(self, path: String): Void` — lowest precedence,
/// `.required(false)`: a missing defaults file is tolerated, never a
/// build-time failure (see this plan's own Decision log).
///
/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `path`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn configbuilder_add_defaults_file(id: i64, path: *const c_char) {
  let path = match read_str(path) {
    Ok(p) => p.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_builder(id, move |cb| {
    cb.add_source(File::with_name(&path).required(false))
  });
}

/// `.add_config_file(self, path: String): Void` — the program's real
/// config file, `.required(true)` (`config::File::with_name`'s own
/// default): a missing config file is a real, surfaced `.build()`-
/// time failure, unlike `.add_defaults_file` above.
///
/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `path`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn configbuilder_add_config_file(id: i64, path: *const c_char) {
  let path = match read_str(path) {
    Ok(p) => p.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_builder(id, move |cb| cb.add_source(File::with_name(&path)));
}

/// `.add_env_prefix(self, prefix: String): Void` — `config::
/// Environment::with_prefix(prefix).separator("__")`, the crate's own
/// documented convention for mapping `PREFIX__SECTION__KEY`
/// environment variables onto nested config keys.
///
/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `prefix`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn configbuilder_add_env_prefix(id: i64, prefix: *const c_char) {
  let prefix = match read_str(prefix) {
    Ok(p) => p.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_builder(id, move |cb| {
    cb.add_source(Environment::with_prefix(&prefix).separator("__"))
  });
}

/// `.add_cli_overrides(self, result: CliParseResult, keys:
/// Array[String], count: Int64): Void` — walks `keys`, reading each
/// one from `result` via `cli::cliparseresult_value` and layering any
/// `Some(v)` as the highest-precedence source via `set_override`.
///
/// A real, disclosed adaptation found only by actually running this
/// plan's own Concrete Proof: a `CliParser` flag's own `long` name
/// (`.option("port", ...)`, matched against a bare `--port` on the
/// command line) and a `ConfigValue` key (`"server.port"`, matched
/// against a nested TOML/YAML table) are two different namespaces —
/// a CLI tool's own flags stay short and flat by convention, while a
/// config schema is free to nest. Calling `cli::cliparseresult_value`
/// with the FULL dotted key verbatim (`"server.port"`) would ask clap
/// for an arg id it was never given — a real, non-`None`-returning
/// PANIC (`clap`'s own `ArgMatches::get_one` treats an unregistered
/// id as a programmer error, not a routine "not supplied" outcome),
/// not this module's own `NativeError`/`None` channels. So: only the
/// key's own LAST dot-separated segment (`"port"` from `"server.
/// port"`, or the whole key when it has no dot) is looked up against
/// `result`; the FULL key is what `set_override` still layers into
/// `ConfigValue`, unchanged — exactly reproducing this plan's own
/// Concrete Proof (`parser.option("port", ...)` + `override_keys =
/// ["server.port"]` + `./app --port 7070` → `cfg.get_string("server.
/// port") == Some("7070")`).
///
/// # Safety
/// `id` must be a live `ConfigBuilder` handle. `result` must be a live
/// `CliParseResult` handle. `keys` must point to a buffer of at least
/// `count` valid, NUL-terminated C string pointers — `emerald-
/// codegen`'s own call-site codegen guarantees this (the identical
/// `Array[String]` header-skipping convention `CliParser.parse`'s own
/// `argv` already relies on); when `count` is `0`, `keys` is never
/// dereferenced and may be null.
pub unsafe fn configbuilder_add_cli_overrides(
  id: i64,
  result_id: i64,
  keys: *const *const c_char,
  count: i64,
) {
  let mut overrides: Vec<(String, String)> = Vec::with_capacity(count.max(0) as usize);
  for i in 0..count {
    let key_ptr = *keys.add(i as usize);
    let key = match read_str(key_ptr) {
      Ok(k) => k.to_string(),
      Err(e) => crate::raise_native_error(&e),
    };
    // The CLI flag's own `long` name is the key's LAST dot segment —
    // see this function's own doc comment.
    let cli_name = match key.rsplit_once('.') {
      Some((_, tail)) => tail.to_string(),
      None => key.clone(),
    };
    let cli_name_c = match std::ffi::CString::new(cli_name) {
      Ok(c) => c,
      Err(_) => crate::raise_native_error(
        "ConfigBuilder.add_cli_overrides: key contains an embedded NUL byte",
      ),
    };
    // `cli::cliparseresult_value` already raises a real `NativeError`
    // (via `crate::raise_native_error`) on an unknown/closed `result`
    // handle — nothing extra needed here for that case.
    let value_block =
      crate::cli::cliparseresult_value(result_id, cli_name_c.as_ptr()) as *const i64;
    if *value_block == OPTION_SOME {
      let s_ptr = *(value_block.add(1) as *const *const c_char);
      let value = std::ffi::CStr::from_ptr(s_ptr)
        .to_string_lossy()
        .into_owned();
      overrides.push((key, value));
    }
  }
  match handle_get_mut::<Builder, Result<(), String>>(id, BUILDER_TAG, |slot| {
    let mut cb = std::mem::take(slot);
    let mut err = None;
    for (k, v) in &overrides {
      // Clones before consuming: `set_override` takes `self` by
      // value, and a failed call must not leave `cb` itself moved-out
      // — the un-consumed clone is what gets written back to `slot`
      // on the `Err` branch below, preserving every override applied
      // by an earlier, successful iteration of this same loop.
      match cb.clone().set_override(k.clone(), v.clone()) {
        Ok(next) => cb = next,
        Err(e) => {
          err = Some(format!("ConfigBuilder.add_cli_overrides: key `{k}`: {e}"));
          break;
        }
      }
    }
    *slot = cb;
    match err {
      Some(e) => Err(e),
      None => Ok(()),
    }
  }) {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.build(self): ConfigValue` — every registered source is actually
/// read here (`config`'s own builder methods do no I/O themselves, per
/// this module's own doc comment); a malformed source (invalid TOML/
/// YAML syntax, a required config file genuinely missing) becomes a
/// real `NativeError` carrying `config`'s own formatted message, never
/// a panic or a silently empty `ConfigValue`.
///
/// # Safety
/// `id` must be a live `ConfigBuilder` handle.
pub unsafe fn configbuilder_build(id: i64) -> i64 {
  let cb = match handle_get_mut::<Builder, Builder>(id, BUILDER_TAG, |slot| slot.clone()) {
    Ok(cb) => cb,
    Err(e) => crate::raise_native_error(&e),
  };
  match cb.build() {
    Ok(cfg) => handle_alloc(Box::new(cfg), VALUE_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `.get_string(self, key: String): Option[String]` — dotted keys
/// (`"server.port"`) work exactly as `config`'s own nested-key
/// addressing already supports, with zero special-casing here.
///
/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
pub unsafe fn configvalue_get_string(id: i64, key: *const c_char) -> *mut c_void {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<Config, Option<String>>(id, VALUE_TAG, |cfg| cfg.get_string(key).ok()) {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.get_int(self, key: String): Option[Int64]`.
///
/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
pub unsafe fn configvalue_get_int(id: i64, key: *const c_char) -> *mut c_void {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<Config, Option<i64>>(id, VALUE_TAG, |cfg| cfg.get_int(key).ok()) {
    Ok(v) => alloc_option_i64(v),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.get_bool(self, key: String): Option[Boolean]`.
///
/// # Safety
/// `id` must be a live `ConfigValue` handle. `key`, if non-null, must
/// point to a valid, NUL-terminated C string.
pub unsafe fn configvalue_get_bool(id: i64, key: *const c_char) -> *mut c_void {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<Config, Option<bool>>(id, VALUE_TAG, |cfg| cfg.get_bool(key).ok()) {
    Ok(v) => alloc_option_bool(v),
    Err(e) => crate::raise_native_error(&e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn opt_i64(ptr: *const i64) -> Option<i64> {
    if *ptr == OPTION_SOME {
      Some(*ptr.add(1))
    } else {
      None
    }
  }

  unsafe fn opt_bool(ptr: *const i64) -> Option<bool> {
    opt_i64(ptr).map(|i| i != 0)
  }

  unsafe fn opt_string(ptr: *const i64) -> Option<String> {
    if *ptr == OPTION_SOME {
      let s_ptr = *(ptr.add(1) as *const *const c_char);
      Some(
        std::ffi::CStr::from_ptr(s_ptr)
          .to_str()
          .unwrap()
          .to_string(),
      )
    } else {
      None
    }
  }

  /// A fresh, unique-per-test absolute base path (no extension) inside
  /// a real temp directory — `config::File::with_name` resolves a
  /// relative path against `env::current_dir()`, which a parallel test
  /// suite cannot safely mutate process-wide; an absolute path sidesteps
  /// that entirely, per `config::file::mod.rs`'s own `find_file`.
  fn fixture_base(name: &str, contents: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-rt-config-test-{name}-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let toml_path = dir.join("app.toml");
    std::fs::write(&toml_path, contents).unwrap();
    dir.join("app")
  }

  #[test]
  fn file_only_resolution_reads_every_key_straight_from_the_config_file() {
    unsafe {
      let base = fixture_base(
        "file_only",
        "[server]\nport = 8080\nhost = \"0.0.0.0\"\ndebug = true\n",
      );
      let builder_id = configbuilder_new();
      let path = c(base.to_str().unwrap());
      configbuilder_add_config_file(builder_id, path.as_ptr());
      let value_id = configbuilder_build(builder_id);

      let port = configvalue_get_int(value_id, c("server.port").as_ptr()) as *const i64;
      assert_eq!(opt_i64(port), Some(8080));

      let host = configvalue_get_string(value_id, c("server.host").as_ptr()) as *const i64;
      assert_eq!(opt_string(host), Some("0.0.0.0".to_string()));

      let debug = configvalue_get_bool(value_id, c("server.debug").as_ptr()) as *const i64;
      assert_eq!(opt_bool(debug), Some(true));
    }
  }

  #[test]
  fn an_environment_variable_overrides_a_config_file_value_at_the_same_key() {
    unsafe {
      let base = fixture_base(
        "env_over_file",
        "[server]\nport = 8080\nhost = \"0.0.0.0\"\n",
      );
      let prefix = format!(
        "EMERALD_RT_CFG_TEST_ENV_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_nanos()
      );
      let env_key = format!("{prefix}__SERVER__PORT");
      std::env::set_var(&env_key, "9091");

      let builder_id = configbuilder_new();
      let path = c(base.to_str().unwrap());
      configbuilder_add_config_file(builder_id, path.as_ptr());
      configbuilder_add_env_prefix(builder_id, c(&prefix).as_ptr());
      let value_id = configbuilder_build(builder_id);

      let port = configvalue_get_int(value_id, c("server.port").as_ptr()) as *const i64;
      assert_eq!(
        opt_i64(port),
        Some(9091),
        "env var should override the file value"
      );

      let host = configvalue_get_string(value_id, c("server.host").as_ptr()) as *const i64;
      assert_eq!(
        opt_string(host),
        Some("0.0.0.0".to_string()),
        "an untouched key should keep the file's own value"
      );

      std::env::remove_var(&env_key);
    }
  }

  #[test]
  fn a_cli_override_wins_over_both_the_environment_variable_and_the_config_file() {
    unsafe {
      let base = fixture_base(
        "cli_over_all",
        "[server]\nport = 8080\nhost = \"0.0.0.0\"\n",
      );
      let prefix = format!(
        "EMERALD_RT_CFG_TEST_CLI_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_nanos()
      );
      let env_key = format!("{prefix}__SERVER__PORT");
      std::env::set_var(&env_key, "9091");

      // Real plan-182 `CliParser`/`CliParseResult` handles, not a
      // hand-built stand-in — proving this module's own bridge reads
      // an actual parsed CLI result, per this plan's own Decision log.
      let name = c("app");
      let version = c("1.0.0");
      let parser_id = crate::cli::cliparser_new(name.as_ptr(), version.as_ptr());
      let (long, short, help) = (c("port"), c("p"), c("server port override"));
      crate::cli::cliparser_option(parser_id, long.as_ptr(), short.as_ptr(), help.as_ptr(), 0);
      let args = [c("--port"), c("7070")];
      let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
      let result_id = crate::cli::cliparser_parse(parser_id, argv.as_ptr(), argv.len() as i64);

      let builder_id = configbuilder_new();
      let path = c(base.to_str().unwrap());
      configbuilder_add_config_file(builder_id, path.as_ptr());
      configbuilder_add_env_prefix(builder_id, c(&prefix).as_ptr());
      let keys = [c("server.port")];
      let keys_ptr: Vec<*const c_char> = keys.iter().map(|k| k.as_ptr()).collect();
      configbuilder_add_cli_overrides(
        builder_id,
        result_id,
        keys_ptr.as_ptr(),
        keys_ptr.len() as i64,
      );
      let value_id = configbuilder_build(builder_id);

      let port = configvalue_get_string(value_id, c("server.port").as_ptr()) as *const i64;
      assert_eq!(
        opt_string(port),
        Some("7070".to_string()),
        "the CLI flag should win over both the env var and the file"
      );

      let host = configvalue_get_string(value_id, c("server.host").as_ptr()) as *const i64;
      assert_eq!(
        opt_string(host),
        Some("0.0.0.0".to_string()),
        "a key untouched by env or CLI should keep the file's own value"
      );

      std::env::remove_var(&env_key);
      crate::cli::cliparser_close(parser_id);
      crate::cli::cliparseresult_close(result_id);
    }
  }

  #[test]
  fn a_missing_key_produces_none_rather_than_a_panic() {
    unsafe {
      let base = fixture_base("missing_key", "[server]\nport = 8080\n");
      let builder_id = configbuilder_new();
      let path = c(base.to_str().unwrap());
      configbuilder_add_config_file(builder_id, path.as_ptr());
      let value_id = configbuilder_build(builder_id);

      let missing =
        configvalue_get_string(value_id, c("server.does_not_exist").as_ptr()) as *const i64;
      assert_eq!(opt_string(missing), None);

      let missing_int = configvalue_get_int(value_id, c("nowhere.at_all").as_ptr()) as *const i64;
      assert_eq!(opt_i64(missing_int), None);
    }
  }

  #[test]
  fn a_missing_optional_defaults_file_is_tolerated_not_a_build_failure() {
    unsafe {
      let dir = std::env::temp_dir().join(format!(
        "emerald-rt-config-test-no-defaults-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap()
          .as_nanos()
      ));
      std::fs::create_dir_all(&dir).unwrap();
      let base = dir.join("does_not_exist");

      let builder_id = configbuilder_new();
      let path = c(base.to_str().unwrap());
      configbuilder_add_defaults_file(builder_id, path.as_ptr());
      // Never raises, even though the defaults file is genuinely
      // absent from disk.
      let value_id = configbuilder_build(builder_id);
      let missing = configvalue_get_string(value_id, c("anything").as_ptr()) as *const i64;
      assert_eq!(opt_string(missing), None);
    }
  }
}
