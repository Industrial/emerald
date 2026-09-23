//! Plan 125 (Binary Serialization: bincode/msgpack), `Bincode` half —
//! wraps `bincode` 2.0.1 (pinned exactly; see `Cargo.toml`'s own
//! comment for the disclosed "the crate is formally unmaintained"
//! finding) via its own `serde` feature's `bincode::serde` module.
//! `JsonValue` (plan 118's dynamic-value representation, lifted/
//! lowered through `crate::json`'s own `lift_json_value`/
//! `lower_json_value` helpers) is still the only Emerald-visible value
//! shape this module accepts/returns — but, as this module's own
//! second doc-comment paragraph below discloses, getting there needs
//! one more step than "just derive `serde` on `serde_json::Value` and
//! hand it to `bincode`" turned out to actually work. Two real,
//! disclosed corrections against this plan's own text, which assumed
//! "a Rust enum can trivially wear both `serde::{Serialize,
//! Deserialize}` and `bincode::{Encode, Decode}` derives," found by
//! actually building and running this module, not by reading either
//! crate's docs: `JsonValue` itself is not a real Rust enum at all
//! (see `json.rs`'s own module doc — it's a compiler-synthesized
//! tagged-union byte layout with no Rust type a `#[derive]` could ever
//! attach to), and `serde_json::Value` itself — the real Rust type
//! this module actually lifts/lowers through — cannot round-trip
//! through `bincode` directly either, for an unrelated, second reason
//! covered next.
//!
//! Named `bincodes`, not `bincode`, in `lib.rs`'s own `#[path]`
//! declaration — this crate's own `mod bincode` would shadow the
//! external `bincode` crate this module wraps, the identical
//! collision `csv.rs`/`toml.rs`/`tempfile.rs`/`url.rs` already hit and
//! disclosed.
//!
//! A second, real finding this module does not paper over, found only
//! by actually running the round-trip test below, not by reading
//! either crate's docs: encoding/decoding `serde_json::Value` directly
//! through `bincode::serde` compiles cleanly but **fails at runtime**
//! with `DecodeError::Serde(AnyNotSupported)` on every decode.
//! `serde_json::Value`'s own `Deserialize` impl is written against
//! JSON's self-describing wire format — it calls `deserializer.
//! deserialize_any(...)`, asking the format itself to say what shape
//! is next. `bincode` is deliberately NOT self-describing (its whole
//! reason to be smaller/faster than MessagePack is omitting exactly
//! those type tags), so its `Deserializer` cannot implement
//! `deserialize_any` at all — confirmed directly against the crate's
//! own `serde::de` module, which returns this exact `AnyNotSupported`
//! error for it. `msgpack.rs`'s own round trip has no such problem
//! (MessagePack tags every value with its own format marker, so
//! `deserialize_any` works there) — this is a real, structural
//! difference between the two formats, not a bug in either crate.
//!
//! The fix kept here: a small, private, concrete `BincodeValue` enum
//! (`#[derive(Serialize, Deserialize)]`, never FFI-visible, never
//! constructed from Emerald source) mirroring `JsonValue`'s own six
//! shapes exactly. A `#[derive]`-generated `Deserialize` impl for a
//! concrete enum calls `deserialize_enum`/`deserialize_struct` with
//! real, statically-known variant/field info, never `deserialize_any`
//! — bincode handles that shape natively. `json_to_bincode_value`/
//! `bincode_value_to_json` convert to/from `serde_json::Value` in
//! plain Rust on either side of the actual `bincode` call, so
//! `JsonValue` itself is still the only dynamic-value shape crossing
//! the FFI boundary — `BincodeValue` is purely this module's own
//! internal plumbing around bincode's non-self-describing format, not
//! a second Emerald-visible value type.

use std::ffi::c_void;

