//! Plan 127 (INI Configuration Files) — `rust-ini` (crates.io package
//! name `rust-ini`, import/library name `ini`) wrapped as an
//! `IniDocument` resource handle (plan 93's own registry, the same
//! shape `Regex`/`XmlReader` already use), plus a required-`Other`
//! `IniError` typed domain error (plan 195's Typed Domain Errors
//! convention, applied fresh here since this plan is executed after
//! plan 195 landed — unlike plan 118/122's own retrofits, this module
//! never shipped a `Result[T, String]` shape to begin with).
//!
//! `IniError`'s own variant tags, declaration order, matching
//! `emerald-sema`/`emerald-codegen`'s own `ini_error_enum_def`
//! byte-for-byte:
//!   0 Syntax(String) — `ini::ParseError`'s own `Display` (via
//!     `Ini.parse`'s `load_from_str` directly, or `Ini.load`'s
//!     `ini::Error::Parse(ParseError)` variant)
//!   1 Io(String)     — `ini::Error::Io`, `Ini.load`/`.write`-only (a
//!     real filesystem I/O failure `Ini.parse`'s string-only entry
//!     point can never produce)
//!   2 Other(String)  — the required escape hatch (a null/non-UTF-8
//!     input pointer)
//!
//! Named `mod inis` (`#[path = "ini.rs"]`) in `lib.rs`, not `mod ini`
//! — this crate's own `mod ini` would shadow the external `ini` crate
//! this module wraps, the identical collision `csvs`/`tomls`/`urls`/
//! `charset` already hit and disclosed.
//!
//! Two real, disclosed findings from actually running this plan's own
//! Concrete Proof against the real, vendored crate (`rust-ini`
//! 0.21.3), not assumed from this plan's own text:
//!
//! 1. Every real `ini::Ini` — including one this module's own
//!    `ini_parse` produces — always carries an implicit, pre-inserted
//!    EMPTY general (no-section) entry: `impl Default for Ini`'s own
//!    doc comment reads "Creates an ini instance with an empty general
//!    section", and `Parser::parse` itself starts from `Ini::new()`
//!    (i.e. that `Default` impl), not a bare empty map. `Ini.parse(
//!    "[server]\nhost=localhost\nport=8080\n")` therefore reports
//!    `.section_count() == 2` (the empty `""` general section plus
//!    `"server"`), not 1 — a real crate behavior this module passes
//!    through faithfully (`.sections()`'s own real iteration order),
//!    rather than filtering the empty general section out, which
//!    would make `.section_name`'s own index arithmetic disagree with
//!    what a `.section_count()` call just reported.
//! 2. `ini::Properties::get`'s own doc comment reads "Get the first
//!    value associate with the key", not last-value-wins as plan
//!    127's own text describes — `IniDocument#get` (backed by
//!    `Ini::get_from`, which calls `Properties::get` directly)
//!    follows the crate's REAL, verified behavior (first-value-wins),
//!    not the plan text's own un-verified claim.

use crate::handle::{handle_alloc, handle_get_mut};
use std::ffi::c_void;
use std::os::raw::c_char;

const TAG: &str = "IniDocument";

const INI_ERROR_TAG_SYNTAX: i32 = 0;
const INI_ERROR_TAG_IO: i32 = 1;
const INI_ERROR_TAG_OTHER: i32 = 2;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `""` (Emerald's own global-section convention, per this plan's own
/// Decision log) maps to `None` — `ini::Ini::with_section(None::
/// <String>)`'s own real convention for the general section; any
/// other string maps to `Some(that string)`.
fn section_arg(section: &str) -> Option<&str> {
  if section.is_empty() {
    None
  } else {
    Some(section)
  }
}

unsafe fn alloc_option_string(v: Option<&str>) -> *mut c_void {
  const OPTION_SOME: i64 = 0;
  const OPTION_NONE: i64 = 1;
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

/// `Ini.parse(s: String): Result[IniDocument, IniError]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn ini_parse(s: *const c_char) -> *mut c_void {
  let text = match read_str(s) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_OTHER, &e),
  };
  match ini::Ini::load_from_str(text) {
    Ok(doc) => {
      let id = handle_alloc(Box::new(doc), TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(e) => crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_SYNTAX, &e.to_string()),
  }
}

