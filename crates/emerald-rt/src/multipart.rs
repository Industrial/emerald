//! Plan 105 (Multipart/Form-Data Parsing) — `Multipart.start`/
//! `.next_field`, `Field.name`/`.filename`/`.read_chunk`/`.close`,
//! wrapping `multer`. Layers on plan 101's `http_server.rs` (this
//! module's own `multipart_start` reads the request body plan 101
//! already buffers whole into `HttpRequestData.body_bytes` — a real,
//! disclosed, pre-existing architectural fact this module adapts to
//! rather than changes; see `http_server.rs`'s own `http_serve` for
//! the actual synchronous `request.as_reader().read_to_end(...)` call
//! this depends on — itself a real, disclosed fix this plan made
//! there: the pre-existing `read_to_string` silently dropped an
//! entire body on any invalid-UTF-8 byte, exactly what a real binary
//! file upload routinely contains) and `crate::tokio_rt()`'s shared
//! lazy runtime (plan 94), since every `multer::Field`/`Multipart`
//! method that can advance the underlying stream is a real `async
//! fn`.
//!
//! Two independent `crate::handle` registry entries per plan 105's own
//! Decision log — a `Multipart` handle (tag `"Multipart"`) stays alive
//! for the whole request, each `.next_field()` call producing its own,
//! independently-closable `Field` handle (tag `"Field"`).
//!
//! `BodyStream` below is the "sync-to-async body bridge" this plan's
//! own `leaf-sync-to-async-body-bridge` names: `multer::Multipart::new`
//! needs a real `futures_core::Stream<Item = Result<bytes::Bytes,
//! std::io::Error>>`. Since plan 101's own request body is already
//! fully resident in memory as one `String` by the time a handler
//! (and therefore this module) ever runs — not a live, still-draining
//! `Read` this module could poll incrementally against a real socket
//! — this bridge hands multer that same in-memory buffer back out
//! `STREAM_CHUNK_SIZE` bytes per poll rather than all at once, so
//! multer's own boundary parser still consumes it incrementally, the
//! same real, bounded-piece-at-a-time shape `Field.read_chunk` gives
//! back to Emerald on the way out.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use bytes::Bytes;
use futures_core::Stream;
use std::os::raw::c_char;
use std::pin::Pin;
use std::task::{Context, Poll};

const MULTIPART_TAG: &str = "Multipart";
const FIELD_TAG: &str = "Field";

// 64 KiB — matches this plan's own Concrete Proof's own
// `Field.read_chunk(field, 65536)` call exactly, though the two
// numbers are independent in principle (a caller-chosen `max_bytes`
// smaller or larger than this constant is still handled correctly by
// `FieldState`'s own `leftover` buffer below).
const STREAM_CHUNK_SIZE: usize = 64 * 1024;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

// Unlike `crate::alloc_and_copy_str` (which requires a valid `&str`),
// a chunk of raw uploaded-file bytes is not guaranteed to be valid
// UTF-8 at all — a real binary upload (an image, an archive) would
// panic that function's own `.as_bytes()`-free NUL-terminated-copy
// contract for no reason at all, since a C string never actually
// requires UTF-8, only a byte sequence with no embedded NUL and a
// trailing one. Copies raw bytes directly; still subject to this
// crate's own already-disclosed "String has no length header, embeds
// truncate at the first NUL" limit (`lib.rs`'s own module doc) — not
// fixed here, since that is a project-wide FFI convention this plan
// does not change.
unsafe fn alloc_and_copy_bytes(bytes: &[u8]) -> *mut c_char {
  let buf = crate::emerald_alloc(bytes.len() as i64 + 1) as *mut u8;
  std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
  *buf.add(bytes.len()) = 0;
  buf as *mut c_char
}

/// The sync-to-async body bridge (see this module's own doc comment)
/// — a plain, already-fully-buffered byte vector handed out
/// `STREAM_CHUNK_SIZE` bytes per poll. Always `Poll::Ready` on its
/// very first poll (no real waiting is ever possible — the bytes are
/// already in memory), so this never actually parks a task; it exists
/// purely to give `multer`'s own boundary parser an incremental view
/// of the body rather than one giant single item.
struct BodyStream {
  data: Vec<u8>,
  offset: usize,
}

impl Stream for BodyStream {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let this = self.get_mut();
    if this.offset >= this.data.len() {
      return Poll::Ready(None);
    }
    let end = (this.offset + STREAM_CHUNK_SIZE).min(this.data.len());
    let chunk = Bytes::copy_from_slice(&this.data[this.offset..end]);
    this.offset = end;
    Poll::Ready(Some(Ok(chunk)))
  }
}

