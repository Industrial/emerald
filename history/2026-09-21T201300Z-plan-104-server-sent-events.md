2026-09-21T20:13:00Z

---
name: Server-Sent Events (SSE)
overview: "No new Rust crate — SSE is a streaming-response pattern built directly on plan 101's `tiny_http` HTTP server, using `tiny_http::Request::into_writer` (a real, verified 0.12.0 API that consumes the request and hands back a raw `Box<dyn Write + Send>` socket writer, bypassing `tiny_http`'s own buffered `Response`/chunked-encoding path entirely) to hold a connection open behind a plan-93 opaque `u64` handle, so Emerald code can call `.send(event, data)` repeatedly against a still-open response instead of returning one value and closing the connection."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-sse-writer-registry
    content: "A process-wide `static SSE_STREAMS: Mutex<HashMap<u64, Box<dyn Write + Send>>>` inside emerald-rt (or wherever plan 101 places its HTTP runtime module), a monotonic `AtomicU64` handle counter, and poison-recovery on every lock acquisition (`.lock().unwrap_or_else(PoisonError::into_inner)`) since a panic caught by this plan's own catch_unwind boundary must not permanently brick every other open SSE stream"
    status: pending
  - id: leaf-sse-upgrade-and-headers
    content: "`emerald_rt_sse_upgrade(request_handle: i64) -> i64` — consumes plan 101's request handle, calls `Request::into_writer()`, writes a raw `HTTP/1.1 200 OK\\r\\nContent-Type: text/event-stream\\r\\nCache-Control: no-cache\\r\\nConnection: keep-alive\\r\\n\\r\\n` head, registers the writer under a new handle, exposed to Emerald as `Sse.upgrade(request: Int64): Int64`"
    status: pending
  - id: leaf-sse-send-and-format
    content: "`emerald_rt_sse_send(stream: i64, event: *const c_char, data: *const c_char) -> i64` and `emerald_rt_sse_comment(stream: i64, text: *const c_char) -> i64` — real SSE wire-format framing (`event: <name>\\n`, one `data: <line>\\n` per line of a multi-line payload, terminating blank line, `: <text>\\n` for comments/keepalives), an explicit `.flush()` after every write, exposed as `Sse.send(stream, event, data): Int64` / `Sse.comment(stream, text): Int64`, both Int64-booleans per plan 59/92's C-side-boolean convention"
    status: pending
  - id: leaf-sse-close
    content: "`emerald_rt_sse_close(stream: i64) -> i64` — removes and drops the registry entry (shutting the socket down), exposed as `Sse.close(stream: Int64): Int64`; a `.send()`/`.comment()` against an already-closed or unknown handle returns `0` rather than panicking (a real, disclosed use-after-close contract, not UB — see Decision log)"
    status: pending
  - id: leaf-example-and-gate
    content: "`examples/sse_ticker.em` (the Concrete Proof below) plus a small shell verification step (`curl -N` against the running example) wired into whatever CI mechanism plan 101's own server examples already use for external-client verification; a Rust `#[test]` in emerald-rt spawning a real `std::thread`-backed server and asserting on raw bytes read back over a plain `std::net::TcpStream` client, proving the mechanism with no `curl` dependency in the test itself"
    status: pending
isProject: false
---

# Plan 104 — Server-Sent Events (SSE)

Plan 101 gives Emerald a synchronous HTTP server: bind, accept one request at
a time, respond, repeat. That covers request/response, but a real class of
program — a live ticker, a build-log tail, a chat notification feed — needs
one more shape: a response that never really "finishes" in the normal sense,
where the server keeps writing to an already-open connection as new data
becomes available, and the client (any browser's `EventSource`, or `curl -N`)
reads each `data: ...` line as it arrives. That is exactly what Server-Sent
Events is: a `Content-Type: text/event-stream` response, kept open, written
to incrementally. This plan does not add a crate — SSE is a wire format
(defined by the WHATWG HTML spec, not by any Rust library) layered on top of
one already-real capability of `tiny_http` 0.12.0 that plan 101's own
request/response cycle does not otherwise reach: `Request::into_writer`,
verified this session directly against `tiny_http`'s published 0.12.0 API
(`docs.rs/tiny_http/latest/tiny_http/struct.Request.html`) — `pub fn
into_writer(self) -> Box<dyn Write + Send + 'static>`. It consumes the
`Request` and hands back the raw socket writer, entirely bypassing
`tiny_http`'s own `Response`/`respond()` abstraction (the one a search this
session turned up a real, disclosed caveat about: "common minimal Rust HTTP
servers, e.g. `tiny_http`, buffer chunked responses until the response
completes, which breaks streaming" — a caveat about the *high-level*
`Response`-based chunked-encoding path, not about `into_writer`, which never
goes through that path at all). This plan's entire mechanism is: get the raw
writer once, hand out an opaque handle for it, and let Emerald code write to
it as many times as it wants before closing it.

## Concrete proof this plan targets

