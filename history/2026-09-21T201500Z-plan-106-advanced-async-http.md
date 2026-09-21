2026-09-21T20:15:00Z

---
name: Advanced Async HTTP (hyper-direct)
overview: "`hyper` 1.11.1 (verified this session against `lib.rs/crates/hyper` and `docs.rs/hyper` — actively released, 14-17M downloads/week, owned by the `hyperium` org) used directly, with `hyper-util` for its client/connection-pool conveniences and the `h2` crate underneath it for real HTTP/2 support, as the heavier, opt-in upgrade path for Emerald programs that need persistent connection reuse or HTTP/2 — not a replacement for plan 100/101's simpler synchronous `ureq`/`tiny_http` modules, which remain the default for ordinary request/response code. This plan is explicit about which of hyper's real capabilities (true streaming response bodies handed to Emerald as an ongoing sequence of chunks, HTTP/2 server push) do not cross a synchronous FFI boundary cleanly, and defers them rather than papering over the gap."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-hyper-dependency-and-vetting
    content: "Add `hyper = { version = \"1\", features = [\"client\", \"http1\", \"http2\"] }`, `hyper-util = { version = \"0.1\", features = [\"client-legacy\", \"http2\"] }`, and `http-body-util` to `crates/emerald-rt/Cargo.toml`; run through plan 95's checklist and `DEPENDENCIES.md`, noting `hyper` 1.x is deliberately low-level (its own crate docs describe it as such) and `hyper-util`'s `client-legacy` module is the real source of the connection-pooling behavior this plan exists for"
    status: pending
  - id: leaf-shared-pooled-client-and-runtime
    content: "One process-wide `hyper_util::client::legacy::Client<HttpConnector, ...>` (or its rustls-backed connector when the target is `https://`, reusing plan 99's TLS setup) constructed once behind a `std::sync::OnceLock`, alongside plan 94's single lazy `tokio::runtime::Runtime` — every request this plan issues reuses the same pooled client/runtime pair, which is the entire mechanism behind the connection-reuse this plan exists to prove (`ureq`'s own simpler per-call connection model, plan 100, does not pool across separate Emerald-level calls the same way)"
    status: pending
  - id: leaf-connect-request-response-buffered
    content: "`emerald_rt_h2_get(url: *const c_char, out_status: *mut i64) -> *mut c_char` — the v1-solved case: issue a GET through the shared pooled client via `runtime.block_on`, negotiate HTTP/2 when the server offers ALPN `h2` (over rustls, plan 99) or fall back to HTTP/1.1 automatically (this is `hyper`/`hyper-util`'s own real, built-in negotiation, not something this plan writes), buffer the full response body via `http_body_util::BodyExt::collect`, return it as one `CString`, exposed as `Http2Client.get(url: String): String` with status via a companion `Http2Client.last_status(): Int64`"
    status: pending
  - id: leaf-connection-reuse-proof
    content: "A Rust `#[test]` issuing three sequential requests to the same host through the shared pooled client and asserting, via a counting `TcpListener`-backed local test server, that only one real TCP connection was accepted — the actual, checked proof of `leaf-shared-pooled-client-and-runtime`'s stated benefit, not merely an assertion that pooling exists because `hyper-util` says so"
    status: pending
  - id: leaf-example-and-gate
    content: "`examples/http2_client.em` (the Concrete Proof below) issuing three requests against a plan-101 `tiny_http` server (HTTP/1.1 only — `tiny_http` itself has no HTTP/2 support, a real, disclosed limitation of this plan's own local proof target, noted in the Decision log) run as a companion background process, wired into the CI verification pattern plan 104/105 already established"
    status: pending
isProject: false
---

# Plan 106 — Advanced Async HTTP (hyper-direct)

