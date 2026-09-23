//! Plan 160 (Date/Time & Timezones) — `DateTime` (a UTC instant) and
//! `ZonedDateTime` (the same instant plus an IANA zone name), backed by
//! `jiff` — chosen over `chrono` (soft-deprecated in writing by its own
//! maintainer, GitHub issue `chronotope/chrono#1768`, opened
//! 2026-01-22, still open/unresolved as of 2026-09-15) and over `time`
//! (no bundled IANA database of its own, needing a second `time-tz`/
//! `tzdb` crate the way `chrono` needs `chrono-tz`). Verified directly
//! against docs.rs/lib.rs this session (2026-09-23), not copied from
//! this plan's own history file, which cited a stale `jiff` 0.2.20 —
//! the actual latest release is 0.2.37 (2026-09-12), and `jiff`'s own
//! default feature set (`tz-system`, `tzdb-bundle-platform`,
//! `tzdb-zoneinfo`, verified via `docs.rs/crate/jiff/latest/features`)
//! already bundles the IANA Time Zone Database, so no second
//! dependency is needed — the plan's claim holds, just under a newer
//! version number than it cited. See `crates/emerald-rt/Cargo.toml`'s
//! own comment and `DEPENDENCIES.md` for the full ledger entry.
//!
//! Representation (plan 163's `Decimal` precedent, `decimal.rs`'s own
//! module doc — packed fields, not an opaque `crate::handle`):
//! `DateTime` is a heap pointer to `[epoch_secs: i64][subsec_nanos:
//! i64]` (16 bytes); `ZonedDateTime` is a heap pointer to `[epoch_secs:
//! i64][subsec_nanos: i64][tz_name: *const c_char]` (24 bytes). Both
//! are reconstructed into a real `jiff::Timestamp`/`jiff::Zoned` only
//! on the native calls that actually need `jiff`'s own calendar/
//! timezone logic. `ZonedDateTime.to_utc` needs none at all: its own
//! `epoch_secs`/`subsec_nanos` already ARE the zone-independent UTC
//! instant — `jiff::Zoned::timestamp()` returns exactly the
//! `Timestamp` a `Zoned` was built from, since attaching a zone never
//! changes the instant, only how it's displayed — so `.to_utc` is a
//! bare repack, dropping `tz_name`, never touching `jiff` at all.
//!
//! `.plus_seconds`/`.diff_seconds` on a bare `DateTime` are plain `i64`
//! arithmetic on the packed `epoch_secs` field directly, never
//! constructing a `jiff::Timestamp` at all — flat epoch-second math
//! needs no calendar/timezone rule of any kind (this plan's own
//! Decision log: "a program that only ever needed seconds-since-epoch
//! arithmetic would need no crate ... at all", the same case plan
//! 162's `humantime` already covers for pure durations). Only
//! `ZonedDateTime.plus_days` (real calendar-day arithmetic across a
//! DST transition) ever needs `jiff::Zoned::checked_add` — that is
//! this whole plan's reason to exist, not a decoration.
//!
//! Error handling (plan 195's Typed Domain Errors convention, applied
//! here exactly as it was for plan 163's `DecimalError`/`BigIntError`):
//! `DateTimeError` carries a single required `Other(String)` variant.
//! `jiff::Error` is deliberately opaque (`pub struct Error(..)`, no
//! public variants, verified directly against `docs.rs/jiff`) — there
//! is no real sub-classification to expose, the identical situation
//! `BigIntError` documents for `num_bigint`'s own bare-`Option` failure
//! mode — so a single `Other(String)` escape hatch, carrying `jiff::
//! Error`'s own real `Display` text, is this domain's entire, honest
//! v1 scope. Shared by both `DateTime.parse_rfc3339` (malformed RFC
//! 3339 input) and `DateTime.in_tz` (an unrecognized IANA zone name) —
//! both surface only `jiff::Error`'s own opaque text either way.

use std::ffi::c_void;
use std::os::raw::c_char;

use jiff::{Timestamp, ToSpan};

const DATETIME_ERROR_TAG_OTHER: i32 = 0;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// Reads a `DateTime`'s own packed `[epoch_secs: i64][subsec_nanos:
/// i64]` block back into its two raw components.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed block —
/// every `DateTime` value this module itself ever produces satisfies
/// this.
unsafe fn decode_datetime(ptr: *const i64) -> (i64, i64) {
  (*ptr, *ptr.add(1))
}

