2026-09-21T21:19:00Z

---
name: Distributed Tracing (OpenTelemetry) — `opentelemetry` + `opentelemetry-otlp` Wrapped as `Trace`
overview: "A `Trace` module (`Trace.configure`, `.span_start`/`.span_start_child`, `.span_set_attr`, `.span_end`) wrapping the official `opentelemetry`/`opentelemetry_sdk`/`opentelemetry-otlp` 0.33 Rust implementation, exporting spans over OTLP/HTTP with a synchronous `SimpleSpanProcessor` and `reqwest`'s blocking client specifically to avoid pulling an async runtime into `emerald-rt` ahead of plan 94's own decision on that; stretch-tier and the heaviest-lift plan in this batch of eight — v1 ships real span creation, parent/child nesting, and real OTLP export, explicitly declining W3C trace-context propagation, sampling, and full semantic-conventions compliance."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-emerald-rt-tracer-provider
    content: "Add `opentelemetry = \"0.33\"`, `opentelemetry_sdk = \"0.33\"` (default features off, `trace` enabled), and `opentelemetry-otlp = { version = \"0.33\", default-features = false, features = [\"trace\", \"http-proto\", \"reqwest-blocking-client\"] }` to `crates/emerald-rt/Cargo.toml` — this exact feature set (verified against `docs.rs/crate/opentelemetry-otlp/0.33.0/features` this session) routes OTLP export over plain HTTP/protobuf via `reqwest`'s synchronous client, never through `grpc-tonic` (which needs `tonic`+`hyper`+an async executor) and never through the crate's async `TonicExporterBuilder` path."
    status: pending
  - id: leaf-configure-once
    content: "`emerald_rt_trace_configure(otlp_endpoint: *const c_char) -> i64`: builds one `opentelemetry_sdk::trace::TracerProvider` with a `SimpleSpanProcessor` (verified real, present in `opentelemetry_sdk` 0.33.0's `trace` module this session) wrapping the OTLP HTTP exporter pointed at the given endpoint, installs it as the process global via `opentelemetry::global::set_tracer_provider`, guarded by the same `OnceLock` idempotency idiom plans 168/169 already established — a second call returns `-1`."
    status: pending
  - id: leaf-span-handle
    content: "A new opaque `TraceSpan` handle (an owned `opentelemetry::global::BoxedSpan`, boxed and returned as a raw pointer): `emerald_rt_trace_span_start(name: *const c_char) -> *mut TraceSpan` (root span, no parent context), `emerald_rt_trace_span_start_child(name: *const c_char, parent: *mut TraceSpan) -> *mut TraceSpan` (starts inside the parent's `Context`, via `opentelemetry::trace::TraceContextExt`), `emerald_rt_trace_span_set_attr(span: *mut TraceSpan, key: *const c_char, value: *const c_char) -> i64` (`Span::set_attribute`, string-valued only), `emerald_rt_trace_span_end(span: *mut TraceSpan) -> i64` (calls `Span::end()`, which is what actually hands the span to the `SimpleSpanProcessor` for synchronous export, then frees the boxed handle)."
    status: pending
  - id: leaf-sema-codegen-dispatch
    content: "`Trace.configure(endpoint: String): Void`, `Trace.span_start(name: String): TraceSpan`, `Trace.span_start_child(name: String, parent: TraceSpan): TraceSpan`, `Trace.span_set_attr(span: TraceSpan, key: String, value: String): Void`, `Trace.span_end(span: TraceSpan): Void` — dispatched via the same `Name.method(args)` intrinsic arm plan 45/168/169 already established for `File`/`Log`/`Metrics`; `TraceSpan` is a new opaque reference `Type`, sema-only, same shape as plan 168's `LogFields`."
    status: pending
  - id: leaf-example-and-tests
    content: "`examples/opentelemetry_proof.em` (Concrete Proof below), run against a throwaway `std::net::TcpListener`-based mock OTLP/HTTP receiver in the example's own harness (accept one connection, read the raw request bytes, assert they parse as a well-formed `ExportTraceServiceRequest` protobuf via `prost`, assert two spans with the expected parent/child `trace_id`) rather than requiring a real OpenTelemetry Collector to be running; matching `#[test]`s inside `emerald-rt` doing the same assertion directly in Rust."
    status: pending
isProject: false
---

# Plan 170 — Distributed Tracing (OpenTelemetry)

This is the third of eight sibling plans (168-175) in the 91-191 batch,
and the most expensive one to get right. Plan 168 (structured logging)
is foundational; plan 169 (metrics) is a real but self-contained
stretch; this plan is stretch-tier *and* genuinely heavier-lift than
either — full OpenTelemetry is a large surface (three signal types,
context propagation, samplers, resource detection, a dozen exporter
transports), and v1 here claims exactly one corner of it: creating
spans, nesting them, attaching string attributes, and exporting them
over OTLP/HTTP to a real collector. Everything else named above is
explicitly deferred — see the Decision log and Out of scope.