```ruby
server: Int64 = HttpServer.listen(0)
port: Int64 = HttpServer.port(server)
puts "listening on #{port}"

req: Int64 = HttpServer.accept(server)
stream: Int64 = Sse.upgrade(req)

Sse.send(stream, "tick", "1")
Sse.send(stream, "tick", "2")
Sse.send(stream, "tick", "3")
Sse.close(stream)
```

`HttpServer.listen(0)` asks the OS to pick a free port (port `0` is the
standard "any free ephemeral port" convention, used specifically so this
example never collides with another process in CI) — `HttpServer.port`
(assumed part of plan 101's own surface) reads it back. Running this program
blocks inside `HttpServer.accept(server)` until a real client connects.
Verification step (external to the `.em` file, the same "run from a fresh
temporary working directory" disclosed-testing-protocol shape plan 45's own
File I/O proof already used): with the program running and the printed port
noted, `curl -N http://127.0.0.1:<port>/` from a second shell produces,
byte-for-byte:

```
event: tick
data: 1

event: tick
data: 2

event: tick
data: 3

```

followed by the connection closing (`curl` exits `0`) once `Sse.close`
drops the writer. The three `event: tick` blocks prove real framing over a
real socket; the connection staying open across all three `.send()` calls
(not a fresh connection per call) proves the "still-open, written-to-
repeatedly" mechanism this plan exists for, not merely three independent
responses.

## Decision log

- **No new crate — `into_writer()` is the real, already-shipped escape
  hatch plan 101 doesn't otherwise need.** Verified this session against
  `tiny_http` 0.12.0's published API (current stable release; `tiny_http`
  itself last tagged a release in October 2022 per `lib.rs/crates/
  tiny_http`, which plan 101 already accepts as its base — this plan adds
  no additional currency risk beyond what plan 101 already carries).
  `Request::into_writer` and `Request::respond` are two independent,
  mutually exclusive ways to finish handling one `Request` — plan 101's
  normal request/response path uses `respond(Response)`; this plan's
  upgrade path uses `into_writer()` instead, on the same `Request` value,
  never both. Because `into_writer` returns the bare `Box<dyn Write +
  Send>` with none of `tiny_http`'s own header/status-line machinery
  attached, this plan's own code is responsible for writing a
  byte-for-byte correct HTTP/1.1 response head by hand before the first
  SSE event — `HTTP/1.1 200 OK\r\n` plus the three headers in
  `leaf-sse-upgrade-and-headers` — a real, disclosed piece of hand-rolled
  protocol text `tiny_http`'s own `Response` type would otherwise
  generate for a plan-101 caller automatically.
- **The opaque handle is a plan-93 resource, not a plan-91 scalar-return
  intrinsic — it holds a live, stateful OS resource across multiple
  calls.** Every function this plan exports (`upgrade`, `send`, `comment`,
  `close`) operates on the same `u64` handle model plan 93 defines project-
  wide: a monotonic counter assigns a fresh handle on `upgrade`, a global
  `Mutex<HashMap<u64, Box<dyn Write + Send>>>` is the single source of
  truth for "which raw writer does this handle name," and `.close()`
  removes the entry (dropping the boxed writer, which shuts the socket
  down on `Drop`) rather than merely marking it dead. No garbage
  collector ever reclaims a stream an Emerald program forgets to
  `.close()` — exactly the no-GC contract plan 93 states project-wide —
  so a real, disclosed leak exists for a program that opens SSE streams
  in a loop and never closes any of them; this is the same accepted
  tradeoff plan 93 already made for every other resource handle, not a
  new one this plan invents.
- **A panic while holding the registry lock must not poison every other
  open SSE stream for the rest of the process's life — a real hazard
  plan 91's own catch_unwind convention doesn't automatically solve by
  itself.** `std::sync::Mutex` poisons on an unwinding panic while held;
  plan 91/92's per-function `catch_unwind` wrapper *catches* the panic
  (so it never crosses the FFI boundary) but does nothing about the now-
  poisoned mutex the panicking closure was still holding when it
  unwound — the *next* call into any of this plan's four functions would
  otherwise hit `Err(PoisonError)` on `.lock()` and, naively `.unwrap()`d,
  panic again, cascading. This plan's registry lock is always acquired
  via `.lock().unwrap_or_else(std::sync::PoisonError::into_inner)` —
  recovering the guard rather than propagating the poison — because the
  `HashMap` itself is never left in a torn state by anything this plan's
  own code does inside the lock (every critical section here is a plain
  insert/remove/lookup, never a multi-step invariant that could be left
  half-updated); this is a real, disclosed judgment call, not a blanket
  "poisoning doesn't matter" claim.
