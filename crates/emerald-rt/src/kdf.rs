//! Plan 115 (Key Derivation Functions) — `Kdf.hkdf`/`Kdf.pbkdf2`,
//! wrapping RustCrypto's `hkdf`/`pbkdf2` crates. Two functions with
//! genuinely different jobs, per RFC 5869's own "Applications"
//! section: HKDF concentrates and re-shapes an already-strong secret
//! (a Diffie-Hellman shared value) into independent keys, fast by
//! design; PBKDF2 stretches a low-entropy human password into a key,
//! slow by design. Plan 112's Argon2id remains the recommended choice
//! for new password-based key derivation — `Kdf.pbkdf2` exists for
//! interop with systems that already mandate PBKDF2 specifically.
//!
//! Both functions take and return hex-encoded `String`s, never raw
//! bytes, the same convention plan 113's `Random.secure_hex` already
//! establishes — Emerald's own null-terminated `String` cannot safely
//! carry arbitrary binary IKM/salt/output (plan 59's finding), and a
//! value produced by `Random.secure_hex` can be passed directly as
//! `Kdf.hkdf`'s `salt` argument with no re-encoding step.

use std::os::raw::c_char;

unsafe fn read_hex(s: *const c_char) -> Result<Vec<u8>, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  let text = std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())?;
  hex::decode(text).map_err(|e| format!("input is not valid hex: {e}"))
}

/// `Kdf.hkdf(ikm: String, salt: String, info: String, length: Int64): String`
/// — hex in, hex out. `length` beyond SHA-256's `L <= 255*32` RFC 5869
/// ceiling raises rather than panicking.
///
/// # Safety
/// `ikm`/`salt`/`info`, if non-null, must point to valid, NUL-
/// terminated hex-text C strings.
pub unsafe fn kdf_hkdf(
  ikm: *const c_char,
  salt: *const c_char,
  info: *const c_char,
  length: i64,
) -> *const c_char {
  let ikm = match read_hex(ikm) {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&e),
  };
  let salt = match read_hex(salt) {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&e),
  };
  let info = match read_hex(info) {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&e),
  };
  if !(0..=(255 * 32)).contains(&length) {
    crate::raise_native_error(
      "Kdf.hkdf: length must be in 0..=8160 (SHA-256's 255*HashLen ceiling)",
    );
  }
  let hk = hkdf::Hkdf::<sha2::Sha256>::new(Some(&salt), &ikm);
  let mut okm = vec![0u8; length as usize];
  match hk.expand(&info, &mut okm) {
    Ok(()) => crate::alloc_and_copy_str(&hex::encode(&okm)),
    Err(e) => crate::raise_native_error(&format!("Kdf.hkdf: {e}")),
  }
}

/// `Kdf.pbkdf2(password: String, salt: String, iterations: Int64, length: Int64): String`
/// — hex-encoded salt/output; `password` is plain text (PBKDF2's own
/// real input shape, not hex — a human password is not hex data).
///
/// # Safety
/// `password`/`salt`, if non-null, must point to valid, NUL-terminated
/// C strings (`salt` hex text, `password` plain UTF-8 text).
pub unsafe fn kdf_pbkdf2(
  password: *const c_char,
  salt: *const c_char,
  iterations: i64,
  length: i64,
) -> *const c_char {
  if password.is_null() {
    crate::raise_native_error("Kdf.pbkdf2: null string pointer");
  }
  let password = match std::ffi::CStr::from_ptr(password).to_str() {
    Ok(s) => s,
    Err(_) => crate::raise_native_error("Kdf.pbkdf2: password is not valid UTF-8"),
  };
  let salt = match read_hex(salt) {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&e),
  };
  if iterations <= 0 {
    crate::raise_native_error("Kdf.pbkdf2: iterations must be positive");
  }
  if length <= 0 {
    crate::raise_native_error("Kdf.pbkdf2: length must be positive");
  }
  let mut key = vec![0u8; length as usize];
  pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, iterations as u32, &mut key);
  crate::alloc_and_copy_str(&hex::encode(&key))
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  // RFC 5869 Test Case 1's own published vector.
  #[test]
  fn hkdf_matches_rfc_5869_test_case_1() {
    unsafe {
      let ikm = c("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
      let salt = c("000102030405060708090a0b0c");
      let info = c("f0f1f2f3f4f5f6f7f8f9");
      let okm = cstr(kdf_hkdf(ikm.as_ptr(), salt.as_ptr(), info.as_ptr(), 42));
      assert_eq!(
        okm,
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
      );
    }
  }

  // The `pbkdf2` crate's own published doctest vector.
  #[test]
  fn pbkdf2_matches_the_crates_own_doctest_vector() {
    unsafe {
      let password = c("password");
      let salt = c("73616c74"); // hex("salt")
      let key = cstr(kdf_pbkdf2(password.as_ptr(), salt.as_ptr(), 600_000, 20));
      assert_eq!(key, "669cfe52482116fda1aa2cbe409b2f56c8e45637");
    }
  }
}
