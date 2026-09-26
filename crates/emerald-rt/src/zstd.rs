//! Plan 131 (Zstandard Compression) — the official `zstd` crate
//! (`gyscos/zstd-rs`), a genuine, disclosed C-binding exception to
//! plan 95's pure-Rust-first policy (see this crate's own `Cargo.toml`
//! entry and `DEPENDENCIES.md` for the full account): `zstd-sys`
//! vendors and `cc`-compiles Facebook's own reference `libzstd` C
//! source tree at build time — a compile-time-only dependency, no
//! system `libzstd`/`pkg-config` requirement, structurally identical
//! to plan 91's own `runtime/emerald_runtime.c` build step, just one
//! dependency layer further down.
//!
//! **Named `zstd.rs`/exported as `mod zstds` (`#[path]`-redirected) in
//! `lib.rs`**, not a bare `mod zstd;` — a bare `mod zstd` would shadow
//! the external `zstd` crate this module wraps, the identical
//! collision `csv.rs`/`toml.rs`/`url.rs`/`ini.rs`/`oauth2.rs` already
//! hit and dodge the same way (see `lib.rs`'s own `mod zstds` comment).
//!
//! **Resolved from plan 131's own "Not yet decided" item 1**: the
//! one-shot API is `zstd::stream::{encode_all, decode_all}`, not
//! `zstd::bulk::{compress, decompress}` — `bulk::decompress` requires
//! the caller to pre-size (or over-allocate) an output buffer before
//! decompressing, and this module has no way to know a compressed
//! payload's original decompressed size ahead of time without parsing
//! the frame header first (zstd's optional content-size field is not
//! always present). `stream::{encode_all, decode_all}` operate over
//! any `Read`, and `&[u8]` implements `Read`, so this still lowers
//! cleanly onto plan 92's `(ptr, len)` `Bytes` convention with no
//! intermediate file or pipe — `bytes_as_slice`/`bytes_from_slice`
//! remain this module's only marshaling primitives, identical to plan
//! 130's `gzip.rs`.
//!
//! One-shot `Zstd.compress`/`.decompress` plus streaming `ZstdWriter`/
//! `ZstdReader` mirror plan 130's `Gzip`/`GzipWriter`/`GzipReader`
//! shape exactly (same `crate::handle` registry mechanism, same
//! empty-`Bytes`-signals-EOF convention, same `Option`-wrapped writer
//! state so `.close()` can `.take()` real ownership out from behind
//! `handle_get_mut`'s `&mut` callback shape to call the underlying
//! encoder's consuming `.finish()`) — a program switching between
//! `Gzip`/`Zstd` for the same streaming workload changes one namespace
//! name and nothing else, per this plan's own Decision log. Errors
//! raise plan 92's `NativeError` channel directly via
//! `crate::raise_native_error`, never a `Result` — the same posture
//! `gzip.rs` already establishes for this exact domain shape (a
//! corrupt/truncated frame is as much an expected, named failure mode
//! here as a mismatched compression format is there), so this module
//! does not introduce a plan-195 typed `DomainError` — `Zstd`/`Gzip`
//! never had one to begin with.
//!
//! **Level is a real, exposed `Int64` parameter**, unlike plan 130's
//! fixed `Compression::default()` choice — Zstandard's genuine
//! `1..=22` range (with `0` meaning "library default," currently `3`,
//! per `zstd::stream::Encoder::new`'s own documented contract, verified
//! directly against `docs.rs/zstd/0.14.0` this session) is this
//! format's headline feature. Out-of-range values are libzstd's own
//! problem to reject (it does, with a real error `encode_all`/
//! `Encoder::new` surface as `io::Result::Err`); this module does not
//! pre-validate a magic numeric range, per this plan's own Decision
//! log.
//!
//! **Correction against this module doc's own first-pass assumption**:
//! an earlier draft of this comment claimed `emerald-sema`/`emerald-
//! codegen` needed zero changes, based on a `grep` that came back
//! empty against a stale search-tool cache — actually compiling
//! `examples/zstd_roundtrip.em` immediately surfaced "undefined
//! variable `Zstd`", disproving that. The plan's own leaf text was
//! right: `Zstd`/`ZstdWriter`/`ZstdReader` each need a real, hard-
//! coded reserved-namespace arm in `emerald-sema`'s own static/
//! instance method type-checking match (mirroring `Gzip`/`Deflate`/
//! `Zlib`'s/`GzipWriter`/`GzipReader`'s own arms exactly, plus
//! `ZstdWriter`/`ZstdReader` added to the reserved-newtype-name list),
//! a real `module.add_function(..., Linkage::External)` declaration
//! per extern symbol in `emerald-codegen`, and `ZstdWriter`/
//! `ZstdReader` added to both of that crate's own newtype registries
//! (the `NEWTYPE_UNDERLYING`-populating list and its separate
//! `newtypes.insert(...)` call set — plan 132's own implementing
//! agent already found and disclosed that `GzipWriter`/`GzipReader`
//! needed both, not just one; this plan repeats that fix rather than
//! rediscovering the gap).

