//! Plan 122 (Regular Expressions) — a `Regex` compiler-synthesized
//! `Int64` newtype (the same zero-cost opaque-handle shape plan 93's
//! own `NativeHandle`/plan 168's `LogFields` already use, reusing
//! `crate::handle`'s registry directly rather than inventing a second
//! resource-lifetime mechanism), wrapping `regex::Regex`. Every method
//! is dispatched by `emerald-codegen`'s own `build_method_call`, keyed
//! on the receiver's `local_classes` entry being exactly `"Regex"` —
//! see that dispatch site's own doc comment for why this differs from
//! `Json`/`Log`/`Base64`/`Hex`'s reserved-FREE-FUNCTION-namespace shape
//! (`Regex.compile` IS one of those; every OTHER method here is a real
//! instance method on an already-compiled pattern, needing the
//! receiver's own handle id, which only an ordinary method-call
//! dispatch — not a static namespace call — carries).
//!
//! Array[T]/Option[T] buffers this module hand-builds (`.find_all`,
//! `.captures`, `.split`) use exactly the same byte-for-byte layouts
//! `json.rs`'s own `lower_array`/`alloc_enum_block` already establish
//! and document — `[len: i64][elem: *mut c_void, ...]` for `Array[T]`,
//! `[tag: i64][payload: 8]` for `Option[T]` (`Some` = 0, `None` = 1,
//! plan 73's own fixed encoding, independent of `T`) — reused verbatim,
//! not re-derived.

use crate::handle::{handle_alloc, handle_get_mut};
use std::ffi::c_void;
use std::os::raw::c_char;

const TAG: &str = "Regex";
const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

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

unsafe fn build_string_array(items: &[&str]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, s) in items.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(s);
  }
  ptr as *mut c_void
}

unsafe fn build_option_string_array(items: &[Option<&str>]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *mut c_void;
  for (i, s) in items.iter().enumerate() {
    *elems.add(i) = alloc_option_string(*s);
  }
  ptr as *mut c_void
}

