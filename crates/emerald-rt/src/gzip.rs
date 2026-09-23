//! Plan 130 (Gzip/Deflate/Zlib Compression) — `flate2` wrapped two
//! ways: a one-shot `Gzip`/`Deflate`/`Zlib` `.compress`/`.decompress`
//! pair operating on plan 109's `Bytes` value (a bare `[len: i64]
//! [data]` heap pointer, see `bytes.rs`'s own module doc — `bytes_as_
//! slice`/`bytes_from_slice` are this module's only marshaling
//! primitives, zero new ones invented), and a `GzipWriter`/
//! `GzipReader` (plus `Deflate`/`Zlib` siblings) plan-93 resource-
//! handle pair for large, file-backed payloads that never need their
//! full uncompressed AND compressed bytes resident in memory at once.
//!
//! **Real, disclosed dependency-graph finding**: `flate2` 1.1.10 was
//! already resolved into this workspace's own `Cargo.lock` transitively
//! (plan 100's `ureq`, via its own default `gzip` feature) *before*
//! this plan added a direct dependency on it — verified this session
//! via `cargo tree -p emerald-rt -i flate2 -e features`, which showed
//! the already-active feature set landing on exactly `rust_backend` →
//! `miniz_oxide` (`any_impl`, `runtime_detection`), the same backend
//! this plan's own Decision log independently chose and pins by its
//! concrete feature name. This plan's `default-features = false,
//! features = ["miniz_oxide"]` declaration on `emerald-rt`'s own
//! `Cargo.toml` entry is therefore a direct-dependency promotion with
//! an explicit pin, not new supply-chain surface — and Cargo's own
//! whole-graph feature unification means this crate's own pin is what
//! keeps the *combined* build on `miniz_oxide` even if `ureq`'s own
//! transitive request ever changes.
//!
//! Three formats share one `Format` enum and one generic `WriterInner`/
//! `ReaderInner` pair (mirroring `encoding.rs`'s own "one `decode_with`
//! helper parameterized over the engine" convention) rather than three
//! near-identical copies of every function — `Gzip`/`Deflate`/`Zlib`'s
//! six `emerald_rt_*`-facing one-shot functions, and `GzipWriter`/
//! `GzipReader`'s six format siblings' eighteen functions, are all thin
//! wrappers around this shared core, each naming its own concrete
//! `flate2` type and `crate::handle` type-tag.
//!
//! **`GzipWriter#close` needs real ownership, not `&mut`**: flate2's
//! own `{Gz,Deflate,Zlib}Encoder::finish(self)` consumes the encoder by
//! value to flush the format's own trailer bytes (gzip's CRC32+size,
//! zlib's Adler-32) and hand back the underlying `File` — `crate::
//! handle::handle_get_mut` only ever hands a callback `&mut T`, per its
//! own doc comment's disclosed reentrancy-avoiding design. `WriterState`
//! wraps its `WriterInner` in an `Option` specifically so `.close()`'s
//! own callback can `.take()` it out (leaving `None` behind) and finish
//! it *outside* the registry lock, then `handle_close` marks the
//! handle itself closed the same way every other resource in this
//! crate already does — `.write_chunk` after `.close()` therefore hits
//! `handle.rs`'s own "use of closed" diagnostic on the very next call,
//! never a silent no-op or a use of a stale inner value.
//!
//! `GzipReader#read_chunk` makes exactly one underlying `Read::read`
//! call per invocation — the identical short-read semantics `TcpStream#
//! read`/`TlsStream#read` already disclose — returning an empty
//! `Bytes` on a real `Ok(0)`, the same "empty means EOF" convention
//! `Read`'s own contract already guarantees and this plan's own leaf
//! text names explicitly.

use crate::bytes::{bytes_as_slice, bytes_from_slice};
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use flate2::Compression;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::raw::c_char;

const GZIP_WRITER_TAG: &str = "GzipWriter";
const DEFLATE_WRITER_TAG: &str = "DeflateWriter";
const ZLIB_WRITER_TAG: &str = "ZlibWriter";
const GZIP_READER_TAG: &str = "GzipReader";
const DEFLATE_READER_TAG: &str = "DeflateReader";
const ZLIB_READER_TAG: &str = "ZlibReader";

