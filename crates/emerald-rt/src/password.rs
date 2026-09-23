//! Plan 112 (Password Hashing) — `Password.hash`/`.verify`, wrapping
//! `argon2` (RustCrypto, defaulting to the Argon2id variant). This
//! module is deliberately separate from `hashing.rs`'s `Hash`-family
//! namespace — see this plan's own Decision log for why a shared
//! namespace would invite exactly the "SHA-256 a password" mistake
//! this module exists to make structurally impossible.
//!
//! Salt generation is automatic and internal — `PasswordHasher::
//! hash_password`'s own single-argument form generates a fresh random
//! salt internally via `password-hash`'s own `getrandom`-backed
//! generator — there is no Emerald-visible salt parameter anywhere in
//! this module. Work-factor parameters (memory/time/parallelism) are
//! hardcoded to OWASP's Password Storage Cheat Sheet's own current
//! second-tier recommendation (`m=19456 KiB, t=2, p=1`) and are not
//! an Emerald-level tuning knob in this plan — raising them requires
//! a source change here, not an Emerald-level call. See plan 112's
//! own Decision log for why both of these are deliberate, not
//! missing features.

use argon2::{
  password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
  Algorithm, Argon2, Params, Version,
};
use std::os::raw::c_char;

/// OWASP's own current second-tier recommendation — a real,
/// currently-recommended number, not a guess (see this module's own
/// doc comment and plan 112's Decision log).
fn owasp_params() -> Params {
  Params::new(19456, 2, 1, None).expect("OWASP m=19456,t=2,p=1 is a valid Argon2 parameter set")
}

/// `Password.hash(plaintext: String): String` — returns a full PHC
/// string (`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`). A fresh
/// random salt is generated on every call, so hashing the same
/// plaintext twice produces two different strings by design.
///
/// # Safety
/// `plaintext`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn password_hash(plaintext: *const c_char) -> *const c_char {
  if plaintext.is_null() {
    crate::raise_native_error("Password.hash: null string pointer");
  }
  let plaintext = match std::ffi::CStr::from_ptr(plaintext).to_str() {
    Ok(s) => s,
    Err(_) => crate::raise_native_error("Password.hash: input is not valid UTF-8"),
  };
  if plaintext.is_empty() {
    crate::raise_native_error("Password.hash: plaintext must not be empty");
  }
  // `PasswordHasher::hash_password` (the crate's own high-level API)
  // generates a fresh random salt internally via `password-hash`'s
  // own `getrandom`-backed generator — no explicit salt value is
  // constructed anywhere in this function.
  let argon2 = Argon2::new(Algorithm::default(), Version::default(), owasp_params());
  match argon2.hash_password(plaintext.as_bytes()) {
    Ok(hash) => crate::alloc_and_copy_str(&hash.to_string()),
    Err(e) => crate::raise_native_error(&format!("Password.hash: {e}")),
  }
}

/// `Password.verify(plaintext: String, stored_hash: String): Boolean`
/// — every failure mode (wrong password, a `stored_hash` that fails
/// to parse as a valid PHC string, a null/non-UTF-8 pointer) collapses
/// to `false` (`0`), never a distinguishable error. This is deliberate
/// per plan 112's own Decision log: a caller must never be able to
/// tell "your stored hash is corrupted" apart from "the password
/// didn't match" from the return value alone. The comparison itself
/// is `argon2`'s own internal constant-time check (plan 117's rule
/// satisfied by inheritance, not a second hand-rolled comparison).
///
/// # Safety
/// `plaintext`/`stored_hash`, if non-null, must point to valid,
/// NUL-terminated C strings.
pub unsafe fn password_verify(plaintext: *const c_char, stored_hash: *const c_char) -> i64 {
  if plaintext.is_null() || stored_hash.is_null() {
    return 0;
  }
  let plaintext = match std::ffi::CStr::from_ptr(plaintext).to_str() {
    Ok(s) => s,
    Err(_) => return 0,
  };
  let stored_hash = match std::ffi::CStr::from_ptr(stored_hash).to_str() {
    Ok(s) => s,
    Err(_) => return 0,
  };
  let parsed = match PasswordHash::new(stored_hash) {
    Ok(h) => h,
    Err(_) => return 0,
  };
  match Argon2::default().verify_password(plaintext.as_bytes(), &parsed) {
    Ok(()) => 1,
    Err(_) => 0,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  unsafe fn hash(plaintext: &str) -> String {
    let p = CString::new(plaintext).unwrap();
    let h = password_hash(p.as_ptr());
    std::ffi::CStr::from_ptr(h).to_str().unwrap().to_string()
  }

  unsafe fn verify(plaintext: &str, stored_hash: &str) -> bool {
    let p = CString::new(plaintext).unwrap();
    let s = CString::new(stored_hash).unwrap();
    password_verify(p.as_ptr(), s.as_ptr()) == 1
  }

  #[test]
  fn round_trip_hash_then_verify_the_same_plaintext_succeeds() {
    unsafe {
      let h = hash("correct horse battery staple");
      assert!(verify("correct horse battery staple", &h));
    }
  }

  #[test]
  fn verify_against_the_wrong_plaintext_fails() {
    unsafe {
      let h = hash("correct horse battery staple");
      assert!(!verify("wrong password", &h));
    }
  }

  #[test]
  fn verify_against_a_hand_corrupted_phc_string_fails_not_panics() {
    unsafe {
      let mut h = hash("correct horse battery staple");
      // Flip one character deep in the base64 hash segment (the very
      // last character, always part of the digest, never the padding-
      // free base64ct alphabet's own structural `$` delimiters).
      let last = h.pop().unwrap();
      let flipped = if last == 'A' { 'B' } else { 'A' };
      h.push(flipped);
      assert!(!verify("correct horse battery staple", &h));
    }
  }

  #[test]
  fn verify_against_a_completely_malformed_stored_hash_fails_not_panics() {
    unsafe {
      assert!(!verify("correct horse battery staple", "not a PHC string"));
    }
  }

  #[test]
  fn two_hashes_of_the_same_plaintext_are_different_strings() {
    unsafe {
      let a = hash("correct horse battery staple");
      let b = hash("correct horse battery staple");
      assert_ne!(a, b, "salting must produce a different hash every call");
    }
  }
}
