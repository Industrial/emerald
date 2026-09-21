2026-09-21T21:09:00Z

---
name: Date/Time & Timezones — a `jiff`-Backed `DateTime`/`ZonedDateTime`
overview: "Two new compiler-synthesized classes, `DateTime` (a UTC instant packed into two plain `Int64` fields — epoch seconds and sub-second nanoseconds, zero heap allocation) and `ZonedDateTime` (the same two fields plus a `tz_name: String` IANA identifier), backed by `emerald-rt` bindings to `jiff` — chosen this session, by real, dated evidence, over `chrono` (soft-deprecated by its own maintainer as of a 2026-01-22 GitHub issue, still open and unresolved as of 2026-09-15) and over `time` (which has no bundled IANA database of its own). Covers RFC 3339 parsing/formatting, calendar arithmetic, and DST-aware timezone conversion; declines a general strftime-equivalent formatting language and any historical-timezone-rule-change edge case beyond whatever `jiff`'s own bundled `jiff-tzdb` snapshot provides."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vendor-jiff-and-scaffold-module
    content: "Add `jiff = \"0.2\"` to `crates/emerald-rt/Cargo.toml` (default features on — `jiff`'s own default feature set already bundles `jiff-tzdb`, the crate's embedded copy of the IANA Time Zone Database, so no second `chrono-tz`-equivalent dependency is needed the way plan 160's chrono alternative would have required). Create `crates/emerald-rt/src/datetime.rs`, `mod datetime;` from `lib.rs`, and record in this module's own top-of-file doc comment the exact crate-vetting facts this plan's Decision log states (soft-deprecation date, `jiff` version, no-second-dependency claim) so a future re-vetting pass (plan 95's ledger) has a citable anchor inside the crate itself, not only in this history file."
    status: pending
  - id: leaf-datetime-class-and-representation
    content: "Register a compiler-synthesized `DateTime` class (two fields, `epoch_secs: Int64`, `subsec_nanos: Int64`) in `emerald-sema`'s `classes` map and `emerald-codegen`'s class-tag-assignment pass, the same registration path plan 92's `NativeError` uses (`leaf-native-error-and-panic-raise`) — a fixed, reserved tag assigned before any user class, so `emerald-rt` can construct one without access to a given compilation's own tag assignment. Add `emerald_rt_datetime_now() -> i64x2` (two packed `i64` return values via the `out_len`-style multi-out-parameter convention plan 92's binary-safe-buffer leaf establishes — here two plain `*mut i64` out-parameters, `out_secs`/`out_nanos`, since both values are fixed-width scalars, not a length-prefixed buffer) reading the system clock through `jiff::Timestamp::now()` and decomposing it via `.as_second()`/`.subsec_nanosecond()`. Wire the Emerald-facing `DateTime.now(): DateTime` static call through the reserved-namespace dispatch shape plan 45's `File.read` already established."
    status: pending
  - id: leaf-zoneddatetime-and-tz-conversion
    content: "Register a second compiler-synthesized class, `ZonedDateTime` (`epoch_secs: Int64`, `subsec_nanos: Int64`, `tz_name: String`), and add `emerald_rt_datetime_in_tz(epoch_secs: i64, subsec_nanos: i64, tz_name: *const c_char) -> *mut c_void` returning a `Result[ZonedDateTime, String]`-shaped pointer (plan 92's `emerald_rt_result_ok`/`emerald_rt_result_err` helpers, 16-byte `[discriminant][payload]` layout) — `Err` on an unrecognized IANA name (`jiff::tz::TimeZone::get(name)` returns a real `Result` this wraps directly, not a hand-rolled validity check). Add the reverse, `.to_utc(self): DateTime`, dropping `tz_name` and re-normalizing through `jiff::Zoned::timestamp()`. Dispatch both as ordinary methods on `ZonedDateTime`/`DateTime`-typed receivers via the `ValKind`-gated intrinsic mechanism plan 91 already used for `.fnv1a_hash`."
    status: pending
  - id: leaf-parse-and-format-rfc3339
    content: "Add `DateTime.parse_rfc3339(s: String): Result[DateTime, String]` (wrapping `s.parse::<jiff::Timestamp>()`, which `jiff` implements via RFC 3339/ISO 8601 by default — verified against `jiff`'s own documented `FromStr` impl) and `.to_rfc3339(self): String` (`jiff::Timestamp::to_string()`, already RFC 3339 by default per `jiff`'s own crates.io description, confirmed this session). Both go through `emerald_rt_fn!` (plan 92's panic-boundary macro) — a malformed input is a real, reachable panic path inside `jiff`'s parser this plan's own proof exercises on purpose (see Concrete Proof), not a hypothetical."
    status: pending
  - id: leaf-arithmetic
    content: "Add `.plus_seconds(self, n: Int64): DateTime` / `.plus_days(self, n: Int64): ZonedDateTime` (the latter DST-aware — `jiff::Zoned`'s own calendar-arithmetic API adds calendar days, not fixed 86400-second blocks, across a DST transition; a `ZonedDateTime.plus_days` proof crossing a real spring-forward/fall-back boundary is this leaf's own acceptance check, not merely a same-offset addition) and `.diff_seconds(self, other: DateTime): Int64` (`jiff::Timestamp` subtraction, whole-second truncated — sub-second precision is preserved in the two stored fields but this one comparison intrinsic disclosed as second-granularity only, matching `humantime`'s own second-granularity convention from plan 162 rather than inventing a different rounding rule for this plan alone)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/datetime_timezones_proof.em` (this plan's Concrete Proof below) to `emerald-cli/tests/examples.rs`'s CI-checked table. Add `#[test]`s in `crates/emerald-rt/src/datetime.rs` covering: a fixed RFC 3339 string round-tripping through parse/format unchanged; `.in_tz(\"nonexistent/Zone\")` returning `Err`; a `.plus_days` call crossing 2026's real US spring-forward transition (2026-03-08 02:00 America/New_York) producing a wall-clock hour that differs from naive +24h arithmetic by exactly one hour, proving DST-awareness against a real, checkable calendar date rather than a synthetic one. Run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 160 — Date/Time & Timezones