## Concrete proof this plan targets

```ruby
Trace.configure("http://localhost:4318/v1/traces")

root: TraceSpan = Trace.span_start("handle_request")
Trace.span_set_attr(root, "route", "/users/42")

child: TraceSpan = Trace.span_start_child("db_query", root)
Trace.span_set_attr(child, "db.statement", "SELECT 1")
Trace.span_end(child)

Trace.span_end(root)

puts "trace exported"
```

Expected behavior: `Trace.span_end(child)` and `Trace.span_end(root)`
each block, synchronously, until the `SimpleSpanProcessor` has finished
handing that one span to the OTLP/HTTP exporter's blocking `reqwest`
`POST` — so `"trace exported"` only prints after both real HTTP
requests have completed. Verified two ways: (1) against a real, locally
running OpenTelemetry Collector with its `otlp` HTTP receiver on
`:4318`, whose own stdout exporter should print both spans with a
shared `trace_id` and the child's `parent_span_id` equal to the root's
`span_id`; (2) in CI, against a throwaway `TcpListener`-based mock
receiver that captures the raw request bytes and asserts they parse as
a real `ExportTraceServiceRequest` protobuf message with the same
two-span, one-parent-one-child shape — no live collector dependency
needed for the automated check.

## Decision log

- **`opentelemetry` + `opentelemetry_sdk` + `opentelemetry-otlp`,
  verified today, is the real, official Rust implementation — not a
  third-party wrapper.** `docs.rs` shows all three at `0.33.0`,
  published 18 September 2026 (three days before this plan is
  authored), owned by `cijothomas`/`TommyCpp` under the
  `open-telemetry` GitHub organization itself, Apache-2.0-licensed.
  This is the CNCF project's own Rust SDK, the same one that produces
  the Collector-compatible OTLP wire format every other language's
  OpenTelemetry SDK also targets — choosing anything else would mean
  Emerald traces speaking a dialect no real backend understands.
- **The central architecture decision: export synchronously, over
  HTTP/protobuf, via `reqwest`'s blocking client — never through
  `grpc-tonic` or the SDK's batch/async path — specifically because
  this project has no async runtime yet.** `opentelemetry_sdk`'s own
  `Cargo.toml` (checked this session) lists `tokio`, `tokio-stream`,
  and `futures-executor` all as *optional* dependencies — the crate is
  usable without any of them — and `opentelemetry-otlp` 0.33.0
  genuinely offers a `reqwest-blocking-client` feature alongside
  `http-proto` (both verified present in its real feature list this
  session) specifically for exactly this synchronous use case.
  `SimpleSpanProcessor` (verified present in `opentelemetry_sdk`
  0.33.0's `trace` module) is the SDK's own synchronous processor —
  unlike `BatchSpanProcessor`, it calls the exporter directly, inline,
  the moment a span ends, with no background task and no executor to
  configure. This plan's whole dependency graph, `emerald-rt`-side, is
  therefore free of `tokio` — a deliberate, disclosed constraint that
  keeps this plan from silently forcing plan 94's async-runtime
  decision ahead of schedule for the rest of the project.
- **The honest cost of that decision: every `Trace.span_end` call is a
  blocking network round-trip, not a fire-and-forget enqueue.** A
  real production tracing setup wants `BatchSpanProcessor` — spans
  queued in memory, flushed periodically or in batches, so the hot
  path never blocks on network I/O. This plan's `SimpleSpanProcessor`
  choice means a program emitting spans in a tight loop pays one full
  HTTP request's latency per span, synchronously, on the thread that
  called `span_end`. This is disclosed here as the real, load-bearing
  tradeoff it is, not hidden behind "v1 is simpler" — the correct fix
  is switching to `BatchSpanProcessor` once plan 94 gives `emerald-rt`
  a real async executor to run its background flush task on, at which
  point this plan's synchronous exporter becomes the batched one with
  no change to the Emerald-facing `Trace.*` API at all.
- **`TraceSpan` is a real plan-93-shaped resource — create, mutate,
  end — and ending it is not optional cleanup, it is the export
  trigger.** `opentelemetry::trace::Span::end()` is what hands the
  finished span to the `SimpleSpanProcessor`; a `TraceSpan` handle an
  Emerald program allocates via `span_start`/`span_start_child` and
  never passes to `span_end` is not merely a memory leak (though it is
  one, per this project's existing disclosed no-free precedent for
  unmanaged handles) — it is silently discarded telemetry, with no
  export ever attempted for it. This is a sharper, more
  operator-visible failure mode than an ordinary forgotten `free()`,
  and this plan states it plainly rather than leaving it implicit in
  "resources must be released."
- **Span attributes are direct `.span_set_attr(span, key, value)`
  calls, not a `LogFields`-style pre-built handle reused from plan
  168.** A log event's fields are all known at one call site and
  handed over once; a span's attributes accumulate across its entire
  lifetime, interleaved with arbitrary other Emerald code running
  between `span_start` and `span_end` — genuinely a different shape,
  matching `opentelemetry`'s own `Span::set_attribute` idiom (called
  repeatedly, any time, not batched into one builder). Forcing plan
  168's one-shot builder onto a fundamentally multi-shot use case would
  be reuse for its own sake, not because the shapes actually match.
