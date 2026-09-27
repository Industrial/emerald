//! Plan 135 (Brotli Compression) — the `brotli` crate (`dropbox/
//! rust-brotli`), verified this session against its own README to be,
//! in the maintainer's own stated project requirement, "Direct
//! no-stdlib port of the C brotli compressor to Rust... no dependency
//! on the Rust stdlib" — a genuine, line-by-line transliteration of
//! Google's reference C implementation into Rust, not a `bindgen`/FFI
//! wrapper around a compiled `libbrotli` the way plan 131's own
//! `zstd.rs` binds `libzstd`. See `Cargo.toml`'s own entry for the
//! full crate-vetting account.
//!
//! **Named `brotli.rs`/exported as `mod brotlis` (`#[path]`-redirected)
//! in `lib.rs`**, not a bare `mod brotli;` — a bare `mod brotli` would
//! shadow the external `brotli` crate this module wraps, the identical
//! collision `csv.rs`/`toml.rs`/`url.rs`/`zstd.rs`/... already hit and
//! dodge the same way (see `lib.rs`'s own `mod brotlis` comment).
//!
//! **One-shot API resolved (plan 135's own "Not yet decided" EXECUTE
//! blocker): the crate's own `BrotliCompress`/`BrotliDecompress`
//! stream-copy free functions**, not `brotli::{CompressorReader,
//! Decompressor}`. `BrotliCompress<R: Read, W: Write>(r: &mut R, w:
//! &mut W, params: &BrotliEncoderParams) -> io::Result<usize>` (and
//! `BrotliDecompress`'s equivalent) take a plain `&mut dyn Read`/`&mut
//! dyn Write` directly — `&[u8]` implements `Read`, `Vec<u8>`
//! implements `Write` — the identical "byte slice in, growable `Vec`
//! out, no intermediate adapter object" shape plan 131's own `zstd::
//! stream::{encode_all, decode_all}` already establishes for this
//! module's sibling, and a closer fit to plan 92's `Bytes`-to-`(ptr,
//! len)` convention than wrapping a `CompressorReader`/`Decompressor`
//! around an in-memory slice would be (that shape would still need an
//! extra `.read_to_end` loop to actually drain into a `Vec<u8>`
//! itself).
//!
//! **Quality is a real, exposed `Int64` parameter (`0..=11`) — not
//! pre-validated before calling into the crate, the identical posture
//! plan 131's own `Zstd.compress` already takes toward its own
//! out-of-range level.** `BrotliEncoderParams::quality: i32`'s own doc
//! comment (read directly from the crate's own `enc/backward_
//! references/mod.rs` this session) documents brotli's real `0..=11`
//! range ("11 is smallest but takes longest to encode"); the crate's
//! own `SanitizeParams` (`enc/encode.rs`, read this session) clamps an
//! out-of-range `quality` to `min(11, max(0, quality))` rather than
//! erroring, so an out-of-range `Int64` here is a real, checked (see
//! this module's own `#[cfg(test)]`), non-panicking clamp, not
//! undefined behavior — this module therefore does not invent a
//! second, redundant range check the underlying crate already
//! performs.
//!
//! **Streaming `BrotliWriter`/`BrotliReader` wrap `brotli::
//! CompressorWriter<File>`/`brotli::Decompressor<File>`** — the same
//! plan-93 opaque-handle shape `ZstdWriter`/`ZstdReader` already
//! establish, with `BrotliWriter.open`'s own real second `quality:
//! Int64` parameter mirroring `ZstdWriter.open`'s own `level: Int64`.
//! **A real, disclosed upstream API constraint**: unlike `flate2`'s/
//! `zstd`'s/`lz4_flex`'s own consuming `.finish() -> Result<_, _>`,
//! `CompressorWriter::into_inner(self) -> W` is unconditionally
//! infallible in this crate's own public API — its own internal
//! terminal-flush `Result` (`BROTLI_OPERATION_FINISH`) is discarded
//! (`enc/writer.rs`, read this session: `match self.flush_or_close(...)
//! { Ok(_) => {} Err(_) => {} }`). `writer_finish` below therefore
//! calls the real, still-fallible `.flush()` first (Brotli's own
//! `BROTLI_OPERATION_FLUSH`, whose `io::Result` IS observable through
//! the `Write` trait) to surface everything that can actually be
//! surfaced, then `.into_inner()` for the unavoidably-silent terminal
//! step — not a shortcut taken here, a real ceiling this crate's own
//! public API imposes.
//!
//! Errors raise plan 92's `NativeError` channel directly via
//! `crate::raise_native_error`, never a `Result` — the identical
//! posture `gzip.rs`/`zstd.rs`/`lz4.rs` already establish for this
//! exact domain shape (a corrupt/truncated compressed buffer is as
//! much an expected, named failure mode here as it is there), so this
//! module does not introduce a plan-195 typed `DomainError` either —
//! `Brotli` never had one to begin with, any more than `Gzip`/`Zstd`/
//! `Lz4` did.

