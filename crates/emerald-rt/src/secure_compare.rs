//! Plan 117 (Constant-Time Comparison) — `SecureCompare.eq`, wrapping
//! `subtle::ConstantTimeEq`. This plan's own central, cross-cutting
//! requirement (every secret-comparison in this stdlib batch must
//! route through this or an equivalent RustCrypto-ecosystem constant-
//! time primitive, never Rust's `==`/Emerald's own `String`/`Array`
//! equality) is a project rule stated in prose, not a static check —
//! see this plan's own Decision log.
//!
//! Real, disclosed exception, not a bug: `subtle`'s own slice
//! comparison short-circuits on a length mismatch, verified directly
//! against its own docs. Accepted because every real caller in this
//! batch compares fixed/public-length secrets (a MAC tag, a PHC
//! string, a base64url signature segment) where length itself isn't
//! the secret.

use subtle::ConstantTimeEq;

/// `SecureCompare.eq(a: String, b: String): Boolean`.
///
/// # Safety
/// `a`/`b`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn secure_compare(
  a: *const std::os::raw::c_char,
  b: *const std::os::raw::c_char,
) -> i64 {
  if a.is_null() || b.is_null() {
    crate::raise_native_error("SecureCompare.eq: null string pointer");
  }
  let a = std::ffi::CStr::from_ptr(a).to_bytes();
  let b = std::ffi::CStr::from_ptr(b).to_bytes();
  if bool::from(a.ct_eq(b)) {
    1
  } else {
    0
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  unsafe fn eq(a: &str, b: &str) -> bool {
    let a = CString::new(a).unwrap();
    let b = CString::new(b).unwrap();
    secure_compare(a.as_ptr(), b.as_ptr()) == 1
  }

  #[test]
  fn identical_strings_are_equal() {
    unsafe {
      assert!(eq(
        "s3cr3t-api-key-do-not-leak",
        "s3cr3t-api-key-do-not-leak"
      ));
    }
  }

  #[test]
  fn different_strings_of_equal_length_are_unequal() {
    unsafe {
      assert!(!eq(
        "s3cr3t-api-key-do-not-leak",
        "s3cr3t-api-key-do-not-leek"
      ));
    }
  }

  #[test]
  fn different_strings_of_different_length_are_unequal_via_the_short_circuit_path() {
    unsafe {
      assert!(!eq("s3cr3t-api-key-do-not-leak", "too-short"));
    }
  }
}
