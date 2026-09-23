//! Plan 133 (Zip Archives) — `zip::ZipWriter`/`zip::ZipArchive` wrapped
//! two ways: a one-shot `Zip.create`/`.extract` pair (`create` writes a
//! fresh archive from an explicit, caller-ordered `Array[String]` of
//! file paths, every entry `CompressionMethod::Deflated` per this
//! plan's own Decision log; `extract` is the crate's own one-call
//! bulk-unpack), and a `ZipReader` plan-93 resource-handle for
//! listing/reading entries by index.
//!
//! **A real, structural difference from plan 132's `TarReader`, not an
//! API inconsistency**: a `.zip` file's central directory sits at the
//! end of the archive and lists every entry (name/size/offset) up
//! front, so `zip::ZipArchive::new` parses it once at open time and
//! `.len()`/`.by_index(i)` are real, cheap, already-available
//! operations — no plan-45-style `.split_count`-alongside-`.split`
//! two-scan workaround is needed here (`entry_count()` is a real count,
//! known immediately), and no eager "read every entry into a `Vec` up
//! front" workaround `TarReader.open` needs either (see that module's
//! own doc comment for why *it* needs one): the live `zip::ZipArchive<
//! File>` itself is simply the handle's own stored value, `.by_index`
//! called fresh on every accessor.

use crate::bytes::bytes_from_slice;
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::fs::File;
use std::io::Read;
use std::os::raw::c_char;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const TAG: &str = "ZipReader";

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// --- One-shot `Zip.create`/`.extract` -------------------------------------

