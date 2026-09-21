2026-09-21T21:10:00Z

---
name: Cron-Style Scheduling Primitives — a `CronSchedule` Opaque Handle
overview: "One new opaque-handle class, `CronSchedule`, wrapping a heap-boxed, parsed `cron::Schedule` (the `zslayton/cron` crate — actively released, verified this session at crates.io's `^0.17.0` range with a 2026-06-18 publish date), exposing `Cron.parse(expr: String): Result[CronSchedule, String]` and `.next_after(self, unix_epoch_secs: Int64): Int64` for computing the next fire time after a given instant. This is a scheduling PRIMITIVE — expression parsing and next-fire-time arithmetic only — not a job-scheduler daemon: no background thread, no timer loop, no process that sleeps and fires callbacks. A program that wants an actual running scheduler composes this plan's two functions with an ordinary Emerald loop and plan 160's `DateTime`/`ZonedDateTime`, or with plan 54/55's actor model for concurrent firing — building that composition is explicitly out of scope here."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vendor-cron-and-scaffold-module
    content: "Add `cron = \"0.17\"` to `crates/emerald-rt/Cargo.toml` (verified this session: crates.io shows the `zslayton/cron` crate actively published within the `^0.17.0` range as of 2026-06-18, with `docs.rs/cron` describing it plainly as \"A cron expression parser and schedule explorer\" — a small, single-purpose crate with no transitive scheduling-daemon machinery to audit away). Create `crates/emerald-rt/src/cron.rs`, `mod cron;` from `lib.rs`."
    status: pending
  - id: leaf-cronschedule-opaque-handle-class
    content: "Register a compiler-synthesized `CronSchedule` class with exactly one field, `handle: Int64`, the same one-field-opaque-handle shape plan 163's `BigInt` uses (see that plan's Decision log for the fuller general argument) — necessary here specifically because `cron::Schedule` is not `Copy`, is not a fixed byte width (it stores parsed per-field bitsets internally, `cron`'s own struct is not `#[repr(C)]` or guaranteed-stable in layout across versions), and genuinely needs heap allocation. `Box::into_raw(Box::new(schedule)) as i64` on construction; every method call reconstructs a `&cron::Schedule` via `&*(handle as *const cron::Schedule)` — never `Box::from_raw` inside a borrowing method, only at an explicit (currently nonexistent — see Decision log) drop/free path, so a schedule used across multiple `.next_after` calls in a loop stays valid for all of them."
    status: pending
  - id: leaf-parse-with-result
    content: "Add `Cron.parse(expr: String): Result[CronSchedule, String]`, calling `cron::Schedule::from_str` and converting its real `Err` (`cron`'s own parse-error type, `Display`-formatted to a `String` — not a hand-rolled validity check) into `emerald_rt_result_err` (plan 92). A malformed expression (verified against the crate's own six-or-seven-field syntax, seconds-included) is this leaf's own disclosed acceptance check via the deliberately-invalid string in this plan's own Concrete Proof."
    status: pending
  - id: leaf-next-after
    content: "Add `.next_after(self, unix_epoch_secs: Int64): Int64` on a `CronSchedule` receiver: reconstruct the boxed `cron::Schedule`, convert the input `Int64` to a `chrono::DateTime<Utc>` (see Decision log for why `cron`'s own API forces exactly one `chrono` value through this one call boundary, and why that is an accepted, disclosed, narrow exception rather than a second full `chrono` dependency for this batch), call `.upcoming(Utc).next()` filtering to the first fire time strictly after the given instant, and convert the result's Unix timestamp back to `Int64`. Return `-1` (a disclosed sentinel, not a panic) for the vanishingly rare case `cron`'s own iterator yields no next occurrence (a schedule that can structurally never fire again, e.g. `* * * * * * 2020` naming a past-only year)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/cron_scheduling_proof.em` (this plan's Concrete Proof below) to `emerald-cli/tests/examples.rs`'s CI-checked table. Add `#[test]`s in `crates/emerald-rt/src/cron.rs`: parsing a fixed valid expression and asserting `.next_after` against a hand-computed expected timestamp for at least two different fixed `unix_epoch_secs` inputs (proving the iterator advances correctly across a call, not just once); parsing a deliberately malformed expression and asserting the `Err` path; and one leak-acknowledgment test (see Decision log) that constructs 1000 schedules in a loop and asserts the process does not crash, not that memory is reclaimed. Run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 161 — Cron-Style Scheduling Primitives

