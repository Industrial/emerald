//! Plan 163 (Arbitrary-Precision Integers & Decimals), `BigInt` half —
//! a `BigInt` compiler-synthesized `Int64` newtype (the identical
//! zero-cost opaque-handle shape plan 122's `Regex`/plan 124's
//! `XmlReader` already use, reusing `crate::handle`'s registry
//! directly), wrapping `num_bigint::BigInt`. `num_bigint::BigInt`'s
//! own internal `Vec<u32>` digit storage is genuinely unbounded, so no
//! fixed number of `Int64` scalar fields could ever hold it — the
//! reason this half of the plan is a handle and not a packed-field
//! class the way `decimal.rs`'s `Decimal` is.
//!
//! Every `.add`/`.mul` call allocates a fresh boxed `num_bigint::
//! BigInt` and returns a NEW handle — the previous operand handles
//! are never freed, the same accepted, disclosed leak plan 93's own
//! Decision log already establishes for every handle-shaped value in
//! this crate (`emerald_alloc`: "no free... no lifetime tracking").
//!
//! `.to_s()` is the ONLY way a `BigInt` value ever becomes visible to
//! the rest of Emerald's type system — no `.to_i64()` best-effort
//! truncate escape hatch exists (this plan's own Decision log: adding
//! one would be exactly the silent-precision-loss trap `Decimal.
//! from_s`'s own `from_str_exact`-over-`from_str` choice, in
//! `decimal.rs`, deliberately rejects).

use crate::handle::{handle_alloc, handle_get_mut};
use num_bigint::BigInt;
use std::ffi::c_void;
use std::os::raw::c_char;

const TAG: &str = "BigInt";

// Plan 195 (Typed Domain Errors): `BigIntError`'s own single variant
// tag — `num_bigint::BigInt::parse_bytes` (verified against the
// actually-vendored `num-bigint` 0.4.8) returns a bare `Option<
// BigInt>`, not a real Rust error TYPE at all, so there is nothing to
// classify beyond this domain's own required minimum escape hatch.
//   0 Other(String)
const BIGINT_ERROR_TAG_OTHER: i32 = 0;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `BigInt.from_i64(n: Int64): BigInt` — infallible, every `Int64`
/// value is a valid `BigInt`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn bigint_from_i64(n: i64) -> i64 {
  handle_alloc(Box::new(BigInt::from(n)), TAG)
}

/// `BigInt.from_s(s: String): Result[BigInt, BigIntError]` — wraps
/// `num_bigint::BigInt::parse_bytes(s.as_bytes(), 10)`, `Err` on a
/// malformed digit string (that function's own real, only failure
/// mode — a bare `None`, no message of its own, so this domain
/// synthesizes one).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn bigint_from_s(s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(BIGINT_ERROR_TAG_OTHER, &e),
  };
  match BigInt::parse_bytes(s.as_bytes(), 10) {
    Some(n) => {
      let id = handle_alloc(Box::new(n), TAG);
      crate::emerald_rt_result_ok(id)
    }
    None => crate::emerald_rt_result_err_tagged_str(
      BIGINT_ERROR_TAG_OTHER,
      &format!("invalid digit string for BigInt: {s:?}"),
    ),
  }
}

/// The pure, panic-boundary-free core of `BigInt.factorial` — split out
/// specifically so its own bounds check is unit-testable without ever
/// calling `crate::raise_native_error`/`emerald_raise`: `lib.rs`'s own
/// `build_native_error_instance` doc comment discloses that unwinding a
/// panic across a real `extern "C" fn` boundary (even this crate's own
/// test-build stub) aborts the process rather than being catchable via
/// `catch_unwind` — verified empirically, not assumed, by hitting
/// exactly that abort first and rewriting this function to avoid it.
fn bigint_factorial_checked(n: i64) -> Result<BigInt, String> {
  if !(0..=10000).contains(&n) {
    return Err(format!(
      "BigInt.factorial: n must be between 0 and 10000, found {n}"
    ));
  }
  let mut acc = BigInt::from(1);
  for i in 2..=n {
    acc *= BigInt::from(i);
  }
  Ok(acc)
}