/// Allocates a fresh, `emerald_alloc`-backed `DateTime` block.
///
/// # Safety
/// Always safe to call.
unsafe fn encode_datetime(epoch_secs: i64, subsec_nanos: i64) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  *ptr = epoch_secs;
  *ptr.add(1) = subsec_nanos;
  ptr as *mut c_void
}

/// Reads a `ZonedDateTime`'s own packed `[epoch_secs: i64][subsec_
/// nanos: i64][tz_name: *const c_char]` block back into its three raw
/// components.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed block —
/// every `ZonedDateTime` value this module itself ever produces
/// satisfies this.
unsafe fn decode_zoned(ptr: *const i64) -> (i64, i64, *const c_char) {
  let epoch_secs = *ptr;
  let subsec_nanos = *ptr.add(1);
  let tz_name = *(ptr.add(2) as *const *const c_char);
  (epoch_secs, subsec_nanos, tz_name)
}

/// Allocates a fresh, `emerald_alloc`-backed `ZonedDateTime` block.
/// `tz_name` is copied into a fresh, permanently allocated buffer —
/// this module never stores a caller-owned/transient pointer directly,
/// the same `alloc_and_copy_str` convention every other `String`-
/// holding value in this crate follows.
///
/// # Safety
/// Always safe to call.
unsafe fn encode_zoned(epoch_secs: i64, subsec_nanos: i64, tz_name: &str) -> *mut c_void {
  let ptr = crate::emerald_alloc(24) as *mut i64;
  *ptr = epoch_secs;
  *ptr.add(1) = subsec_nanos;
  *(ptr.add(2) as *mut *const c_char) = crate::alloc_and_copy_str(tz_name);
  ptr as *mut c_void
}

/// Reconstructs a real `jiff::Timestamp` from a raw `epoch_secs`/
/// `subsec_nanos` pair. The only way this can fail is if a prior
/// `.plus_seconds` pushed `epoch_secs` outside `jiff`'s own supported
/// range (`Timestamp::MIN`/`MAX`, roughly ±9999 years) — a genuine
/// misuse, not an anticipated, `Result`-worthy input, the same rule
/// `Decimal.add`'s own overflow raise already follows.
///
/// # Safety
/// Always safe to call.
unsafe fn timestamp_or_raise(epoch_secs: i64, subsec_nanos: i64, context: &str) -> Timestamp {
  match Timestamp::new(epoch_secs, subsec_nanos as i32) {
    Ok(ts) => ts,
    Err(e) => crate::raise_native_error(&format!("{context}: {e}")),
  }
}

/// `DateTime.now(): DateTime` — `jiff::Timestamp::now()`, decomposed
/// via its own documented `as_second`/`subsec_nanosecond` accessors.
///
/// # Safety
/// Always safe to call.
pub unsafe fn datetime_now() -> *mut c_void {
  let ts = Timestamp::now();
  encode_datetime(ts.as_second(), ts.subsec_nanosecond() as i64)
}

/// `DateTime.parse_rfc3339(s: String): Result[DateTime, DateTimeError]`
/// — `s.parse::<jiff::Timestamp>()`, which `jiff` implements via RFC
/// 3339/ISO 8601 by default (verified against `jiff`'s own documented
/// `FromStr` impl).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn datetime_parse_rfc3339(s: *const c_char) -> *mut c_void {
  let s = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(DATETIME_ERROR_TAG_OTHER, &e),
  };
  match s.parse::<Timestamp>() {
    Ok(ts) => crate::emerald_rt_result_ok(encode_datetime(
      ts.as_second(),
      ts.subsec_nanosecond() as i64,
    ) as i64),
    Err(e) => crate::emerald_rt_result_err_tagged_str(DATETIME_ERROR_TAG_OTHER, &e.to_string()),
  }
}

/// `.to_rfc3339(self): String` — `jiff::Timestamp`'s own `Display`,
/// RFC 3339 by default.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `DateTime` block.
pub unsafe fn datetime_to_rfc3339(ptr: *const i64) -> *const c_char {
  let (secs, nanos) = decode_datetime(ptr);
  let ts = timestamp_or_raise(secs, nanos, "DateTime.to_rfc3339");
  crate::alloc_and_copy_str(&ts.to_string())
}

