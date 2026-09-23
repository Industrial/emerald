//! Plan 163 (Arbitrary-Precision Integers & Decimals), `Decimal` half —
//! deliberately NOT a `crate::handle`-registry opaque handle the way
//! `bignum.rs`'s `BigInt` is. `rust_decimal::Decimal` is fixed at
//! exactly 16 bytes (its own documented `serialize()`/`deserialize
//! ([u8; 16])` byte-for-byte contract, verified against the actually-
//! vendored `rust_decimal` 1.43.0 directly) — every native call in
//! this module reconstructs a real `rust_decimal::Decimal` from the
//! Emerald-visible two-`Int64`-field packed representation (`lo`/`hi`,
//! the first/second 8 bytes of that 16-byte wire format, read via
//! `i64::from_le_bytes`/written via `i64::to_le_bytes` — an arbitrary
//! but fixed, internally-consistent byte order, never observed from
//! Emerald source, which never reads `.lo`/`.hi` directly), and
//! re-serializes the result back into a fresh two-`Int64` block before
//! returning. `Decimal` values are therefore never a `crate::handle`
//! registry entry — no leak the way `BigInt`'s own handles leak —
//! though each native call below still allocates one small, fixed
//! 16-byte `emerald_alloc` block for its own return value, the same
//! way every other pointer-sized Emerald value (a `String`, a
//! `JsonValue` block, ...) already does.
//!
//! `Decimal.from_s` uses `rust_decimal::Decimal::from_str_exact`, NOT
//! the crate's own looser `from_str` (which accepts scientific
//! notation and silently rounds beyond the type's maximum
//! representable scale) — accepting silent precision loss here would
//! directly undercut this whole plan's reason to exist (plan 163's own
//! Decision log).

use std::ffi::c_void;
use std::os::raw::c_char;

use rust_decimal::Decimal;

// Plan 195 (Typed Domain Errors): `DecimalError`'s own variant tags,
// classifying the real, six-variant `rust_decimal::Error` enum
// (verified against the actually-vendored `rust_decimal` 1.43.0's own
// `src/error.rs` directly, not assumed) that `Decimal.from_s`'s own
// `Decimal::from_str_exact` returns, plus one synthetic
// `DivisionByZero` tag this domain adds itself for `.div` (whose own
// `checked_div` returns a bare `Option<Decimal>` on both
// division-by-zero and overflow, with no error object of its own to
// distinguish the two — `.div`'s own implementation below checks the
// divisor for zero first). Declaration order matches `emerald-sema`/
// `emerald-codegen`'s own mirrored `DecimalError` enum byte-for-byte.
// Every variant carries a `String` message (or no field at all) —
// never a raw `Int64` payload — because the shared FFI mechanism this
// domain builds on (`emerald_rt_result_err_tagged`, plan 195) always
// stores `[tag: i64][msg: *const c_char]`; `ScaleExceeded`'s own scale
// number is folded into its message text rather than carried as a
// second field for exactly this reason.
//   0 Syntax(String)        — rust_decimal::Error::ErrorString
//   1 ExceedsMax            — rust_decimal::Error::ExceedsMaximumPossibleValue
//   2 BelowMin              — rust_decimal::Error::LessThanMinimumPossibleValue
//   3 Underflow             — rust_decimal::Error::Underflow
//   4 ScaleExceeded(String) — rust_decimal::Error::ScaleExceedsMaximumPrecision(u32)
//   5 DivisionByZero        — synthetic, `.div` only
//   6 Other(String)         — rust_decimal::Error::ConversionTo, or any
//                             future variant this match doesn't yet
//                             name individually, or `.div`'s own
//                             generic overflow fallback
const DECIMAL_ERROR_TAG_SYNTAX: i32 = 0;
const DECIMAL_ERROR_TAG_EXCEEDS_MAX: i32 = 1;
const DECIMAL_ERROR_TAG_BELOW_MIN: i32 = 2;
const DECIMAL_ERROR_TAG_UNDERFLOW: i32 = 3;
const DECIMAL_ERROR_TAG_SCALE_EXCEEDED: i32 = 4;
const DECIMAL_ERROR_TAG_DIVISION_BY_ZERO: i32 = 5;
const DECIMAL_ERROR_TAG_OTHER: i32 = 6;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// Reads the receiver's own packed `[lo: i64][hi: i64]` block (16
/// bytes, `emerald_alloc`-backed) back into a real `rust_decimal::
/// Decimal` via its own documented `deserialize([u8; 16])` contract.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block —
/// every `Decimal` value this module itself ever produces satisfies
/// this.
unsafe fn decode(ptr: *const i64) -> Decimal {
  let lo = *ptr;
  let hi = *ptr.add(1);
  let mut bytes = [0u8; 16];
  bytes[0..8].copy_from_slice(&lo.to_le_bytes());
  bytes[8..16].copy_from_slice(&hi.to_le_bytes());
  Decimal::deserialize(bytes)
}