/// `BigInt.factorial(n: Int64): BigInt` — an internal, ITERATIVE (not
/// recursive) accumulator loop, `n` bounds-checked to a disclosed sane
/// maximum (`n > 10000` — and `n < 0`, a real, disclosed input this
/// plan's own text didn't separately name — is rejected via a real
/// `NativeError`, a genuine misuse of this function's documented
/// contract, not an anticipated `Result`-worthy input — plan 163's own
/// Decision log).
///
/// # Safety
/// Always safe to call.
pub unsafe fn bigint_factorial(n: i64) -> i64 {
  match bigint_factorial_checked(n) {
    Ok(acc) => handle_alloc(Box::new(acc), TAG),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.to_s(self): String` — exact, arbitrary-length decimal text, the
/// ONLY lossless way a `BigInt` value ever reaches Emerald's other
/// types (`Display`).
///
/// # Safety
/// Always safe to call with a handle this module itself issued.
pub unsafe fn bigint_to_s(id: i64) -> *const c_char {
  match handle_get_mut::<BigInt, String>(id, TAG, |n| n.to_string()) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.add(self, other: BigInt): BigInt` — allocates a fresh boxed
/// result; the operand handles are never freed (plan 163's own
/// disclosed, accepted leak — see this module's own doc comment).
///
/// # Safety
/// Always safe to call with handles this module itself issued.
pub unsafe fn bigint_add(id: i64, other_id: i64) -> i64 {
  let other = match handle_get_mut::<BigInt, BigInt>(other_id, TAG, |n| n.clone()) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  };
  match handle_get_mut::<BigInt, BigInt>(id, TAG, |n| &*n + &other) {
    Ok(sum) => handle_alloc(Box::new(sum), TAG),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.mul(self, other: BigInt): BigInt` — same shape as `.add` above.
///
/// # Safety
/// Always safe to call with handles this module itself issued.
pub unsafe fn bigint_mul(id: i64, other_id: i64) -> i64 {
  let other = match handle_get_mut::<BigInt, BigInt>(other_id, TAG, |n| n.clone()) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  };
  match handle_get_mut::<BigInt, BigInt>(id, TAG, |n| &*n * &other) {
    Ok(prod) => handle_alloc(Box::new(prod), TAG),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  #[test]
  fn factorial_20_fits_int64_and_matches_the_known_exact_value() {
    unsafe {
      let id = bigint_factorial(20);
      let s_ptr = bigint_to_s(id);
      let s = std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap();
      assert_eq!(s, "2432902008176640000");
    }
  }

  #[test]
  fn factorial_25_exceeds_int64_and_is_still_exact() {
    unsafe {
      let id = bigint_factorial(25);
      let s_ptr = bigint_to_s(id);
      let s = std::ffi::CStr::from_ptr(s_ptr).to_str().unwrap();
      assert_eq!(s, "15511210043330985984000000");
    }
  }

  #[test]
  fn from_s_of_a_malformed_digit_string_is_a_real_err_not_a_panic() {
    unsafe {
      let bad = c("not a number");
      let result_ptr = bigint_from_s(bad.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let err_block = *(result_ptr.add(1)) as *const i64;
      assert_eq!(*err_block, BIGINT_ERROR_TAG_OTHER as i64);
    }
  }

  #[test]
  fn add_and_mul_round_trip_through_to_s() {
    unsafe {
      let a = bigint_from_i64(20);
      let b = bigint_from_i64(22);
      let sum = bigint_add(a, b);
      let sum_s = std::ffi::CStr::from_ptr(bigint_to_s(sum)).to_str().unwrap();
      assert_eq!(sum_s, "42");
      let prod = bigint_mul(a, b);
      let prod_s = std::ffi::CStr::from_ptr(bigint_to_s(prod))
        .to_str()
        .unwrap();
      assert_eq!(prod_s, "440");
    }
  }

  #[test]
  fn factorial_rejects_n_above_the_sanity_ceiling() {
    assert!(bigint_factorial_checked(10001).is_err());
  }

  #[test]
  fn factorial_rejects_negative_n() {
    assert!(bigint_factorial_checked(-1).is_err());
  }
}
