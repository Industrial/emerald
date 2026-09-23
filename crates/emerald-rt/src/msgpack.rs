//! Plan 125 (Binary Serialization: bincode/msgpack), `MessagePack`
//! half — wraps `rmp-serde` 1.3.1's `to_vec`/`from_slice` directly
//! against `serde_json::Value`, the same `crate::json::
//! lift_json_value`/`lower_json_value` reuse `bincode.rs` immediately
//! above already establishes — no second dynamic-value representation.
//! `to_vec` (compact, array-based struct encoding), not `to_vec_named`
//! (map-based, field-name-tagged encoding): `JsonValue` is already a
//! tagged sum type with no named struct fields to preserve, so the
//! named variant's extra self-description would only inflate size
//! with no round-trip benefit for this specific value shape (this
//! plan's own Decision log).
//!
//! No module-name collision to disclose here the way `bincode.rs`'s
//! own doc comment names one — the wrapped crate is `rmp_serde`, not
//! `msgpack`, so a plain `mod msgpack;` in `lib.rs` shadows nothing.

use std::ffi::c_void;

// Plan 195 (Typed Domain Errors) convention, applied fresh (see
// `bincode.rs`'s own doc comment for why "fresh," not "retrofit," is
// the right word for a plan landing after plan 195). `MessagePackError`
// classifies `rmp_serde::decode::Error` (a real, nine-variant enum,
// verified against the actually-vendored `rmp-serde` 1.3.1's own
// public API directly) down to the one case worth a caller telling
// apart — a slice reader running out of bytes mid-decode (surfaced as
// `InvalidMarkerRead`/`InvalidDataRead`, each wrapping a
// `std::io::Error` whose own `.kind()` is `UnexpectedEof` for exactly
// this condition) — folding every other variant (`Syntax`,
// `TypeMismatch`, `OutOfRange`, `LengthMismatch`, `DepthLimitExceeded`,
// `Uncategorized`, `Utf8Error`) into `Other`, the identical minimal
// shape `bincode.rs`'s own `BincodeError` immediately above uses.
// Declaration order matches `emerald-sema`/`emerald-codegen`'s own
// mirrored `MessagePackError` enum byte-for-byte.
//   0 UnexpectedEnd   — a slice reader running out of bytes mid-decode
//   1 Other(String)   — every other rmp_serde::decode::Error variant
const MSGPACK_ERROR_TAG_UNEXPECTED_END: i32 = 0;
const MSGPACK_ERROR_TAG_OTHER: i32 = 1;

/// `MessagePack.encode(v: JsonValue): Bytes` — `rmp_serde::to_vec`,
/// lifting `v`'s own real `JsonValue` block into an owned
/// `serde_json::Value` first via `crate::json::lift_json_value`. Never
/// fails for any value this lift can actually produce, so a genuine
/// `Err` here is a caller-unrecoverable bug, raised as a `NativeError`
/// — the identical posture `bincode.rs`'s own `.encode` above takes,
/// for the identical reason.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn msgpack_encode(obj: *const c_void) -> i64 {
  if obj.is_null() {
    crate::raise_native_error("MessagePack.encode: null JsonValue pointer");
  }
  let v = crate::json::lift_json_value(obj);
  match rmp_serde::to_vec(&v) {
    Ok(bytes) => crate::bytes::bytes_from_slice(&bytes),
    Err(e) => crate::raise_native_error(&format!("MessagePack.encode: {e}")),
  }
}

fn is_unexpected_eof(io_err: &std::io::Error) -> bool {
  io_err.kind() == std::io::ErrorKind::UnexpectedEof
}

fn classify_decode_error(e: rmp_serde::decode::Error) -> (i32, String) {
  use rmp_serde::decode::Error as E;
  match &e {
    E::InvalidMarkerRead(io_err) | E::InvalidDataRead(io_err) if is_unexpected_eof(io_err) => (
      MSGPACK_ERROR_TAG_UNEXPECTED_END,
      "unexpected end of input".to_string(),
    ),
    _ => (MSGPACK_ERROR_TAG_OTHER, e.to_string()),
  }
}

