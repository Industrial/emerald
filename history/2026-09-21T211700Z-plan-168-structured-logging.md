2026-09-21T21:17:00Z

---
name: Structured Logging — `tracing`/`tracing-subscriber` Wrapped as `Log`
overview: "A compiler-provided `Log` module (`Log.trace`/`.debug`/`.info`/`.warn`/`.error`, each with a `Log.<level>_fields` sibling taking a new opaque `LogFields` builder handle) backed by the `tracing` 0.1 event/level machinery and a hand-written `tracing_subscriber::Layer` living in plan 91's `emerald-rt` crate, switchable at process start between compact plain-text and single-line JSON output; the foundational plan of this batch in the literal sense that every later plan's native Rust code — HTTP, TLS, databases, image/audio/PDF codecs — only becomes debuggable in a real running Emerald program once this module exists to observe it."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-emerald-rt-log-core
    content: "Add `tracing = \"0.1\"` and `tracing-subscriber = { version = \"0.3\", features = [\"registry\"] }` to `crates/emerald-rt/Cargo.toml` (the crate plan 91 scaffolds); write `emerald-rt/src/log.rs` exporting `#[no_mangle] extern \"C\" fn emerald_rt_log_configure(level: *const c_char, format: *const c_char) -> i64`, `emerald_rt_log_event(level: i64, message: *const c_char) -> i64`, and the `LogFields`-consuming pair below — every one of them `std::panic::catch_unwind`-wrapped per plan 91's established convention, returning `-1` on any panic or invalid-UTF8/null-pointer input, `0` on success."
    status: pending
  - id: leaf-logfields-handle
    content: "A new opaque handle type, `emerald_rt_log_fields_new() -> *mut LogFields`, `emerald_rt_log_fields_set(handle: *mut LogFields, key: *const c_char, value: *const c_char) -> i64`, and `emerald_rt_log_event_fields(level: i64, message: *const c_char, handle: *mut LogFields) -> i64` (consumes and frees the handle unconditionally, whether or not the event was actually emitted by the active level filter) — a self-contained, minimal worked instance of the create/mutate/consume-and-free resource lifecycle plan 93 will generalize, chosen specifically so this plan does not have to wait on plan 93's file to exist to ship something correct today."
    status: pending
  - id: leaf-json-layer
    content: "A hand-written `tracing_subscriber::Layer` (not `fmt().json()`) whose `on_event` visits each event's fields with a custom `tracing::field::Visit` impl, accumulates them into a `serde_json::Map<String, Value>`, and writes one `serde_json::Value::Object` per line to stderr via `serde_json::to_writer` — chosen over the subscriber's built-in JSON formatter because this plan's own dynamic `LogFields` payload needs to land as a real nested JSON object, not a second, already-escaped JSON string embedded inside a string field."
    status: pending
  - id: leaf-sema-codegen-dispatch
    content: "`Log.<level>(message: String): Void` and `Log.<level>_fields(message: String, fields: LogFields): Void` dispatched exactly the way plan 45's `File.read`/`File.write` are: a `matches!(recv.as_ref(), Expr::Ident(n) if n == \"Log\")` arm in `infer_expr_type`, checked before the real `ClassInfo`/module-dispatch arm (never colliding, since `Log` is never declared via a source `ModuleDef`), and a matching early-return in `build_method_call` calling the five `emerald_rt_log_*` functions directly; `LogFields` becomes a new opaque reference `Type`/`ValKind` variant, codegen-identical in shape to how plan 59's `Type::CString` is \"sema-only... same bare pointer, no new runtime representation.\""
    status: pending
  - id: leaf-configure-once-semantics
    content: "`Log.configure` installs a global `tracing_subscriber::registry().with(layer).init()`-equivalent default dispatcher exactly once, guarded by a `std::sync::OnceLock<()>` inside `emerald-rt`; a second call returns `-1` (already configured) rather than panicking on `tracing::subscriber::SetGlobalDefaultError`, and any `Log.*` call made before the first `Log.configure` uses a built-in default (level `info`, format `text`) so a program that never calls `Log.configure` still logs something sane rather than silently dropping every event."
    status: pending
  - id: leaf-example-and-tests
    content: "`examples/structured_logging_proof.em` (the Concrete Proof below), wired into `emerald-cli/tests/examples.rs`'s checked table per plan 91's own `leaf-example-and-full-gate` precedent, plus `#[test]`s in `emerald-rt` itself asserting: (a) `emerald_rt_log_configure` is idempotent (second call returns `-1`, first returns `0`), (b) the JSON layer emits valid, `serde_json::from_str`-parseable output for a message with a two-entry `LogFields`, and (c) a level below the configured filter produces zero output bytes."
    status: pending