- **No semantic-conventions compliance in v1 — attribute and span
  names are whatever `String` the caller passes, unchecked against the
  OpenTelemetry specification's controlled vocabulary
  (`http.request.method`, `db.system`, etc.).** Real semantic
  conventions are a large, versioned, still-evolving specification
  document, not a fixed API surface this plan could realistically
  encode as compiler-checked constants without committing to tracking
  every future revision. `Trace.span_set_attr(span, "db.statement",
  "SELECT 1")` in the worked proof above uses a real semantic-
  convention-shaped key purely as a good-practice example for the
  reader — the compiler enforces nothing about it.
- **The global `TracerProvider` reuses the exact install-once idiom
  plans 168 and 169 already established, for the same underlying
  reason: `opentelemetry::global::set_tracer_provider` sets a true
  process-wide singleton.** `emerald_rt_trace_configure`'s
  `OnceLock`-guarded wrapper returns `-1` on a redundant call rather
  than silently replacing an already-exporting provider mid-program (a
  correctness hazard: an in-flight span holding a reference to the old
  provider's processor could otherwise export against a torn-down
  exporter).
- **A genuinely strong architectural fit for Emerald specifically,
  named here even though v1 does not build it: plan 60's distributed
  actors are exactly the kind of network-crossing operation
  trace-context propagation exists for.** Verified against plan 60's
  own worked proof (`2026-09-09T134000Z-plan-60-distributed-actors.md`):
  a `remote.method(args)` call compiles to `emerald_actor_dispatch_
  remote`, which already serializes the method's arguments through a
  compiler-generated wire codec and sends them over one real TCP
  socket to a second OS process (`ClassName.remote(addr, name)`/
  `.register(name, port)`). That wire codec is precisely where a
  W3C-trace-context-shaped `(trace_id, parent_span_id)` pair would ride
  alongside the method arguments already being framed and sent — the
  receiving process's `emerald_actor_dispatch_remote` handler would
  extract it and start a child span (via this plan's own
  `span_start_child`, given a reconstructed remote `Context` rather
  than a local `TraceSpan` handle) before invoking the actor method
  body, giving an operator one continuous trace across the process
  boundary instead of two disconnected ones. This plan does not
  implement that extension — plan 60's wire codec has no spare field
  for it today, and adding one is real, separate design work touching
  plan 60's file, not this one — but it is worth naming as the
  strongest concrete case for why this stretch-tier plan belongs in
  Emerald's stdlib at all, not a generic "tracing is generally useful"
  claim.
- **FFI/ABI notes.** Plan 92's general convention (`catch_unwind` at
  every exported function, `i64` status returns, opaque pointer
  handles) governs everything here; its binary-safe `(ptr, len)`
  buffer convention is not needed — spans, attribute keys, attribute
  values, and the OTLP endpoint URL are all plain null-terminated
  `String`s (plan 59), and `TraceSpan` is an opaque pointer exactly
  like plan 168's `LogFields`.
- **Out of scope.** W3C `traceparent`/`tracestate` propagation across
  plan 60's actor network boundary (named above as the natural future
  home, not built here); any sampler beyond "always sample everything"
  (`opentelemetry_sdk`'s `AlwaysOn` default, unconfigurable from
  Emerald source in v1); resource attribution (`service.name`,
  deployment environment, etc. — no `Trace.set_resource` intrinsic);
  the `logs` and `metrics` OTel signal types (`opentelemetry-otlp`'s
  `logs`/`metrics` features exist and were verified present this
  session but are not enabled — plan 168 and plan 169 already cover
  those two signals through their own, unrelated mechanisms, and
  mixing three different export pipelines into one OTLP collector
  connection is real, separate design work this plan declines);
  `grpc-tonic` transport; and `BatchSpanProcessor`/async export
  (deferred to whenever plan 94 lands, per the tradeoff disclosed
  above).