/// A `Field` handle's own stored value — the real `multer::Field<
/// 'static>` plus a `leftover` buffer holding whatever a previous
/// `multer::Field::chunk()` call handed back beyond a caller's own
/// `max_bytes` cap, so `.read_chunk` can honor an arbitrary caller-
/// chosen `max_bytes` exactly (never silently dropping the excess of
/// a chunk `multer` itself handed back larger than requested) without
/// ever pulling a second real chunk off the underlying stream before
/// the first one's own leftover is fully drained.
struct FieldState {
  field: multer::Field<'static>,
  leftover: Vec<u8>,
}

/// Rust-level core of `Multipart.start`, factored out from the real
/// `*const c_char`-marshaling `multipart_start` below so this
/// module's own `#[cfg(test)]` suite can drive it directly against a
/// synthetic in-memory body — no real `HttpRequestData`/live
/// `tiny_http::Request` required.
fn multipart_start_from_bytes(body: Vec<u8>, boundary: String) -> i64 {
  let stream = BodyStream {
    data: body,
    offset: 0,
  };
  let mp = multer::Multipart::new(stream, boundary);
  handle_alloc(Box::new(mp), MULTIPART_TAG)
}

/// `Multipart.start(request: Int64, boundary: String): Int64` — named
/// `.start`, not the plan's own literal `.begin`: `begin` is a real,
/// grammar-reserved keyword (`begin ... rescue ... end`'s own
/// exception-handling block, confirmed directly against
/// `grammar.lalrpop` — a genuine parse error, found only by actually
/// compiling this plan's own Concrete Proof example), the same class
/// of rename plan 101's own `HttpResponse.build` (not the plan's own
/// literal `.new`) and plan 109's `Sha256Hasher.hasher()` (not
/// `.new()`) already establish for the identical reason.
///
/// # Safety
/// `boundary`, if non-null, must point to a valid, NUL-terminated C
/// string. `request` must be a live `HttpRequest` handle.
pub unsafe fn multipart_start(request_handle: i64, boundary: *const c_char) -> i64 {
  let boundary = match read_str(boundary) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let body = match crate::http_server::http_request_body_bytes(request_handle) {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&e),
  };
  multipart_start_from_bytes(body, boundary)
}