Plan 100 wraps `ureq` for the common case: one blocking call, one request,
one buffered response, no connection state carried between calls. Plan 101
does the mirror image on the server side with `tiny_http`. Both are
deliberately simple, and that simplicity is the right default for the large
majority of Emerald HTTP code — most programs issue a handful of requests
and don't care whether the underlying TCP connection was reused. A real,
narrower class of program does care: one that issues many requests to the
same host in a loop (a scraper, a polling client, an API-heavy backend
service) pays a real, measurable cost — a fresh TCP handshake, and a fresh
TLS 1.3 handshake if `https://`, per `ureq`-style call — that persistent
connection reuse and HTTP/2 multiplexing both exist specifically to avoid.
`hyper`, verified this session to be current (`hyper` 1.11.1, released
August 28, 2026, per `lib.rs/crates/hyper`) and by a wide margin the
Rust ecosystem's dominant HTTP implementation (14-17 million downloads per
week, `hyperium`-org owned, the crate `h2` itself — HTTP/2's own real
implementation, verified this session, "now used by hyper" per `h2`'s own
`crates.io` description), is the real, lower-level building block underneath
that reuse. This plan wraps it directly, deliberately bypassing plan
100/101's simpler crates, as an explicit opt-in upgrade — not the default
HTTP path, and not a plan to eventually replace plan 100/101 with.

## Concrete proof this plan targets

Two cooperating processes, matching the pattern plan 104/105 already use for
a real client/server proof without inventing new Emerald-level concurrency:
a plain plan-101 `tiny_http` server (`examples/http2_target_server.em`,
listening on a fixed local port, responding `200 OK` with a small counter
body to every request) started first, then:

```ruby
i: Int64 = 0
while i < 3
  body: String = Http2Client.get("http://127.0.0.1:8099/")
  status: Int64 = Http2Client.last_status()
  puts status
  puts body
  i += 1
end
```

Expected output: `200` followed by the server's response body, three times
in a row. The Concrete Proof itself only checks the *values* are correct
(any client could produce this output) — the actual claim this plan makes
(connection reuse) is proven separately, deterministically, by the Rust-side
`#[test]` in `leaf-connection-reuse-proof`, which counts real accepted TCP
connections rather than trusting output text alone; see the Decision log for
why the `.em`-level proof cannot observe pooling directly.

## Decision log

- **`hyper` verified current and dominant, not assumed from prior
  familiarity.** This session's search confirmed `hyper` 1.11.1 (Aug 28,
  2026) and `h2` (Aug 24, 2026) are both freshly released as of this
  plan's authoring date, and `hyper`'s own `lib.rs` listing shows
  14-17 million weekly downloads — an order of magnitude above `ureq`
  or `tiny_http`'s own download counts, consistent with `hyper` being
  the transitive dependency underneath most of the ecosystem's own HTTP
  tooling (including, per plan 105's own verified finding, being cited
  directly in `multer`'s own usage docs as its reference integration
  target) rather than a niche choice.
- **`hyper` 1.x is deliberately low-level — this plan needs `hyper-util`
  too, and says so rather than presenting `hyper` alone as sufficient.**
  `hyper`'s own crate documentation (verified this session via
  `docs.rs/hyper`) states plainly it is meant as "a building block for
  libraries and applications," not a batteries-included client — the
  connection-pooling `Client` type living in `hyper-util::client::
  legacy` is what this plan actually needs for its own stated purpose
  (this is a real, disclosed architectural fact about the crate, not
  friction this plan hides). `http-body-util` supplies the
  `BodyExt::collect` helper this plan's v1-buffered-response path uses,
  since `hyper`'s own `Incoming` body type is a raw, low-level streaming
  body with no built-in "just give me the whole thing" convenience.