- **SSE's wire format is a fixed external spec, not an Emerald design
  choice, and this plan implements a deliberately narrow, useful subset
  of it.** The WHATWG-defined format is `field: value\n`, blank-line-
  terminated per event, with `event`, `data`, `id`, `retry`, and a bare
  `:`-prefixed comment line all being real, spec-legal fields; a `data`
  value containing an embedded newline must be split into one `data:`
  line per line of payload (a single `data: <embedded \n>` line is
  wire-invalid). This plan's `.send(stream, event, data)` handles exactly
  `event:`/`data:`(with correct multi-line splitting)/the blank-line
  terminator, and `.comment(stream, text)` handles the bare `:` form
  (the standard SSE keepalive-ping idiom, since intermediary proxies and
  browsers alike treat an idle SSE connection as suspect after some
  timeout). `id:`/`retry:` and the client-side `Last-Event-ID`
  reconnection-replay contract they enable are real, spec-legal SSE
  features this plan does not implement — see Out of scope.
- **Every write is followed by an explicit flush — SSE's entire value
  proposition (a client sees each event promptly, not batched) depends
  on it, and nothing about `Box<dyn Write>` over a raw socket guarantees
  that for free.** A `TcpStream`-backed writer (what `into_writer`
  concretely returns on every platform this project targets) has no
  internal userspace buffering by default, but this plan's own framing
  code composes each event as several `write_all` calls (`event: `, the
  name, `\n`, `data: `, the payload, `\n\n`) rather than one — an
  explicit `.flush()` call after the terminating blank line is the only
  thing that actually guarantees the OS socket buffer gets handed the
  complete event promptly rather than waiting on whatever the platform's
  own Nagle/buffering defaults would otherwise do. This mirrors
  `emerald_runtime.c`'s own disclosed `fflush(stdout)` discipline
  wherever `puts`-equivalent output must be observably prompt, applied
  here to a socket instead of stdout.
- **`HttpServer.accept()` blocking and one-request-at-a-time is inherited
  from plan 101, not re-decided here — this plan adds no concurrency
  primitive of its own, per plan 94's "prefer the synchronous shape"
  default.** A consequence stated plainly rather than glossed over: this
  plan's v1 API lets exactly one SSE stream be actively driven by
  `.send()` calls per accepted request, and serving many concurrent SSE
  subscribers from one Emerald program needs either plan 101's own
  accept loop to run inside Emerald's already-shipped actor model (plan
  54/55 — real, already-implemented mechanism, not hypothetical) with
  one actor per accepted connection, or repeated sequential `accept()`
  calls that necessarily serve subscribers one at a time. This plan does
  not build that fan-out story — it is the smallest real unit (one open
  stream, driven by explicit calls) that a fan-out design would sit on
  top of, and is deliberately left there.
- **Boolean returns are `Int64`, not a new `Boolean`-typed extern
  surface — reusing plan 59's own verified finding, not re-litigating
  it.** `.send`/`.comment`/`.close` all return `0`/`1` as `Int64`, per
  plan 59's Decision log (`Boolean` is a bare LLVM `i1`; a real ABI
  boundary needs a full-width value) and per this batch's own stated
  C-FFI convention (plan 92) that every crate in this batch reuses
  rather than re-deriving per plan. A `.send()`/`.comment()` call
  against an already-closed or never-registered handle returns `0`
  (a disclosed, non-panicking "no-op on a dead handle" contract) rather
  than aborting the whole process — an Emerald program that races its
  own bookkeeping (calling `.send()` after its own `.close()`) degrades
  to a silently-dropped event, not a crash, matching plan 93's own
  general handle-lifetime philosophy of graceful no-ops over hard
  failures on stale handles.
- **Verification story: curl for the disclosed external-client check,
  a Rust `#[test]` for the CI-enforced one.** The Concrete Proof's
  `curl -N` step is real but not something `cargo nextest` itself runs —
  the mandatory Rust-side `#[test]` in `leaf-example-and-gate` instead
  spawns the same server logic on a real `std::thread` (ordinary Rust
  concurrency, nothing Emerald-level, exactly the kind of internal-only
  threading plan 91's own crate is free to use since it's never exposed
  across the FFI boundary as a new Emerald primitive) and drives a plain
  `std::net::TcpStream` client against it from the test's own main
  thread, asserting on the exact byte sequence above — a deterministic,
  `cargo nextest run --workspace`-enforced proof that needs no external
  tool and no live network.
- **Out of scope.** WebSocket support — a completely different protocol
  (its own opcode/frame-length binary framing, full duplex, a
  `Sec-WebSocket-Accept` handshake) sharing nothing with SSE's plain-text,
  server-to-client-only wire format; not attempted here, not a
  by-product of this plan's `into_writer` mechanism despite both riding
  the same underlying "hold the raw writer open" capability. `id:`/
  `retry:` fields and `Last-Event-ID` reconnection replay — real, useful
  SSE features this plan's narrow `.send`/`.comment` pair does not
  implement; a client that reconnects after a dropped connection starts
  over with no replay, a disclosed, accepted v1 limitation. Fan-out to
  many concurrently open SSE subscribers from one server process — see
  the accept-loop bullet above; this plan proves the single-stream
  mechanism, not a subscriber-broadcast design. TLS — an SSE endpoint
  served over plain HTTP here; running it behind TLS is plan 99's
  concern applied to plan 101's server generally, not something this
  plan changes.