/// Re-serializes `d` into a fresh, `emerald_alloc`-backed `[lo: i64]
/// [hi: i64]` block via `Decimal::serialize`'s own documented 16-byte
/// contract — the only way this module ever hands a `Decimal` value
/// back to Emerald.
///
/// # Safety
/// Always safe to call.
unsafe fn encode(d: Decimal) -> *mut c_void {
  let bytes = d.serialize();
  let lo = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
  let hi = i64::from_le_bytes(bytes[8..16].try_into().unwrap());
  let ptr = crate::emerald_alloc(16) as *mut i64;
  *ptr = lo;
  *ptr.add(1) = hi;
  ptr as *mut c_void
}

fn classify_error(e: rust_decimal::Error) -> (i32, String) {
  match e {
    rust_decimal::Error::ErrorString(msg) => (DECIMAL_ERROR_TAG_SYNTAX, msg),
    rust_decimal::Error::ExceedsMaximumPossibleValue => (
      DECIMAL_ERROR_TAG_EXCEEDS_MAX,
      "value exceeds Decimal::MAX".to_string(),
    ),
    rust_decimal::Error::LessThanMinimumPossibleValue => (
      DECIMAL_ERROR_TAG_BELOW_MIN,
      "value is less than Decimal::MIN".to_string(),
    ),
    rust_decimal::Error::Underflow => (
      DECIMAL_ERROR_TAG_UNDERFLOW,
      "value has more fractional digits than Decimal can represent".to_string(),
    ),
    rust_decimal::Error::ScaleExceedsMaximumPrecision(scale) => (
      DECIMAL_ERROR_TAG_SCALE_EXCEEDED,
      format!("scale {scale} exceeds Decimal's maximum precision"),
    ),
    rust_decimal::Error::ConversionTo(ty) => (
      DECIMAL_ERROR_TAG_OTHER,
      format!("conversion to {ty} failed"),
    ),
  }
}

/// `Decimal.from_s(s: String): Result[Decimal, DecimalError]` —
/// `Decimal::from_str_exact`, rejecting silent precision loss (see
/// this module's own doc comment for why `from_str` is deliberately
/// not used).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn decimal_from_s(s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(DECIMAL_ERROR_TAG_OTHER, &e),
  };
  match Decimal::from_str_exact(s) {
    Ok(d) => crate::emerald_rt_result_ok(encode(d) as i64),
    Err(e) => {
      let (tag, msg) = classify_error(e);
      crate::emerald_rt_result_err_tagged_str(tag, &msg)
    }
  }
}

/// `.to_s(self): String` — exact `Display`, no exponential notation,
/// no rounding.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block.
pub unsafe fn decimal_to_s(ptr: *const i64) -> *const c_char {
  let d = decode(ptr);
  crate::alloc_and_copy_str(&d.to_string())
}

/// `.add(self, other: Decimal): Decimal` — `Decimal::checked_add`,
/// raising a `NativeError` on overflow (this plan's own worked
/// Concrete Proof never overflows; ordinary arithmetic overflowing
/// `Decimal::MAX` — roughly 7.9×10^28 — is treated as a genuine
/// misuse, the same disclosed rule `BigInt.factorial`'s own
/// sanity-bound rejection already follows, not an anticipated
/// `Result`-worthy input for `.add`/`.sub`/`.mul` — unlike `.div`'s
/// own division-by-zero below, a real, ordinary-caller-recoverable
/// condition this plan does route through `Result`).
///
/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
pub unsafe fn decimal_add(ptr: *const i64, other_ptr: *const i64) -> *mut c_void {
  let a = decode(ptr);
  let b = decode(other_ptr);
  match a.checked_add(b) {
    Some(sum) => encode(sum),
    None => crate::raise_native_error("Decimal.add: overflow"),
  }
}