use crate::bytes::{bytes_as_slice, bytes_from_slice};
use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use brotli::enc::BrotliEncoderParams;
use brotli::{BrotliCompress, BrotliDecompress, CompressorWriter, Decompressor};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::raw::c_char;

const BROTLI_WRITER_TAG: &str = "BrotliWriter";
const BROTLI_READER_TAG: &str = "BrotliReader";

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// A real `quality`-to-`BrotliEncoderParams` builder shared by both the
// one-shot `compress_bytes` and the streaming `writer_open` below —
// see this module's own doc comment for why an out-of-range `quality`
// is intentionally left for the crate's own `SanitizeParams` to clamp,
// not pre-validated here.
fn brotli_encoder_params(quality: i64) -> BrotliEncoderParams {
  let mut params = BrotliEncoderParams::default();
  params.quality = quality as i32;
  params
}

// --- One-shot `Brotli.compress`/`.decompress` --------------------------

// Compressing a `&[u8]` (an infallible `Read`) into a `Vec<u8>` (an
// infallible `Write`) can never actually fail — the same reasoning
// `gzip.rs`'s own `compress_bytes` documents for `flate2`'s
// `Vec<u8>`-backed encoders.
fn compress_bytes(data: &[u8], quality: i64) -> Vec<u8> {
  let params = brotli_encoder_params(quality);
  let mut input = data;
  let mut out = Vec::new();
  BrotliCompress(&mut input, &mut out, &params)
    .expect("compressing a &[u8] into a Vec<u8> never fails");
  out
}

// A malformed/truncated/non-brotli input IS a real, expected `Err`
// here (this module attempts no format-sniffing, the same posture
// `gzip.rs`/`zstd.rs`/`lz4.rs` already establish) — never a panic.
fn decompress_bytes(data: &[u8]) -> io::Result<Vec<u8>> {
  let mut input = data;
  let mut out = Vec::new();
  BrotliDecompress(&mut input, &mut out)?;
  Ok(out)
}

/// `Brotli.compress(data: Bytes, quality: Int64): Bytes`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn brotli_compress(data_id: i64, quality: i64) -> i64 {
  let out = compress_bytes(bytes_as_slice(data_id), quality);
  bytes_from_slice(&out)
}

/// `Brotli.decompress(data: Bytes): Bytes`.
///
/// # Safety
/// `data_id` must be a live `Bytes` value.
pub unsafe fn brotli_decompress(data_id: i64) -> i64 {
  match decompress_bytes(bytes_as_slice(data_id)) {
    Ok(out) => bytes_from_slice(&out),
    Err(e) => crate::raise_native_error(&format!("Brotli.decompress: {e}")),
  }
}

// --- Streaming `BrotliWriter`/`BrotliReader` ----------------------------

// The `Option` exists solely so `.close()` can `.take()` real ownership
// out from behind `handle_get_mut`'s `&mut` callback shape, to call
// `CompressorWriter`'s own consuming `.into_inner()` — identical
// reasoning to `gzip.rs`'s/`zstd.rs`'s/`lz4.rs`'s own `WriterState`.
struct WriterState {
  inner: Option<CompressorWriter<File>>,
}