/// `Zip.create(archive_path: String, paths: Array[String]): Void` —
/// `path_ptrs`/`count` are `paths`'s own already-unpacked `Array[
/// String]` header, the identical marshaling shape plan 132's `Tar.
/// create` already established (`Array[T]` carries no runtime length
/// metadata of its own — plan 45's own finding — hence `count`
/// crossing this boundary as an explicit, separate parameter). Archive
/// member order is caller order (`paths`' own order); every entry is
/// written `CompressionMethod::Deflated`, per this plan's own Decision
/// log (no per-entry `Stored` choice exposed in v1).
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string. `path_ptrs` must point to `count` valid `*const c_char`
/// entries, each itself a valid, NUL-terminated C string.
pub unsafe fn zip_create(archive_path: *const c_char, path_ptrs: *const *const c_char, count: i64) {
  let archive_path = match read_str(archive_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let file = match File::create(archive_path) {
    Ok(f) => f,
    Err(e) => crate::raise_native_error(&format!("Zip.create: {archive_path}: {e}")),
  };
  let mut writer = ZipWriter::new(file);
  let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
  for i in 0..count {
    let path = match read_str(*path_ptrs.add(i as usize)) {
      Ok(p) => p,
      Err(e) => crate::raise_native_error(&e),
    };
    if let Err(e) = writer.start_file(path, options) {
      crate::raise_native_error(&format!("Zip.create: {path}: {e}"));
    }
    let mut src = match File::open(path) {
      Ok(f) => f,
      Err(e) => crate::raise_native_error(&format!("Zip.create: {path}: {e}")),
    };
    if let Err(e) = std::io::copy(&mut src, &mut writer) {
      crate::raise_native_error(&format!("Zip.create: {path}: {e}"));
    }
  }
  if let Err(e) = writer.finish() {
    crate::raise_native_error(&format!("Zip.create: {archive_path}: {e}"));
  }
}

/// `Zip.extract(archive_path: String, dest_dir: String): Void` — via
/// `zip::ZipArchive::extract`, the crate's own one-call bulk-extract
/// API (creates `dest_dir`, including any missing parents, itself).
///
/// # Safety
/// `archive_path`/`dest_dir`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn zip_extract(archive_path: *const c_char, dest_dir: *const c_char) {
  let archive_path = match read_str(archive_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let dest_dir = match read_str(dest_dir) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let file = match File::open(archive_path) {
    Ok(f) => f,
    Err(e) => crate::raise_native_error(&format!("Zip.extract: {archive_path}: {e}")),
  };
  let mut archive = match ZipArchive::new(file) {
    Ok(a) => a,
    Err(e) => crate::raise_native_error(&format!("Zip.extract: {archive_path}: {e}")),
  };
  if let Err(e) = archive.extract(dest_dir) {
    crate::raise_native_error(&format!("Zip.extract: {e}"));
  }
}

// --- Index-based `ZipReader` handle ---------------------------------------

/// `ZipReader.open(archive_path: String): ZipReader` — the live `zip::
/// ZipArchive<File>` itself is the handle's own stored value (see this
/// module's own doc comment for why no eager per-entry read, unlike
/// `TarReader.open`, is needed here).
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string.
pub unsafe fn zip_reader_open(archive_path: *const c_char) -> i64 {
  let archive_path = match read_str(archive_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let file = match File::open(archive_path) {
    Ok(f) => f,
    Err(e) => crate::raise_native_error(&format!("ZipReader.open: {archive_path}: {e}")),
  };
  match ZipArchive::new(file) {
    Ok(archive) => handle_alloc(Box::new(archive), TAG),
    Err(e) => crate::raise_native_error(&format!("ZipReader.open: {archive_path}: {e}")),
  }
}

/// `.entry_count(self): Int64` — `zip::ZipArchive::len`, a real,
/// already-known-up-front count (this plan's own Decision log — the
/// central directory makes it so, unlike plan 132's `TarReader`, which
/// has no such count without a full scan).
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
pub unsafe fn zip_reader_entry_count(id: i64) -> i64 {
  match handle_get_mut::<ZipArchive<File>, i64>(id, TAG, |a| a.len() as i64) {
    Ok(n) => n,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.entry_name(self, i: Int64): String` — random-access by index, not
/// iterator-advance-only, per this plan's own Decision log.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
pub unsafe fn zip_reader_entry_name(id: i64, index: i64) -> *const c_char {
  let name = handle_get_mut::<ZipArchive<File>, Result<String, String>>(id, TAG, |a| {
    match a.by_index(index as usize) {
      Ok(f) => Ok(f.name().to_string()),
      Err(e) => Err(format!("ZipReader#entry_name: index {index}: {e}")),
    }
  });
  match name {
    Ok(Ok(name)) => crate::alloc_and_copy_str(&name),
    Ok(Err(e)) => crate::raise_native_error(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.entry_size(self, i: Int64): Int64` — the entry's own real,
/// uncompressed byte size (`zip::read::ZipFile::size`).
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
pub unsafe fn zip_reader_entry_size(id: i64, index: i64) -> i64 {
  let size = handle_get_mut::<ZipArchive<File>, Result<i64, String>>(id, TAG, |a| {
    match a.by_index(index as usize) {
      Ok(f) => Ok(f.size() as i64),
      Err(e) => Err(format!("ZipReader#entry_size: index {index}: {e}")),
    }
  });
  match size {
    Ok(Ok(n)) => n,
    Ok(Err(e)) => crate::raise_native_error(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.read_entry_data(self, i: Int64): Bytes` — the entry's own full,
/// decompressed content.
///
/// # Safety
/// `id` must be a live `ZipReader` handle.
pub unsafe fn zip_reader_read_entry_data(id: i64, index: i64) -> i64 {
  let data = handle_get_mut::<ZipArchive<File>, Result<Vec<u8>, String>>(id, TAG, |a| {
    match a.by_index(index as usize) {
      Ok(mut f) => {
        let mut buf = Vec::new();
        match f.read_to_end(&mut buf) {
          Ok(_) => Ok(buf),
          Err(e) => Err(format!("ZipReader#read_entry_data: index {index}: {e}")),
        }
      }
      Err(e) => Err(format!("ZipReader#read_entry_data: index {index}: {e}")),
    }
  });
  match data {
    Ok(Ok(data)) => bytes_from_slice(&data),
    Ok(Err(e)) => crate::raise_native_error(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.close(self): Void`.
pub fn zip_reader_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn unique_label(label: &str) -> String {
    format!(
      "{label}-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    )
  }

  fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("emerald-rt-zip-test-{}", unique_label(label)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn create_then_list_then_extract_round_trips_two_real_files() {
    let src = temp_dir("roundtrip-src");
    std::fs::write(src.join("a.txt"), "hello").unwrap();
    std::fs::write(
      src.join("b.txt"),
      "world, a longer second file that compresses well well well well",
    )
    .unwrap();

    let a_path = src.join("a.txt").to_str().unwrap().to_string();
    let b_path = src.join("b.txt").to_str().unwrap().to_string();
    let out_dir = temp_dir("roundtrip-out");
    let archive_path = out_dir.join("demo.zip").to_str().unwrap().to_string();

    unsafe {
      let a_c = std::ffi::CString::new(a_path.clone()).unwrap();
      let b_c = std::ffi::CString::new(b_path.clone()).unwrap();
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      let ptrs = [a_c.as_ptr(), b_c.as_ptr()];
      zip_create(archive_c.as_ptr(), ptrs.as_ptr(), 2);

      let id = zip_reader_open(archive_c.as_ptr());
      assert_eq!(zip_reader_entry_count(id), 2);

      let name0 = zip_reader_entry_name(id, 0);
      assert_eq!(std::ffi::CStr::from_ptr(name0).to_str().unwrap(), a_path);
      assert_eq!(zip_reader_entry_size(id, 0), 5);
      let data0 = zip_reader_read_entry_data(id, 0);
      assert_eq!(crate::bytes::bytes_as_slice(data0), b"hello");

      let name1 = zip_reader_entry_name(id, 1);
      assert_eq!(std::ffi::CStr::from_ptr(name1).to_str().unwrap(), b_path);
      let data1 = zip_reader_read_entry_data(id, 1);
      assert_eq!(
        crate::bytes::bytes_as_slice(data1),
        b"world, a longer second file that compresses well well well well"
      );

      zip_reader_close(id);

      let dest_dir = out_dir.join("out");
      let dest_dir_c = std::ffi::CString::new(dest_dir.to_str().unwrap()).unwrap();
      zip_extract(archive_c.as_ptr(), dest_dir_c.as_ptr());
      assert_eq!(
        std::fs::read_to_string(dest_dir.join(&a_path)).unwrap(),
        "hello"
      );
      assert_eq!(
        std::fs::read_to_string(dest_dir.join(&b_path)).unwrap(),
        "world, a longer second file that compresses well well well well"
      );
    }

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&out_dir).ok();
  }

  #[test]
  fn an_empty_paths_array_produces_a_real_valid_empty_archive() {
    let dir = temp_dir("empty");
    let archive_path = dir.join("empty.zip").to_str().unwrap().to_string();
    unsafe {
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      zip_create(archive_c.as_ptr(), std::ptr::null(), 0);
      let id = zip_reader_open(archive_c.as_ptr());
      assert_eq!(zip_reader_entry_count(id), 0);
      zip_reader_close(id);
    }
    std::fs::remove_dir_all(&dir).ok();
  }

  // Proves the `deflate-flate2-zlib-rs` feature is actually wired up
  // and this plan's own `Zip.create` is not silently falling back to
  // `Stored` (uncompressed) — a genuinely compressible, repetitive
  // input's own real, raw archive bytes on disk are asserted smaller
  // than the input itself, and (independently, more directly) `zip`'s
  // own `ZipFile::compression()` accessor is asked for the entry's
  // real, stored compression method.
  #[test]
  fn zip_create_genuinely_deflates_a_compressible_entry() {
    let src = temp_dir("deflate-src");
    let compressible = "aaaaaaaaaa".repeat(500);
    std::fs::write(src.join("big.txt"), &compressible).unwrap();
    let big_path = src.join("big.txt").to_str().unwrap().to_string();
    let out_dir = temp_dir("deflate-out");
    let archive_path = out_dir.join("demo.zip").to_str().unwrap().to_string();

    unsafe {
      let big_c = std::ffi::CString::new(big_path).unwrap();
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      let ptrs = [big_c.as_ptr()];
      zip_create(archive_c.as_ptr(), ptrs.as_ptr(), 1);
    }

    let archive_bytes = std::fs::metadata(&archive_path).unwrap().len();
    assert!(
      (archive_bytes as usize) < compressible.len(),
      "archive ({archive_bytes} bytes) should be smaller than the raw, repetitive 5000-byte input"
    );

    let file = File::open(&archive_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let entry = archive.by_index(0).unwrap();
    assert_eq!(entry.compression(), CompressionMethod::Deflated);

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&out_dir).ok();
  }
}