/// Purely internal — never FFI-visible, never constructed from
/// Emerald source. Mirrors `JsonValue`'s own six shapes (see this
/// module's own doc comment for why this exists at all: `bincode`'s
/// non-self-describing format cannot `deserialize_any` a
/// `serde_json::Value` directly). `Object` is `Vec<(String,
/// BincodeValue)>`, not a `Map`, to preserve real insertion order
/// through the round trip — `serde_json`'s own `Value::Object` is
/// itself order-preserving here (this crate's `preserve_order`
/// feature, already enabled for `serde_json` in `Cargo.toml`).
#[derive(serde::Serialize, serde::Deserialize)]
enum BincodeValue {
  Null,
  Bool(bool),
  Number(f64),
  String(String),
  Array(Vec<BincodeValue>),
  Object(Vec<(String, BincodeValue)>),
}

fn json_to_bincode_value(v: &serde_json::Value) -> BincodeValue {
  match v {
    serde_json::Value::Null => BincodeValue::Null,
    serde_json::Value::Bool(b) => BincodeValue::Bool(*b),
    serde_json::Value::Number(n) => BincodeValue::Number(n.as_f64().unwrap_or(f64::NAN)),
    serde_json::Value::String(s) => BincodeValue::String(s.clone()),
    serde_json::Value::Array(items) => {
      BincodeValue::Array(items.iter().map(json_to_bincode_value).collect())
    }
    serde_json::Value::Object(map) => BincodeValue::Object(
      map
        .iter()
        .map(|(k, v)| (k.clone(), json_to_bincode_value(v)))
        .collect(),
    ),
  }
}

fn bincode_value_to_json(v: BincodeValue) -> serde_json::Value {
  match v {
    BincodeValue::Null => serde_json::Value::Null,
    BincodeValue::Bool(b) => serde_json::Value::Bool(b),
    BincodeValue::Number(n) => {
      serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, serde_json::Value::Number)
    }
    BincodeValue::String(s) => serde_json::Value::String(s),
    BincodeValue::Array(items) => {
      serde_json::Value::Array(items.into_iter().map(bincode_value_to_json).collect())
    }
    BincodeValue::Object(pairs) => {
      let mut map = serde_json::Map::with_capacity(pairs.len());
      for (k, v) in pairs {
        map.insert(k, bincode_value_to_json(v));
      }
      serde_json::Value::Object(map)
    }
  }
}

// Plan 195 (Typed Domain Errors) convention, applied fresh here (this
// plan lands after plan 195, so it opts in directly rather than
// shipping the older `Result[JsonValue, String]` shape plan 118 used
// before its own plan-195 retrofit — a real, disclosed deviation from
// this plan's own literal text, which still shows the pre-195 shape).
// `BincodeError`'s two variants classify `bincode::error::DecodeError`
// (a real, nineteen-variant enum, verified against the actually-
// vendored `bincode` 2.0.1's own public API directly, not assumed)
// down to the one case worth a caller telling apart — truncated
// input, the routine, always-possible outcome of decoding corrupted
// or attacker-controlled bytes — folding every other variant
// (`Serde`, `Io`, `InvalidIntegerType`, `Utf8`, ...) into `Other`,
// mirroring `RegexError`'s own "keep the convention's required
// minimum" shape (`decimal.rs`'s `DecimalError`/`json.rs`'s
// `JsonError` doc comments name the same rule). Declaration order
// matches `emerald-sema`/`emerald-codegen`'s own mirrored
// `BincodeError` enum byte-for-byte.
//   0 UnexpectedEnd   — bincode::error::DecodeError::UnexpectedEnd
//   1 Other(String)   — every other DecodeError variant
const BINCODE_ERROR_TAG_UNEXPECTED_END: i32 = 0;
const BINCODE_ERROR_TAG_OTHER: i32 = 1;