fn writer_open(path: &str, quality: i64) -> io::Result<CompressorWriter<File>> {
  let file = File::create(path)?;
  let params = brotli_encoder_params(quality);
  // `buffer_size: 0` -- the crate's own `CompressorWriter::new`
  // substitutes its real default (4096) for a zero buffer size, per
  // `enc/writer.rs`, read this session.
  Ok(CompressorWriter::with_params(file, 0, &params))
}

fn writer_write_all(state: &mut WriterState, data: &[u8]) -> io::Result<()> {
  match state.inner.as_mut() {
    Some(w) => w.write_all(data),
    None => Err(io::Error::other("write_chunk on a closed writer")),
  }
}

// See this module's own doc comment for the real, disclosed upstream
// constraint `.into_inner()`'s own infallible signature imposes here.
fn writer_finish(state: &mut WriterState) -> io::Result<()> {
  match state.inner.take() {
    Some(mut inner) => {
      inner.flush()?;
      let _file = inner.into_inner();
      Ok(())
    }
    None => Ok(()),
  }
}

unsafe fn writer_open_entry(path: *const c_char, quality: i64) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match writer_open(path, quality) {
    Ok(inner) => handle_alloc(
      Box::new(WriterState { inner: Some(inner) }),
      BROTLI_WRITER_TAG,
    ),
    Err(e) => crate::raise_native_error(&format!("BrotliWriter.open: {e}")),
  }
}

