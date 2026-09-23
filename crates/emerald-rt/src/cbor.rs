//! Plan 189 (CBOR Binary Format) — wraps `ciborium` 0.2.2's
//! `into_writer`/`from_reader` against `ciborium::Value`, reusing
//! plan 118's `JsonValue` (via the same `crate::json::
//! lift_json_value`/`lower_json_value` helpers `bincode.rs`/
//! `msgpack.rs` immediately above already establish) as the *only*
//! Emerald-visible dynamic-value shape — no second dynamic-value
//! representation. This plan's own Decision log states the reason
//! plainly: CBOR and JSON solve the identical "represent an
//! arbitrary, schema-less tree of scalars/arrays/maps" problem, so
//! Emerald needs exactly one answer to it, not two.
//!
//! Two real, disclosed asymmetries this module's own conversion has
//! to name explicitly, neither of which `bincode.rs`/`msgpack.rs`
//! (whose wrapped formats have no equivalent) had to handle:
//!
//! 1. `ciborium::Value` has two real variants with no `JsonValue`
//!    counterpart at all — `Bytes` (a native binary-blob type; JSON
//!    has none) and `Tag` (RFC 8949's own tag mechanism; JSON has
//!    none of that either). A CBOR map key that isn't itself a
//!    `Text` value (a real, legal CBOR possibility RFC 8949 permits
//!    and JSON's own object-key grammar does not) has no `JsonValue`
//!    counterpart either. Decoding any of the three anywhere in a
//!    document is a genuine, disclosed `Err` (`CborError::Other`),
//!    never a silent, lossy downgrade (e.g. base64-encoding bytes
//!    into a string) — this plan's own Decision log names this
//!    boundary explicitly rather than quietly mishandling it.
//! 2. `ciborium::value::Integer` is a full 128-bit-backed abstract
//!    integer (RFC 8949's own bignum-adjacent range), wider than
//!    `JsonValue`'s own `JsonNumber(Float64)` — every JSON number
//!    already widens to `Float64` (plan 118's own Decision log), so
//!    `cbor_encode` never has this problem (a `JsonValue` number is
//!    always encoded as `Value::Float`, never `Value::Integer`), but
//!    a CBOR integer outside `i64`'s own range on decode is a
//!    genuine, disclosed `Err`, not a silently truncated or wrapped
//!    value.
//!
//! No module-name collision to disclose here the way `bincode.rs`'s
//! own doc comment names one — the wrapped crate is `ciborium`, not
//! `cbor`, so a plain `mod cbor;` in `lib.rs` shadows nothing.

use std::ffi::c_void;

// Plan 195 (Typed Domain Errors) convention, applied fresh here (this
// plan lands after plan 195, so it opts in directly — the same
// `BincodeError`/`MessagePackError` two-variant minimum `bincode.rs`/
// `msgpack.rs` already establish). `CborError` classifies
// `ciborium::de::Error<std::io::Error>` down to the one case worth a
// caller telling apart — a reader running out of bytes mid-decode
// (`Error::Io` wrapping a `std::io::Error` whose own `.kind()` is
// `UnexpectedEof`, the exact condition `std::io::Read`'s own blanket
// `&[u8]` impl produces on a short read, verified directly against
// `ciborium-io` 0.2.2's own `std`-feature blanket impl) — folding
// every other classification (a real CBOR syntax/semantic error, and
// this module's own `Bytes`/`Tag`/non-string-map-key/out-of-`i64`-
// range rejections named in this module's own doc comment above)
// into `Other`. Declaration order matches `emerald-sema`/
// `emerald-codegen`'s own mirrored `CborError` enum byte-for-byte.
//   0 UnexpectedEnd   — a reader running out of bytes mid-decode
//   1 Other(String)   — every other classification
const CBOR_ERROR_TAG_UNEXPECTED_END: i32 = 0;
const CBOR_ERROR_TAG_OTHER: i32 = 1;

/// Mechanical, one-for-one walk from plan 118's own `serde_json::
/// Value` (obtained via `crate::json::lift_json_value`) into
/// `ciborium`'s own dynamic `Value` type — object → `Map`, array →
/// `Array`, string → `Text`, number → `Float` (every `JsonValue`
/// number is already a `Float64` per plan 118's own Decision log, so
/// this direction never needs `Value::Integer` at all), bool →
/// `Bool`, null → `Null`. This is a pure structural re-tagging, not a
/// value-reinterpretation — see this module's own doc comment.
fn json_to_cbor_value(v: &serde_json::Value) -> ciborium::Value {
  match v {
    serde_json::Value::Null => ciborium::Value::Null,
    serde_json::Value::Bool(b) => ciborium::Value::Bool(*b),
    serde_json::Value::Number(n) => ciborium::Value::Float(n.as_f64().unwrap_or(f64::NAN)),
    serde_json::Value::String(s) => ciborium::Value::Text(s.clone()),
    serde_json::Value::Array(items) => {
      ciborium::Value::Array(items.iter().map(json_to_cbor_value).collect())
    }
    serde_json::Value::Object(map) => ciborium::Value::Map(
      map
        .iter()
        .map(|(k, v)| (ciborium::Value::Text(k.clone()), json_to_cbor_value(v)))
        .collect(),
    ),
  }
}