/// `Ini.load(path: String): Result[IniDocument, IniError]`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn ini_load(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_OTHER, &e),
  };
  match ini::Ini::load_from_file(path) {
    Ok(doc) => {
      let id = handle_alloc(Box::new(doc), TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(ini::Error::Io(e)) => {
      crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_IO, &e.to_string())
    }
    Err(ini::Error::Parse(e)) => {
      crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_SYNTAX, &e.to_string())
    }
  }
}

/// `IniDocument.new(): IniDocument` — a fresh document, never wrapped
/// in `Result` (matching `ConfigBuilder.new`/`LogFields.new`'s own
/// shape, not `Regex.compile`'s — nothing about constructing an empty
/// handle can fail). Already carries a real, empty general section,
/// per this module's own doc comment above (`ini::Ini::new`'s own
/// real behavior, not this module's own choice).
pub fn ini_new() -> i64 {
  handle_alloc(Box::new(ini::Ini::new()), TAG)
}

/// `.section_count(self): Int64`.
pub fn ini_section_count(id: i64) -> i64 {
  match handle_get_mut::<ini::Ini, i64>(id, TAG, |doc| doc.sections().count() as i64) {
    Ok(n) => n,
    Err(msg) => unsafe { crate::raise_native_error(&msg) },
  }
}