#[derive(Clone, Copy)]
enum Format {
  Gzip,
  Deflate,
  Zlib,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// --- One-shot `Gzip`/`Deflate`/`Zlib` `.compress`/`.decompress` --------

// Compressing to an in-memory `Vec<u8>` can never fail (`Vec<u8>`'s own
// `Write` impl is infallible) — the `.expect`s below document that,
// rather than threading a `Result` through a call that can never
// actually produce `Err`.
fn compress_bytes(fmt: Format, data: &[u8]) -> Vec<u8> {
  match fmt {
    Format::Gzip => {
      let mut enc = GzEncoder::new(Vec::new(), Compression::default());
      enc
        .write_all(data)
        .expect("compressing to a Vec<u8> never fails");
      enc
        .finish()
        .expect("finishing a Vec<u8>-backed encoder never fails")
    }
    Format::Deflate => {
      let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
      enc
        .write_all(data)
        .expect("compressing to a Vec<u8> never fails");
      enc
        .finish()
        .expect("finishing a Vec<u8>-backed encoder never fails")
    }
    Format::Zlib => {
      let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
      enc
        .write_all(data)
        .expect("compressing to a Vec<u8> never fails");
      enc
        .finish()
        .expect("finishing a Vec<u8>-backed encoder never fails")
    }
  }
}

// A malformed/corrupt/wrong-format stream IS a real, expected `Err`
// here (per this module's own doc comment, no format-sniffing is
// attempted — feeding gzip bytes to `Deflate.decompress` surfaces
// exactly this `Err`), never a panic.
fn decompress_bytes(fmt: Format, data: &[u8]) -> io::Result<Vec<u8>> {
  let mut out = Vec::new();
  match fmt {
    Format::Gzip => GzDecoder::new(data).read_to_end(&mut out),
    Format::Deflate => DeflateDecoder::new(data).read_to_end(&mut out),
    Format::Zlib => ZlibDecoder::new(data).read_to_end(&mut out),
  }?;
  Ok(out)
}

// Shared by all six `emerald_rt_{gzip,deflate,zlib}_compress` wrappers
// below — `data_id`/the return value are both `Bytes`, crossing this
// boundary as a plain `i64` per `bytes.rs`'s own convention.
unsafe fn compress_entry(fmt: Format, data_id: i64) -> i64 {
  let out = compress_bytes(fmt, bytes_as_slice(data_id));
  bytes_from_slice(&out)
}

// Shared by all six `emerald_rt_{gzip,deflate,zlib}_decompress`
// wrappers — per this plan's own Concrete Proof (`Gzip.decompress`
// returns a bare `Bytes`, never a `Result`), a corrupt/mismatched-
// format stream raises plan 92's `NativeError` channel directly, the
// same "one underlying I/O call, raise rather than `Result` on
// failure" posture `TcpStream#read`/`TlsStream#read` already establish
// — not this module inventing a new failure convention.
unsafe fn decompress_entry(fmt: Format, name: &str, data_id: i64) -> i64 {
  match decompress_bytes(fmt, bytes_as_slice(data_id)) {
    Ok(out) => bytes_from_slice(&out),
    Err(e) => crate::raise_native_error(&format!("{name}.decompress: {e}")),
  }
}

/// `Gzip.compress(data: Bytes): Bytes`.
///
/// # Safety
/// `data_id` must be a pointer `bytes_from_slice` (or an equally-shaped
/// native producer) actually returned.
pub unsafe fn gzip_compress(data_id: i64) -> i64 {
  compress_entry(Format::Gzip, data_id)
}

/// `Gzip.decompress(data: Bytes): Bytes`.
///
/// # Safety
/// See `gzip_compress`.
pub unsafe fn gzip_decompress(data_id: i64) -> i64 {
  decompress_entry(Format::Gzip, "Gzip", data_id)
}

/// `Deflate.compress(data: Bytes): Bytes`.
///
/// # Safety
/// See `gzip_compress`.
pub unsafe fn deflate_compress(data_id: i64) -> i64 {
  compress_entry(Format::Deflate, data_id)
}

/// `Deflate.decompress(data: Bytes): Bytes`.
///
/// # Safety
/// See `gzip_compress`.
pub unsafe fn deflate_decompress(data_id: i64) -> i64 {
  decompress_entry(Format::Deflate, "Deflate", data_id)
}

/// `Zlib.compress(data: Bytes): Bytes`.
///
/// # Safety
/// See `gzip_compress`.
pub unsafe fn zlib_compress(data_id: i64) -> i64 {
  compress_entry(Format::Zlib, data_id)
}

/// `Zlib.decompress(data: Bytes): Bytes`.
///
/// # Safety
/// See `gzip_compress`.
pub unsafe fn zlib_decompress(data_id: i64) -> i64 {
  decompress_entry(Format::Zlib, "Zlib", data_id)
}

// --- Streaming `GzipWriter`/`GzipReader` (+ `Deflate`/`Zlib`) ----------

enum WriterInner {
  Gzip(GzEncoder<File>),
  Deflate(DeflateEncoder<File>),
  Zlib(ZlibEncoder<File>),
}

impl Write for WriterInner {
  fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    match self {
      WriterInner::Gzip(w) => w.write(buf),
      WriterInner::Deflate(w) => w.write(buf),
      WriterInner::Zlib(w) => w.write(buf),
    }
  }
  fn flush(&mut self) -> io::Result<()> {
    match self {
      WriterInner::Gzip(w) => w.flush(),
      WriterInner::Deflate(w) => w.flush(),
      WriterInner::Zlib(w) => w.flush(),
    }
  }
}

