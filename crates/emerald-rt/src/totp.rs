//! Plan 184 (TOTP/HOTP Two-Factor Authentication) — `Totp`, a
//! compiler-synthesized `Int64` newtype (plan 93's own zero-cost
//! handle shape, reusing `crate::handle`'s registry directly rather
//! than inventing a second resource-lifetime mechanism), wrapping
//! `totp_rs::Totp` (`totp-rs` 6.0.0). RFC 6238 time-based one-time
//! codes at the fixed SHA1 / 6-digit / 30-second-step / 1-step-skew
//! defaults `totp-rs`'s own `Builder::new()` already picks — the same
//! defaults Google Authenticator/Authy/every real authenticator app
//! expects (this plan's own Decision log).
//!
//! Real, disclosed deviation from this plan's own literal text found
//! only by checking the actually-vendored crate: the plan's own text
//! describes a `totp_rs::TOTP::new(Algorithm::SHA1, 6, 1, 30, Secret::
//! Encoded(secret).to_bytes())`-shaped constructor and a `.get_url()`
//! provisioning-URI method. Neither exists in the real, currently
//! pinned `totp-rs` 6.0.0 (verified directly against the vendored
//! crate source in `~/.cargo/registry/src/.../totp-rs-6.0.0/src/`, not
//! assumed from the plan's own text). The real 6.0.0 surface is a
//! `Builder` (`Builder::new().with_secret(..).with_account_name(..).
//! with_issuer(..).build()`), `Secret::try_from_base32`/`Secret::
//! generate()` (the `gen_secret` feature, not a method named `Secret::
//! generate_secret()` as the plan's own text guessed), and `Totp::
//! to_url()` (the `otpauth` feature) — this module is written against
//! the real, checked API.
//!
//! A second real, disclosed finding: the plan's own Concrete Proof
//! secret literal, `"JBSWY3DPEHPK3PXP"`, base32-decodes to only 10
//! bytes (80 bits) — below the 128-bit minimum `totp_rs::Builder::
//! build()` itself enforces (`rfc::assert_secret_length`, verified
//! directly in the vendored source), so it would make `Totp.new`
//! return `Err(TotpError::SecretTooShort)` rather than the plan's own
//! expected `Ok`. `examples/totp_2fa.em` (this module's own worked
//! proof) uses a real, valid >=128-bit secret instead (the plan's own
//! literal string, doubled, to stay recognizably close to its
//! original intent) rather than silently switching to `totp_rs`'s own
//! RFC-skipping `build_noncompliant()` — a 2FA library accepting an
//! under-strength secret by construction is exactly the kind of
//! silent security footgun this module should not paper over.
//!
//! Error handling (plan 195's Typed Domain Errors convention, applied
//! fresh here rather than this plan's own originally-described
//! `NativeError`-at-construction-time shape): plan 184 was authored
//! 2026-09-21T21:33:00Z, plan 195 landed 2026-09-22T22:41:00Z — this
//! module is executed after 195, so `Totp.new`'s malformed/too-short
//! secret path is `Result[Totp, TotpError]` (`TotpError =
//! InvalidSecret(String) | Other(String)`), the same "lands after 195,
//! retrofit directly rather than shipping the older shape" posture
//! `yaml.rs`/`path.rs`/`datetime.rs`/`decimal.rs` already established
//! — not a literal edit to plan 184's own immutable history file. A
//! malformed or under-length secret supplied by a caller (e.g.
//! re-importing a user-provided secret) is exactly the "anticipated,
//! routinely-checked" failure category plan 92's own two-channel model
//! assigns to `Result[T,E]`, not `NativeError` — the identical
//! reasoning `Regex.compile`'s own fallible-parse constructor already
//! applies for a malformed pattern string. `.generate_current`/
//! `.check_current`/`.provisioning_uri` stay exactly as the plan's own
//! text describes (no `Result`; a clock or URL-generation failure on
//! an already-successfully-built `Totp` is a genuine, unanticipated
//! `NativeError`, not a routinely-checked outcome).

use crate::handle::{handle_alloc, handle_get_mut};
use std::ffi::c_void;
use std::os::raw::c_char;
use totp_rs::{Builder, Secret, Totp as TotpRs};

const TAG: &str = "Totp";