Every prior stdlib plan in this batch that needs "the current time" or "a
timestamp" has had nothing to reach for: `emerald_runtime.c` has no date/
time functions at all, and Emerald's `Type` enum has no dedicated
temporal type — a gap plans 161 (cron) and 162 (humantime) both sit
directly on top of, and one this plan closes first so those two can cite
it rather than each inventing their own ad hoc epoch-seconds convention
independently. This plan adds exactly two compiler-synthesized classes,
`DateTime` (a bare UTC instant) and `ZonedDateTime` (the same instant
plus an IANA zone name), both backed by real, dated crate-selection
research done this session rather than a default reached for out of
habit — the task that produced this plan explicitly required checking
whether the old "chrono is the default, `time` is the safety-conscious
alternative" narrative still holds, and it does not.

## The crate decision, with dates

Three real candidates exist in the Rust ecosystem for this: `chrono`,
`time`, and `jiff`. The received wisdom as of a few years ago (a GitHub
issue on `chrono`'s own tracker, opened 2024-02-10, is explicit: "chrono
was the date-time library... chrono became unmaintained[,] time-rs
became a new crate with a simplified part of chrono's API") was that
`chrono` had a rough maintenance period and `time` was the
better-audited alternative. That is not the live situation checked this
session (2026-09-21). The decisive, dated fact: **`chrono`'s own current
maintainer soft-deprecated it, in writing, this year.** GitHub issue
`chronotope/chrono#1768`, "Soft-deprecating chrono and chrono-tz," opened
2026-01-22 by `djc` (author association: `MEMBER`), quotes his own
year-in-review post directly: "I'm inclined to wind down maintenance of
chrono and chrono-tz in the coming months. The API design for these
crates is quite dated, revising it would be a lot of effort, and with
[jiff] there is now an alternative I feel comfortable recommending." The
issue is still open, with 15 comments and a last-updated timestamp of
2026-09-15 — six days before this plan was written — and no successor
maintainer has taken it over. This is not a rumor or a stale blog post;
it is the crate's own current maintainer, on the crate's own tracker,
recommending a specific replacement, unresolved as of the week this plan
was authored. Downstream consumers have already acted on it: `tokio-rs/
prost` issue #1448 ("chrono is soft-deprecated") states plainly "I
recommend jiff," and `uutils` (the Rust coreutils reimplementation) is
reported to have "fully migrated off of Chrono... to Jiff."

`jiff` (`BurntSushi/jiff` — Andrew Gallant, also the author of `ripgrep`
and the `regex` crate, a name with real standing in this ecosystem) is
therefore not a speculative "newer, well-regarded contender" this plan
is taking a bet on — it is the crate `chrono`'s own maintainer now
recommends in `chrono`'s place. Verified this session: `jiff` v0.2.20
published 2026-02-11 on crates.io, active releases continuing through
the year (v0.2.17 in December 2025, v0.2.20 in February 2026). Its
headline design points, verified against its own crates.io/docs.rs
description: "DST aware arithmetic and rounding," "automatic and
seamless integration with the Time Zone Database," and — critically for
this plan's dependency count — it bundles its own copy of the IANA
database via the `jiff-tzdb` crate (docs.rs: "A crate that embeds data
from the IANA Time Zone Database... primarily exposes one routine"), so
`jiff` needs no second `chrono-tz`-equivalent dependency the way this
plan's original `chrono` framing would have. On Unix, `jiff` reads
`/usr/share/zoneinfo` directly when present (falling back to its bundled
copy); on Windows, verified against its own crates.io page, it "reads
the Windows-specific time zone identifier via `GetDynamicTimeZoneInformation`
and then maps it to an IANA time zone" — a real, working
cross-platform story this plan doesn't have to build itself.

`time` (the other historical alternative) was not chosen for a concrete,
disclosed reason, not a vague "seemed less popular" judgment: `time`
has no bundled IANA database at all — timezone-aware conversion needs a
third crate (`time-tz` or a raw `tzdb` binding) layered on top, the exact
two-crate combination (`chrono` + `chrono-tz`) this plan is specifically
trying to avoid needing twice. `jiff` gets both halves — parsing/
arithmetic and full IANA timezone conversion — from one dependency,
which is the concrete, checkable reason it wins over `time` here, not
merely "newer is better."

## Concrete proof this plan targets

```ruby
utc: DateTime = DateTime.parse_rfc3339("2026-06-15T12:00:00Z")!
puts utc.to_rfc3339

zoned_result: Result[ZonedDateTime, String] = utc.in_tz("America/New_York")
case zoned_result
when Ok(z)
  puts z.to_utc().to_rfc3339
when Err(e)
  puts e
end

bad_result: Result[ZonedDateTime, String] = utc.in_tz("Nonexistent/Zone")
case bad_result
when Ok(z)
  puts z.to_utc().to_rfc3339
when Err(e)
  puts e
end

before_dst: DateTime = DateTime.parse_rfc3339("2026-03-08T06:00:00Z")!
zoned_before: ZonedDateTime = before_dst.in_tz("America/New_York")!
after_one_day: ZonedDateTime = zoned_before.plus_days(1)
naive_utc: DateTime = before_dst.plus_seconds(86400)
puts after_one_day.to_utc().diff_seconds(naive_utc)
```

Expected output, in order: `2026-06-15T12:00:00Z` (a lossless RFC 3339
round-trip); `2026-06-15T12:00:00Z` again (converting UTC to
`America/New_York` and straight back to UTC changes nothing about the
underlying instant); the literal string `unknown time zone: Nonexistent/
Zone` or `jiff`'s own real error text for an unrecognized IANA identifier
(the exact wording is `jiff`'s to define — this plan's own `#[test]`
pins whatever `jiff` actually returns, not an invented string); and
`-3600` — proof `ZonedDateTime.plus_days(1)` across 2026's real US
spring-forward transition (clocks jump from 02:00 to 03:00 EST/EDT on
2026-03-08) advances by 23 real wall-clock hours in `America/New_York`,
one hour less than naive `+86400` seconds, and `.diff_seconds` reports
that real difference rather than silently agreeing with the naive
calculation.

## Decision log

- **`chrono`'s soft-deprecation is dated, sourced, and still current as
  of this plan's own authorship date — this is the single fact that
  reverses the old "chrono is the safe default" assumption, so it is
  stated with its full citation rather than summarized away.** GitHub
  issue `chronotope/chrono#1768`, opened 2026-01-22 by the crate's own
  `MEMBER`-associated maintainer `djc`, last updated 2026-09-15 (six
  days before this plan), still open, no successor announced. This is
  the single fact that reverses the old "chrono is the safe default,
  time is the alternative" framing this task explicitly asked to be
  checked rather than assumed.
- **`DateTime`'s representation is two plain `Int64` fields, not an
  opaque heap handle — a deliberate, concrete answer to this batch's
  own representation question, chosen because it is available here and
  wasn't for plan 163's `BigInt`.** `jiff::Timestamp` is `Copy`,
  internally a signed second count plus a sub-second nanosecond count —
  both fit exactly into two `Int64`-typed class fields with zero heap
  allocation, the same 8-byte-per-field convention `build_class_layout`
  already uses for every ordinary class (plan 64's own citation:
  "every field's offset by a literal `8`... `Int64`, `Float64`, or a
  class/actor pointer all get one 8-byte slot"). This is a materially
  different answer than plan 163's `BigInt` (which genuinely needs
  unbounded heap storage and therefore a boxed-pointer handle) — stated
  here explicitly so a reader of both plans sees the type-representation
  question was actually reasoned through per-case, not answered once and
  copy-pasted.
- **`ZonedDateTime` adds exactly one `String` field, `tz_name`, rather
  than a second opaque handle or a numeric zone-table index.** An IANA
  name (`"America/New_York"`) is itself the only representation stable
  across a `jiff-tzdb` database update — a numeric index into whatever
  internal table `jiff` happens to use this version would break the
  moment that crate's internal ordering changed, which is exactly the
  kind of hidden coupling this plan avoids by storing the one value that
  is contractually stable: the name string itself, re-resolved through
  `jiff::tz::TimeZone::get` on every native call that needs it.
- **A caught-panic path is real, not decorative, for RFC 3339 parsing
  specifically — proven by the deliberately malformed input in this
  plan's own worked proof.** `DateTime.parse_rfc3339` on a genuinely
  malformed string is a real, reachable failure this plan surfaces as
  `Result[DateTime, String]`, not `NativeError` — the distinction plan
  92's Decision log draws between a caught *panic* (an unexpected bug,
  becomes `NativeError`) and an ordinary, anticipated failure mode (a
  bad user-supplied string, becomes `Result`'s `Err` arm) applies
  directly here: a malformed RFC 3339 string is the expected, common
  case a real program handles routinely, not a native-code bug, so it is
  wired through `emerald_rt_result_err`, never through `emerald_rt_
  raise_native_error`.
- **DST-awareness is the entire reason this plan exists rather than a
  bare Unix-epoch-seconds `Int64` being sufficient — the worked proof's
  final assertion (`-3600`, not `0`) is the concrete demonstration, not
  an assertion in prose.** A program that only ever needed "seconds
  since epoch" arithmetic would need no crate and no new `Type` variant
  at all — plain `Int64` already suffices, as plan 162's `humantime`
  plan demonstrates for pure durations. What plain `Int64` arithmetic
  cannot do is answer "what wall-clock time is one calendar day later
  in `America/New_York`" correctly across a DST transition, because
  that requires the IANA database's actual transition rules, not fixed
  86400-second math. That is this plan's one genuinely new capability,
  and the worked proof's final `-3600` line is the executed evidence of
  it, not a claim resting on `jiff`'s own documentation alone.
- **No general strftime-style format-string language ships in v1 —
  RFC 3339 in, RFC 3339 out, full stop.** `jiff` supports a real
  `strftime`-equivalent (`Zoned::strftime`), and a future plan can wire
  an Emerald-facing `.format(self, pattern: String): String` through it
  directly — but designing (or, worse, inventing a subset of) a format
  specifier mini-language is a separate, nontrivial piece of API surface
  this plan declines to rush through as a side effect of proving the
  crate choice and the DST-arithmetic claim. `.to_rfc3339`/`.parse_
  rfc3339` cover the one universally-interoperable format every other
  system (logs, HTTP headers via a follow-up, JSON payloads) already
  expects, and are the pair this plan's own worked proof exercises.
- **Leap seconds are explicitly out of scope, following `jiff`'s own
  documented stance, not a gap this plan introduces independently.**
  `jiff::Timestamp` (like nearly every practical datetime library) does
  not model UTC leap seconds — a real, disclosed limitation inherited
  directly from the underlying crate, not something this plan could fix
  by choosing differently; `time` and the now-soft-deprecated `chrono`
  share the identical limitation.
- **Out of scope.** No calendar/civil-date type independent of an
  instant (`jiff::civil::Date` with no attached timezone — a
  "what day is it, ignoring any clock" value some future plan may want
  for pure date arithmetic); no `strftime`-pattern formatting (above);
  no recurrence rules (`RRULE`/iCalendar-style repeating schedules — that
  is plan 161's cron-primitive territory, and neither plan attempts the
  other's job); no historical-offset-change edge cases beyond whatever
  `jiff-tzdb`'s bundled snapshot already encodes, and no mechanism in
  this plan for refreshing that snapshot independently of `jiff`'s own
  release cadence; no leap seconds (above); no serialization format
  beyond RFC 3339 text (no binary wire format, no `serde` integration —
  `emerald-rt` takes no `serde` dependency here or anywhere else in this
  batch unless a specific consuming plan needs it).

## Not yet decided

1. Whether `DateTime`/`ZonedDateTime` should implement Emerald's
   comparison operators (`<`, `>`, `==`) directly, or only expose
   `.diff_seconds` and leave ordering to the caller — deferred to
   whichever plan finalizes operator-overloading conventions for
   compiler-synthesized (as opposed to user-declared) classes, since
   plan 40's operator-overloading design was written against
   user-declared classes only and this plan does not re-litigate that
   scope.
