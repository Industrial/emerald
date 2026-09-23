# Plan 160 (Date/Time & Timezones) — `DateTime` (a UTC instant, packed
# into two plain `Int64` fields: `epoch_secs`/`subsec_nanos`, zero heap
# allocation of its own beyond the fixed 16-byte block) and
# `ZonedDateTime` (the same two fields plus a `tz_name: String` IANA
# identifier), both backed by `jiff` (chosen over `chrono` — soft-
# deprecated in writing by its own maintainer, GitHub issue
# `chronotope/chrono#1768`, opened 2026-01-22, still open as of
# 2026-09-15 — and over `time`, which has no bundled IANA database of
# its own).
#
# Plan 195 (Typed Domain Errors) applied at this plan's own EXECUTE
# time: `DateTime.parse_rfc3339`/`.in_tz` return `Result[T,
# DateTimeError]`, never a bare `Result[T, String]` — `DateTimeError`
# carries a single required `Other(String)` variant, since `jiff::
# Error` is itself deliberately opaque (no public variants to classify
# further, the identical situation `BigIntError` documents for
# `num_bigint`'s own bare-`Option` failure mode — see `crates/
# emerald-rt/src/datetime.rs`'s own module doc).
#
# This grammar has no `case`/`when`/`else` keywords (plan 71) and no
# postfix `!` Result-unwrap operator (verified against the current
# grammar directly, not assumed from this plan's own history file,
# whose worked proof predates both changes) — `match X do Variant(binds)
# do ... end ... end` is the real, current syntax (`examples/
# bignum_decimal_proof.em` precedent, reused here).
#
# The DST proof below exercises 2026's real US spring-forward
# transition: clocks in `America/New_York` jump from 02:00 to 03:00 on
# 2026-03-08 (the real, second-Sunday-in-March rule, not a synthetic
# date) — `ZonedDateTime.plus_days(1)` across that boundary advances by
# 23 real wall-clock hours, one hour short of naive `+86400` seconds,
# and `.diff_seconds` reports that real difference (`-3600`) rather
# than silently agreeing with the naive calculation.

parse_result: Result[DateTime, DateTimeError] = DateTime.parse_rfc3339("2026-06-15T12:00:00Z")
match parse_result do
Ok(utc) do
  puts utc.to_rfc3339()

  zoned_result: Result[ZonedDateTime, DateTimeError] = utc.in_tz("America/New_York")
  match zoned_result do
  Ok(z) do
    back_to_utc: DateTime = z.to_utc()
    puts back_to_utc.to_rfc3339()
  end
  Err(e) do
    match e do
    Other(detail) do
      puts detail
    end
    end
  end
  end

  bad_result: Result[ZonedDateTime, DateTimeError] = utc.in_tz("Nonexistent/Zone")
  match bad_result do
  Ok(z) do
    back_to_utc: DateTime = z.to_utc()
    puts back_to_utc.to_rfc3339()
  end
  Err(e) do
    match e do
    Other(detail) do
      puts detail
    end
    end
  end
  end
end
Err(e) do
  puts "unexpected parse error"
end
end

before_dst_result: Result[DateTime, DateTimeError] = DateTime.parse_rfc3339("2026-03-08T06:00:00Z")
match before_dst_result do
Ok(before_dst) do
  zoned_before_result: Result[ZonedDateTime, DateTimeError] = before_dst.in_tz("America/New_York")
  match zoned_before_result do
  Ok(zoned_before) do
    after_one_day: ZonedDateTime = zoned_before.plus_days(1)
    after_one_day_utc: DateTime = after_one_day.to_utc()
    naive_utc: DateTime = before_dst.plus_seconds(86400)
    puts after_one_day_utc.diff_seconds(naive_utc)
  end
  Err(e) do
    puts "unexpected in_tz error"
  end
  end
end
Err(e) do
  puts "unexpected parse error"
end
end