/// The reverse walk — the exact reverse of `json_to_cbor_value`, with
/// the real, disclosed asymmetries this module's own doc comment
/// names: `Value::Bytes`/`Value::Tag`, a non-`Text` map key, and an
/// out-of-`i64`-range `Value::Integer` each produce `Err(String)`
/// rather than a silent, lossy conversion. `#[non_exhaustive]` on
/// `ciborium::Value` itself (a real, current attribute on that type)
/// is why this match needs its own trailing `_` arm even though every
/// variant that attribute currently admits is already named above it.
fn cbor_value_to_json(v: ciborium::Value) -> Result<serde_json::Value, String> {
  match v {
    ciborium::Value::Null => Ok(serde_json::Value::Null),
    ciborium::Value::Bool(b) => Ok(serde_json::Value::Bool(b)),
    ciborium::Value::Integer(int) => {
      let i: i64 = int
        .try_into()
        .map_err(|_| "CBOR integer out of i64 range".to_string())?;
      Ok(
        serde_json::Number::from_f64(i as f64)
          .map_or(serde_json::Value::Null, serde_json::Value::Number),
      )
    }
    ciborium::Value::Float(f) => {
      Ok(serde_json::Number::from_f64(f).map_or(serde_json::Value::Null, serde_json::Value::Number))
    }
    ciborium::Value::Text(s) => Ok(serde_json::Value::String(s)),
    ciborium::Value::Array(items) => {
      let mut out = Vec::with_capacity(items.len());
      for item in items {
        out.push(cbor_value_to_json(item)?);
      }
      Ok(serde_json::Value::Array(out))
    }
    ciborium::Value::Map(pairs) => {
      let mut map = serde_json::Map::with_capacity(pairs.len());
      for (k, v) in pairs {
        let key = match k {
          ciborium::Value::Text(s) => s,
          _ => return Err("CBOR map key is not a string".to_string()),
        };
        map.insert(key, cbor_value_to_json(v)?);
      }
      Ok(serde_json::Value::Object(map))
    }
    ciborium::Value::Bytes(_) => Err("CBOR byte string has no JsonValue counterpart".to_string()),
    ciborium::Value::Tag(_, _) => Err("CBOR tagged value has no JsonValue counterpart".to_string()),
    _ => Err("unsupported CBOR value".to_string()),
  }
}

fn is_unexpected_eof(io_err: &std::io::Error) -> bool {
  io_err.kind() == std::io::ErrorKind::UnexpectedEof
}

fn classify_decode_error(e: ciborium::de::Error<std::io::Error>) -> (i32, String) {
  match &e {
    ciborium::de::Error::Io(io_err) if is_unexpected_eof(io_err) => (
      CBOR_ERROR_TAG_UNEXPECTED_END,
      "unexpected end of input".to_string(),
    ),
    _ => (CBOR_ERROR_TAG_OTHER, e.to_string()),
  }
}

/// `Cbor.encode(v: JsonValue): Bytes` — `ciborium::into_writer`,
/// lifting `v`'s own real `JsonValue` block into an owned
/// `serde_json::Value` first via `crate::json::lift_json_value`, then
/// walking it into `ciborium::Value` via `json_to_cbor_value`. Never
/// fails for any value this lift/walk can actually produce (every
/// `JsonValue` shape has exactly one direct `ciborium::Value`
/// counterpart — see this module's own doc comment), so a genuine
/// `Err` here is a caller-unrecoverable bug, raised as a
/// `NativeError` — the identical posture `bincode.rs`'s/`msgpack.rs`'s
/// own `.encode` already take, for the identical reason.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn cbor_encode(obj: *const c_void) -> i64 {
  if obj.is_null() {
    crate::raise_native_error("Cbor.encode: null JsonValue pointer");
  }
  let v = json_to_cbor_value(&crate::json::lift_json_value(obj));
  let mut buf = Vec::new();
  match ciborium::into_writer(&v, &mut buf) {
    Ok(()) => crate::bytes::bytes_from_slice(&buf),
    Err(e) => crate::raise_native_error(&format!("Cbor.encode: {e}")),
  }
}