/// `.plus_seconds(self, n: Int64): DateTime` — plain `i64` arithmetic
/// on the packed `epoch_secs` field, never touching `jiff` at all (see
/// this module's own doc comment for why flat epoch-second math needs
/// no calendar/timezone rule).
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `DateTime` block.
pub unsafe fn datetime_plus_seconds(ptr: *const i64, n: i64) -> *mut c_void {
  let (secs, nanos) = decode_datetime(ptr);
  match secs.checked_add(n) {
    Some(new_secs) => encode_datetime(new_secs, nanos),
    None => crate::raise_native_error("DateTime.plus_seconds: overflow"),
  }
}

/// `.diff_seconds(self, other: DateTime): Int64` — plain `i64`
/// subtraction of the two packed `epoch_secs` fields, whole-second
/// truncated (this plan's own disclosed second-granularity rule,
/// matching plan 162's `humantime` convention rather than inventing a
/// different rounding rule for this plan alone).
///
/// # Safety
/// `ptr`/`other_ptr` must each point to a live, 16-byte,
/// `emerald_alloc`-backed `DateTime` block.
pub unsafe fn datetime_diff_seconds(ptr: *const i64, other_ptr: *const i64) -> i64 {
  let (secs, _) = decode_datetime(ptr);
  let (other_secs, _) = decode_datetime(other_ptr);
  match secs.checked_sub(other_secs) {
    Some(diff) => diff,
    None => crate::raise_native_error("DateTime.diff_seconds: overflow"),
  }
}

/// `.in_tz(self, tz_name: String): Result[ZonedDateTime,
/// DateTimeError]` — `jiff::Timestamp::in_tz`, which resolves
/// `tz_name` against the bundled IANA database itself — `Err` on an
/// unrecognized name is `jiff`'s own real error text, not a
/// hand-rolled validity check.
///
/// # Safety
/// `ptr` must point to a live, 16-byte, `emerald_alloc`-backed
/// `DateTime` block. `tz_name`, if non-null, must point to a valid,
/// NUL-terminated C string.
pub unsafe fn datetime_in_tz(ptr: *const i64, tz_name: *const c_char) -> *mut c_void {
  let (secs, nanos) = decode_datetime(ptr);
  let name = match read_str(tz_name) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_tagged_str(DATETIME_ERROR_TAG_OTHER, &e),
  };
  let ts = timestamp_or_raise(secs, nanos, "DateTime.in_tz");
  match ts.in_tz(name) {
    Ok(zoned) => {
      let zts = zoned.timestamp();
      crate::emerald_rt_result_ok(encode_zoned(
        zts.as_second(),
        zts.subsec_nanosecond() as i64,
        name,
      ) as i64)
    }
    Err(e) => crate::emerald_rt_result_err_tagged_str(DATETIME_ERROR_TAG_OTHER, &e.to_string()),
  }
}

/// `.to_utc(self): DateTime` — a bare repack, dropping `tz_name`: a
/// `ZonedDateTime`'s own `epoch_secs`/`subsec_nanos` already ARE the
/// zone-independent UTC instant (see this module's own doc comment) —
/// never reconstructs a real `jiff::Zoned` at all, and cannot fail.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ZonedDateTime` block.
pub unsafe fn zoned_to_utc(ptr: *const i64) -> *mut c_void {
  let (secs, nanos, _) = decode_zoned(ptr);
  encode_datetime(secs, nanos)
}

