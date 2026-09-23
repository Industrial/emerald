//! Plan 144 (Extended Filesystem Operations), `Path`/`FileMetadata`
//! half — existence/kind predicates, metadata snapshotting, and
//! Unix permission/symlink operations, all backed by plain `std::fs`/
//! `std::os::unix::fs` (zero third-party dependency for this half —
//! see this crate's own `dir.rs` module for the one third-party
//! dependency this whole plan adds, `walkdir`, which this module
//! never touches).
//!
//! `FileMetadata` — a compiler-synthesized `Type::Class` (the
//! identical "packed fields, not a `crate::handle`" shape `decimal.
//! rs`'s own module doc already establishes for `Decimal`/`DateTime`,
//! reused here rather than plan 144's own original, now-superseded
//! text proposing a dedicated new `ValKind::FileMetadata` — no such
//! mechanism exists anywhere else in this codebase, and every sibling
//! compiler-synthesized value type landed since this plan was
//! authored (`Decimal`, `DateTime`/`ZonedDateTime`) uses this simpler,
//! already-proven "opaque heap pointer + hardcoded dispatch arm"
//! shape instead) — is a heap pointer to a packed `[size: i64]
//! [modified_unix: i64][is_dir: i64][is_file: i64][readonly: i64]`
//! block (40 bytes; boolean fields cross as 0/1, narrowed back to a
//! real `i1` by `emerald-codegen`'s own call site, the same direction
//! `Regex#is_match` already narrows). Built once, at `Path.metadata`'s
//! own call site, from a real `std::fs::Metadata` snapshot — never
//! re-queried, and never itself the target of another native call
//! (its own five accessor methods just read the packed fields back
//! out).
//!
//! Error handling (plan 195's Typed Domain Errors convention):
//! `PathError` classifies exactly the two `std::io::ErrorKind`
//! variants that are both real and genuinely common for the two
//! operations this plan's own Decision log names as recoverable-
//! failure (`Path.metadata`/`Path.read_link`) — `NotFound` (the path
//! doesn't exist) and `PermissionDenied` — folding every other
//! `io::Error` (a platform-specific or rare kind, e.g. `NotADirectory`
//! on a component of the path) into `Other(String)`, the same
//! "classify what's real, fold the rest" rule `DecimalError`/
//! `RegexError` already established. This plan's own Decision log
//! originally specified a raw nullable-pointer return (`FileMetadata?`
//! / `String?`) for these two operations — dead syntax: `T?`/`nil`
//! were removed outright by plan 73, well before plan 195 existed, in
//! favor of a real `Option[T]` ADT — so this plan adopts plan 195's
//! newer, strictly more informative `Result[T, PathError]` shape
//! instead of trying to resurrect a syntax the grammar no longer has,
//! consistent with this task's own instruction to apply the
//! freshly-landed error convention to new fallible native operations.
//!
//! `Path.exists`/`.is_file`/`.is_dir`/`.is_symlink` are NOT fallible —
//! `std::path::Path`'s own inherent methods of the same name already
//! return a bare `bool`, folding every error (including "doesn't
//! exist") into `false` at the `std` layer itself, so there is no
//! error to classify or propagate. `Path.unix_mode`/`.set_unix_mode`/
//! `.symlink` keep this plan's own originally-declared, non-`Result`
//! return types (`Int64`/`Void`/`Void`) verbatim — the plan never
//! marks these three as recoverable-failure the way it marks
//! `.metadata`/`.read_link`, so a real OS failure here raises a
//! catchable `NativeError` exception instead (`dir.rs`'s own module
//! doc explains why this is a real exception, not a process exit).

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

// Plan 195 (Typed Domain Errors): `PathError`'s own variant tags —
// see this module's own doc comment for why only these two `io::
// ErrorKind` variants are named individually. Declaration order
// matches `emerald-sema`/`emerald-codegen`'s own mirrored `PathError`
// enum byte-for-byte.
//   0 NotFound
//   1 PermissionDenied
//   2 Other(String)
const PATH_ERROR_TAG_NOT_FOUND: i32 = 0;
const PATH_ERROR_TAG_PERMISSION_DENIED: i32 = 1;
const PATH_ERROR_TAG_OTHER: i32 = 2;

fn classify_io_error(e: &std::io::Error) -> i32 {
  match e.kind() {
    std::io::ErrorKind::NotFound => PATH_ERROR_TAG_NOT_FOUND,
    std::io::ErrorKind::PermissionDenied => PATH_ERROR_TAG_PERMISSION_DENIED,
    _ => PATH_ERROR_TAG_OTHER,
  }
}

/// `Path.exists(path: String): Boolean`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_exists(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  std::path::Path::new(path).exists() as i64
}

/// `Path.is_file(path: String): Boolean`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_is_file(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  std::path::Path::new(path).is_file() as i64
}

