//! Plan 162 (Human-Readable Duration/Time Formatting) — a narrow,
//! four-function wrapping of `humantime`. No new `Type`/`Class`
//! anywhere: every value in and out is a plain `Int64` (whole Unix
//! seconds or a whole-second duration) or `String` — this plan's own
//! deliberate contrast case in this batch, "not every new stdlib
//! capability needs a new representation."
//!
//! Negative durations/timestamps are rejected outright (`std::time::
//! Duration::from_secs` takes a `u64`; there is no negative-duration
//! rendering to defer to in `humantime` itself), never silently
//! clamped or wrapped — see this plan's own Decision log.

use std::os::raw::c_char;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Duration.humanize(seconds: Int64): String`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn format_duration(seconds: i64) -> *const c_char {
  if seconds < 0 {
    crate::raise_native_error(&format!(
      "Duration.humanize: seconds must be non-negative, found {seconds}"
    ));
  }
  let d = Duration::from_secs(seconds as u64);
  let s = humantime::format_duration(d).to_string();
  crate::alloc_and_copy_str(&s)
}

/// `Duration.parse_human(s: String): Result[Int64, String]` — any
/// sub-second remainder `humantime::parse_duration` returns is
/// truncated via `.as_secs()`, a real, disclosed precision loss.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn parse_duration(s: *const c_char) -> *mut std::ffi::c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match humantime::parse_duration(s) {
    Ok(d) => crate::emerald_rt_result_ok(d.as_secs() as i64),
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

/// `Timestamp.to_rfc3339(unix_secs: Int64): String`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn format_rfc3339(unix_secs: i64) -> *const c_char {
  if unix_secs < 0 {
    crate::raise_native_error(&format!(
      "Timestamp.to_rfc3339: unix_secs must be non-negative, found {unix_secs}"
    ));
  }
  let t = UNIX_EPOCH + Duration::from_secs(unix_secs as u64);
  let s = humantime::format_rfc3339(t).to_string();
  crate::alloc_and_copy_str(&s)
}

/// `Timestamp.parse_rfc3339(s: String): Result[Int64, String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn parse_rfc3339(s: *const c_char) -> *mut std::ffi::c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match humantime::parse_rfc3339(s) {
    Ok(t) => match t.duration_since(SystemTime::UNIX_EPOCH) {
      Ok(d) => crate::emerald_rt_result_ok(d.as_secs() as i64),
      Err(_) => {
        crate::emerald_rt_result_err_str("timestamp is before the Unix epoch, not representable")
      }
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
  fn format_duration_matches_the_crates_own_real_rendering() {
    unsafe {
      let ptr = format_duration(266400);
      assert_eq!(std::ffi::CStr::from_ptr(ptr).to_str().unwrap(), "3days 2h");
    }
  }

  #[test]
  fn parse_duration_of_3d_2h_matches_the_hand_computed_second_count() {
    unsafe {
      let s = c("3d 2h");
      let result_ptr = parse_duration(s.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let secs = *result_ptr.add(1);
      assert_eq!(secs, 3 * 86400 + 2 * 3600);
    }
  }

  #[test]
  fn parse_duration_of_a_malformed_string_is_a_real_err_not_a_panic() {
    unsafe {
      let s = c("not a duration");
      let result_ptr = parse_duration(s.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
      let msg_ptr = *(result_ptr.add(1) as *const *const c_char);
      assert!(!std::ffi::CStr::from_ptr(msg_ptr)
        .to_str()
        .unwrap()
        .is_empty());
    }
  }

  #[test]
  fn format_and_parse_rfc3339_round_trip_a_fixed_unix_timestamp() {
    unsafe {
      let formatted = format_rfc3339(1_780_000_000);
      let formatted_str = std::ffi::CStr::from_ptr(formatted)
        .to_str()
        .unwrap()
        .to_string();
      let formatted_cstr = c(&formatted_str);
      let result_ptr = parse_rfc3339(formatted_cstr.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let secs = *result_ptr.add(1);
      assert_eq!(secs, 1_780_000_000);
    }
  }
}