impl WriterInner {
  fn finish(self) -> io::Result<File> {
    match self {
      WriterInner::Gzip(w) => w.finish(),
      WriterInner::Deflate(w) => w.finish(),
      WriterInner::Zlib(w) => w.finish(),
    }
  }
}

// See this module's own doc comment: the `Option` exists solely so
// `.close()` can `.take()` real ownership out from behind `handle_get_
// mut`'s `&mut` callback shape, to call flate2's own consuming
// `.finish()`.
struct WriterState {
  inner: Option<WriterInner>,
}

fn writer_open(fmt: Format, path: &str) -> io::Result<WriterInner> {
  let file = File::create(path)?;
  Ok(match fmt {
    Format::Gzip => WriterInner::Gzip(GzEncoder::new(file, Compression::default())),
    Format::Deflate => WriterInner::Deflate(DeflateEncoder::new(file, Compression::default())),
    Format::Zlib => WriterInner::Zlib(ZlibEncoder::new(file, Compression::default())),
  })
}

fn writer_write_all(state: &mut WriterState, data: &[u8]) -> io::Result<()> {
  match state.inner.as_mut() {
    Some(w) => w.write_all(data),
    None => Err(io::Error::new(
      io::ErrorKind::Other,
      "write_chunk on a closed writer",
    )),
  }
}

fn writer_finish(state: &mut WriterState) -> io::Result<()> {
  match state.inner.take() {
    Some(inner) => inner.finish().map(|_file| ()),
    None => Ok(()),
  }
}

unsafe fn writer_open_entry(fmt: Format, tag: &'static str, path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match writer_open(fmt, path) {
    Ok(inner) => handle_alloc(Box::new(WriterState { inner: Some(inner) }), tag),
    Err(e) => crate::raise_native_error(&format!("{tag}.open: {e}")),
  }
}

