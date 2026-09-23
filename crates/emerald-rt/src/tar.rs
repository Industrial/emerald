//! Plan 132 (Tar Archives) — `tar::Builder`/`tar::Archive` wrapped two
//! ways: a one-shot `Tar.create`/`.extract` pair (`create` writes a
//! fresh archive from an explicit, caller-ordered `Array[String]` of
//! file paths; `extract` is the crate's own one-call bulk-unpack), and
//! a `TarReader` plan-93 resource-handle for listing/reading entries
//! one at a time. Tar is a container format only — no compression code
//! anywhere in this module or in `tar`'s own dependency graph (see
//! `Cargo.toml`'s own ledger comment); `.tar.gz`/`.tar.zst` is plan
//! 130's/131's job, composed on top of this module's own plain bytes,
//! never built in here (this module never depends on `flate2`/`zstd`).
//!
//! **A real, disclosed simplification from a "true" lazy streaming
//! reader**: `tar::Archive::entries()` returns a `tar::Entries<'a, R>`
//! borrowing `&'a Archive<R>` — a genuinely self-referential shape
//! (the iterator borrows from the very resource `crate::handle`'s
//! `'static`-bound registry would need to own) that `xml.rs`'s own
//! `XmlReader` sidesteps by driving `quick_xml::Reader::read_event_
//! into` directly (no comparable "read one entry, no borrowed
//! iterator" method exists on `tar::Archive` itself). Rather than a
//! hand-rolled unsafe self-referential struct for no real benefit this
//! plan's own Concrete Proof needs, `TarReader.open` eagerly reads
//! every entry's name AND full content into a plain `Vec` once, up
//! front — `.next_entry`/`.entry_size`/`.read_entry_data` then just
//! walk that `Vec` with a cursor. A real, disclosed capability loss
//! for a future caller streaming a multi-gigabyte archive entry by
//! entry without ever wanting it all resident in memory at once — not
//! a gap this plan's own worked proof (two small files) exercises.

use crate::bytes::bytes_from_slice;
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::ffi::c_void;
use std::fs::File;
use std::io::Read;
use std::os::raw::c_char;

const TAG: &str = "TarReader";

// Plan 73's own fixed `Option[T]` encoding (`Some` = 0, `None` = 1,
// independent of `T`) — `regex.rs`'s own `OPTION_SOME`/`OPTION_NONE`
// constants and `alloc_option_string` helper, reused verbatim rather
// than imported (every module hand-building this layout keeps its own
// copy, the same convention `regex.rs`'s own module doc already
// establishes for `Array[T]`/`Option[T]` buffers).
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

// --- One-shot `Tar.create`/`.extract` -----------------------------------

/// `Tar.create(archive_path: String, paths: Array[String]): Void` —
/// `path_ptrs`/`count` are `paths`'s own already-unpacked `Array[
/// String]` header (`emerald-codegen`'s own call site reads the
/// array's `[length: Int64][elements...]` layout directly, the same
/// `build_array_each` already reads, rather than this function ever
/// seeing the array's own header-inclusive base pointer) — `Array[T]`
/// carries no runtime length metadata of its own (plan 45's own
/// finding), hence `count` crossing this boundary as an explicit,
/// separate parameter. Archive member order is caller order (`paths`'
/// own order), matching `tar::Builder::append_path`'s real behavior —
/// this function never sorts.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string. `path_ptrs` must point to `count` valid `*const c_char`
/// entries, each itself a valid, NUL-terminated C string.
pub unsafe fn tar_create(archive_path: *const c_char, path_ptrs: *const *const c_char, count: i64) {
  let archive_path = match read_str(archive_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  let file = match File::create(archive_path) {
    Ok(f) => f,
    Err(e) => crate::raise_native_error(&format!("Tar.create: {archive_path}: {e}")),
  };
  let mut builder = tar::Builder::new(file);
  for i in 0..count {
    let path = match read_str(*path_ptrs.add(i as usize)) {
      Ok(p) => p,
      Err(e) => crate::raise_native_error(&e),
    };
    if let Err(e) = builder.append_path(path) {
      crate::raise_native_error(&format!("Tar.create: {path}: {e}"));
    }
  }
  if let Err(e) = builder.finish() {
    crate::raise_native_error(&format!("Tar.create: {archive_path}: {e}"));
  }
}

/// `Tar.extract(archive_path: String, dest_dir: String): Void` — via
/// `tar::Archive::unpack`, the crate's own one-call bulk-extract API.
///
/// # Safety
/// `archive_path`/`dest_dir`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn tar_extract(archive_path: *const c_char, dest_dir: *const c_char) {
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
    Err(e) => crate::raise_native_error(&format!("Tar.extract: {archive_path}: {e}")),
  };
  let mut archive = tar::Archive::new(file);
  if let Err(e) = archive.unpack(dest_dir) {
    crate::raise_native_error(&format!("Tar.extract: {e}"));
  }
}

// --- Streaming `TarReader` handle ----------------------------------------