// Plan 195 (Typed Domain Errors): `TotpError`'s own variant tags,
// matching `emerald-sema`/`emerald-codegen`'s own `totp_error_enum_def`
// byte-for-byte.
//   0 InvalidSecret(String) — the base32 string failed to decode
//                             (`Secret::try_from_base32`'s own
//                             `SecretParseError::ParseBase32`), or
//                             decoded to fewer than 128 bits
//                             (`totp_rs::TotpError::SecretTooShort`)
//   1 Other(String)         — the convention's own required escape
//                             hatch for any other `totp_rs::TotpError`
//                             this plan's fixed-shape constructor is
//                             not actually expected to reach (e.g.
//                             `InvalidAccountName`/`InvalidIssuer` if
//                             a caller's issuer/account string
//                             contains `:`)
const TOTP_ERROR_TAG_INVALID_SECRET: i32 = 0;
const TOTP_ERROR_TAG_OTHER: i32 = 1;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Totp.new(issuer: String, account: String, secret_base32: String):
/// Result[Totp, TotpError]` — SHA1, 6 digits, 30-second step, 1-step
/// skew tolerance: `totp_rs::Builder::new()`'s own real, unchanged
/// defaults (this plan's own Decision log: these ARE RFC 6238's own
/// recommended values and what every real authenticator app expects,
/// not values this v1 constructor's fixed shape merely settles for).
///
/// # Safety
/// `issuer`/`account`/`secret_base32`, if non-null, must each point to
/// a valid, NUL-terminated C string.
pub unsafe fn totp_new(
  issuer: *const c_char,
  account: *const c_char,
  secret_base32: *const c_char,
) -> *mut c_void {
  let issuer = match read_str(issuer) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_OTHER, &e),
  };
  let account = match read_str(account) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_OTHER, &e),
  };
  let secret_base32 = match read_str(secret_base32) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_OTHER, &e),
  };
  let secret = match Secret::try_from_base32(secret_base32) {
    Ok(s) => s,
    Err(e) => {
      return crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_INVALID_SECRET, &e.to_string())
    }
  };
  match Builder::new()
    .with_secret(secret)
    .with_account_name(account)
    .with_issuer(Some(issuer))
    .build()
  {
    Ok(totp) => {
      let id = handle_alloc(Box::new(totp), TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(e @ totp_rs::TotpError::SecretTooShort { .. }) => {
      crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_INVALID_SECRET, &e.to_string())
    }
    Err(e) => crate::emerald_rt_result_err_tagged_str(TOTP_ERROR_TAG_OTHER, &e.to_string()),
  }
}

/// `Totp.generate_secret(): String` — a new, cryptographically random
/// base32-encoded secret (160 bits, `totp_rs::Secret::generate()`'s
/// own fixed size), backed by plan 113's blessed CSPRNG the same way
/// `totp-rs`'s own optional `rand` (`gen_secret`) feature is — this
/// module enables that feature rather than seeding an independent RNG
/// (this plan's own Decision log).
///
/// # Safety
/// Always safe to call.
pub fn totp_generate_secret() -> *const c_char {
  let secret = Secret::generate();
  unsafe { crate::alloc_and_copy_str(&secret.to_base32()) }
}