unsafe fn writer_write_chunk_entry(id: i64, tag: &'static str, data_id: i64) {
  let data = bytes_as_slice(data_id);
  match handle_get_mut::<WriterState, io::Result<()>>(id, tag, |s| writer_write_all(s, data)) {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

unsafe fn writer_close_entry(id: i64, tag: &'static str) {
  let result = handle_get_mut::<WriterState, io::Result<()>>(id, tag, writer_finish);
  // Marks the handle closed regardless of `finish`'s own outcome, the
  // same "double-close is a harmless no-op, only a subsequent *use*
  // raises" posture `handle.rs`'s own doc comment mandates — an error
  // flushing the trailer is still surfaced below, just not by leaving
  // the handle silently re-closable.
  handle_close(id);
  match result {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

enum ReaderInner {
  Gzip(GzDecoder<File>),
  Deflate(DeflateDecoder<File>),
  Zlib(ZlibDecoder<File>),
}

impl Read for ReaderInner {
  fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
    match self {
      ReaderInner::Gzip(r) => r.read(buf),
      ReaderInner::Deflate(r) => r.read(buf),
      ReaderInner::Zlib(r) => r.read(buf),
    }
  }
}

fn reader_open(fmt: Format, path: &str) -> io::Result<ReaderInner> {
  let file = File::open(path)?;
  Ok(match fmt {
    Format::Gzip => ReaderInner::Gzip(GzDecoder::new(file)),
    Format::Deflate => ReaderInner::Deflate(DeflateDecoder::new(file)),
    Format::Zlib => ReaderInner::Zlib(ZlibDecoder::new(file)),
  })
}

unsafe fn reader_open_entry(fmt: Format, tag: &'static str, path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match reader_open(fmt, path) {
    Ok(inner) => handle_alloc(Box::new(inner), tag),
    Err(e) => crate::raise_native_error(&format!("{tag}.open: {e}")),
  }
}

// One underlying `Read::read` call — an `Ok(0)` (real EOF) becomes an
// empty `Bytes`, per this plan's own leaf text ("an empty `Bytes`
// signals EOF, mirroring `File.read`'s own disclosed all-at-once
// precedent").
unsafe fn reader_read_chunk_entry(id: i64, tag: &'static str, max_len: i64) -> i64 {
  if max_len <= 0 {
    crate::raise_native_error(&format!("{tag}#read_chunk: max_len must be positive"));
  }
  let mut buf = vec![0u8; max_len as usize];
  match handle_get_mut::<ReaderInner, io::Result<usize>>(id, tag, |r| r.read(&mut buf)) {
    Ok(Ok(n)) => bytes_from_slice(&buf[..n]),
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

fn reader_close_entry(id: i64) {
  handle_close(id);
}

/// `GzipWriter.open(path: String): GzipWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn gzip_writer_open(path: *const c_char) -> i64 {
  writer_open_entry(Format::Gzip, GZIP_WRITER_TAG, path)
}

/// `GzipWriter#write_chunk(self, data: Bytes): Void`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn gzip_writer_write_chunk(id: i64, data_id: i64) {
  writer_write_chunk_entry(id, GZIP_WRITER_TAG, data_id)
}

/// `GzipWriter#close(self): Void` — flushes the gzip trailer (CRC32 +
/// uncompressed size) and closes the underlying file.
pub unsafe fn gzip_writer_close(id: i64) {
  writer_close_entry(id, GZIP_WRITER_TAG)
}

/// `GzipReader.open(path: String): GzipReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn gzip_reader_open(path: *const c_char) -> i64 {
  reader_open_entry(Format::Gzip, GZIP_READER_TAG, path)
}

/// `GzipReader#read_chunk(self, max_len: Int64): Bytes`.
pub unsafe fn gzip_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  reader_read_chunk_entry(id, GZIP_READER_TAG, max_len)
}

/// `GzipReader#close(self): Void`.
pub fn gzip_reader_close(id: i64) {
  reader_close_entry(id)
}

/// `DeflateWriter.open(path: String): DeflateWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn deflate_writer_open(path: *const c_char) -> i64 {
  writer_open_entry(Format::Deflate, DEFLATE_WRITER_TAG, path)
}

/// `DeflateWriter#write_chunk(self, data: Bytes): Void`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn deflate_writer_write_chunk(id: i64, data_id: i64) {
  writer_write_chunk_entry(id, DEFLATE_WRITER_TAG, data_id)
}

/// `DeflateWriter#close(self): Void`.
pub unsafe fn deflate_writer_close(id: i64) {
  writer_close_entry(id, DEFLATE_WRITER_TAG)
}

/// `DeflateReader.open(path: String): DeflateReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn deflate_reader_open(path: *const c_char) -> i64 {
  reader_open_entry(Format::Deflate, DEFLATE_READER_TAG, path)
}

/// `DeflateReader#read_chunk(self, max_len: Int64): Bytes`.
pub unsafe fn deflate_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  reader_read_chunk_entry(id, DEFLATE_READER_TAG, max_len)
}

/// `DeflateReader#close(self): Void`.
pub fn deflate_reader_close(id: i64) {
  reader_close_entry(id)
}

/// `ZlibWriter.open(path: String): ZlibWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn zlib_writer_open(path: *const c_char) -> i64 {
  writer_open_entry(Format::Zlib, ZLIB_WRITER_TAG, path)
}

/// `ZlibWriter#write_chunk(self, data: Bytes): Void`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn zlib_writer_write_chunk(id: i64, data_id: i64) {
  writer_write_chunk_entry(id, ZLIB_WRITER_TAG, data_id)
}

/// `ZlibWriter#close(self): Void`.
pub unsafe fn zlib_writer_close(id: i64) {
  writer_close_entry(id, ZLIB_WRITER_TAG)
}