isProject: false
---

# Plan 168 — Structured Logging

This is the first of eight sibling plans (168-175) in the 91-191 batch
extending Emerald's stdlib by wrapping vetted, widely-used Rust crates
through the `emerald-rt` mechanism plan 91 built and plan 92 (FFI/ABI
conventions, authored in parallel this session, not yet on disk) will
formalize project-wide. Like plan 60 stating its assumptions about
plan 59 before that file existed, this plan states its assumptions
about plans 92, 93, 95, and 118 prospectively — by number, using the
shapes the commissioning brief describes for them — rather than waiting
for their files to land first.

Logging is being written first in this batch for a concrete, non-hype
reason: every one of the other 99 plans in 92-191 adds real, fallible
Rust code — network calls, file codecs, cryptography, database
clients — and none of it is observable from inside a running Emerald
program without some way to emit a line of text a human or a log
aggregator can read. `puts` (plan 45) is not that mechanism: it has no
levels, no structured fields, and no way to distinguish "the program
printed something for the user" from "the program is telling an
operator what its HTTP client just did." This plan is deliberately
scoped small and ships one real, working thing — leveled events with
string-valued structured fields, in two output formats — rather than
`tracing`'s full surface (spans, instrumentation macros, per-target
filtering), because the smaller thing is what every later plan actually
needs, verified against what those plans are described to build.

## Concrete proof this plan targets

```ruby
Log.configure("info", "json")

Log.info("service starting")

fields: LogFields = LogFields.new()
fields.set("user_id", "42")
fields.set("plan", "pro")
Log.info_fields("user signed in", fields)

Log.warn("cache miss")
Log.debug("this line is below the configured level and prints nothing")
```

Expected output, one JSON object per line on stderr (stdout carries only
whatever `puts` writes, kept structurally separate):

```
{"level":"INFO","message":"service starting"}
{"level":"INFO","message":"user signed in","fields":{"user_id":"42","plan":"pro"}}
{"level":"WARN","message":"cache miss"}
```

The fourth line never appears — the real proof that `Log.configure("info",
...)`'s level filter is doing its job, not merely that `Log.debug`
compiles. Re-running with `Log.configure("debug", "text")` instead
produces four human-readable lines with no JSON at all, proving both
formats are real, live paths through the same event, not one wrapping
the other.

## Decision log

- **`tracing` + `tracing-subscriber` is the correct, current de facto
  standard, verified today rather than assumed from general Rust
  familiarity.** `docs.rs` shows `tracing-subscriber` at `0.3.23`,
  published 18 September 2026 (three days before this plan is
  authored), owned by `hawkw`/`davidbarsky` under the `tokio-rs`
  GitHub organization, MIT-licensed, with the `tracing` core crate as
  its only hard dependency and `serde`/`serde_json`/`chrono`/
  `nu-ansi-term` all present but optional — meaning this plan's JSON
  and plain-text output modes both come from features already designed
  into the ecosystem's own dependency graph, not bolted on. This is a
  pure-Rust, C-dependency-free choice by construction (no codec, no
  system library, nothing plan 95's crate-vetting policy would need to
  flag as C-backed).
