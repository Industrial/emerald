//! Plan 123 (Base64 & Hex Encoding), revised scope — see this session's
//! own `history/2026-09-22T0*-base64-hex-encoding.md` Decision log for
//! the full account. The plan's own original text specified `Bytes`
//! (a `(ptr, len)` Emerald-facing binary-buffer type) on both sides of
//! every function here — verified directly against `crates/emerald-
//! sema/src/lib.rs`'s own `Type` enum and against plan 92's own history
//! doc ("Not yet decided... whether a genuine Emerald-facing `Bytes`/
//! binary-literal type... belongs to this batch at all") that no such
//! type exists anywhere in this compiler today. Rather than build that
//! whole new language-surface feature as an undisclosed side effect of
//! "the most mechanical plan in this batch," every function here
//! operates on Emerald's existing `String` instead — real, useful (the
//! base64url/JWT and hex/digest-display use cases this plan's own
//! Decision log names are exactly this shape: printable identifiers and
//! tokens, not arbitrary embedded-NUL binary blobs), but narrower than
//! originally scoped: a `String` whose own bytes are not valid UTF-8
//! after decoding (round-tripping genuinely arbitrary binary data) is
//! out of scope here, deferred to whenever a real `Bytes` type lands.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use std::os::raw::c_char;

unsafe fn read_str(s: *const c_char) -> Result<&'static str, &'static str> {
  if s.is_null() {
    return Err("null string pointer");
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8")
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_encode(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&STANDARD.encode(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_decode(s: *const c_char) -> *mut std::ffi::c_void {
  decode_with(s, &STANDARD)
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_encode_no_pad(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&STANDARD_NO_PAD.encode(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_decode_no_pad(s: *const c_char) -> *mut std::ffi::c_void {
  decode_with(s, &STANDARD_NO_PAD)
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_encode_url_safe(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&URL_SAFE_NO_PAD.encode(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_decode_url_safe(s: *const c_char) -> *mut std::ffi::c_void {
  decode_with(s, &URL_SAFE_NO_PAD)
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_encode_url_safe_padded(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&URL_SAFE.encode(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn base64_decode_url_safe_padded(s: *const c_char) -> *mut std::ffi::c_void {
  decode_with(s, &URL_SAFE)
}

unsafe fn decode_with(
  s: *const c_char,
  engine: &impl base64::engine::Engine,
) -> *mut std::ffi::c_void {
  let text = match read_str(s) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_str(e),
  };
  match engine.decode(text) {
    Ok(bytes) => match String::from_utf8(bytes) {
      Ok(decoded) => {
        let ptr = crate::alloc_and_copy_str(&decoded);
        crate::emerald_rt_result_ok(ptr as i64)
      }
      Err(_) => crate::emerald_rt_result_err_str(
        "decoded bytes are not valid UTF-8 (Base64.decode only supports text payloads until a real Bytes type exists)",
      ),
    },
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn hex_encode(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&hex::encode(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn hex_encode_upper(s: *const c_char) -> *const c_char {
  let text = read_str(s).unwrap_or("");
  crate::alloc_and_copy_str(&hex::encode_upper(text.as_bytes()))
}

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn hex_decode(s: *const c_char) -> *mut std::ffi::c_void {
  let text = match read_str(s) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_str(e),
  };
  match hex::decode(text) {
    Ok(bytes) => match String::from_utf8(bytes) {
      Ok(decoded) => {
        let ptr = crate::alloc_and_copy_str(&decoded);
        crate::emerald_rt_result_ok(ptr as i64)
      }
      Err(_) => crate::emerald_rt_result_err_str(
        "decoded bytes are not valid UTF-8 (Hex.decode only supports text payloads until a real Bytes type exists)",
      ),
    },
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
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
  fn base64_standard_round_trips_rfc4648_example() {
    unsafe {
      let input = c("hello world");
      let enc = base64_encode(input.as_ptr());
      let enc_str = std::ffi::CStr::from_ptr(enc).to_str().unwrap();
      assert_eq!(enc_str, "aGVsbG8gd29ybGQ=");
      let enc_cstr = c(enc_str);
      let result_ptr = base64_decode(enc_cstr.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let payload = *(result_ptr.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(payload).to_str().unwrap(),
        "hello world"
      );
    }
  }

  #[test]
  fn base64_url_safe_is_unpadded_by_default() {
    unsafe {
      let input = c("hello world");
      let enc = base64_encode_url_safe(input.as_ptr());
      let enc_str = std::ffi::CStr::from_ptr(enc).to_str().unwrap();
      assert_eq!(enc_str, "aGVsbG8gd29ybGQ");
    }
  }

  #[test]
  fn base64_decode_of_malformed_input_is_a_real_err_not_a_panic() {
    unsafe {
      let input = c("not valid base64!!!");
      let result_ptr = base64_decode(input.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let msg_ptr = *(result_ptr.add(1) as *const *const c_char);
      let msg = std::ffi::CStr::from_ptr(msg_ptr).to_str().unwrap();
      assert!(!msg.is_empty());
    }
  }

  #[test]
  fn hex_round_trips_rfc4648_style_example() {
    unsafe {
      let input = c("hello world");
      let enc = hex_encode(input.as_ptr());
      let enc_str = std::ffi::CStr::from_ptr(enc).to_str().unwrap();
      assert_eq!(enc_str, "68656c6c6f20776f726c64");
      let upper = hex_encode_upper(input.as_ptr());
      let upper_str = std::ffi::CStr::from_ptr(upper).to_str().unwrap();
      assert_eq!(upper_str, "68656C6C6F20776F726C64");
      let enc_cstr = c(&enc_str.to_uppercase());
      let result_ptr = hex_decode(enc_cstr.as_ptr()) as *const i64;
      assert_eq!(
        *result_ptr, 0,
        "expected Ok discriminant — hex::decode is case-insensitive"
      );
      let payload = *(result_ptr.add(1) as *const *const c_char);
      assert_eq!(
        std::ffi::CStr::from_ptr(payload).to_str().unwrap(),
        "hello world"
      );
    }
  }

  #[test]
  fn hex_decode_of_an_odd_digit_count_is_a_real_err_not_a_panic() {
    unsafe {
      let input = c("abc");
      let result_ptr = hex_decode(input.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
    }
  }
}