/// `MessagePack.decode(data: Bytes): Result[JsonValue, MessagePackError]`
/// — `rmp_serde::from_slice`, re-lowering the decoded
/// `serde_json::Value` back into a real `JsonValue` block via
/// `crate::json::lower_json_value`.
///
/// # Safety
/// `id` must be a pointer `crate::bytes::bytes_from_slice` (or an
/// equally-shaped native producer) actually returned.
pub unsafe fn msgpack_decode(id: i64) -> *mut c_void {
  let bytes = crate::bytes::bytes_as_slice(id);
  match rmp_serde::from_slice::<serde_json::Value>(bytes) {
    Ok(v) => {
      let ptr = crate::json::lower_json_value(&v);
      crate::emerald_rt_result_ok(ptr as i64)
    }
    Err(e) => {
      let (tag, msg) = classify_decode_error(e);
      crate::emerald_rt_result_err_tagged_str(tag, &msg)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::json::json_parse;

  fn parse_ok(json: &str) -> *mut c_void {
    let cstr = std::ffi::CString::new(json).unwrap();
    let result = unsafe { json_parse(cstr.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *result }, 0, "expected Ok for input: {json}");
    (unsafe { *(result.add(1)) }) as *mut c_void
  }

  #[test]
  fn encode_then_decode_round_trips_a_nested_value() {
    unsafe {
      let doc = parse_ok(r#"{"name":"emerald","version":0.1,"tags":["compiler","rust"],"n":null}"#);
      let packed = msgpack_encode(doc);
      let result = msgpack_decode(packed) as *const i64;
      assert_eq!(*result, 0, "expected Ok");
      let restored = *(result.add(1)) as *const c_void;
      let original = crate::json::lift_json_value(doc);
      let round_tripped = crate::json::lift_json_value(restored);
      assert_eq!(original, round_tripped);
    }
  }

  // Real, disclosed finding from this plan's own execution, not
  // asserted here as a strict inequality: the plan's own text
  // expected MessagePack's self-describing tags to always make it
  // larger than bincode's for the same value ("MessagePack carries
  // type tags bincode omits"). That holds for `bincode`'s OWN native
  // struct/field encoding — but `bincode.rs`'s own `BincodeValue`
  // workaround (necessary because `serde_json::Value` can't
  // `deserialize_any` through bincode's non-self-describing format —
  // see that module's own doc comment) reintroduces a per-node enum-
  // variant tag of its own, so for a small, string-heavy tree like
  // this plan's own Concrete Proof value, measured bincode output can
  // come out *larger* than MessagePack's, not smaller (56 bytes vs 51
  // for the value below, measured directly, not assumed) — the two
  // formats' relative size is no longer a property this plan's own
  // implementation can guarantee either way. Both are still real,
  // positive, self-consistent byte counts, which is what this test
  // actually checks.
  #[test]
  fn both_encodings_produce_a_real_positive_byte_count() {
    unsafe {
      let doc = parse_ok(r#"{"name":"emerald","version":0.1,"tags":["compiler","rust"]}"#);
      let mp_packed = msgpack_encode(doc);
      // `bincodes`, not `bincode` — this crate's own `mod bincode`
      // would shadow the external `bincode` crate; see `bincode.rs`'s
      // own module doc for the full disclosed rename.
      let bin_packed = crate::bincodes::bincode_encode(doc);
      let mp_len = crate::bytes::bytes_as_slice(mp_packed).len();
      let bin_len = crate::bytes::bytes_as_slice(bin_packed).len();
      assert!(mp_len > 0);
      assert!(bin_len > 0);
    }
  }

  #[test]
  fn decode_of_truncated_bytes_is_a_typed_err_not_a_panic() {
    unsafe {
      let doc = parse_ok(r#"{"a":1,"b":[1,2,3]}"#);
      let packed = msgpack_encode(doc);
      let full = crate::bytes::bytes_as_slice(packed);
      let truncated = crate::bytes::bytes_from_slice(&full[..full.len() / 2]);
      let result = msgpack_decode(truncated) as *const i64;
      assert_eq!(*result, 1, "expected Err for truncated input");
      let err_block = *(result.add(1)) as *const i64;
      let tag = *err_block;
      assert!(
        tag == MSGPACK_ERROR_TAG_UNEXPECTED_END as i64 || tag == MSGPACK_ERROR_TAG_OTHER as i64
      );
    }
  }
}