/// `.section_name(self, i: Int64): String` — the empty string for the
/// general (no-section) entry, matching `Ini.parse`'s own `.get`
/// convention. `i` out of range raises — the same "programmer error,
/// not a Result" treatment this crate's own `Array[T]` indexing
/// already gives an out-of-bounds index.
///
/// # Safety
/// Always safe to call for a live `IniDocument` handle.
pub unsafe fn ini_section_name(id: i64, i: i64) -> *const c_char {
  let result = handle_get_mut::<ini::Ini, Option<String>>(id, TAG, |doc| {
    let idx = usize::try_from(i).ok()?;
    doc.sections().nth(idx).map(|s| s.unwrap_or("").to_string())
  });
  match result {
    Ok(Some(name)) => crate::alloc_and_copy_str(&name),
    Ok(None) => {
      crate::raise_native_error(&format!("IniDocument#section_name: index {i} out of range"))
    }
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.key_count(self, section: String): Int64` — `0` for a section this
/// document does not contain (an expected, ordinary case — a caller
/// probing before it knows a section exists — unlike `.section_name`'s
/// out-of-range numeric index above, which is a real programmer
/// error).
///
/// # Safety
/// `section`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn ini_key_count(id: i64, section: *const c_char) -> i64 {
  let section = match read_str(section) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ini::Ini, i64>(id, TAG, |doc| {
    doc
      .section(section_arg(section))
      .map(|p| p.len() as i64)
      .unwrap_or(0)
  }) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.key_at(self, section: String, i: Int64): String` — the `i`-th
/// UNIQUE key in `section`'s own declaration order (this plan's own
/// Decision log: duplicate keys are never exposed as a multimap, so
/// `key_at`'s own valid index range is exactly `0..key_count`, the
/// same unique-key count `Properties::len` already reports — never a
/// duplicate-key section's raw, possibly-larger per-VALUE entry count
/// `Properties::iter` would otherwise yield).
///
/// # Safety
/// `section`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn ini_key_at(id: i64, section: *const c_char, i: i64) -> *const c_char {
  let section = match read_str(section) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<ini::Ini, Option<String>>(id, TAG, |doc| {
    let props = doc.section(section_arg(section))?;
    let mut seen = std::collections::HashSet::new();
    let mut keys: Vec<&str> = Vec::new();
    for (k, _) in props.iter() {
      if seen.insert(k) {
        keys.push(k);
      }
    }
    let idx = usize::try_from(i).ok()?;
    keys.get(idx).map(|s| (*s).to_string())
  });
  match result {
    Ok(Some(key)) => crate::alloc_and_copy_str(&key),
    Ok(None) => crate::raise_native_error(&format!(
      "IniDocument#key_at: no key at index {i} in section `{section}`"
    )),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.get(self, section: String, key: String): Option[String]` — the
/// crate's own real `Properties::get` behavior, first-value-wins for a
/// duplicated key (see this module's own doc comment above for why
/// this differs from plan 127's own text).
///
/// # Safety
/// `section`/`key`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn ini_get(id: i64, section: *const c_char, key: *const c_char) -> *mut c_void {
  let section = match read_str(section) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ini::Ini, Option<String>>(id, TAG, |doc| {
    doc.get_from(section_arg(section), key).map(str::to_string)
  }) {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.set(self, section: String, key: String, value: String): Void` —
/// a real, disclosed exception to the ordinary newtype `.value`-only
/// rule, the same carved-out shape `LogFields#set`/`Regex`'s own
/// instance methods already establish.
///
/// # Safety
/// `section`/`key`/`value`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn ini_set(
  id: i64,
  section: *const c_char,
  key: *const c_char,
  value: *const c_char,
) -> i64 {
  let section = match read_str(section) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  let value = match read_str(value) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ini::Ini, ()>(id, TAG, |doc| {
    doc
      .with_section(section_arg(section).map(str::to_string))
      .set(key, value);
  }) {
    Ok(()) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.write(self, path: String): Result[Void, IniError]` — `Err` is
/// always tagged `Io` (`ini::Ini::write_to_file`'s own real return
/// type, `io::Result<()>`), never `Syntax` — writing cannot fail to
/// parse anything. A closed/unknown handle raises rather than
/// producing an `Err`, matching every other method above (a bad
/// handle is a programmer error, not a domain-level write failure).
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn ini_write(id: i64, path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_OTHER, &e),
  };
  match handle_get_mut::<ini::Ini, std::io::Result<()>>(id, TAG, |doc| doc.write_to_file(path)) {
    Ok(Ok(())) => crate::emerald_rt_result_ok(0),
    Ok(Err(e)) => crate::emerald_rt_result_err_tagged_str(INI_ERROR_TAG_IO, &e.to_string()),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.to_string(self): String` — serializes via `Ini::write_to`'s own
/// real writer-based API into an in-memory buffer (this format has no
/// `Display`/`.to_string()` impl on `Ini` itself).
///
/// # Safety
/// Always safe to call for a live `IniDocument` handle.
pub unsafe fn ini_to_string(id: i64) -> *const c_char {
  let result = handle_get_mut::<ini::Ini, std::io::Result<Vec<u8>>>(id, TAG, |doc| {
    let mut buf = Vec::new();
    doc.write_to(&mut buf)?;
    Ok(buf)
  });
  match result {
    Ok(Ok(bytes)) => crate::alloc_and_copy_str(&String::from_utf8_lossy(&bytes)),
    Ok(Err(e)) => crate::raise_native_error(&format!("IniDocument#to_string: {e}")),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  const SAMPLE: &str = "[server]\nhost=localhost\nport=8080\n\n[client]\ntimeout=30\n";

  unsafe fn parse_ok(input: &str) -> i64 {
    let cstr = c(input);
    let result_ptr = ini_parse(cstr.as_ptr()) as *const i64;
    assert_eq!(*result_ptr, 0, "expected Ok for input: {input}");
    *result_ptr.add(1)
  }

  #[test]
  fn parse_reports_the_real_section_count_including_the_implicit_general_section() {
    unsafe {
      let id = parse_ok(SAMPLE);
      // Real, disclosed crate behavior (this module's own doc comment
      // above): the implicit empty `""` general section + `server` +
      // `client` = 3, not 2.
      assert_eq!(ini_section_count(id), 3);
    }
  }

  #[test]
  fn section_name_reports_the_general_section_as_an_empty_string() {
    unsafe {
      let id = parse_ok(SAMPLE);
      assert_eq!(
        std::ffi::CStr::from_ptr(ini_section_name(id, 0))
          .to_str()
          .unwrap(),
        ""
      );
      assert_eq!(
        std::ffi::CStr::from_ptr(ini_section_name(id, 1))
          .to_str()
          .unwrap(),
        "server"
      );
      assert_eq!(
        std::ffi::CStr::from_ptr(ini_section_name(id, 2))
          .to_str()
          .unwrap(),
        "client"
      );
    }
  }

  #[test]
  fn key_count_and_get_read_back_a_real_section() {
    unsafe {
      let id = parse_ok(SAMPLE);
      let section = c("server");
      assert_eq!(ini_key_count(id, section.as_ptr()), 2);
      let host_key = c("host");
      let host = ini_get(id, section.as_ptr(), host_key.as_ptr()) as *const i64;
      assert_eq!(*host, 0, "expected Some");
      let s_ptr = *(host.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap(),
        "localhost"
      );
    }
  }

  #[test]
  fn get_of_a_missing_key_is_a_real_none_not_a_crash() {
    unsafe {
      let id = parse_ok(SAMPLE);
      let section = c("server");
      let missing = c("nope");
      let result = ini_get(id, section.as_ptr(), missing.as_ptr()) as *const i64;
      assert_eq!(*result, 1, "expected None");
    }
  }

  #[test]
  fn key_count_of_a_missing_section_is_zero_not_an_error() {
    unsafe {
      let id = parse_ok(SAMPLE);
      let missing = c("nope");
      assert_eq!(ini_key_count(id, missing.as_ptr()), 0);
    }
  }

  #[test]
  fn key_at_returns_keys_in_declaration_order() {
    unsafe {
      let id = parse_ok(SAMPLE);
      let section = c("server");
      let first = ini_key_at(id, section.as_ptr(), 0);
      assert_eq!(std::ffi::CStr::from_ptr(first).to_str().unwrap(), "host");
      let second = ini_key_at(id, section.as_ptr(), 1);
      assert_eq!(std::ffi::CStr::from_ptr(second).to_str().unwrap(), "port");
    }
  }

  #[test]
  fn parse_of_an_unterminated_section_header_returns_a_real_syntax_err_not_a_panic() {
    unsafe {
      let bad = c("[unterminated\nkey=value\n");
      let result_ptr = ini_parse(bad.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err");
      let err_block = *(result_ptr.add(1)) as *const i64;
      let tag = *err_block;
      assert_eq!(tag, i64::from(INI_ERROR_TAG_SYNTAX));
      let msg_ptr = *(err_block.add(1) as *const *const c_char);
      assert!(!std::ffi::CStr::from_ptr(msg_ptr)
        .to_str()
        .unwrap()
        .is_empty());
    }
  }

  #[test]
  fn load_of_a_nonexistent_file_returns_a_real_io_err_not_a_panic() {
    unsafe {
      let path = c("/nonexistent/path/that/does/not/exist/emerald_ini_test.ini");
      let result_ptr = ini_load(path.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err");
      let err_block = *(result_ptr.add(1)) as *const i64;
      let tag = *err_block;
      assert_eq!(tag, i64::from(INI_ERROR_TAG_IO));
    }
  }

  #[test]
  fn write_then_reparse_round_trips_a_document_through_a_real_file() {
    unsafe {
      let id = ini_new();
      let section = c("app");
      let key = c("name");
      let value = c("demo");
      ini_set(id, section.as_ptr(), key.as_ptr(), value.as_ptr());
      let tmp = std::env::temp_dir().join(format!(
        "emerald_ini_test_{}_{}.ini",
        std::process::id(),
        id
      ));
      let path = CString::new(tmp.to_str().unwrap()).unwrap();
      let write_result = ini_write(id, path.as_ptr()) as *const i64;
      assert_eq!(*write_result, 0, "expected Ok");
      let reloaded = ini_load(path.as_ptr()) as *const i64;
      assert_eq!(*reloaded, 0, "expected Ok");
      let reloaded_id = *reloaded.add(1);
      let got = ini_get(reloaded_id, section.as_ptr(), key.as_ptr()) as *const i64;
      assert_eq!(*got, 0, "expected Some");
      let s_ptr = *(got.add(1) as *const *const c_char);
      assert_eq!(std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap(), "demo");
      std::fs::remove_file(&tmp).ok();
    }
  }

  #[test]
  fn to_string_round_trips_a_set_value_without_touching_disk() {
    unsafe {
      let id = ini_new();
      let section = c("app");
      let key = c("name");
      let value = c("demo");
      ini_set(id, section.as_ptr(), key.as_ptr(), value.as_ptr());
      let s = ini_to_string(id);
      let text = std::ffi::CStr::from_ptr(s).to_str().unwrap();
      assert!(text.contains("[app]"));
      assert!(text.contains("name"));
      assert!(text.contains("demo"));
    }
  }
}