A recurring theme across this stdlib-expansion batch is that Emerald
programs increasingly need to reason about *when* something should
happen, not only *what* happens — plan 160 gave programs a real,
DST-aware notion of an instant and a timezone; this plan gives them a
real way to answer "given this cron expression, when does it next
fire?" without hand-rolling a five-or-six-field cron parser in Emerald
source or falling back to a fixed, hardcoded interval. It is
deliberately, explicitly a **primitive**, not a scheduler: this plan
computes one next-fire-time given one instant and one parsed
expression, synchronously, and returns. It starts no background thread,
registers no OS timer, and runs no callback — a program that wants an
actual running scheduled job loop must write that loop itself in
Emerald (or a future plan may build one on top of this primitive plus
plan 54's actor model), the same way plan 92's Decision log states
about every other domain plan in this batch: this crate is one crate
with one coherent shape, not a place for each domain to invent its own
daemon architecture.

## Concrete proof this plan targets

```ruby
schedule_result: Result[CronSchedule, String] = Cron.parse("0 30 9 * * *")
case schedule_result
when Ok(schedule)
  first: Int64 = schedule.next_after(1780000000)
  puts first
  second: Int64 = schedule.next_after(first)
  puts second
  puts second - first
when Err(e)
  puts e
end

bad_result: Result[CronSchedule, String] = Cron.parse("not a cron expression")
case bad_result
when Ok(schedule)
  puts schedule.next_after(0)
when Err(e)
  puts e
end
```

Expected output, in order: a real `Int64` Unix timestamp for the first
9:30:00 AM UTC strictly after 2026-06-24T13:06:40Z (the instant
`1780000000` seconds after the epoch denotes — the exact value is
`cron`'s own, hand-verifiable arithmetic, pinned by this plan's own
`#[test]`, not invented here); a second, later timestamp for the
following day's 9:30:00 AM UTC; and `86400` — proof two consecutive
daily fires are exactly one day apart, computed by `cron`'s own real
iterator, not a hardcoded interval this plan substitutes for actual
parsing. Then the crate's own real parse-error text (whatever `cron`'s
`Display` impl produces for a malformed six-field string — pinned by
the `#[test]` in `leaf-parse-with-result`, not asserted here as a fixed
literal) for the deliberately invalid expression.

## Decision log

- **`cron` (the `zslayton/cron` crate) was chosen because it is small,
  actively published, and does exactly this plan's job and nothing
  more — verified this session, not assumed from a stale search
  result.** `crates.io/crates/cron/range/^0.17.0` shows a real publish
  date of 2026-06-18; `docs.rs/cron`'s own top-line description is "A
  cron expression parser and schedule explorer" — no bundled scheduler
  loop, no thread pool, no async runtime dependency to audit away
  (satisfying plan 95's pure-Rust-first, minimal-transitive-surface
  posture directly). A sibling crate, `tokio-cron-scheduler`, was
  considered and declined specifically because it *is* a running
  scheduler daemon built on `tokio` — exactly the piece of scope this
  plan's own overview states it will not build; pulling in a `tokio`
  runtime dependency for a synchronous next-fire-time computation would
  be a wildly disproportionate dependency footprint for what this plan
  actually needs.