- **The pooled client and the async runtime are both singletons behind
  the same `OnceLock`, constructed once per process — this is the
  entire mechanism, not a detail.** `hyper-util`'s connection pool only
  provides reuse across calls that share the same `Client` value; a
  naive implementation that constructed a fresh `Client` per FFI call
  (mirroring plan 100's own simpler per-call `ureq` model) would defeat
  this plan's entire purpose while still compiling and running
  correctly — the bug would be silent (every metric still "works," just
  never actually pools). This plan states the shared-singleton
  requirement as a first-class design constraint specifically so a
  future implementer doesn't accidentally regress it into a
  correctly-typed but pointless wrapper.
- **Real, honest gap: exposing HTTP/2 response bodies as an ongoing
  sequence of chunks handed to Emerald code is not solved by this
  plan, and the reason is structural, not a missing leaf.** `hyper`'s
  `Incoming` body is a `Stream`-like type whose next chunk only becomes
  available by polling a `Future` inside an async task — there is no
  synchronous "give me whatever bytes are ready right now" call on it.
  A synchronous FFI function (`Type::CString`/`Int64`/`Float64`-typed
  return, per plan 59/92's own allow-list) can only produce one
  complete value per call; making that value "the next chunk of an
  in-progress response" would require either (a) a background task
  that keeps polling the body independently of any Emerald call and
  buffers chunks into a queue an Emerald program later drains — real,
  buildable, but re-introduces exactly the unbounded-buffering risk
  plan 105's own bounded-chunk design was built to avoid, since nothing
  stops that background task from racing ahead of a slow Emerald
  consumer — or (b) a callback-into-Emerald mechanism (a function
  pointer the runtime invokes per chunk) that does not exist anywhere
  in this codebase today and is a substantial, separate FFI-design
  problem this plan does not attempt to solve as a side effect. This
  plan's v1 surface buffers the whole response instead (matching plan
  100's own `ureq`-based shape) and states plainly that true streaming
  response bodies are deferred, not solved.
- **Real, honest gap: HTTP/2 multiplexing (many concurrent logical
  requests over one physical connection) has no Emerald-visible
  expression in this plan's synchronous, one-call-per-request API.**
  `h2`'s actual value proposition is multiple in-flight requests
  sharing one connection without head-of-line blocking at the HTTP
  layer; this plan's `Http2Client.get` is still one blocking call in,
  one buffered `String` out — issuing three such calls in a loop (the
  Concrete Proof's own shape) may ride the same underlying HTTP/2
  connection and even interleave at the wire level courtesy of
  `hyper`'s own internal scheduling, but nothing in this plan's API
  lets an Emerald program *issue* two requests concurrently and observe
  them completing out of order — that would need a real Emerald-level
  concurrency primitive (plan 54/55's actor model, most plausibly) to
  fire off multiple `Http2Client.get` calls from separate actors
  against the same shared client, an integration this plan does not
  attempt to design.
- **Connection reuse is proven by counting real TCP accepts, not by
  reading output text — because output text cannot distinguish pooled
  from non-pooled requests.** Three sequential `Http2Client.get` calls
  to the same host produce byte-identical `.em`-visible output whether
  or not the underlying client actually reused one TCP connection
  across them (a broken, unpooled implementation that opened a fresh
  connection per call would still return the same three response
  bodies). The Concrete Proof section says this plainly rather than
  implying the `.em` output alone demonstrates the plan's real claim;
  `leaf-connection-reuse-proof`'s Rust `#[test]`, which counts actual
  accepted connections on a local listener, is the one place this
  plan's stated benefit is actually checked.
- **The local proof target is HTTP/1.1-only, a real, disclosed
  limitation of using plan 101's own `tiny_http` server as the test
  target — not a claim this plan tested HTTP/2 negotiation end-to-end.**
  `tiny_http` has no HTTP/2 support at all; `hyper`'s own client will
  correctly negotiate down to HTTP/1.1 against it (real, standard
  ALPN-absent behavior, not a bug), which is sufficient to prove
  connection *reuse* (this plan's stated benefit, independent of HTTP
  version) but does not exercise `h2`'s own multiplexing wire format at
  all. A genuine HTTP/2 server target is a real, separate need this
  plan does not build — either a small `hyper`-based server example
  (out of scope here, since this plan is framed as client-focused) or
  an external, already-HTTP/2 test fixture.
- **Boolean/handle conventions reused verbatim from plan 59/92/93.**
  `Http2Client.get`'s error path (a connection or TLS failure) sets
  `Http2Client.last_status()` to a disclosed sentinel (`-1`, since no
  real HTTP status code is negative) and returns an empty `String`
  rather than aborting the process — a plain `Int64`-status-check
  convention, not a new `Result`/exception-typed return; this plan adds
  no new marshaling convention beyond what plans 59/91/92 already
  established.
- **Out of scope.** True streaming response/request bodies exposed as
  an Emerald-visible sequence of chunks (see above — structurally
  deferred, not a missing leaf). HTTP/2 server push — a real HTTP/2
  feature `h2` itself supports at the Rust level, with no Emerald
  surface here at all; most real-world HTTP/2 deployments have already
  deprecated server push server-side by 2026, making this a low-value
  addition even setting the FFI difficulty aside. A `hyper`-based HTTP
  server module (the mirror image of this plan's client focus) — plan
  101's `tiny_http`-based server remains the only server-side module in
  this batch; a `hyper`-based server would face the identical
  streaming-body and multiplexing problems this plan defers on the
  client side, doubled. Multiplexed concurrent requests driven from
  Emerald-level concurrency (actors) against one shared client — a
  real, plausible follow-up, not attempted here.