/// `.plus_days(self, n: Int64): ZonedDateTime` — real, DST-aware
/// calendar arithmetic via `jiff::Zoned::checked_add`, adding `n`
/// calendar days in `self`'s own IANA zone, not fixed 86400-second
/// blocks — this is this plan's entire reason to exist (see this
/// module's own doc comment).
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ZonedDateTime` block.
pub unsafe fn zoned_plus_days(ptr: *const i64, n: i64) -> *mut c_void {
  let (secs, nanos, tz_name_ptr) = decode_zoned(ptr);
  let name = match read_str(tz_name_ptr) {
    Ok(s) => s,
    Err(_) => crate::raise_native_error("ZonedDateTime.plus_days: corrupt tz_name"),
  };
  let ts = timestamp_or_raise(secs, nanos, "ZonedDateTime.plus_days");
  let zoned = match ts.in_tz(name) {
    Ok(z) => z,
    Err(e) => crate::raise_native_error(&format!("ZonedDateTime.plus_days: {e}")),
  };
  let moved = match zoned.checked_add(n.days()) {
    Ok(z) => z,
    Err(e) => crate::raise_native_error(&format!("ZonedDateTime.plus_days: {e}")),
  };
  let zts = moved.timestamp();
  encode_zoned(zts.as_second(), zts.subsec_nanosecond() as i64, name)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn to_s(ptr: *const c_char) -> String {
    std::ffi::CStr::from_ptr(ptr).to_str().unwrap().to_string()
  }

  unsafe fn parse_ok(s: &str) -> *const i64 {
    let cs = c(s);
    let result_ptr = datetime_parse_rfc3339(cs.as_ptr()) as *const i64;
    assert_eq!(*result_ptr, 0, "expected Ok discriminant for {s}");
    *(result_ptr.add(1)) as *const i64
  }

  #[test]
  fn now_is_within_a_sane_range() {
    // A loose sanity bound, not a determinism check (`.now()` is
    // inherently non-deterministic, which is exactly why it stays out
    // of `examples/datetime_timezones_proof.em`'s own fixed-output
    // proof): the returned epoch is after 2020-01-01 and before
    // 2100-01-01.
    unsafe {
      let ptr = datetime_now() as *const i64;
      let secs = *ptr;
      assert!(secs > 1_577_836_800, "epoch_secs before 2020: {secs}");
      assert!(secs < 4_102_444_800, "epoch_secs after 2100: {secs}");
    }
  }

  #[test]
  fn rfc3339_round_trips_unchanged() {
    unsafe {
      let dt = parse_ok("2026-06-15T12:00:00Z");
      let s = to_s(datetime_to_rfc3339(dt));
      assert_eq!(s, "2026-06-15T12:00:00Z");
    }
  }

  #[test]
  fn parse_of_a_malformed_string_is_a_real_err_not_a_panic() {
    unsafe {
      let bad = c("not a timestamp");
      let result_ptr = datetime_parse_rfc3339(bad.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
    }
  }

  #[test]
  fn in_tz_of_an_unknown_zone_is_a_real_err_not_a_panic() {
    unsafe {
      let dt = parse_ok("2026-06-15T12:00:00Z");
      let tz = c("Nonexistent/Zone");
      let result_ptr = datetime_in_tz(dt, tz.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 1, "expected Err discriminant");
    }
  }

  #[test]
  fn in_tz_then_to_utc_round_trips_the_same_instant() {
    unsafe {
      let dt = parse_ok("2026-06-15T12:00:00Z");
      let tz = c("America/New_York");
      let result_ptr = datetime_in_tz(dt, tz.as_ptr()) as *const i64;
      assert_eq!(*result_ptr, 0, "expected Ok discriminant");
      let zoned = *(result_ptr.add(1)) as *const i64;
      let utc = zoned_to_utc(zoned) as *const i64;
      let s = to_s(datetime_to_rfc3339(utc));
      assert_eq!(s, "2026-06-15T12:00:00Z");
    }
  }

  #[test]
  fn plus_days_across_the_2026_spring_forward_transition_is_dst_aware() {
    // 2026-03-08 is the real US spring-forward date (clocks in
    // America/New_York jump from 02:00 to 03:00) — this plan's own
    // Concrete Proof exercises exactly this date, not a synthetic one.
    unsafe {
      let before_dst = parse_ok("2026-03-08T06:00:00Z");
      let tz = c("America/New_York");
      let zoned_result = datetime_in_tz(before_dst, tz.as_ptr()) as *const i64;
      assert_eq!(*zoned_result, 0, "expected Ok discriminant");
      let zoned_before = *(zoned_result.add(1)) as *const i64;

      let after_one_day = zoned_plus_days(zoned_before, 1) as *const i64;
      let after_utc = zoned_to_utc(after_one_day) as *const i64;

      let naive_utc = datetime_plus_seconds(before_dst, 86_400) as *const i64;

      let diff = datetime_diff_seconds(after_utc, naive_utc);
      assert_eq!(
        diff, -3600,
        "expected DST-aware +1 day to be 1 hour short of naive +86400s"
      );
    }
  }
}