use crate::bytes::{bytes_as_slice, bytes_from_slice};
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::os::raw::c_char;

const ZSTD_WRITER_TAG: &str = "ZstdWriter";
const ZSTD_READER_TAG: &str = "ZstdReader";

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// --- One-shot `Zstd.compress`/`.decompress` ----------------------------

fn compress_bytes(data: &[u8], level: i32) -> io::Result<Vec<u8>> {
  zstd::stream::encode_all(data, level)
}

// A malformed/truncated/non-zstd input IS a real, expected `Err` here
// (this module attempts no format-sniffing, the same posture `gzip.rs`
// already establishes) — never a panic.
fn decompress_bytes(data: &[u8]) -> io::Result<Vec<u8>> {
  zstd::stream::decode_all(data)
}

/// `Zstd.compress(data: Bytes, level: Int64): Bytes`.
///
/// # Safety
/// `data_id` must be a pointer `bytes_from_slice` (or an equally-shaped
/// native producer) actually returned.
pub unsafe fn zstd_compress(data_id: i64, level: i64) -> i64 {
  match compress_bytes(bytes_as_slice(data_id), level as i32) {
    Ok(out) => bytes_from_slice(&out),
    Err(e) => crate::raise_native_error(&format!("Zstd.compress: {e}")),
  }
}

/// `Zstd.decompress(data: Bytes): Bytes`.
///
/// # Safety
/// See `zstd_compress`.
pub unsafe fn zstd_decompress(data_id: i64) -> i64 {
  match decompress_bytes(bytes_as_slice(data_id)) {
    Ok(out) => bytes_from_slice(&out),
    Err(e) => crate::raise_native_error(&format!("Zstd.decompress: {e}")),
  }
}

// --- Streaming `ZstdWriter`/`ZstdReader` --------------------------------

// The `Option` exists solely so `.close()` can `.take()` real ownership
// out from behind `handle_get_mut`'s `&mut` callback shape, to call
// `zstd::stream::Encoder`'s own consuming `.finish()` — identical
// reasoning to `gzip.rs`'s own `WriterState`.
struct WriterState {
  inner: Option<zstd::stream::Encoder<'static, File>>,
}

fn writer_open(path: &str, level: i32) -> io::Result<zstd::stream::Encoder<'static, File>> {
  let file = File::create(path)?;
  zstd::stream::Encoder::new(file, level)
}

fn writer_write_all(state: &mut WriterState, data: &[u8]) -> io::Result<()> {
  match state.inner.as_mut() {
    Some(w) => w.write_all(data),
    None => Err(io::Error::other("write_chunk on a closed writer")),
  }
}

fn writer_finish(state: &mut WriterState) -> io::Result<()> {
  match state.inner.take() {
    Some(inner) => inner.finish().map(|_file| ()),
    None => Ok(()),
  }
}

unsafe fn writer_open_entry(path: *const c_char, level: i64) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match writer_open(path, level as i32) {
    Ok(inner) => handle_alloc(
      Box::new(WriterState { inner: Some(inner) }),
      ZSTD_WRITER_TAG,
    ),
    Err(e) => crate::raise_native_error(&format!("ZstdWriter.open: {e}")),
  }
}