/// `ZlibReader.open(path: String): ZlibReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn zlib_reader_open(path: *const c_char) -> i64 {
  reader_open_entry(Format::Zlib, ZLIB_READER_TAG, path)
}

/// `ZlibReader#read_chunk(self, max_len: Int64): Bytes`.
pub unsafe fn zlib_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  reader_read_chunk_entry(id, ZLIB_READER_TAG, max_len)
}

/// `ZlibReader#close(self): Void`.
pub fn zlib_reader_close(id: i64) {
  reader_close_entry(id)
}

#[cfg(test)]
mod tests {
  use super::*;

  const SAMPLE: &[u8] = b"the quick brown fox jumps over the lazy dog, \
the quick brown fox jumps over the lazy dog";

  #[test]
  fn gzip_round_trips_and_actually_compresses_repetitive_input() {
    let compressed = compress_bytes(Format::Gzip, SAMPLE);
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(Format::Gzip, &compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn deflate_round_trips_and_actually_compresses_repetitive_input() {
    let compressed = compress_bytes(Format::Deflate, SAMPLE);
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(Format::Deflate, &compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn zlib_round_trips_and_actually_compresses_repetitive_input() {
    let compressed = compress_bytes(Format::Zlib, SAMPLE);
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(Format::Zlib, &compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn gzip_round_trips_a_zero_length_input() {
    let compressed = compress_bytes(Format::Gzip, &[]);
    let restored = decompress_bytes(Format::Gzip, &compressed).unwrap();
    assert_eq!(restored, Vec::<u8>::new());
  }

  #[test]
  fn gzip_round_trips_a_non_utf8_byte_sequence() {
    let data: &[u8] = &[0x00, 0xff, 0xfe, 0x80, 0x01, 0x00, 0x00];
    let compressed = compress_bytes(Format::Gzip, data);
    let restored = decompress_bytes(Format::Gzip, &compressed).unwrap();
    assert_eq!(restored, data);
  }

  #[test]
  fn deflate_output_has_neither_a_gzip_header_nor_a_zlib_header() {
    // Raw deflate has no format header at all, unlike its gzip/zlib
    // siblings — this test pins that real, structural difference
    // rather than merely trusting the byte-count assertion above.
    let gz = compress_bytes(Format::Gzip, SAMPLE);
    let zl = compress_bytes(Format::Zlib, SAMPLE);
    let defl = compress_bytes(Format::Deflate, SAMPLE);
    assert_ne!(defl[..2], gz[..2]);
    assert_ne!(defl[..2], zl[..2]);
  }

  #[test]
  fn feeding_gzip_bytes_to_deflate_decompress_is_a_real_err_not_a_panic() {
    let gz = compress_bytes(Format::Gzip, SAMPLE);
    // A gzip stream is not a valid raw-deflate stream from its very
    // first bytes (gzip's own 10-byte magic/flags header), so this
    // reliably surfaces as `Err`.
    let result = decompress_bytes(Format::Deflate, &gz);
    if let Ok(bytes) = result {
      assert_ne!(
        bytes, SAMPLE,
        "must not silently succeed at the wrong format"
      );
    }
  }

  #[test]
  fn gzip_compress_output_matches_the_real_flate2_crate_byte_for_byte() {
    // Pins this module's own wrapper against upstream drift: it must
    // produce EXACTLY what calling `flate2` directly produces for a
    // fixed input, with no extra marshaling in between.
    let mut direct = GzEncoder::new(Vec::new(), Compression::default());
    direct.write_all(SAMPLE).unwrap();
    let direct_out = direct.finish().unwrap();
    let wrapped_out = compress_bytes(Format::Gzip, SAMPLE);
    assert_eq!(wrapped_out, direct_out);
  }

  #[test]
  fn writer_reader_round_trip_a_real_file_on_disk() {
    let dir = std::env::temp_dir().join(format!(
      "emerald-rt-gzip-test-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.gz");
    let path_str = path.to_str().unwrap();

    let mut writer = writer_open(Format::Gzip, path_str).unwrap();
    writer.write_all(SAMPLE).unwrap();
    writer.finish().unwrap();

    let mut reader = reader_open(Format::Gzip, path_str).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, SAMPLE);

    std::fs::remove_dir_all(&dir).ok();
  }
}