/// `Regex.compile(pattern: String): Result[Regex, String]`.
///
/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn regex_compile(pattern: *const c_char) -> *mut c_void {
  let pattern = match read_str(pattern) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match regex::Regex::new(pattern) {
    Ok(re) => {
      let id = handle_alloc(Box::new(re), TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

/// `.is_match(self, s: String): Boolean` — crosses the FFI boundary as
/// a plain `i64` (0 or 1), narrowed back to a real `i1` by
/// `emerald-codegen`'s own call site, the same direction `Boolean`
/// already narrows the other way for `emerald_bool_to_string`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_is_match(id: i64, s: *const c_char) -> i64 {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, bool>(id, TAG, |re| re.is_match(s)) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.find(self, s: String): Option[String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_find(id: i64, s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, Option<String>>(id, TAG, |re| {
    re.find(s).map(|m| m.as_str().to_string())
  }) {
    Ok(found) => alloc_option_string(found.as_deref()),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.find_all(self, s: String): Array[String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_find_all(id: i64, s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, Vec<String>>(id, TAG, |re| {
    re.find_iter(s).map(|m| m.as_str().to_string()).collect()
  }) {
    Ok(found) => {
      let refs: Vec<&str> = found.iter().map(String::as_str).collect();
      build_string_array(&refs)
    }
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.find_all_count(self, s: String): Int64` — plan 45's own forced
/// `Array[T]`-has-no-length-metadata companion-count workaround
/// (`String.split`/`.split_count`'s precedent), reused verbatim.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_find_all_count(id: i64, s: *const c_char) -> i64 {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, i64>(id, TAG, |re| re.find_iter(s).count() as i64) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.captures(self, s: String): Option[Array[Option[String]]]` —
/// index 0 is always the whole match (`Some`, since a `Captures`
/// value only ever exists when the whole pattern matched); index `i`
/// (`i >= 1`) is capture group `i`, `None` when that group didn't
/// participate in this particular match (e.g. inside an alternation).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_captures(id: i64, s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, Option<Vec<Option<String>>>>(id, TAG, |re| {
    re.captures(s).map(|caps| {
      (0..caps.len())
        .map(|i| caps.get(i).map(|m| m.as_str().to_string()))
        .collect()
    })
  }) {
    Ok(Some(groups)) => {
      let refs: Vec<Option<&str>> = groups.iter().map(|g| g.as_deref()).collect();
      let arr = build_option_string_array(&refs);
      let ptr = crate::emerald_alloc(16) as *mut i64;
      *ptr = OPTION_SOME;
      *(ptr.add(1) as *mut *mut c_void) = arr;
      ptr as *mut c_void
    }
    Ok(None) => {
      let ptr = crate::emerald_alloc(16) as *mut i64;
      *ptr = OPTION_NONE;
      ptr as *mut c_void
    }
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.replace(self, s: String, replacement: String): String` — first
/// match only. `replacement` is passed straight through to `regex`'s
/// own `&str`-as-`Replacer` impl, so `$1`/`${name}` capture-reference
/// syntax is the crate's own, free.
///
/// # Safety
/// `s`/`replacement`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn regex_replace(
  id: i64,
  s: *const c_char,
  replacement: *const c_char,
) -> *const c_char {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let replacement = match read_str(replacement) {
    Ok(r) => r,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, String>(id, TAG, |re| {
    re.replace(s, replacement).into_owned()
  }) {
    Ok(result) => crate::alloc_and_copy_str(&result),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.replace_all(self, s: String, replacement: String): String`.
///
/// # Safety
/// `s`/`replacement`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn regex_replace_all(
  id: i64,
  s: *const c_char,
  replacement: *const c_char,
) -> *const c_char {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let replacement = match read_str(replacement) {
    Ok(r) => r,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, String>(id, TAG, |re| {
    re.replace_all(s, replacement).into_owned()
  }) {
    Ok(result) => crate::alloc_and_copy_str(&result),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.split(self, s: String): Array[String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_split(id: i64, s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, Vec<String>>(id, TAG, |re| {
    re.split(s).map(|p| p.to_string()).collect()
  }) {
    Ok(parts) => {
      let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
      build_string_array(&refs)
    }
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.split_count(self, s: String): Int64`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn regex_split_count(id: i64, s: *const c_char) -> i64 {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<regex::Regex, i64>(id, TAG, |re| re.split(s).count() as i64) {
    Ok(n) => n,
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

  unsafe fn compile_ok(pattern: &str) -> i64 {
    let p = c(pattern);
    let result_ptr = regex_compile(p.as_ptr()) as *const i64;
    assert_eq!(*result_ptr, 0, "expected Ok discriminant");
    *result_ptr.add(1)
  }

  #[test]
  fn compile_of_an_invalid_pattern_is_a_real_err_not_a_panic() {
    unsafe {
      let bad = c("(unclosed");
      let result_ptr = regex_compile(bad.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let msg_ptr = *(result_ptr.add(1) as *const *const c_char);
      assert!(!std::ffi::CStr::from_ptr(msg_ptr)
        .to_str()
        .unwrap()
        .is_empty());
    }
  }

  #[test]
  fn is_match_and_find_against_a_date_pattern() {
    unsafe {
      let id = compile_ok(r"(\d{4})-(\d{2})-(\d{2})");
      let hay = c("shipped on 2026-09-21");
      assert_eq!(regex_is_match(id, hay.as_ptr()), 1);
      let found = regex_find(id, hay.as_ptr()) as *const i64;
      assert_eq!(*found, 0, "expected Some");
      let s_ptr = *(found.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap(),
        "2026-09-21"
      );
    }
  }

  #[test]
  fn captures_group_one_is_the_year() {
    unsafe {
      let id = compile_ok(r"(\d{4})-(\d{2})-(\d{2})");
      let hay = c("shipped on 2026-09-21");
      let caps = regex_captures(id, hay.as_ptr()) as *const i64;
      assert_eq!(*caps, 0, "expected Some");
      let arr = *(caps.add(1) as *const *const i64);
      let len = *arr;
      assert_eq!(len, 4, "whole match + 3 groups");
      let year_slot = *(arr.add(2) as *const *const i64);
      assert_eq!(*year_slot, 0, "group 1 participated");
      let year_ptr = *(year_slot.add(1) as *const *const c_char);
      assert_eq!(std::ffi::CStr::from_ptr(year_ptr).to_str().unwrap(), "2026");
    }
  }

  #[test]
  fn replace_all_reformats_every_date_via_capture_references() {
    unsafe {
      let id = compile_ok(r"(\d{4})-(\d{2})-(\d{2})");
      let hay = c("2026-09-21 and 2026-01-08");
      let repl = c("$3/$2/$1");
      let result = regex_replace_all(id, hay.as_ptr(), repl.as_ptr());
      assert_eq!(
        std::ffi::CStr::from_ptr(result).to_str().unwrap(),
        "21/09/2026 and 08/01/2026"
      );
    }
  }

  #[test]
  fn split_on_a_comma_pattern() {
    unsafe {
      let id = compile_ok(r",\s*");
      let hay = c("a, b,c");
      let result = regex_split(id, hay.as_ptr()) as *const i64;
      assert_eq!(*result, 3);
      let elems = result.add(1) as *const *const c_char;
      assert_eq!(std::ffi::CStr::from_ptr(*elems).to_str().unwrap(), "a");
      assert_eq!(
        std::ffi::CStr::from_ptr(*elems.add(1)).to_str().unwrap(),
        "b"
      );
      assert_eq!(
        std::ffi::CStr::from_ptr(*elems.add(2)).to_str().unwrap(),
        "c"
      );
    }
  }
}