/// `.sub(self, other: Decimal): Decimal` — same shape as `.add` above.
///
/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
pub unsafe fn decimal_sub(ptr: *const i64, other_ptr: *const i64) -> *mut c_void {
  let a = decode(ptr);
  let b = decode(other_ptr);
  match a.checked_sub(b) {
    Some(diff) => encode(diff),
    None => crate::raise_native_error("Decimal.sub: overflow"),
  }
}

/// `.mul(self, other: Decimal): Decimal` — same shape as `.add` above.
///
/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
pub unsafe fn decimal_mul(ptr: *const i64, other_ptr: *const i64) -> *mut c_void {
  let a = decode(ptr);
  let b = decode(other_ptr);
  match a.checked_mul(b) {
    Some(prod) => encode(prod),
    None => crate::raise_native_error("Decimal.mul: overflow"),
  }
}

/// `.div(self, other: Decimal): Result[Decimal, DecimalError]` —
/// `Decimal::checked_div`, which returns a bare `Option<Decimal>` on
/// BOTH division-by-zero and overflow with no error object of its own
/// to distinguish them; this function checks the divisor for zero
/// itself first so the two real, distinct failure modes stay
/// distinguishable to an Emerald caller.
///
/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed block.
pub unsafe fn decimal_div(ptr: *const i64, other_ptr: *const i64) -> *mut c_void {
  let a = decode(ptr);
  let b = decode(other_ptr);
  if b.is_zero() {
    return crate::emerald_rt_result_err_tagged_str(
      DECIMAL_ERROR_TAG_DIVISION_BY_ZERO,
      "division by zero",
    );
  }
  match a.checked_div(b) {
    Some(q) => crate::emerald_rt_result_ok(encode(q) as i64),
    None => crate::emerald_rt_result_err_tagged_str(DECIMAL_ERROR_TAG_OTHER, "decimal overflow"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn from_s_ok(s: &str) -> *const i64 {
    let cs = c(s);
    let result_ptr = decimal_from_s(cs.as_ptr()) as *const i64;
    assert_eq!(*result_ptr, 0, "expected Ok discriminant");
    *(result_ptr.add(1)) as *const i64
  }

  #[test]
  fn zero_point_one_plus_zero_point_two_is_exactly_zero_point_three() {
    unsafe {
      let a = from_s_ok("0.1");
      let b = from_s_ok("0.2");
      let sum = decimal_add(a, b) as *const i64;
      let s_ptr = decimal_to_s(sum);
      let s = std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap();
      assert_eq!(s, "0.3");
    }
  }

  #[test]
  fn round_trip_through_the_16_byte_pack_loses_no_bits() {
    unsafe {
      let a = from_s_ok("12345.6789");
      let s = std::ffi::CStr::from_ptr(decimal_to_s(a)).to_str().unwrap();
      assert_eq!(s, "12345.6789");
    }
  }

  #[test]
  fn from_s_of_a_malformed_string_is_a_real_err_not_a_panic() {
    unsafe {
      let bad = c("not a decimal");
      let result_ptr = decimal_from_s(bad.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
    }
  }

  #[test]
  fn division_by_zero_is_a_typed_err_not_a_panic() {
    unsafe {
      let a = from_s_ok("1");
      let zero = from_s_ok("0");
      let result_ptr = decimal_div(a, zero) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let err_block = *(result_ptr.add(1)) as *const i64;
      assert_eq!(*err_block, DECIMAL_ERROR_TAG_DIVISION_BY_ZERO as i64);
    }
  }

  #[test]
  fn sub_and_mul_are_exact_too() {
    unsafe {
      let a = from_s_ok("1.0");
      let b = from_s_ok("0.3");
      let diff = decimal_sub(a, b) as *const i64;
      let s = std::ffi::CStr::from_ptr(decimal_to_s(diff))
        .to_str()
        .unwrap();
      assert_eq!(s, "0.7");
      let c = from_s_ok("2");
      let prod = decimal_mul(diff, c) as *const i64;
      let s = std::ffi::CStr::from_ptr(decimal_to_s(prod))
        .to_str()
        .unwrap();
      assert_eq!(s, "1.4");
    }
  }
}