unsafe fn writer_write_chunk_entry(id: i64, data_id: i64) {
  let data = bytes_as_slice(data_id);
  match handle_get_mut::<WriterState, io::Result<()>>(id, ZSTD_WRITER_TAG, |s| {
    writer_write_all(s, data)
  }) {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

unsafe fn writer_close_entry(id: i64) {
  let result = handle_get_mut::<WriterState, io::Result<()>>(id, ZSTD_WRITER_TAG, writer_finish);
  // Marks the handle closed regardless of `finish`'s own outcome, the
  // same "double-close is a harmless no-op, only a subsequent *use*
  // raises" posture `handle.rs`'s own doc comment mandates.
  handle_close(id);
  match result {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

type ReaderInner = zstd::stream::Decoder<'static, BufReader<File>>;

fn reader_open(path: &str) -> io::Result<ReaderInner> {
  let file = File::open(path)?;
  zstd::stream::Decoder::new(file)
}

unsafe fn reader_open_entry(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match reader_open(path) {
    Ok(inner) => handle_alloc(Box::new(inner), ZSTD_READER_TAG),
    Err(e) => crate::raise_native_error(&format!("ZstdReader.open: {e}")),
  }
}

// One underlying `Read::read` call — an `Ok(0)` (real EOF) becomes an
// empty `Bytes`, the identical convention `gzip.rs`'s own `GzipReader#
// read_chunk` already establishes.
unsafe fn reader_read_chunk_entry(id: i64, max_len: i64) -> i64 {
  if max_len <= 0 {
    crate::raise_native_error("ZstdReader#read_chunk: max_len must be positive");
  }
  let mut buf = vec![0u8; max_len as usize];
  match handle_get_mut::<ReaderInner, io::Result<usize>>(id, ZSTD_READER_TAG, |r| r.read(&mut buf))
  {
    Ok(Ok(n)) => bytes_from_slice(&buf[..n]),
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

fn reader_close_entry(id: i64) {
  handle_close(id);
}

/// `ZstdWriter.open(path: String, level: Int64): ZstdWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn zstd_writer_open(path: *const c_char, level: i64) -> i64 {
  writer_open_entry(path, level)
}

/// `ZstdWriter#write_chunk(self, data: Bytes): Void`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn zstd_writer_write_chunk(id: i64, data_id: i64) {
  writer_write_chunk_entry(id, data_id)
}

/// `ZstdWriter#close(self): Void` — flushes the zstd frame epilogue and
/// closes the underlying file.
pub unsafe fn zstd_writer_close(id: i64) {
  writer_close_entry(id)
}

/// `ZstdReader.open(path: String): ZstdReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn zstd_reader_open(path: *const c_char) -> i64 {
  reader_open_entry(path)
}

/// `ZstdReader#read_chunk(self, max_len: Int64): Bytes`.
pub unsafe fn zstd_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  reader_read_chunk_entry(id, max_len)
}

/// `ZstdReader#close(self): Void`.
pub fn zstd_reader_close(id: i64) {
  reader_close_entry(id)
}

#[cfg(test)]
mod tests {
  use super::*;

  const SAMPLE: &[u8] = b"the quick brown fox jumps over the lazy dog, \
the quick brown fox jumps over the lazy dog";

  #[test]
  fn round_trips_and_actually_compresses_repetitive_input_at_level_1() {
    let compressed = compress_bytes(SAMPLE, 1).unwrap();
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn round_trips_at_level_3_the_library_default() {
    let compressed = compress_bytes(SAMPLE, 3).unwrap();
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn round_trips_at_level_19_a_real_high_level() {
    let compressed = compress_bytes(SAMPLE, 19).unwrap();
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  // A real, higher compression level must never produce a *larger*
  // compressed output than a lower level for the same input — the
  // actual, verifiable ratio/level tradeoff Zstandard is chosen for.
  #[test]
  fn a_higher_level_never_compresses_worse_than_a_lower_one() {
    let low = compress_bytes(SAMPLE, 3).unwrap();
    let high = compress_bytes(SAMPLE, 19).unwrap();
    assert!(high.len() <= low.len());
  }

  #[test]
  fn level_zero_means_use_the_library_default() {
    let default_level = compress_bytes(SAMPLE, 0).unwrap();
    let explicit_default = compress_bytes(SAMPLE, 3).unwrap();
    assert_eq!(default_level.len(), explicit_default.len());
    let restored = decompress_bytes(&default_level).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn decompressing_a_corrupt_frame_is_a_real_err_not_a_panic() {
    let err = decompress_bytes(b"not a real zstd frame at all").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
  }

  #[test]
  fn writer_and_reader_round_trip_through_a_real_file() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
      "emerald-rt-zstd-test-{}-{}.zst",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    let path_str = path.to_str().unwrap();

    let mut encoder = writer_open(path_str, 3).unwrap();
    encoder.write_all(SAMPLE).unwrap();
    encoder.finish().unwrap();

    let mut decoder = reader_open(path_str).unwrap();
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).unwrap();
    assert_eq!(out, SAMPLE);

    std::fs::remove_file(&path).unwrap();
  }
}
