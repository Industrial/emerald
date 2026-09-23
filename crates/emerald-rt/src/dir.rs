//! Plan 144 (Extended Filesystem Operations), `Dir` half — non-
//! recursive listing (`std::fs::read_dir`) and recursive traversal
//! (`walkdir::WalkDir`), both returning an `Array[String]` of full,
//! path-joined entries (never bare file names) — the same shape
//! `std::fs::DirEntry::path()`/`walkdir::DirEntry::path()` already
//! hand back natively, so no extra join step is invented here.
//!
//! `Array[String]`'s own real, current wire layout — `[len: i64]
//! [elem: *const c_char, ...]`, `emerald_alloc`-backed — is built
//! directly in Rust by `build_string_array` below, the identical
//! layout `regex.rs`'s own `build_string_array` already establishes
//! for `Regex#find_all`/`#split` (copied here, not re-derived, per
//! this codebase's own "reused verbatim" convention for this exact
//! buffer shape). `.entries_count`/`.walk_count` are plan 45's own
//! forced companion-count workaround for `Array[T]` having no
//! self-describing length at the TYPE level from Emerald's own
//! perspective (`String#split`/`#split_count`'s precedent) — each
//! independently re-scans the directory rather than sharing one scan
//! with `.entries`/`.walk`, the same disclosed inefficiency plan 45
//! already accepted for `String#split_count`.
//!
//! Error handling: this plan's own Decision log explicitly reserves
//! the new `Result[T, PathError]`-typed, plan-195 tagged-error
//! treatment for exactly the two operations it names as recoverable-
//! failure (`Path.metadata`/`Path.read_link`) — `Dir.entries`/`.walk`
//! (and their `_count` companions) are never named there, and the
//! plan's own Concrete Proof types them as bare `Array[String]`/
//! `Int64`, never `Result[...]`/nullable. A directory that doesn't
//! exist (or isn't a directory) is therefore a `raise_native_error`
//! (a real, catchable Emerald `NativeError` exception via the real
//! setjmp/longjmp `emerald_raise` boundary — NOT a hard process exit
//! the way `File.read`/`.write`'s own C-runtime implementation
//! aborts) rather than a typed `Result`, matching `Decimal.add`'s own
//! "genuine misuse, not an anticipated Result-worthy input" precedent
//! for this specific operation. Not unit-tested directly in this
//! module's own `#[cfg(test)]` block, matching `decimal.rs`'s/
//! `datetime.rs`'s own established precedent: `raise_native_error`
//! calls through to `emerald_raise`, whose `#[cfg(test)]` stub (`lib.
//! rs`'s own `test_stubs` module) deliberately `std::process::abort()`s
//! rather than unwind — a real, working Rust-level substitute for a
//! genuine `longjmp` does not exist, so this path is verified only via
//! a real `.em` example run through the real CLI, never from inside
//! `cargo test`.

use std::ffi::c_void;
use std::os::raw::c_char;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// Builds a real `Array[String]` value — `[len: i64][elem: *const
/// c_char, ...]`, `emerald_alloc`-backed — the identical layout
/// `regex.rs`'s own `build_string_array` already establishes.
///
/// # Safety
/// Always safe to call.
unsafe fn build_string_array(items: &[String]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, s) in items.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(s);
  }
  ptr as *mut c_void
}

/// `Dir.entries(path: String): Array[String]` — non-recursive
/// `std::fs::read_dir`, one entry per direct child, each as its own
/// full joined path (e.g. `"plan144_demo/a.txt"`), matching `std::fs::
/// DirEntry::path()`'s own native return shape.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn dir_entries(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let read_dir = match std::fs::read_dir(path) {
    Ok(rd) => rd,
    Err(e) => crate::raise_native_error(&format!("Dir.entries: {e}")),
  };
  let mut items: Vec<String> = Vec::new();
  for entry in read_dir {
    match entry {
      Ok(e) => items.push(e.path().to_string_lossy().into_owned()),
      Err(e) => crate::raise_native_error(&format!("Dir.entries: {e}")),
    }
  }
  build_string_array(&items)
}