unsafe fn writer_write_chunk_entry(id: i64, data_id: i64) {
  let data = bytes_as_slice(data_id);
  match handle_get_mut::<WriterState, io::Result<()>>(id, BROTLI_WRITER_TAG, |s| {
    writer_write_all(s, data)
  }) {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

unsafe fn writer_close_entry(id: i64) {
  let result = handle_get_mut::<WriterState, io::Result<()>>(id, BROTLI_WRITER_TAG, writer_finish);
  // Marks the handle closed regardless of `writer_finish`'s own
  // outcome, the same "double-close is a harmless no-op, only a
  // subsequent *use* raises" posture `handle.rs`'s own doc comment
  // mandates.
  handle_close(id);
  match result {
    Ok(Ok(())) => {}
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

type ReaderInner = Decompressor<File>;

fn reader_open(path: &str) -> io::Result<ReaderInner> {
  let file = File::open(path)?;
  // `Decompressor::new` is infallible (like `lz4_flex::frame::
  // FrameDecoder::new`, unlike `zstd::stream::Decoder::new`) -- the
  // real brotli stream header is only parsed lazily, on the first
  // `.read` call. `buffer_size: 0` substitutes the crate's own real
  // default (4096), identically to `writer_open` above.
  Ok(Decompressor::new(file, 0))
}

unsafe fn reader_open_entry(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match reader_open(path) {
    Ok(inner) => handle_alloc(Box::new(inner), BROTLI_READER_TAG),
    Err(e) => crate::raise_native_error(&format!("BrotliReader.open: {e}")),
  }
}

// One underlying `Read::read` call — an `Ok(0)` (real EOF) becomes an
// empty `Bytes`, the identical convention `gzip.rs`'s/`zstd.rs`'s/
// `lz4.rs`'s own `*Reader#read_chunk` already establish.
unsafe fn reader_read_chunk_entry(id: i64, max_len: i64) -> i64 {
  if max_len <= 0 {
    crate::raise_native_error("BrotliReader#read_chunk: max_len must be positive");
  }
  let mut buf = vec![0u8; max_len as usize];
  match handle_get_mut::<ReaderInner, io::Result<usize>>(id, BROTLI_READER_TAG, |r| {
    r.read(&mut buf)
  }) {
    Ok(Ok(n)) => bytes_from_slice(&buf[..n]),
    Ok(Err(e)) => crate::raise_native_error(&e.to_string()),
    Err(e) => crate::raise_native_error(&e),
  }
}

fn reader_close_entry(id: i64) {
  handle_close(id);
}

/// `BrotliWriter.open(path: String, quality: Int64): BrotliWriter`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn brotli_writer_open(path: *const c_char, quality: i64) -> i64 {
  writer_open_entry(path, quality)
}

/// `BrotliWriter#write_chunk(self, data: Bytes): Void`.
///
/// # Safety
/// `id` must be a live `BrotliWriter` handle; `data_id` must be a live
/// `Bytes` value.
pub unsafe fn brotli_writer_write_chunk(id: i64, data_id: i64) {
  writer_write_chunk_entry(id, data_id)
}

/// `BrotliWriter#close(self): Void` — flushes as much of the brotli
/// stream epilogue as this crate's own public API can surface a real
/// error for, then finishes it (see this module's own doc comment),
/// and closes the underlying file.
///
/// # Safety
/// `id` must be a live `BrotliWriter` handle.
pub unsafe fn brotli_writer_close(id: i64) {
  writer_close_entry(id)
}

/// `BrotliReader.open(path: String): BrotliReader`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn brotli_reader_open(path: *const c_char) -> i64 {
  reader_open_entry(path)
}

/// `BrotliReader#read_chunk(self, max_len: Int64): Bytes`.
///
/// # Safety
/// `id` must be a live `BrotliReader` handle.
pub unsafe fn brotli_reader_read_chunk(id: i64, max_len: i64) -> i64 {
  reader_read_chunk_entry(id, max_len)
}

/// `BrotliReader#close(self): Void`.
///
/// # Safety
/// `id` must be a live `BrotliReader` handle.
pub unsafe fn brotli_reader_close(id: i64) {
  reader_close_entry(id)
}

#[cfg(test)]
mod tests {
  use super::*;

  const SAMPLE: &[u8] = b"the quick brown fox jumps over the lazy dog, \
the quick brown fox jumps over the lazy dog";

  #[test]
  fn round_trips_at_quality_0_the_fastest_level() {
    let compressed = compress_bytes(SAMPLE, 0);
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn round_trips_at_quality_5_a_real_mid_range_default() {
    let compressed = compress_bytes(SAMPLE, 5);
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn round_trips_at_quality_11_the_slowest_best_level() {
    let compressed = compress_bytes(SAMPLE, 11);
    assert!(compressed.len() < SAMPLE.len());
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  // This plan's own real, checkable claim: a higher quality must never
  // produce a *larger* compressed output than a lower quality for the
  // same input — the identical ratio/level relationship plan 131's own
  // `a_higher_level_never_compresses_worse_than_a_lower_one` checks for
  // `Zstd`.
  #[test]
  fn a_higher_quality_never_compresses_worse_than_a_lower_one() {
    let low = compress_bytes(SAMPLE, 1);
    let high = compress_bytes(SAMPLE, 11);
    assert!(high.len() <= low.len());
  }

  // This module's own doc comment claims the crate's own
  // `SanitizeParams` clamps an out-of-range `quality` to `0..=11`
  // rather than erroring or exhibiting undefined behavior -- checked
  // directly here, not merely trusted from reading the crate's source.
  #[test]
  fn an_out_of_range_quality_is_clamped_not_rejected_or_ub() {
    let negative = compress_bytes(SAMPLE, -1);
    let restored = decompress_bytes(&negative).unwrap();
    assert_eq!(restored, SAMPLE);

    let too_high = compress_bytes(SAMPLE, 999);
    let restored = decompress_bytes(&too_high).unwrap();
    assert_eq!(restored, SAMPLE);
  }

  #[test]
  fn round_trips_a_zero_length_input() {
    let compressed = compress_bytes(&[], 5);
    let restored = decompress_bytes(&compressed).unwrap();
    assert_eq!(restored, Vec::<u8>::new());
  }

  #[test]
  fn decompressing_a_corrupt_stream_is_a_real_err_not_a_panic() {
    let err = decompress_bytes(b"not a real brotli stream at all");
    assert!(err.is_err());
  }

  #[test]
  fn writer_and_reader_round_trip_through_a_real_file() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
      "emerald-rt-brotli-test-{}-{}.br",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    let path_str = path.to_str().unwrap();

    let mut writer = writer_open(path_str, 5).unwrap();
    writer.write_all(SAMPLE).unwrap();
    writer.flush().unwrap();
    let _file = writer.into_inner();

    let mut reader = reader_open(path_str).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, SAMPLE);

    std::fs::remove_file(&path).unwrap();
  }
}