// This module's own doc comment: `entries`/`cursor` are populated
// entirely up front by `read_all_entries`, not lazily as `.next_entry`
// is called.
struct TarReaderState {
  entries: Vec<(String, Vec<u8>)>,
  cursor: usize,
}

fn read_all_entries(path: &str) -> std::io::Result<Vec<(String, Vec<u8>)>> {
  let file = File::open(path)?;
  let mut archive = tar::Archive::new(file);
  let mut out = Vec::new();
  for entry in archive.entries()? {
    let mut entry = entry?;
    let name = entry.path()?.to_string_lossy().into_owned();
    let mut data = Vec::new();
    entry.read_to_end(&mut data)?;
    out.push((name, data));
  }
  Ok(out)
}

/// `TarReader.open(archive_path: String): TarReader`.
///
/// # Safety
/// `archive_path`, if non-null, must point to a valid, NUL-terminated
/// C string.
pub unsafe fn tar_reader_open(archive_path: *const c_char) -> i64 {
  let archive_path = match read_str(archive_path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match read_all_entries(archive_path) {
    Ok(entries) => handle_alloc(Box::new(TarReaderState { entries, cursor: 0 }), TAG),
    Err(e) => crate::raise_native_error(&format!("TarReader.open: {archive_path}: {e}")),
  }
}

/// `.next_entry(self): Option[String]` — advances the cursor by one
/// and returns the just-yielded entry's own path, `None` once every
/// entry has already been consumed (this plan's own Concrete Proof's
/// terminating third call, past the last real entry).
///
/// # Safety
/// `id` must be a live `TarReader` handle.
pub unsafe fn tar_reader_next_entry(id: i64) -> *mut c_void {
  match handle_get_mut::<TarReaderState, Option<String>>(id, TAG, |s| {
    if s.cursor < s.entries.len() {
      let name = s.entries[s.cursor].0.clone();
      s.cursor += 1;
      Some(name)
    } else {
      None
    }
  }) {
    Ok(name) => alloc_option_string(name.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.entry_size(self): Int64` — the just-yielded entry's own real byte
/// size, plan 45's own `.split_count`-alongside-`.split` two-call
/// workaround for `Array[T]`'s lack of runtime length metadata, applied
/// here to "how big is the entry I just got a name for" instead.
///
/// # Safety
/// `id` must be a live `TarReader` handle.
pub unsafe fn tar_reader_entry_size(id: i64) -> i64 {
  match handle_get_mut::<TarReaderState, Option<i64>>(id, TAG, |s| {
    if s.cursor == 0 {
      None
    } else {
      Some(s.entries[s.cursor - 1].1.len() as i64)
    }
  }) {
    Ok(Some(n)) => n,
    Ok(None) => {
      crate::raise_native_error("TarReader#entry_size: no entry yet -- call next_entry() first")
    }
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.read_entry_data(self): Bytes` — the just-yielded entry's own full
/// content.
///
/// # Safety
/// `id` must be a live `TarReader` handle.
pub unsafe fn tar_reader_read_entry_data(id: i64) -> i64 {
  match handle_get_mut::<TarReaderState, Option<Vec<u8>>>(id, TAG, |s| {
    if s.cursor == 0 {
      None
    } else {
      Some(s.entries[s.cursor - 1].1.clone())
    }
  }) {
    Ok(Some(data)) => bytes_from_slice(&data),
    Ok(None) => crate::raise_native_error(
      "TarReader#read_entry_data: no entry yet -- call next_entry() first",
    ),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.close(self): Void`.
pub fn tar_reader_close(id: i64) {
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

  // Archive/destination paths (never embedded inside any tar header)
  // are free to be absolute — the same `std::env::temp_dir()`-backed
  // per-test isolation every other module's own tests already use.
  fn temp_dir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("emerald-rt-tar-test-{}", unique_label(label)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  // A REAL, disclosed finding from actually running this against
  // `tar`'s own real, default-on security check: `tar::Builder::
  // append_path`'s own `Header::set_path` rejects (a real `Err`, this
  // module's own `Tar.create` then genuinely raises) any path whose
  // own `Component::RootDir` is present unless the crate's own
  // `preserve_absolute` opt-in (not exposed by this plan's Emerald-
  // level API) is set — the identical "refuse to silently write an
  // absolute member path" default GNU tar's own `--absolute-names`
  // flag exists to override. Every path this module's own tests hand
  // to `tar_create` must therefore itself be relative, which — since
  // `append_path` ALSO opens the given path from the process's own
  // real working directory to read its content — means the file must
  // actually live under `target/` (Cargo's own guaranteed `cargo test`
  // working directory: the crate's own manifest directory), not under
  // `std::env::temp_dir()`.
  fn relative_src_dir(label: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from("target")
      .join(format!("emerald-rt-tar-test-src-{}", unique_label(label)));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn create_then_list_then_extract_round_trips_two_real_files() {
    let src = relative_src_dir("roundtrip");
    std::fs::write(src.join("a.txt"), "hello").unwrap();
    std::fs::write(src.join("b.txt"), "world, a longer second file").unwrap();

    let a_path = src.join("a.txt").to_str().unwrap().to_string();
    let b_path = src.join("b.txt").to_str().unwrap().to_string();
    let out_dir = temp_dir("roundtrip");
    let archive_path = out_dir.join("demo.tar").to_str().unwrap().to_string();

    unsafe {
      let a_c = std::ffi::CString::new(a_path.clone()).unwrap();
      let b_c = std::ffi::CString::new(b_path.clone()).unwrap();
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      let ptrs = [a_c.as_ptr(), b_c.as_ptr()];
      tar_create(archive_c.as_ptr(), ptrs.as_ptr(), 2);

      let id = tar_reader_open(archive_c.as_ptr());

      let name1 = tar_reader_next_entry(id) as *const i64;
      assert_eq!(*name1, OPTION_SOME);
      let name1_ptr = *(name1.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(name1_ptr).to_str().unwrap(),
        a_path
      );
      assert_eq!(tar_reader_entry_size(id), 5);
      let data1 = tar_reader_read_entry_data(id);
      assert_eq!(crate::bytes::bytes_as_slice(data1), b"hello");

      let name2 = tar_reader_next_entry(id) as *const i64;
      assert_eq!(*name2, OPTION_SOME);
      let name2_ptr = *(name2.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(name2_ptr).to_str().unwrap(),
        b_path
      );
      let data2 = tar_reader_read_entry_data(id);
      assert_eq!(
        crate::bytes::bytes_as_slice(data2),
        b"world, a longer second file"
      );

      // Past the last real entry, `.next_entry()` resolves to `None`
      // -- proving iteration genuinely terminates, this plan's own
      // Concrete Proof's third `match`.
      let name3 = tar_reader_next_entry(id) as *const i64;
      assert_eq!(*name3, OPTION_NONE);

      tar_reader_close(id);

      let dest_dir = out_dir.join("out");
      let dest_dir_c = std::ffi::CString::new(dest_dir.to_str().unwrap()).unwrap();
      tar_extract(archive_c.as_ptr(), dest_dir_c.as_ptr());
      assert_eq!(
        std::fs::read_to_string(dest_dir.join(&a_path)).unwrap(),
        "hello"
      );
      assert_eq!(
        std::fs::read_to_string(dest_dir.join(&b_path)).unwrap(),
        "world, a longer second file"
      );
    }

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&out_dir).ok();
  }

  #[test]
  fn an_empty_paths_array_produces_a_real_valid_empty_archive() {
    let dir = temp_dir("empty");
    let archive_path = dir.join("empty.tar").to_str().unwrap().to_string();
    unsafe {
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      tar_create(archive_c.as_ptr(), std::ptr::null(), 0);
      let id = tar_reader_open(archive_c.as_ptr());
      let first = tar_reader_next_entry(id) as *const i64;
      assert_eq!(*first, OPTION_NONE);
      tar_reader_close(id);
    }
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn tar_bytes_compose_through_gzip_compress_and_back() {
    // Proves plan 130's `Gzip`/plan 132's `Tar` genuinely compose --
    // `Tar.create`'s own raw output bytes, fed through the real,
    // public `Gzip.compress`/`.decompress` entry points, decompress
    // back to byte-identical tar bytes that `tar::Archive` then reads
    // correctly, matching real `tar | gzip` pipeline precedent.
    let src = relative_src_dir("gz-compose");
    std::fs::write(src.join("only.txt"), "compose me").unwrap();
    let only_path = src.join("only.txt").to_str().unwrap().to_string();
    let out_dir = temp_dir("gz-compose");
    let archive_path = out_dir.join("demo.tar").to_str().unwrap().to_string();

    unsafe {
      let only_c = std::ffi::CString::new(only_path.clone()).unwrap();
      let archive_c = std::ffi::CString::new(archive_path.clone()).unwrap();
      let ptrs = [only_c.as_ptr()];
      tar_create(archive_c.as_ptr(), ptrs.as_ptr(), 1);
    }

    let raw_tar_bytes = std::fs::read(&archive_path).unwrap();
    let gz_back_path = out_dir.join("roundtrip_from_gz.tar");
    unsafe {
      let raw_id = bytes_from_slice(&raw_tar_bytes);
      let compressed_id = crate::gzip::gzip_compress(raw_id);
      let decompressed_id = crate::gzip::gzip_decompress(compressed_id);
      let decompressed = crate::bytes::bytes_as_slice(decompressed_id).to_vec();
      assert_eq!(decompressed, raw_tar_bytes);
      std::fs::write(&gz_back_path, &decompressed).unwrap();

      let gz_back_c = std::ffi::CString::new(gz_back_path.to_str().unwrap()).unwrap();
      let id = tar_reader_open(gz_back_c.as_ptr());
      let name = tar_reader_next_entry(id) as *const i64;
      assert_eq!(*name, OPTION_SOME);
      let data = tar_reader_read_entry_data(id);
      assert_eq!(crate::bytes::bytes_as_slice(data), b"compose me");
      tar_reader_close(id);
    }

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&out_dir).ok();
  }
}