/// `Cbor.decode(data: Bytes): Result[JsonValue, CborError]` —
/// `ciborium::from_reader::<ciborium::Value, _>`, re-lowering the
/// decoded `ciborium::Value` tree back into a real `JsonValue` block
/// via `cbor_value_to_json`/`crate::json::lower_json_value`.
///
/// # Safety
/// `id` must be a pointer `crate::bytes::bytes_from_slice` (or an
/// equally-shaped native producer) actually returned.
pub unsafe fn cbor_decode(id: i64) -> *mut c_void {
  let bytes = crate::bytes::bytes_as_slice(id);
  match ciborium::from_reader::<ciborium::Value, _>(bytes) {
    Ok(cbor_val) => match cbor_value_to_json(cbor_val) {
      Ok(json_val) => {
        let ptr = crate::json::lower_json_value(&json_val);
        crate::emerald_rt_result_ok(ptr as i64)
      }
      Err(msg) => crate::emerald_rt_result_err_tagged_str(CBOR_ERROR_TAG_OTHER, &msg),
    },
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
      let packed = cbor_encode(doc);
      let result = cbor_decode(packed) as *const i64;
      assert_eq!(*result, 0, "expected Ok");
      let restored = *(result.add(1)) as *const c_void;
      let original = crate::json::lift_json_value(doc);
      let round_tripped = crate::json::lift_json_value(restored);
      assert_eq!(original, round_tripped);
    }
  }

  // Plan 189's own `leaf-example-and-gate`: every `JsonValue` variant
  // round-tripping individually (object, array, string, integer-
  // valued number, float-valued number, bool, null).
  #[test]
  fn round_trips_every_json_value_variant_individually() {
    let cases = [
      "null",
      "true",
      "false",
      "\"hello\"",
      "42",
      "3.5",
      "[1,2,3]",
      "{\"a\":1,\"b\":2}",
    ];
    for json in cases {
      unsafe {
        let doc = parse_ok(json);
        let packed = cbor_encode(doc);
        let result = cbor_decode(packed) as *const i64;
        assert_eq!(*result, 0, "expected Ok for {json}");
        let restored = *(result.add(1)) as *const c_void;
        let original = crate::json::lift_json_value(doc);
        let round_tripped = crate::json::lift_json_value(restored);
        assert_eq!(original, round_tripped, "mismatch for {json}");
      }
    }
  }

  #[test]
  fn round_trips_a_deeply_nested_object_of_arrays_of_objects() {
    unsafe {
      let doc = parse_ok(
        r#"{"items":[{"id":1,"tags":["x","y"]},{"id":2,"tags":[]}],"meta":{"count":2,"ok":true}}"#,
      );
      let packed = cbor_encode(doc);
      let result = cbor_decode(packed) as *const i64;
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
      let packed = cbor_encode(doc);
      let full = crate::bytes::bytes_as_slice(packed);
      // A truncated copy of a real, valid encoding — genuinely
      // malformed input, not an empty/degenerate edge case.
      let truncated = crate::bytes::bytes_from_slice(&full[..full.len() / 2]);
      let result = cbor_decode(truncated) as *const i64;
      assert_eq!(*result, 1, "expected Err for truncated input");
      let err_block = *(result.add(1)) as *const i64;
      let tag = *err_block;
      assert!(tag == CBOR_ERROR_TAG_UNEXPECTED_END as i64 || tag == CBOR_ERROR_TAG_OTHER as i64);
    }
  }

  #[test]
  fn decode_of_garbage_bytes_is_a_typed_err_not_a_panic() {
    unsafe {
      let garbage = crate::bytes::bytes_from_slice(&[0xff, 0x00, 0xff, 0x00, 0xff]);
      let result = cbor_decode(garbage) as *const i64;
      assert_eq!(*result, 1, "expected Err for garbage input");
    }
  }

  // Plan 189's own disclosed asymmetry: `ciborium::value::Integer` is
  // a full 128-bit-backed abstract integer, wider than `i64` — a real
  // CBOR document encoding an integer outside `i64`'s own range
  // (constructed directly here via `ciborium::Value::Integer`, not
  // through `cbor_encode`, since `cbor_encode`'s own input — a
  // `JsonValue` — can never produce one) must decode to a typed `Err`
  // naming this specific classification, not silently truncate/wrap.
  #[test]
  fn decode_of_an_out_of_i64_range_cbor_integer_is_a_typed_err_not_a_panic() {
    unsafe {
      let huge = ciborium::value::Integer::try_from(u64::MAX as i128).unwrap();
      let v = ciborium::Value::Integer(huge);
      let mut buf = Vec::new();
      ciborium::into_writer(&v, &mut buf).unwrap();
      let packed = crate::bytes::bytes_from_slice(&buf);
      let result = cbor_decode(packed) as *const i64;
      assert_eq!(*result, 1, "expected Err for an out-of-i64-range integer");
      let err_block = *(result.add(1)) as *const i64;
      let tag = *err_block;
      assert_eq!(tag, CBOR_ERROR_TAG_OTHER as i64);
    }
  }

  // Plan 189's own disclosed asymmetry: `Value::Bytes` has no
  // `JsonValue` counterpart — decoding one is a named `Err`, never a
  // silent, lossy downgrade (e.g. base64-encoding it into a string).
  #[test]
  fn decode_of_a_cbor_byte_string_is_a_typed_err_not_a_silent_downgrade() {
    unsafe {
      let v = ciborium::Value::Bytes(vec![1, 2, 3]);
      let mut buf = Vec::new();
      ciborium::into_writer(&v, &mut buf).unwrap();
      let packed = crate::bytes::bytes_from_slice(&buf);
      let result = cbor_decode(packed) as *const i64;
      assert_eq!(*result, 1, "expected Err for a CBOR byte string");
      let err_block = *(result.add(1)) as *const i64;
      let tag = *err_block;
      assert_eq!(tag, CBOR_ERROR_TAG_OTHER as i64);
    }
  }
}