/// `Dir.entries_count(path: String): Int64` — an independent second
/// `std::fs::read_dir` scan, see this module's own doc comment.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn dir_entries_count(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let read_dir = match std::fs::read_dir(path) {
    Ok(rd) => rd,
    Err(e) => crate::raise_native_error(&format!("Dir.entries_count: {e}")),
  };
  let mut count: i64 = 0;
  for entry in read_dir {
    match entry {
      Ok(_) => count += 1,
      Err(e) => crate::raise_native_error(&format!("Dir.entries_count: {e}")),
    }
  }
  count
}

/// `Dir.walk(path: String): Array[String]` — recursive `walkdir::
/// WalkDir`, cycle-safe on symlinked loops by default (see this
/// crate's own module doc / plan 144's Decision log). `.min_depth(1)`
/// excludes the root `path` itself from the results — `walkdir`
/// yields the root as its own first entry by default, which would
/// otherwise make this plan's own Concrete Proof's expected count of
/// 3 (not 4) wrong.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn dir_walk(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let mut items: Vec<String> = Vec::new();
  for entry in walkdir::WalkDir::new(path).min_depth(1) {
    match entry {
      Ok(e) => items.push(e.path().to_string_lossy().into_owned()),
      Err(e) => crate::raise_native_error(&format!("Dir.walk: {e}")),
    }
  }
  build_string_array(&items)
}

/// `Dir.walk_count(path: String): Int64` — an independent second
/// `walkdir::WalkDir` scan, see this module's own doc comment.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn dir_walk_count(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let mut count: i64 = 0;
  for entry in walkdir::WalkDir::new(path).min_depth(1) {
    match entry {
      Ok(_) => count += 1,
      Err(e) => crate::raise_native_error(&format!("Dir.walk_count: {e}")),
    }
  }
  count
}

#[cfg(test)]
mod tests {
  use super::*;

  unsafe fn read_string_array(ptr: *const i64) -> Vec<String> {
    let len = *ptr;
    let elems = ptr.add(1) as *const *const c_char;
    (0..len)
      .map(|i| {
        let p = *elems.add(i as usize);
        std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
      })
      .collect()
  }

  fn fixture_tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("nested").join("b.txt"), "world").unwrap();
    dir
  }

  #[test]
  fn entries_lists_only_direct_children() {
    unsafe {
      let dir = fixture_tree();
      let path = std::ffi::CString::new(dir.path().to_str().unwrap()).unwrap();
      let n = dir_entries_count(path.as_ptr());
      assert_eq!(n, 2, "a.txt + nested, non-recursive");
      let arr = dir_entries(path.as_ptr()) as *const i64;
      let items = read_string_array(arr);
      assert_eq!(items.len(), 2);
      assert!(items.iter().any(|s| s.ends_with("a.txt")));
      assert!(items.iter().any(|s| s.ends_with("nested")));
      assert!(!items.iter().any(|s| s.ends_with("b.txt")));
    }
  }

  #[test]
  fn walk_visits_every_file_in_the_fixture_tree() {
    unsafe {
      let dir = fixture_tree();
      let path = std::ffi::CString::new(dir.path().to_str().unwrap()).unwrap();
      let n = dir_walk_count(path.as_ptr());
      assert_eq!(n, 3, "a.txt + nested + nested/b.txt, recursive");
      let arr = dir_walk(path.as_ptr()) as *const i64;
      let items = read_string_array(arr);
      assert_eq!(items.len(), 3);
      assert!(items.iter().any(|s| s.ends_with("a.txt")));
      assert!(items.iter().any(|s| s.ends_with("nested")));
      let b_suffix = format!("nested{}b.txt", std::path::MAIN_SEPARATOR);
      assert!(items.iter().any(|s| s.ends_with(&b_suffix)));
    }
  }
}