/// `Path.is_dir(path: String): Boolean`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_is_dir(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  std::path::Path::new(path).is_dir() as i64
}

/// `Path.is_symlink(path: String): Boolean` — `std::path::Path::
/// is_symlink` (a `symlink_metadata` call, unlike `.exists`/`.is_
/// file`/`.is_dir` which all follow symlinks).
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_is_symlink(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  std::path::Path::new(path).is_symlink() as i64
}

/// `Path.metadata(path: String): Result[FileMetadata, PathError]` —
/// see this module's own doc comment for the packed 40-byte layout
/// and the `T?`-is-dead-syntax reasoning for why this is `Result`,
/// not nullable.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_metadata(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(PATH_ERROR_TAG_OTHER, &e),
  };
  match std::fs::metadata(path) {
    Ok(meta) => {
      let size = meta.len() as i64;
      let modified_unix = match meta.modified() {
        Ok(mtime) => match mtime.duration_since(std::time::UNIX_EPOCH) {
          Ok(d) => d.as_secs() as i64,
          Err(e) => -(e.duration().as_secs() as i64),
        },
        // Not every platform/filesystem tracks a modification time —
        // a real, disclosed `0` sentinel (Unix epoch) rather than
        // failing the whole `.metadata` call over one unavailable
        // field.
        Err(_) => 0,
      };
      let is_dir = meta.is_dir() as i64;
      let is_file = meta.is_file() as i64;
      let readonly = meta.permissions().readonly() as i64;
      let ptr = crate::emerald_alloc(40) as *mut i64;
      *ptr = size;
      *ptr.add(1) = modified_unix;
      *ptr.add(2) = is_dir;
      *ptr.add(3) = is_file;
      *ptr.add(4) = readonly;
      crate::emerald_rt_result_ok(ptr as i64)
    }
    Err(e) => {
      let tag = classify_io_error(&e);
      crate::emerald_rt_result_err_tagged_str(tag, &e.to_string())
    }
  }
}

/// `.size(self): Int64`.
///
/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
pub unsafe fn filemetadata_size(ptr: *const i64) -> i64 {
  *ptr
}

/// `.modified_unix(self): Int64`.
///
/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
pub unsafe fn filemetadata_modified_unix(ptr: *const i64) -> i64 {
  *ptr.add(1)
}

/// `.is_dir(self): Boolean`.
///
/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
pub unsafe fn filemetadata_is_dir(ptr: *const i64) -> i64 {
  *ptr.add(2)
}

/// `.is_file(self): Boolean`.
///
/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
pub unsafe fn filemetadata_is_file(ptr: *const i64) -> i64 {
  *ptr.add(3)
}

/// `.readonly(self): Boolean`.
///
/// # Safety
/// `ptr` must point to a live, 40-byte, `emerald_alloc`-backed
/// `FileMetadata` block.
pub unsafe fn filemetadata_readonly(ptr: *const i64) -> i64 {
  *ptr.add(4)
}

/// `Path.unix_mode(path: String): Int64` — raw `st_mode` bits via
/// `std::os::unix::fs::PermissionsExt::mode`. Unix-only — see plan
/// 144's own Decision log (`#[cfg(unix)]`-gated, matching `std`'s own
/// `PermissionsExt` gating).
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[cfg(unix)]
pub unsafe fn path_unix_mode(path: *const c_char) -> i64 {
  use std::os::unix::fs::PermissionsExt;
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match std::fs::metadata(path) {
    Ok(meta) => meta.permissions().mode() as i64,
    Err(e) => crate::raise_native_error(&format!("Path.unix_mode: {e}")),
  }
}

/// `Path.set_unix_mode(path: String, mode: Int64): Void`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[cfg(unix)]
pub unsafe fn path_set_unix_mode(path: *const c_char, mode: i64) {
  use std::os::unix::fs::PermissionsExt;
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let perms = std::fs::Permissions::from_mode(mode as u32);
  if let Err(e) = std::fs::set_permissions(path, perms) {
    crate::raise_native_error(&format!("Path.set_unix_mode: {e}"));
  }
}