/// `.generate_current(self): String` — the 6-digit code valid for the
/// current wall-clock 30-second step, using the host's real system
/// time.
///
/// # Safety
/// Always safe to call for a live `Totp` handle.
pub unsafe fn totp_generate_current(id: i64) -> *const c_char {
  match handle_get_mut::<TotpRs, String>(id, TAG, |totp| totp.generate_current().to_string()) {
    Ok(code) => crate::alloc_and_copy_str(&code),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.check_current(self, code: String): Boolean` — crosses the FFI
/// boundary as a plain `i64` (0 or 1), the same convention `regex.rs`'s
/// own `regex_is_match` already establishes for `Boolean`. Accounts
/// for the constructed 1-step clock-skew tolerance the same way every
/// real authenticator-app-verifying server does.
///
/// # Safety
/// `code`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn totp_check_current(id: i64, code: *const c_char) -> i64 {
  let code = match read_str(code) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<TotpRs, bool>(id, TAG, |totp| totp.check_current(code).is_some()) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `.provisioning_uri(self): String` — the standard `otpauth://totp/
/// ...` URI a QR-code generator turns into a scannable enrollment
/// code (rendering it as an actual QR code image is out of scope,
/// this plan's own Decision log). A failure here (only reachable if
/// `totp_rs` itself refuses to serialize a `Totp` this module's own
/// constructor already successfully built — e.g. `AccountNameNotSet`,
/// which this module's `totp_new` never actually leaves unset) is a
/// genuine, unanticipated `NativeError`, not `Result[T,E]`.
///
/// # Safety
/// Always safe to call for a live `Totp` handle.
pub unsafe fn totp_provisioning_uri(id: i64) -> *const c_char {
  let result = handle_get_mut::<TotpRs, Result<String, String>>(id, TAG, |totp| {
    totp.to_url().map_err(|e| e.to_string())
  });
  match result {
    Ok(Ok(uri)) => crate::alloc_and_copy_str(&uri),
    Ok(Err(e)) => crate::raise_native_error(&e),
    Err(e) => crate::raise_native_error(&e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn ok_id(result_ptr: *mut c_void) -> i64 {
    let ptr = result_ptr as *const i64;
    assert_eq!(*ptr, 0, "expected Ok");
    *ptr.add(1)
  }

  unsafe fn err_tag_and_message(result_ptr: *mut c_void) -> (i64, String) {
    let ptr = result_ptr as *const i64;
    assert_eq!(*ptr, 1, "expected Err");
    let err_block = *(ptr.add(1)) as *const i64;
    let tag = *err_block;
    let msg_ptr = *(err_block.add(1) as *const *const c_char);
    let msg = std::ffi::CStr::from_ptr(msg_ptr)
      .to_str()
      .unwrap()
      .to_string();
    (tag, msg)
  }

  // A real, valid >=128-bit base32 secret — the plan's own
  // `"JBSWY3DPEHPK3PXP"` literal, doubled (see this module's own doc
  // comment for why the plan's original literal alone is too short).
  const VALID_SECRET: &str = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";

  #[test]
  fn new_of_an_invalid_base32_secret_is_a_real_err_not_a_panic() {
    unsafe {
      let issuer = c("Emerald Corp");
      let account = c("alice@example.com");
      let bad_secret = c("not valid base32!!!");
      let result_ptr = totp_new(issuer.as_ptr(), account.as_ptr(), bad_secret.as_ptr());
      let (tag, msg) = err_tag_and_message(result_ptr);
      assert_eq!(tag, TOTP_ERROR_TAG_INVALID_SECRET as i64);
      assert!(!msg.is_empty());
    }
  }

  #[test]
  fn new_of_a_too_short_secret_is_a_real_err_not_a_panic() {
    unsafe {
      let issuer = c("Emerald Corp");
      let account = c("alice@example.com");
      // The plan's own literal Concrete Proof secret — valid base32,
      // but only 80 bits, below `totp_rs`'s own 128-bit minimum.
      let short_secret = c("JBSWY3DPEHPK3PXP");
      let result_ptr = totp_new(issuer.as_ptr(), account.as_ptr(), short_secret.as_ptr());
      let (tag, msg) = err_tag_and_message(result_ptr);
      assert_eq!(tag, TOTP_ERROR_TAG_INVALID_SECRET as i64);
      assert!(!msg.is_empty());
    }
  }

  #[test]
  fn generate_current_is_accepted_by_check_current_and_a_wrong_code_is_rejected() {
    unsafe {
      let issuer = c("Emerald Corp");
      let account = c("alice@example.com");
      let secret = c(VALID_SECRET);
      let id = ok_id(totp_new(issuer.as_ptr(), account.as_ptr(), secret.as_ptr()));
      let code_ptr = totp_generate_current(id);
      let code = std::ffi::CStr::from_ptr(code_ptr).to_str().unwrap();
      assert_eq!(code.len(), 6);
      assert!(code.chars().all(|ch| ch.is_ascii_digit()));
      let code_c = CString::new(code).unwrap();
      assert_eq!(totp_check_current(id, code_c.as_ptr()), 1);
      // Vanishingly unlikely (1 in a million) to collide with the
      // real current code — an acceptable, disclosed test-flakiness
      // odds this plan's own Concrete Proof already names as
      // unavoidable for any real TOTP conformance check.
      if code != "000000" {
        let wrong = c("000000");
        assert_eq!(totp_check_current(id, wrong.as_ptr()), 0);
      }
    }
  }

  #[test]
  fn provisioning_uri_names_the_issuer_and_account() {
    unsafe {
      let issuer = c("Emerald Corp");
      let account = c("alice@example.com");
      let secret = c(VALID_SECRET);
      let id = ok_id(totp_new(issuer.as_ptr(), account.as_ptr(), secret.as_ptr()));
      let uri_ptr = totp_provisioning_uri(id);
      let uri = std::ffi::CStr::from_ptr(uri_ptr).to_str().unwrap();
      assert!(uri.starts_with("otpauth://totp/"));
      assert!(uri.contains("issuer=Emerald"));
      assert!(uri.contains("alice%40example.com") || uri.contains("alice@example.com"));
    }
  }

  #[test]
  fn generate_secret_produces_a_valid_128_bit_plus_base32_string() {
    unsafe {
      let secret_ptr = totp_generate_secret();
      let secret_str = std::ffi::CStr::from_ptr(secret_ptr).to_str().unwrap();
      let decoded = Secret::try_from_base32(secret_str).unwrap();
      assert!(decoded.as_bytes().len() >= 16);
    }
  }

  // RFC 6238 Appendix B: SHA1, secret "12345678901234567890" (ASCII,
  // 20 bytes), time=59s -> published 8-digit code 94287082. This
  // plan's own fixed 6-digit constructor produces the LAST 6 digits
  // of that same pre-truncation value (`code mod 10^6 == (code mod
  // 10^8) mod 10^6` — basic modular arithmetic, since dynamic
  // truncation is just "value mod 10^digits" of the identical
  // HMAC-derived integer regardless of digit count), independently
  // checkable against the RFC's own published table without this
  // test depending on this plan's own Emerald-facing base32 wrapper
  // at all — built directly against `totp_rs::Builder`/`.generate()`,
  // per this plan's own Decision log ("RFC 6238's own published test
  // vectors are the correctness bar, not `totp-rs`'s self-consistency
  // alone").
  #[test]
  fn matches_the_rfc_6238_appendix_b_sha1_test_vector_at_59_seconds() {
    let totp = totp_rs::Builder::new()
      .with_secret("12345678901234567890".as_bytes())
      .build_noncompliant();
    assert_eq!(totp.generate(59).to_string(), "287082");
  }
}
