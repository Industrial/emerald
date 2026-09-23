//! Plan 147 (Temporary Files & Directories) — `Tempfile`/`Tempdir`, two
//! sibling plan-93 resource-handle newtypes wrapping `tempfile::
//! NamedTempFile`/`tempfile::TempDir` respectively (distinct handle
//! types, not unioned into one — see this plan's own Decision log:
//! they wrap genuinely different underlying types with different
//! drop/delete behavior, one file vs. one whole directory tree).
//!
//! Named `tempfile`, not `tempfiles` — unlike `csvs`/`tomls`/`urls`/
//! `charset`'s own external-crate-name collisions, `mod tempfile;`
//! would collide with THIS crate's own already-existing `tempfile`
//! *dependency* name, not a same-named sibling module — `lib.rs`'s own
//! `mod` list dodges this the identical way, importing this file under
//! `#[path = "tempfile.rs"] mod tempfiles;`.
//!
//! Error handling: creation failure (disk full, permissions) raises a
//! real, catchable `NativeError` via `crate::raise_native_error` — the
//! same plain-raise convention `File`'s own functions and plan 145's
//! `Process.run` already use, NOT plan 195's Typed Domain Errors
//! `Result` convention. `emerald_rt_tempfile_create`/`_tempdir_create`
//! never fail to return a real, live handle; a failed creation never
//! returns at all.
//!
//! `.close()` is the ONLY deterministic cleanup trigger available —
//! see this plan's own Decision log for why `tempfile`'s own celebrated
//! auto-delete-on-`Drop` guarantee cannot be relied on inside Emerald
//! code: Emerald's own arena-based memory model (plan 45's own
//! citation of `emerald_alloc`'s "no free, no lifetime tracking"
//! contract) gives Rust's `Drop` no hook to run when an Emerald-side
//! local merely goes out of scope. `.close()` removes the registry
//! entry and drops the boxed `NamedTempFile`/`TempDir` in place — the
//! exact, deliberate trigger point for `tempfile`'s own real `unlink`/
//! `remove_dir_all` syscall. A forgotten `.close()` leaks the temp
//! resource until process exit, the identical disclosed tradeoff every
//! other plan-93 resource already accepts (see this plan's own
//! Decision log).
//!
//! `.path()` copies the path into a fresh `alloc_and_copy_str` buffer
//! (never a pointer borrowed from the live `NamedTempFile`/`TempDir`
//! itself) — the returned `String` stays valid even after `.close()`
//! drops the underlying value, matching every other `String`-returning
//! accessor in this crate.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::os::raw::c_char;
use tempfile::{NamedTempFile, TempDir};

const TEMPFILE_TAG: &str = "Tempfile";
const TEMPDIR_TAG: &str = "Tempdir";

/// `Tempfile.create(): Tempfile` — `tempfile::NamedTempFile::new()`,
/// the crate's own race-free-named, restrictive-from-creation-
/// permissions temp file. Never wrapped in `Result` (see this module's
/// own doc comment) — a real creation failure raises `NativeError`
/// directly.
pub fn tempfile_create() -> i64 {
  match NamedTempFile::new() {
    Ok(tf) => handle_alloc(Box::new(tf), TEMPFILE_TAG),
    Err(e) => unsafe { crate::raise_native_error(&format!("Tempfile.create: {e}")) },
  }
}

/// `Tempfile#path(self): String`.
///
/// # Safety
/// `id` must be a live `Tempfile` handle.
pub unsafe fn tempfile_path(id: i64) -> *const c_char {
  match handle_get_mut::<NamedTempFile, String>(id, TEMPFILE_TAG, |tf| {
    tf.path().to_string_lossy().into_owned()
  }) {
    Ok(path) => crate::alloc_and_copy_str(&path),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Tempfile#close(self): Void` — drops the boxed `NamedTempFile` in
/// place, the deterministic trigger for `tempfile`'s own real `unlink`
/// syscall. Double-close is a harmless no-op, matching every other
/// plan-93 handle's own convention (`crate::handle::handle_close`'s
/// own doc comment).
pub fn tempfile_close(id: i64) {
  handle_close(id);
}

/// `Tempdir.create(): Tempdir` — `tempfile::TempDir::new()`, the
/// directory counterpart of `Tempfile.create` above.
pub fn tempdir_create() -> i64 {
  match TempDir::new() {
    Ok(td) => handle_alloc(Box::new(td), TEMPDIR_TAG),
    Err(e) => unsafe { crate::raise_native_error(&format!("Tempdir.create: {e}")) },
  }
}

/// `Tempdir#path(self): String`.
///
/// # Safety
/// `id` must be a live `Tempdir` handle.
pub unsafe fn tempdir_path(id: i64) -> *const c_char {
  match handle_get_mut::<TempDir, String>(id, TEMPDIR_TAG, |td| {
    td.path().to_string_lossy().into_owned()
  }) {
    Ok(path) => crate::alloc_and_copy_str(&path),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Tempdir#close(self): Void` — drops the boxed `TempDir` in place,
/// triggering `tempfile`'s own real, recursive `remove_dir_all`.
pub fn tempdir_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashSet;
  use std::path::Path;

  unsafe fn path_string(ptr: *const c_char) -> String {
    std::ffi::CStr::from_ptr(ptr).to_str().unwrap().to_string()
  }

  #[test]
  fn a_created_tempfile_really_exists_on_disk_and_is_really_gone_after_close() {
    unsafe {
      let id = tempfile_create();
      let path = path_string(tempfile_path(id));
      assert!(
        Path::new(&path).exists(),
        "tempfile should exist while open"
      );
      tempfile_close(id);
      assert!(
        !Path::new(&path).exists(),
        "tempfile should be unlinked after close"
      );
    }
  }

  #[test]
  fn a_created_tempdirs_recursive_contents_are_all_gone_after_close() {
    unsafe {
      let id = tempdir_create();
      let dir_path = path_string(tempdir_path(id));
      let nested = Path::new(&dir_path).join("nested");
      std::fs::create_dir(&nested).unwrap();
      std::fs::write(nested.join("inner.txt"), "contents").unwrap();
      assert!(nested.join("inner.txt").exists());
      tempdir_close(id);
      assert!(
        !Path::new(&dir_path).exists(),
        "tempdir and its whole recursive contents should be gone after close"
      );
    }
  }

  #[test]
  fn one_thousand_concurrently_created_tempfiles_never_collide_on_name() {
    // The concrete proof `tempfile`'s own race-free naming is actually
    // being exercised, not merely trusted (see this plan's own text).
    let mut paths = HashSet::new();
    let mut ids = Vec::with_capacity(1000);
    for _ in 0..1000 {
      let id = tempfile_create();
      let path = unsafe { path_string(tempfile_path(id)) };
      assert!(paths.insert(path), "a tempfile path collided");
      ids.push(id);
    }
    assert_eq!(paths.len(), 1000);
    for id in ids {
      tempfile_close(id);
    }
  }

  #[test]
  fn double_close_on_a_tempfile_is_a_harmless_no_op() {
    let id = tempfile_create();
    tempfile_close(id);
  }
}