/// `Path.symlink(target: String, link_path: String): Void` —
/// `std::os::unix::fs::symlink`. `target` need not exist (a dangling
/// symlink is a valid, real filesystem state, not an error this
/// function itself checks for).
///
/// # Safety
/// `target`/`link_path`, if non-null, must each point to a valid,
/// NUL-terminated C string.
#[cfg(unix)]
pub unsafe fn path_symlink(target: *const c_char, link_path: *const c_char) {
  let target = match read_str(target) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let link_path = match read_str(link_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  if let Err(e) = std::os::unix::fs::symlink(target, link_path) {
    crate::raise_native_error(&format!("Path.symlink: {e}"));
  }
}

/// `Path.read_link(path: String): Result[String, PathError]` —
/// `std::fs::read_link`. See this module's own doc comment for why
/// this is `Result`, not nullable.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn path_read_link(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(PATH_ERROR_TAG_OTHER, &e),
  };
  match std::fs::read_link(path) {
    Ok(target) => {
      crate::emerald_rt_result_ok(crate::alloc_and_copy_str(&target.to_string_lossy()) as i64)
    }
    Err(e) => {
      let tag = classify_io_error(&e);
      crate::emerald_rt_result_err_tagged_str(tag, &e.to_string())
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn result_ok_payload(ptr: *const i64) -> *const i64 {
    assert_eq!(*ptr, 0, "expected Ok discriminant");
    *(ptr.add(1)) as *const i64
  }

  unsafe fn result_err_tag(ptr: *const i64) -> i64 {
    assert_eq!(*ptr, 1, "expected Err discriminant");
    let err_block = *(ptr.add(1)) as *const i64;
    *err_block
  }

  #[test]
  fn exists_is_file_is_dir_against_a_real_fixture() {
    unsafe {
      let dir = tempfile::tempdir().unwrap();
      let file_path = dir.path().join("a.txt");
      std::fs::write(&file_path, "hello").unwrap();
      let file_c = c(file_path.to_str().unwrap());
      let dir_c = c(dir.path().to_str().unwrap());
      let missing_c = c(dir.path().join("nope.txt").to_str().unwrap());

      assert_eq!(path_exists(file_c.as_ptr()), 1);
      assert_eq!(path_is_file(file_c.as_ptr()), 1);
      assert_eq!(path_is_dir(file_c.as_ptr()), 0);

      assert_eq!(path_exists(dir_c.as_ptr()), 1);
      assert_eq!(path_is_dir(dir_c.as_ptr()), 1);
      assert_eq!(path_is_file(dir_c.as_ptr()), 0);

      assert_eq!(path_exists(missing_c.as_ptr()), 0);
    }
  }

  #[test]
  fn metadata_of_a_real_file_reports_its_real_size_and_kind() {
    unsafe {
      let dir = tempfile::tempdir().unwrap();
      let file_path = dir.path().join("a.txt");
      std::fs::write(&file_path, "hello").unwrap();
      let file_c = c(file_path.to_str().unwrap());

      let result_ptr = path_metadata(file_c.as_ptr()) as *const i64;
      let meta_ptr = result_ok_payload(result_ptr);
      assert_eq!(filemetadata_size(meta_ptr), 5);
      assert_eq!(filemetadata_is_dir(meta_ptr), 0);
      assert_eq!(filemetadata_is_file(meta_ptr), 1);
    }
  }

  #[test]
  fn metadata_of_a_nonexistent_path_is_a_typed_not_found_err() {
    unsafe {
      let dir = tempfile::tempdir().unwrap();
      let missing_c = c(dir.path().join("nope.txt").to_str().unwrap());
      let result_ptr = path_metadata(missing_c.as_ptr()) as *const i64;
      let tag = result_err_tag(result_ptr);
      assert_eq!(tag, PATH_ERROR_TAG_NOT_FOUND as i64);
    }
  }

  #[cfg(unix)]
  #[test]
  fn symlink_and_read_link_round_trip_a_real_target() {
    unsafe {
      let dir = tempfile::tempdir().unwrap();
      let target_path = dir.path().join("a.txt");
      std::fs::write(&target_path, "hello").unwrap();
      let link_path = dir.path().join("a_link.txt");

      let target_c = c(target_path.to_str().unwrap());
      let link_c = c(link_path.to_str().unwrap());
      path_symlink(target_c.as_ptr(), link_c.as_ptr());

      assert_eq!(path_is_symlink(link_c.as_ptr()), 1);

      // `path_read_link`'s own `Ok` payload is directly the `String`
      // pointer itself (`alloc_and_copy_str`'s own return value) —
      // unlike `path_metadata`'s own `Ok` payload, a genuine second-
      // level pointer to a 40-byte struct, there is no further
      // pointer-to-pointer indirection to dereference here.
      let result_ptr = path_read_link(link_c.as_ptr()) as *const i64;
      let s_ptr = result_ok_payload(result_ptr) as *const c_char;
      let s = std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap();
      assert_eq!(s, target_path.to_str().unwrap());
    }
  }

  #[cfg(unix)]
  #[test]
  fn unix_mode_round_trips_through_set_unix_mode() {
    unsafe {
      let dir = tempfile::tempdir().unwrap();
      let file_path = dir.path().join("a.txt");
      std::fs::write(&file_path, "hello").unwrap();
      let file_c = c(file_path.to_str().unwrap());

      path_set_unix_mode(file_c.as_ptr(), 0o600);
      let mode = path_unix_mode(file_c.as_ptr());
      assert_eq!(mode & 0o777, 0o600);
    }
  }
}
