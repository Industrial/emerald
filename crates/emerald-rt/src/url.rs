//! Plan 98 (URL Parsing) — `Url`, wrapping the `url` crate (Servo's
//! WHATWG-Standard-compliant reference implementation, the crate the
//! rest of the Rust web ecosystem already depends on). The shared
//! foundation plans 100/101 (HTTP client/server) both parse a URL
//! through, rather than each hand-rolling string-splitting logic.
//!
//! Real, disclosed note: plan 96 (raw TCP/UDP sockets), which this
//! plan's own text cites for its handle-representation convention,
//! was not executed before this plan — this module instead reuses the
//! identical `Int64`-newtype-over-`crate::handle` shape `Regex`/
//! `AeadKey`/every other compiler-synthesized handle type in this
//! session already establishes, functionally equivalent to what plan
//! 96 would have set up. `Url` values hold no OS resource (no file
//! descriptor, no lock) — per this plan's own Decision log, no
//! `.free`/`.close` method is added; a `Url` handle simply outlives
//! the process, the same as every other never-explicitly-freed value
//! in this compiler today (this crate has no destructor/drop-glue on
//! scope exit for any type, a pre-existing, unrelated gap).

use crate::handle::{handle_alloc, handle_get_mut};
use std::os::raw::c_char;

const URL_TAG: &str = "Url";

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Url.parse(s: String): Url` — raises `url::ParseError`'s own
/// `Display` message (a distinct message per real failure variant,
/// e.g. `RelativeUrlWithoutBase`/`EmptyHost`/`InvalidPort`), not one
/// generic "invalid URL" string.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn url_parse(s: *const c_char) -> i64 {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match url::Url::parse(s) {
    Ok(u) => handle_alloc(Box::new(u), URL_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `Url.build(scheme: String, host: String, path: String): Url` — the
/// fixed `scheme`/`host`/`path` triple, built by constructing
/// `"{scheme}://{host}{path}"` and re-parsing it through the same
/// `url::Url::parse` every other producer in this module uses, rather
/// than hand-assembling the crate's own internal `Url` struct fields.
///
/// # Safety
/// `scheme`/`host`/`path`, if non-null, must point to valid, NUL-
/// terminated C strings.
pub unsafe fn url_build(scheme: *const c_char, host: *const c_char, path: *const c_char) -> i64 {
  let scheme = match read_str(scheme) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let host = match read_str(host) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let path = match read_str(path) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let combined = format!("{scheme}://{host}{path}");
  match url::Url::parse(&combined) {
    Ok(u) => handle_alloc(Box::new(u), URL_TAG),
    Err(e) => crate::raise_native_error(&e.to_string()),
  }
}

/// `Url#scheme(self): String`.
pub fn url_scheme(id: i64) -> *const c_char {
  match handle_get_mut::<url::Url, String>(id, URL_TAG, |u| u.scheme().to_string()) {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#host(self): String` — raises when the URL has no host at all
/// (e.g. `mailto:`/`data:` URLs) rather than collapsing "absent" and
/// "empty" into the same `""` value — see this plan's own Decision
/// log.
pub fn url_host(id: i64) -> *const c_char {
  let result = handle_get_mut::<url::Url, Option<String>>(id, URL_TAG, |u| {
    u.host_str().map(|h| h.to_string())
  });
  match result {
    Ok(Some(h)) => unsafe { crate::alloc_and_copy_str(&h) },
    Ok(None) => unsafe { crate::raise_native_error("Url#host: this URL has no host component") },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#port(self): Int64` — `port_or_known_default()`, the scheme-
/// default-aware accessor, not the raw `port()` (which is legitimately
/// absent even for a well-known scheme when no explicit port was
/// written). Raises only when the URL both omits an explicit port AND
/// the scheme has no known default (a non-standard/custom scheme).
pub fn url_port(id: i64) -> i64 {
  let result = handle_get_mut::<url::Url, Option<u16>>(id, URL_TAG, |u| u.port_or_known_default());
  match result {
    Ok(Some(p)) => p as i64,
    Ok(None) => unsafe {
      crate::raise_native_error("Url#port: no explicit port and this scheme has no known default")
    },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#path(self): String`.
pub fn url_path(id: i64) -> *const c_char {
  match handle_get_mut::<url::Url, String>(id, URL_TAG, |u| u.path().to_string()) {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#query(self): String` — `""`, not raised, when the URL has no
/// query component (unlike `#host`; see this plan's own Decision log
/// for why the two components don't get the same absent-value
/// treatment).
pub fn url_query(id: i64) -> *const c_char {
  let result =
    handle_get_mut::<url::Url, String>(id, URL_TAG, |u| u.query().unwrap_or("").to_string());
  match result {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#fragment(self): String` — `""` on absence, same posture as
/// `#query`.
pub fn url_fragment(id: i64) -> *const c_char {
  let result =
    handle_get_mut::<url::Url, String>(id, URL_TAG, |u| u.fragment().unwrap_or("").to_string());
  match result {
    Ok(s) => unsafe { crate::alloc_and_copy_str(&s) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Url#with_path(self, path: String): Url` — clones the underlying
/// `url::Url`, mutates the clone via the crate's own in-place
/// `set_path`, and stores the clone as a NEW handle, returning it —
/// never mutates the receiver's own handle in place, matching this
/// codebase's project-wide "no method mutates its receiver" convention
/// (plan 45's own Decision log) rather than `url::Url`'s own real
/// in-place API shape.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn url_with_path(id: i64, path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<url::Url, url::Url>(id, URL_TAG, |u| {
    let mut clone = u.clone();
    clone.set_path(path);
    clone
  });
  match result {
    Ok(clone) => handle_alloc(Box::new(clone), URL_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Url#with_query(self, query: String): Url` — same clone-then-set
/// shape as `#with_path`.
///
/// # Safety
/// `query`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn url_with_query(id: i64, query: *const c_char) -> i64 {
  let query = match read_str(query) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let result = handle_get_mut::<url::Url, url::Url>(id, URL_TAG, |u| {
    let mut clone = u.clone();
    clone.set_query(Some(query));
    clone
  });
  match result {
    Ok(clone) => handle_alloc(Box::new(clone), URL_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Url#with_port(self, port: Int64): Url` — same clone-then-set
/// shape; raises on a port `url::Url::set_port` itself rejects (e.g.
/// a cannot-be-a-base URL, or a value outside `u16`'s range).
///
/// # Safety
/// Always safe to call for a live `Url` handle.
pub unsafe fn url_with_port(id: i64, port: i64) -> i64 {
  if !(0..=u16::MAX as i64).contains(&port) {
    crate::raise_native_error("Url#with_port: port must be in 0..=65535");
  }
  let result = handle_get_mut::<url::Url, Result<url::Url, ()>>(id, URL_TAG, |u| {
    let mut clone = u.clone();
    clone.set_port(Some(port as u16)).map(|()| clone)
  });
  match result {
    Ok(Ok(clone)) => handle_alloc(Box::new(clone), URL_TAG),
    Ok(Err(())) => crate::raise_native_error(
      "Url#with_port: this URL cannot carry an explicit port (e.g. a cannot-be-a-base URL)",
    ),
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

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  #[test]
  fn parses_every_component_of_a_full_url() {
    unsafe {
      let s = c("https://example.com/search?q=emerald#results");
      let id = url_parse(s.as_ptr());
      assert_eq!(cstr(url_scheme(id)), "https");
      assert_eq!(cstr(url_host(id)), "example.com");
      assert_eq!(url_port(id), 443);
      assert_eq!(cstr(url_path(id)), "/search");
      assert_eq!(cstr(url_query(id)), "q=emerald");
      assert_eq!(cstr(url_fragment(id)), "results");
    }
  }

  #[test]
  fn port_falls_back_to_the_scheme_default_when_absent() {
    unsafe {
      let s = c("https://example.com");
      let id = url_parse(s.as_ptr());
      assert_eq!(url_port(id), 443);
    }
  }

  #[test]
  fn query_and_fragment_are_empty_strings_when_absent_not_raised() {
    unsafe {
      let s = c("https://example.com/x");
      let id = url_parse(s.as_ptr());
      assert_eq!(cstr(url_query(id)), "");
      assert_eq!(cstr(url_fragment(id)), "");
    }
  }

  #[test]
  fn build_and_with_query_round_trip() {
    unsafe {
      let scheme = c("https");
      let host = c("example.com");
      let path = c("/api");
      let id = url_build(scheme.as_ptr(), host.as_ptr(), path.as_ptr());
      let query = c("id=42");
      let q_id = url_with_query(id, query.as_ptr());
      assert_eq!(cstr(url_path(q_id)), "/api");
      assert_eq!(cstr(url_query(q_id)), "id=42");
    }
  }

  #[test]
  fn with_port_returns_a_new_handle_leaving_the_original_untouched() {
    unsafe {
      let s = c("https://example.com");
      let id = url_parse(s.as_ptr());
      let new_id = url_with_port(id, 8443);
      assert_eq!(url_port(new_id), 8443);
      assert_eq!(url_port(id), 443, "original handle must be unmodified");
    }
  }

  // A genuine parse failure must raise a real, distinct message, not
  // panic — not re-verified by an in-process unit test here (this
  // crate's `raise_native_error` ultimately calls the real
  // `emerald_raise` C export, stubbed in test builds to
  // `std::process::abort()` — see `test_stubs`'s own doc comment in
  // `lib.rs`); the real, end-to-end behavior is verified via a real
  // `.em` example run through the real CLI instead.
}