- **`CronSchedule` is an opaque one-field handle (`handle: Int64`), not
  a packed multi-field class the way plan 160's `DateTime` or plan
  163's `Decimal` are.** The concrete, disclosed reason: `cron::
  Schedule`'s internal representation is not `Copy`, is not a fixed byte
  width, and is not part of that crate's own stable public contract —
  it stores parsed per-field bitsets (which seconds/minutes/hours/days/
  months/weekdays/years are legal) whose exact size depends on the
  expression's own complexity (a `*` field costs nothing; an explicit
  `1,15,30` list costs three entries). There is no fixed number of
  `Int64` fields that could hold every possible parsed schedule, the
  same structural reason plan 163's `BigInt` needs a heap handle and
  plan 160's `DateTime` does not.
- **Constructing a schedule allocates once; every `.next_after` call
  reuses the same boxed value — a real, deliberate amortization, not
  an accident of the handle design.** Re-parsing the expression string
  on every `.next_after` call (treating the handle as disposable) would
  work correctly but throw away the entire point of holding a parsed,
  validated `cron::Schedule` across a loop that calls `.next_after`
  repeatedly — exactly the shape this plan's own worked proof exercises
  (`first`, then `second` off the same `schedule` value). The one-time
  parse cost (and its one, disclosed, possible `Err`) is paid exactly
  once per `Cron.parse` call, not once per fire-time query.
- **No `Drop`/free path exists for a `CronSchedule` handle in this
  plan — a leak, explicitly disclosed and explicitly accepted, not
  hidden.** Emerald's own memory model, verified against plan 45's
  citation of `emerald_alloc`'s own doc comment ("no free... no
  lifetime tracking") and plan 51's arena allocator, already has no
  general object-lifetime tracking for ordinary class instances; a
  boxed `cron::Schedule` behind a `CronSchedule` handle leaks by the
  exact same rule every other heap allocation in this compiler already
  leaks by, for the exact same reason (no tracing GC, no refcounting,
  no borrow-checked ownership crossing the Emerald/native boundary
  today). This plan does not attempt to fix that — a real, scoped
  resource/handle lifetime model is explicitly plan 93's job (cited by
  number in plan 91's own Out of scope bullet: "no resource/handle
  lifetime model (plan 93...)"), not something this narrow scheduling
  primitive invents unilaterally. `leaf-example-and-gate`'s
  thousand-schedule loop test exists specifically to make this leak's
  practical bound visible and checked (it must not crash, even though
  it does not reclaim memory), rather than leaving the leak
  undiscovered until some future plan's much larger workload hits it.
- **`.next_after` forces exactly one `chrono` value through one call
  boundary, a real, disclosed exception this plan does not pretend
  away.** `cron::Schedule`'s own public API (`.upcoming(Utc)`,
  `.after(&DateTime<Utc>)`) is typed against `chrono::DateTime<Utc>`,
  not against plain Unix-second integers or against `jiff`'s types —
  `cron` was last observed (via its own dependency manifest, checked
  this session) to depend on `chrono` itself, independent of plan 160's
  own crate choice. This plan does not attempt to fork or reimplement
  `cron`'s internals to avoid that dependency, and does not attempt to
  thread plan 160's `jiff`-backed `DateTime` through `cron`'s own typed
  API either — the one, narrow conversion (`Int64` Unix seconds ->
  `chrono::DateTime<Utc>` -> back to `Int64`) happens entirely inside
  `emerald_rt_cron_next_after`'s own Rust body and is never exposed to
  Emerald source, so no Emerald-facing type or plan-160 dependency is
  introduced by this plan — the `chrono` dependency is real, but fully
  contained, transitive, and invisible to any Emerald program calling
  this plan's API. This is a real, if slightly uncomfortable,
  consequence of `cron`'s own choice of `chrono` as its wire type,
  disclosed here rather than glossed over; it does not contradict plan
  160's own separate finding that `chrono` is soft-deprecated for
  *this project's own, Emerald-facing, user-visible* date/time surface
  — a small, pinned transitive dependency pulled in unavoidably by one
  narrow, vetted crate is a different risk profile than building this
  project's own primary datetime type on it.
- **`.next_after` returns `-1` on "no further occurrence," a disclosed
  sentinel, not `NativeError` and not `Result`.** A schedule that
  structurally cannot fire again (an explicit past year field) is a
  vanishingly rare, arguably malformed-by-construction case `cron`'s
  own iterator already handles by yielding `None` rather than panicking
  — treating it as a native bug (`NativeError`) would be wrong (nothing
  crashed), and wrapping every `.next_after` call in `Result[Int64,
  String]` for a case this narrow would burden every ordinary call site
  with `?`/`case` handling for an occurrence that, in ordinary
  cron-expression usage (recurring daily/weekly/monthly schedules with
  no year field at all), can never actually happen. `-1` is a real,
  disclosed narrowing (a valid cron expression by construction, per the
  parse step, but a "will never fire again" runtime fact only
  discoverable by actually querying it) — a future plan revisiting
  error ergonomics project-wide may reconsider this, but is not this
  plan's job to pre-empt.
- **Out of scope.** No running scheduler/daemon of any kind — no
  background thread, no `sleep`-and-fire loop, no callback registration
  API (stated plainly in the overview, restated here for the Decision
  log's own completeness); no cron-expression *generation* (producing
  a cron string from a human description, the reverse direction of
  `Cron.parse`); no timezone-aware cron evaluation beyond whatever UTC
  arithmetic `cron`'s own `.upcoming(Utc)` provides (a schedule
  evaluated "in `America/New_York`" needs the caller to convert via
  plan 160's `ZonedDateTime` before and after calling this plan's
  UTC-only primitive — this plan does not attempt to marry `cron`'s
  `chrono`-typed API directly to plan 160's `jiff`-typed one); no
  handle-freeing/lifetime mechanism (plan 93, above); no persistence or
  distributed-scheduling story of any kind (that would compose plan 60's
  distributed actors with this primitive in some future plan, not this
  one).

## Not yet decided

1. Whether a convenience `.next_n_after(self, unix_epoch_secs: Int64,
   n: Int64): Array[Int64]` (returning several upcoming fire times at
   once, sidestepping repeated FFI-boundary crossings for a caller that
   wants, say, the next ten occurrences) is worth adding given `Array[T]`'s
   own no-length-metadata representation (plan 45's finding) — the
   caller would need a paired count the same way `.split_count` pairs
   with `.split`, and whether that ergonomic win justifies the added
   surface is left for a follow-up rather than decided here.