/// `Bincode.encode(v: JsonValue): Bytes` — `bincode::serde::
/// encode_to_vec` against `bincode::config::standard()` (this plan's
/// only exposed configuration — see this plan's own Decision log for
/// why no varint/endianness knobs are surfaced), lifting `v`'s own
/// real `JsonValue` block into an owned `serde_json::Value` first via
/// `crate::json::lift_json_value` — the same lift `toml.rs`'s own
/// `.to_toml` already reuses rather than re-deriving. Never fails for
/// any value this lift can actually produce, so a genuine `Err` here
/// is a caller-unrecoverable bug, raised as a `NativeError` (the same
/// "ordinary arithmetic overflowing `Decimal::MAX` is a bug, not a
/// `Result`-worthy input" posture `Decimal.add` already establishes)
/// rather than folded into this plan's own `Bytes`-typed (non-
/// `Result`) return signature.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn bincode_encode(obj: *const c_void) -> i64 {
  if obj.is_null() {
    crate::raise_native_error("Bincode.encode: null JsonValue pointer");
  }
  let v = json_to_bincode_value(&crate::json::lift_json_value(obj));
  match bincode::serde::encode_to_vec(&v, bincode::config::standard()) {
    Ok(bytes) => crate::bytes::bytes_from_slice(&bytes),
    Err(e) => crate::raise_native_error(&format!("Bincode.encode: {e}")),
  }
}

fn classify_decode_error(e: bincode::error::DecodeError) -> (i32, String) {
  match e {
    bincode::error::DecodeError::UnexpectedEnd { .. } => (
      BINCODE_ERROR_TAG_UNEXPECTED_END,
      "unexpected end of input".to_string(),
    ),
    other => (BINCODE_ERROR_TAG_OTHER, other.to_string()),
  }
}

/// `Bincode.decode(data: Bytes): Result[JsonValue, BincodeError]` —
/// `bincode::serde::decode_from_slice` against the same `bincode::
/// config::standard()` `.encode` uses, re-lowering the decoded
/// `serde_json::Value` back into a real `JsonValue` block via
/// `crate::json::lower_json_value`.
///
/// # Safety
/// `id` must be a pointer `crate::bytes::bytes_from_slice` (or an
/// equally-shaped native producer) actually returned.
pub unsafe fn bincode_decode(id: i64) -> *mut c_void {
  let bytes = crate::bytes::bytes_as_slice(id);
  match bincode::serde::decode_from_slice::<BincodeValue, _>(bytes, bincode::config::standard()) {
    Ok((v, _consumed)) => {
      let ptr = crate::json::lower_json_value(&bincode_value_to_json(v));
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
      let packed = bincode_encode(doc);
      let result = bincode_decode(packed) as *const i64;
      assert_eq!(*result, 0, "expected Ok");
      let restored = *(result.add(1)) as *const c_void;
      let original = crate::json::lift_json_value(doc);
      let round_tripped = crate::json::lift_json_value(restored);
      assert_eq!(original, round_tripped);
    }
  }

  #[test]
  fn decode_of_truncated_bytes_is_a_typed_err_not_a_panic() {
    unsafe {
      let doc = parse_ok(r#"{"a":1,"b":[1,2,3]}"#);
      let packed = bincode_encode(doc);
      let full = crate::bytes::bytes_as_slice(packed);
      // A truncated copy of a real, valid encoding — genuinely
      // malformed input, not an empty/degenerate edge case.
      let truncated = crate::bytes::bytes_from_slice(&full[..full.len() / 2]);
      let result = bincode_decode(truncated) as *const i64;
      assert_eq!(*result, 1, "expected Err for truncated input");
      let err_block = *(result.add(1)) as *const i64;
      let tag = *err_block;
      assert!(
        tag == BINCODE_ERROR_TAG_UNEXPECTED_END as i64 || tag == BINCODE_ERROR_TAG_OTHER as i64
      );
    }
  }

  #[test]
  fn decode_of_garbage_bytes_is_a_typed_err_not_a_panic() {
    unsafe {
      let garbage = crate::bytes::bytes_from_slice(&[0xff, 0x00, 0xff, 0x00, 0xff]);
      let result = bincode_decode(garbage) as *const i64;
      assert_eq!(*result, 1, "expected Err for garbage input");
    }
  }
}