- **The central design tension this plan resolves: `tracing`'s own
  macros (`tracing::info!(user_id = 42, "message")`) are a
  compile-time, per-Rust-callsite mechanism — `Metadata`'s field names
  are `&'static [&'static str]`, fixed once at the macro-expansion
  site — and Emerald call sites are the opposite: an arbitrary,
  runtime-determined set of field names and values, unknown to `rustc`
  when `emerald-rt` itself compiles.** There is no way to make one
  `tracing::info!` invocation accept a variable-length, caller-chosen
  field list; the crate's zero-cost design depends on the field set
  being fixed forever at that one call site. This plan does not fight
  that design — it does not attempt to synthesize a `tracing!` macro
  invocation per Emerald call site (impossible, since Emerald compiles
  ahead-of-time to native code with no Rust source generation step in
  the loop) — instead it uses `tracing`'s lower-level, genuinely
  dynamic path: `tracing::event!(target: "emerald", Level::INFO,
  message = %msg)` for the fixed, one-argument case, and, for the
  fields case, a manually constructed event whose `Visit` consumers
  (see `leaf-json-layer`) read whatever the `LogFields` handle
  happens to contain at call time. The field *names* Emerald code
  chooses are real, runtime `String`s the whole way down; they never
  need to be known to `rustc`.