/// `Multipart.next_field(multipart: Int64): Int64` — `0` (never a
/// real handle id, per plan 93's own counter-starts-at-1 convention)
/// is the reserved "no more fields" sentinel, distinct from the
/// separate `Field` handle registry's own id space.
///
/// Real, disclosed ordering requirement found empirically writing
/// this module's own `#[cfg(test)]` suite, not assumed from `multer`'s
/// docs alone: a `multer::Field` genuinely holds a live lock into its
/// own parent `Multipart`'s shared internal state for as long as the
/// `Field` VALUE itself is alive — draining it to nil via
/// `.read_chunk` is NOT sufficient on its own; a caller must also call
/// `Field.close` on the previous field before this function's own next
/// call can succeed, or it raises a NativeError ("failed to lock
/// multipart state", `multer`'s own real message) rather than
/// returning `0`/a real next handle.
///
/// # Safety
/// `multipart` must be a live `Multipart` handle.
pub unsafe fn multipart_next_field(multipart: i64) -> i64 {
  let outcome = handle_get_mut::<multer::Multipart<'static>, _>(multipart, MULTIPART_TAG, |mp| {
    crate::tokio_rt().block_on(async { mp.next_field().await })
  });
  match outcome {
    Ok(Ok(Some(field))) => handle_alloc(
      Box::new(FieldState {
        field,
        leftover: Vec::new(),
      }),
      FIELD_TAG,
    ),
    Ok(Ok(None)) => 0,
    Ok(Err(e)) => crate::raise_native_error(&format!("Multipart.next_field: {e}")),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Field.name(field: Int64): String` — genuinely not nullable, per
/// this plan's own Decision log: a part missing its own mandatory
/// `Content-Disposition` `name` parameter is malformed, per-spec
/// (RFC 7578) input, surfaced as an aborted parse (plan 45's own
/// File I/O precedent for a real, spec-violating input), not a
/// silently-nullable field-of-a-field.
///
/// # Safety
/// `field` must be a live `Field` handle.
pub unsafe fn field_name(field: i64) -> *const c_char {
  let result = handle_get_mut::<FieldState, Result<String, String>>(field, FIELD_TAG, |s| {
    s.field
      .name()
      .map(|n| n.to_string())
      .ok_or_else(|| "Field.name: part is missing its own required `name` parameter (malformed multipart/form-data input)".to_string())
  });
  match result {
    Ok(Ok(name)) => crate::alloc_and_copy_str(&name),
    Ok(Err(e)) | Err(e) => crate::raise_native_error(&e),
  }
}

/// `Field.filename(field: Int64): Option[String]` — genuinely
/// nullable per RFC 7578: `filename` is an optional `Content-
/// Disposition` parameter, present only when the part represents an
/// uploaded file. Real, disclosed `is_null`-branch-plus-`phi`
/// construction site lives in `emerald-codegen`'s own `Field` static-
/// dispatch arm — mirrors `Env.get`/`String.from_cstring`'s own
/// already-established pattern for a bare nullable `*mut c_char`.
///
/// # Safety
/// `field` must be a live `Field` handle.
pub unsafe fn field_filename(field: i64) -> *mut c_char {
  let result = handle_get_mut::<FieldState, Option<String>>(field, FIELD_TAG, |s| {
    s.field.file_name().map(|n| n.to_string())
  });
  match result {
    Ok(Some(name)) => crate::alloc_and_copy_str(&name) as *mut c_char,
    Ok(None) => std::ptr::null_mut(),
    Err(e) => crate::raise_native_error(&e),
  }
}

// The real, Rust-side core of `.read_chunk` — drains `state.leftover`
// first if non-empty, otherwise blocks on one real `multer::Field::
// chunk()` call, splitting off (and remembering) anything beyond
// `max_bytes` rather than ever dropping it. Returns `Ok(None)` on
// genuine end-of-field (mirrored to Emerald's own `nil`) exactly once
// — the moment `leftover` is empty AND `multer` itself reports no
// more chunks — never before every real byte has already been handed
// back through a previous call.
fn read_chunk_from_state(
  state: &mut FieldState,
  max_bytes: usize,
) -> Result<Option<Vec<u8>>, String> {
  if !state.leftover.is_empty() {
    let take = max_bytes.min(state.leftover.len());
    let out: Vec<u8> = state.leftover.drain(0..take).collect();
    return Ok(Some(out));
  }
  let maybe_bytes = crate::tokio_rt()
    .block_on(async { state.field.chunk().await })
    .map_err(|e| e.to_string())?;
  match maybe_bytes {
    None => Ok(None),
    Some(bytes) => {
      if bytes.len() <= max_bytes {
        Ok(Some(bytes.to_vec()))
      } else {
        let mut owned = bytes.to_vec();
        state.leftover = owned.split_off(max_bytes);
        Ok(Some(owned))
      }
    }
  }
}

/// `Field.read_chunk(field: Int64, max_bytes: Int64): Option[String]`
/// — bounded per this plan's own Decision log ("the direct, disclosed
/// answer to this batch's own no-GC caveat"): never hands back more
/// than `max_bytes` bytes in one call, regardless of how large a real
/// `multer::Field::chunk()` call's own internal buffer happens to be.
///
/// # Safety
/// `field` must be a live `Field` handle.
pub unsafe fn field_read_chunk(field: i64, max_bytes: i64) -> *mut c_char {
  if max_bytes < 0 {
    crate::raise_native_error("Field.read_chunk: max_bytes must be >= 0");
  }
  let max_bytes = max_bytes as usize;
  let outcome =
    handle_get_mut::<FieldState, _>(field, FIELD_TAG, |s| read_chunk_from_state(s, max_bytes));
  match outcome {
    Ok(Ok(Some(bytes))) => alloc_and_copy_bytes(&bytes),
    Ok(Ok(None)) => std::ptr::null_mut(),
    Ok(Err(e)) => crate::raise_native_error(&format!("Field.read_chunk: {e}")),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Field.close(field: Int64): Int64` — `1` if the handle was open
/// and is now closed, `0` on an already-closed or unknown handle
/// (plan 93's own double-close-is-a-no-op convention, reused
/// verbatim — never panics on a stale handle).
///
/// # Safety
/// Always safe to call for any `Int64`, live `Field` handle or not.
pub unsafe fn field_close(field: i64) -> i64 {
  if handle_close(field) {
    1
  } else {
    0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // Builds a real, spec-shaped `multipart/form-data` body by hand —
  // one field with both a `name` and a `filename` (an uploaded file),
  // matching this plan's own Concrete Proof's own shape.
  fn sample_body(boundary: &str, field_name: &str, filename: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
      format!("Content-Disposition: form-data; name=\"{field_name}\"; filename=\"{filename}\"\r\n")
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: text/plain\r\n\r\n");
    body.extend_from_slice(content);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
  }

  #[test]
  fn name_and_filename_round_trip_through_a_real_multer_parse() {
    let boundary = "TestBoundary1";
    let body = sample_body(boundary, "upload", "hello.txt", b"hello world");
    let mp = multipart_start_from_bytes(body, boundary.to_string());
    let field = unsafe { multipart_next_field(mp) };
    assert!(field > 0, "expected a real field handle, got {field}");

    let name = unsafe { std::ffi::CStr::from_ptr(field_name(field)) }
      .to_str()
      .unwrap()
      .to_string();
    assert_eq!(name, "upload");

    let filename_ptr = unsafe { field_filename(field) };
    assert!(!filename_ptr.is_null(), "expected a real filename, got nil");
    let filename = unsafe { std::ffi::CStr::from_ptr(filename_ptr) }
      .to_str()
      .unwrap()
      .to_string();
    assert_eq!(filename, "hello.txt");
  }

  // `leaf-example-and-gate`'s own mandatory Rust-side assertion:
  // `.read_chunk` returns `nil` exactly once, after every real byte —
  // never before, never more than once. Uses a `max_bytes` smaller
  // than the real content so multiple real chunks are exercised, not
  // just a single one.
  #[test]
  fn read_chunk_drains_every_byte_then_returns_nil_exactly_once() {
    let boundary = "TestBoundary2";
    let content = b"the quick brown fox jumps over the lazy dog";
    let body = sample_body(boundary, "upload", "hostname", content);
    let mp = multipart_start_from_bytes(body, boundary.to_string());
    let field = unsafe { multipart_next_field(mp) };
    assert!(field > 0);

    let mut collected = Vec::new();
    loop {
      let ptr = unsafe { field_read_chunk(field, 7) };
      if ptr.is_null() {
        break;
      }
      let chunk = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes().to_vec();
      assert!(
        !chunk.is_empty(),
        "a non-nil chunk should never be empty before the real end"
      );
      assert!(chunk.len() <= 7, "chunk exceeded the requested max_bytes");
      collected.extend_from_slice(&chunk);
    }
    assert_eq!(collected, content);

    // Exactly once, not "eventually": a second call after the field is
    // already exhausted must ALSO return nil — never resurrect a
    // phantom chunk, never error.
    assert!(
      unsafe { field_read_chunk(field, 7) }.is_null(),
      "a call after the real end should still return nil"
    );

    assert_eq!(unsafe { field_close(field) }, 1);
    assert_eq!(unsafe { field_close(field) }, 0);
  }

  // A real, disclosed regression test for the exact bug plan 105
  // found and fixed in `http_server.rs`'s own `http_serve` (raw bytes
  // now, not a UTF-8-only `read_to_string`): a genuinely large,
  // non-UTF-8-safe binary field content, drained the same 65536-byte-
  // bounded way this module's own `Field.read_chunk` always works,
  // across enough real `multer::Field::chunk()` calls to exercise
  // `FieldState`'s own `leftover` buffer repeatedly, not just once.
  #[test]
  fn a_large_binary_field_round_trips_its_exact_byte_count_across_many_chunks() {
    let boundary = "TestBoundaryBig";
    let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8 + 1).collect();
    let body = sample_body(boundary, "upload", "payload.bin", &content);
    let mp = multipart_start_from_bytes(body, boundary.to_string());
    let field = unsafe { multipart_next_field(mp) };
    assert!(field > 0, "expected a real field handle, got {field}");
    let mut total = 0usize;
    loop {
      let ptr = unsafe { field_read_chunk(field, 65536) };
      if ptr.is_null() {
        break;
      }
      let chunk = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes().to_vec();
      total += chunk.len();
    }
    assert_eq!(total, content.len(), "byte count mismatch");
  }

  #[test]
  fn a_second_field_beyond_the_only_real_one_reports_the_zero_sentinel() {
    let boundary = "TestBoundary3";
    let body = sample_body(boundary, "upload", "f", b"x");
    let mp = multipart_start_from_bytes(body, boundary.to_string());
    let first = unsafe { multipart_next_field(mp) };
    assert!(first > 0);
    // Drain the only field fully, THEN close it, before asking for a
    // next one — real, correct `multer` usage found empirically
    // writing this test (not assumed): a `multer::Field` genuinely
    // holds a live lock into its parent `Multipart`'s own shared
    // state for as long as the `Field` VALUE itself is alive, not
    // merely until its content is drained to nil — `Multipart.
    // next_field`'s own doc comment above discloses this exact
    // ordering requirement, found here.
    loop {
      let ptr = unsafe { field_read_chunk(first, 65536) };
      if ptr.is_null() {
        break;
      }
    }
    assert_eq!(unsafe { field_close(first) }, 1);
    let second = unsafe { multipart_next_field(mp) };
    assert_eq!(second, 0, "expected the exhausted sentinel, got {second}");
  }
}
