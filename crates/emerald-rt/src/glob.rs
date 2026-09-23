//! Plan 150 (Path Globbing) — `Glob.glob`/`.glob_count` (the plan's
//! own literal `Glob.match`/`.match_count` is unparseable: `match` is
//! a grammar-reserved keyword, confirmed by actually compiling this
//! plan's own worked example — the same category of collision plan
//! 109's own `Sha256.new()` already hit, resolved there by renaming
//! rather than further widening `CallMethodName`; this plan follows
//! that same precedent and renames to `Glob.glob`/`.glob_count`,
//! mirroring the `glob` crate's own `glob::glob` function name), a
//! thin, pattern-aware layer on top of plan 144's plain `Dir.walk`
//! enumeration, backed by the `glob` crate (the `rust-lang`-owned,
//! long-standing choice for exactly this — see `crates/emerald-rt/
//! Cargo.toml`'s own ledger comment). Unlike `Dir.walk`, which always
//! descends every directory unconditionally and leaves filtering to
//! Emerald source, `glob::glob`'s own matching walk descends only
//! directories a pattern's static prefix could still match — real
//! shell-style `*`/`?`/`[...]`/`**` syntax, not a new dialect this
//! module defines: an Emerald `Glob.glob` pattern behaves identically
//! to the same string passed to `glob::glob` directly.
//!
//! `Array[String]`'s own real, current wire layout — `[len: i64]
//! [elem: *const c_char, ...]`, `emerald_alloc`-backed — is built by
//! `build_string_array` below, the identical layout `dir.rs`'s own
//! copy already establishes (copied here, not re-derived, per this
//! codebase's own "reused verbatim" convention for this exact buffer
//! shape). `.match_count` is plan 45's own forced companion-count
//! workaround for `Array[T]` having no self-describing length at the
//! TYPE level from Emerald's own perspective, an independent second
//! `glob::glob` scan rather than sharing one scan with `.match` — the
//! same disclosed inefficiency `Dir.entries`/`.entries_count` already
//! accept.
//!
//! Results are sorted lexicographically by this module's own code
//! before crossing the FFI boundary — a deliberate correction of the
//! underlying crate's own unspecified (real, OS-directory-read-order-
//! dependent) iteration order, not an accident of whatever order the
//! filesystem happens to hand entries back in. See this plan's own
//! Concrete Proof, which writes `b.txt` before `a.txt` and asserts the
//! returned order is still `a.txt` before `b.txt`.
//!
//! Error handling: a directory `glob` cannot read mid-walk
//! (permission denied, a race where a directory is removed mid-scan)
//! is silently skipped, not surfaced as a partial-failure error — the
//! same "collect what you can, don't build new machinery for a rare
//! partial-failure case" posture plan 45 already took, and plan 144's
//! own `Dir.entries`/`.walk` do NOT take (they raise on the first
//! `Err` entry) — a real, disclosed divergence from those two
//! siblings, deliberately chosen here because `glob`'s own `GlobError`
//! most commonly names one specific unreadable subdirectory within a
//! pattern's scan, not the whole match's own root path being invalid
//! the way `Dir.entries`/`.walk`'s failures usually are. A malformed
//! *pattern* itself (e.g. an unclosed `[`), which `glob::glob` reports
//! immediately as a `PatternError` before any real filesystem access
//! happens, is a genuine misuse — the plan's own Concrete Proof types
//! `Glob.glob`/`.glob_count` as bare `Array[String]`/`Int64`, never
//! `Result[...]`, so this is a `raise_native_error` (a real, catchable
//! `NativeError`), matching `Dir.walk`'s own "genuine misuse, not an
//! anticipated `Result`-worthy input" precedent, not plan 195's newer
//! `Result[T, GlobError]` convention.

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
/// `dir.rs`'s own `build_string_array` already establishes.
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

/// Runs `glob::glob(pattern)`, collecting every `Ok(PathBuf)` result
/// into a `Vec<String>` — silently skipping `Err` entries (see this
/// module's own doc comment) — then sorting the result lexically
/// before returning. A malformed pattern itself raises a catchable
/// `NativeError` immediately, before any real filesystem access.
unsafe fn run_glob(pattern: &str) -> Vec<String> {
  let paths = match glob::glob(pattern) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&format!("Glob.glob: {e}")),
  };
  let mut items: Vec<String> = Vec::new();
  for path in paths.flatten() {
    items.push(path.to_string_lossy().into_owned());
  }
  items.sort();
  items
}

/// `Glob.glob(pattern: String): Array[String]`.
///
/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn glob_match(pattern: *const c_char) -> *mut c_void {
  let pattern = match read_str(pattern) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let items = run_glob(pattern);
  build_string_array(&items)
}

/// `Glob.glob_count(pattern: String): Int64` — an independent second
/// `glob::glob` scan, see this module's own doc comment.
///
/// # Safety
/// `pattern`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn glob_match_count(pattern: *const c_char) -> i64 {
  let pattern = match read_str(pattern) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  run_glob(pattern).len() as i64
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
    // Deliberately non-alphabetical write order — `b.txt` before
    // `a.txt` — the same falsifiable proof this plan's own Concrete
    // Proof depends on: the returned order must still be sorted,
    // never an accident of directory-read order.
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("nested").join("c.txt"), "c").unwrap();
    std::fs::write(dir.path().join("nested").join("d.rs"), "d").unwrap();
    dir
  }

  #[test]
  fn single_star_matches_only_top_level_files_not_subdirectories() {
    unsafe {
      let dir = fixture_tree();
      let pattern = format!("{}/*.txt", dir.path().to_str().unwrap());
      let pattern_c = std::ffi::CString::new(pattern.clone()).unwrap();

      let n = glob_match_count(pattern_c.as_ptr());
      assert_eq!(n, 2, "a.txt + b.txt, not nested/c.txt");

      let arr = glob_match(pattern_c.as_ptr()) as *const i64;
      let items = read_string_array(arr);
      assert_eq!(items.len(), 2);
      assert!(items.iter().any(|s| s.ends_with("a.txt")));
      assert!(items.iter().any(|s| s.ends_with("b.txt")));
      assert!(!items.iter().any(|s| s.contains("nested")));
    }
  }

  #[test]
  fn double_star_reaches_files_at_every_depth() {
    unsafe {
      let dir = fixture_tree();
      let pattern = format!("{}/**/*.rs", dir.path().to_str().unwrap());
      let pattern_c = std::ffi::CString::new(pattern).unwrap();

      let n = glob_match_count(pattern_c.as_ptr());
      assert_eq!(n, 1, "only nested/d.rs");

      let arr = glob_match(pattern_c.as_ptr()) as *const i64;
      let items = read_string_array(arr);
      assert_eq!(items.len(), 1);
      let d_suffix = format!("nested{}d.rs", std::path::MAIN_SEPARATOR);
      assert!(items[0].ends_with(&d_suffix));
    }
  }

  #[test]
  fn results_are_returned_sorted_regardless_of_write_order() {
    unsafe {
      let dir = fixture_tree();
      let pattern = format!("{}/*.txt", dir.path().to_str().unwrap());
      let pattern_c = std::ffi::CString::new(pattern).unwrap();

      let arr = glob_match(pattern_c.as_ptr()) as *const i64;
      let items = read_string_array(arr);
      assert_eq!(items.len(), 2);
      assert!(
        items[0].ends_with("a.txt") && items[1].ends_with("b.txt"),
        "expected sorted [a.txt, b.txt], got {items:?}"
      );
    }
  }
}