- **`LogFields` carries `String`-valued fields only in v1, not plan
  118's eventual fully dynamic JSON value — a deliberate, disclosed
  scope cut, not an oversight.** `Hash[K,V]` already exists as a real
  `Type` variant (verified this session against `emerald-sema/src/
  lib.rs`'s `Type` enum, per plan 59's own finding), but this plan does
  not build on `Hash[String, String]` either: this session could not
  verify `Hash[K,V]`'s runtime enumeration mechanism (whether it even
  supports iterating its entries at all, and in what order) against
  real, current source, and building a marshaling path on an
  unverified foundation is exactly the kind of unchecked claim
  `AGENTS.md` already records one false compiler-bug report for. A
  dedicated `LogFields` builder handle sidesteps the question entirely:
  each `.set(key, value)` call is one independent two-`String`-argument
  FFI call (an already-proven-safe shape, identical to
  `emerald_file_write`'s two-`String`-argument call per plan 45), with
  no array, no `Hash` iteration, and no bulk marshaling of any kind.
  Once plan 118 lands with a real dynamic-value type, a
  `Log.<level>_value(message, value)` overload taking that type instead
  of `LogFields` is the natural, additive follow-up — not a redesign of
  what ships here.
- **JSON output is a hand-written `tracing_subscriber::Layer`, not
  `tracing_subscriber::fmt().json()`.** The built-in JSON formatter
  renders whatever `tracing::Value` the event carries for each field —
  fine for the fixed-name, statically-typed fields ordinary Rust code
  passes it, wrong for this plan's dynamic `LogFields`, whose entries
  need to land as sibling keys of one real nested `"fields": {...}`
  JSON object, not as one pre-serialized string value nested inside a
  string (which would double-escape every quote). The custom `Layer`'s
  `on_event` calls `event.record(&mut visitor)` with a `Visit` impl
  that special-cases exactly two well-known field names this plan
  itself defines (`message`, and a JSON-object-valued field carrying
  the `LogFields` entries) and emits real `serde_json::Value::Object`
  structure for both — correctness bought with a small amount of
  hand-written code, not a formatting flag.
- **The global subscriber is a genuinely different resource shape than
  what plan 93 is likely built to model, and this plan says so rather
  than forcing a fit.** Every other stateful thing in this batch (an
  open image decoder, a PDF document being built, an audio decode
  stream) is a *per-call* handle: create it, use it, free it, and a
  program can have several alive at once. `tracing`'s global default
  dispatcher is the opposite — a true process-wide singleton, settable
  exactly once for the process's entire lifetime (`tracing::subscriber
  ::set_global_default` errors on a second call, by the crate's own
  design). This plan defines its own minimal idempotency rule
  (`Log.configure` a second time is a disclosed, non-panicking no-op
  returning `-1`) rather than presupposing plan 93's shape covers
  singletons at all; if plan 93 turns out to generalize this case too,
  this plan's `OnceLock`-guarded implementation is the one real
  existing instance it would need to account for, not something this
  plan needs to change in response.
- **`LogFields` itself is a real, if small, instance of the
  create/mutate/consume-and-free resource lifecycle plan 93 is
  expected to formalize — offered here as a concrete worked example,
  not a promise to redesign later.** `emerald_rt_log_fields_new`
  allocates a boxed `Vec<(String, String)>` behind an opaque pointer;
  `emerald_rt_log_fields_set` pushes one entry; the two
  `Log.*_fields` intrinsics consume the handle unconditionally inside
  `emerald_rt_log_event_fields`, freeing it via `Box::from_raw` whether
  or not the event's level actually passed the active filter (an event
  suppressed by level still owns and must free the `LogFields` handle
  its call site allocated — this plan does not special-case the
  filtered-out path to leak memory instead). Codegen never gives
  Emerald source a way to call `.set` on a handle after it has been
  passed to `.info_fields`/etc. — this plan does not add a
  use-after-free *check*, mirroring plan 45's own disclosed
  unchecked-access precedent for `String`/`Array` rather than inventing
  new safety machinery this one leaf doesn't need.
- **Plan 92's FFI/ABI conventions govern the calling shape (`catch_unwind`
  at every boundary, an `i64` status-code return), but its binary-safe
  `(ptr, len)` buffer convention does not apply here.** Every value
  this plan moves across the FFI boundary is either a plain Emerald
  `String` (already a bare null-terminated `char*` per plan 59, no
  length needed) or a small `i64`/opaque pointer — there is no raw
  binary payload anywhere in structured logging, unlike the image,
  PDF, and audio plans later in this batch. This plan is cited by
  those plans as the first real, working instance of plan 91's
  panic-boundary discipline applied to something beyond the one-line
  proof function plan 91 itself shipped, not the other way around.
- **Level filtering is a configured floor, not a per-call decision —
  `tracing`'s own `Level` ordering (`TRACE < DEBUG < INFO < WARN <
  ERROR`) is reused directly, with no Emerald-side reimplementation.**
  `Log.configure`'s `level` string parses to a `tracing::Level` once;
  every subsequent `Log.<level>(...)` call constructs a
  `tracing::Level` for its own call site and compares it against the
  configured floor before doing any formatting work at all — a
  filtered-out `Log.debug` call, once level checking is added, costs a
  handful of comparisons and zero allocation, matching `tracing`'s own
  stated zero-cost-when-disabled design intent.
- **Out of scope.** No spans, no `#[instrument]`-equivalent nested
  context, no `RUST_LOG`-style environment-variable filtering (level
  and format are both explicit `Log.configure` arguments — an Emerald
  program's logging configuration is source-visible, not
  environment-dependent, matching this project's static-everything
  identity), no per-target or per-module filtering, no log rotation or
  file sinks (output is always stderr in v1; redirecting it is the
  operating system's job, the same posture `puts`/stdout already take),
  no async/non-blocking appender (`tracing-appender`), and no
  `LogFields` value types beyond `String` (see above — that is plan
  118's eventual job, additive, not a redesign of this plan's surface).

## Not yet decided

1. Whether `Hash[K,V]` supports runtime enumeration at all today, and
   if so in what order — genuinely not verified this session against
   real, current `emerald-sema`/`emerald-codegen` source. This plan
   does not block on the answer (see the `LogFields`-over-`Hash`
   decision above), but a future revision collapsing `LogFields` into
   plain `Hash[String, String]` once that mechanism is confirmed real
   is a plausible, smaller follow-up, not a blocking prerequisite here.
