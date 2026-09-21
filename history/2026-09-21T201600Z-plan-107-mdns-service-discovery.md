2026-09-21T20:16:00Z

---
name: mDNS/Service Discovery (lower-priority, stretch-tier)
overview: "`mdns-sd` (v0.21.3, verified this session against `lib.rs/crates/mdns-sd` — actively maintained, 33 contributors, a release as recent as September 8, 2026, no async-runtime dependency of its own) wrapped as a narrow `Mdns` module for advertising and discovering services on a local network — the same Bonjour/Avahi/DNS-SD mechanism, not a general DNS resolver (see plan 97). Flagged explicitly as lower-priority/stretch-tier in this batch: the use case (LAN device/peer discovery) is real but narrow, even though the crate itself, checked directly rather than assumed, turned out healthier than a stretch-tier label might suggest."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-mdns-dependency-and-vetting
    content: "Add `mdns-sd = \"0.21\"` to `crates/emerald-rt/Cargo.toml`, run through plan 95's checklist and `DEPENDENCIES.md`, noting the honest finding from the Decision log: this crate is healthier (80 releases, a release this week, no async runtime coupling) than the task's own stretch-tier framing assumed going in"
    status: pending
  - id: leaf-service-daemon-singleton
    content: "One process-wide `mdns_sd::ServiceDaemon` behind a `std::sync::OnceLock` (the crate's own required entry point — every advertise/browse call goes through one running daemon, which owns its own internal background thread per the crate's own architecture, not `tokio`), constructed lazily on first use of either `Mdns.advertise` or `Mdns.discover_one`"
    status: pending
  - id: leaf-advertise
    content: "`emerald_rt_mdns_advertise(service_type: *const c_char, instance_name: *const c_char, port: i64) -> i64` — builds a real `mdns_sd::ServiceInfo` (host name derived from the local hostname, a synthesized `.local.` address record) and calls `ServiceDaemon::register`, registering the resulting unregister-token under a plan-93 `u64` handle, exposed as `Mdns.advertise(service_type: String, instance_name: String, port: Int64): Int64`"
    status: pending
  - id: leaf-discover-one-bounded
    content: "`emerald_rt_mdns_discover_one(service_type: *const c_char, timeout_ms: i64) -> *mut c_char` — calls `ServiceDaemon::browse`, blocks on the daemon's own `mpsc`-style event receiver with an explicit timeout (`crossbeam_channel`'s `recv_timeout`, or the plain `std::sync::mpsc` equivalent depending on which `mdns-sd` uses internally) until either a `ServiceResolved` event arrives or the timeout elapses, formats `<instance>@<ip>:<port>` on success, `nil` on timeout, exposed as `Mdns.discover_one(service_type: String, timeout_ms: Int64): String?`"
    status: pending
  - id: leaf-stop-advertising
    content: "`emerald_rt_mdns_stop(handle: i64) -> i64` — calls `ServiceDaemon::unregister` for the given handle's service, exposed as `Mdns.stop_advertising(handle: Int64): Int64`"
    status: pending
  - id: leaf-example-and-gate
    content: "`examples/mdns_self_discover.em` (the Concrete Proof below — advertise a service and discover it from the same process over real loopback multicast, no second process needed, unlike plan 104/105/106's client/server proofs) plus a Rust `#[test]` asserting the same round trip deterministically within a bounded timeout"
    status: pending
isProject: false
---

# Plan 107 — mDNS/Service Discovery

Every other networking plan in this batch assumes both sides of a
connection already know each other's address — an HTTP client is handed a
URL, a QUIC client is handed a host and port. mDNS (Multicast DNS, RFC
6762) plus DNS-SD (DNS Service Discovery, RFC 6763) solve the step before
that: how does a program on a local network find *what's there* — a
printer, a chat peer, another instance of itself — with no central
directory, no configuration, nothing but the LAN's own multicast group.
This is a real, useful, but genuinely narrow capability (it works only
within one broadcast domain, never across the open internet, and is not a
substitute for real DNS resolution — plan 97's concern, not this plan's);
this plan is explicitly the lower-priority, stretch-tier entry in this
batch, placed here honestly rather than dressed up as more central than it
is. That said, the crate this plan depends on checked out better than the
"niche corner of the ecosystem" framing might predict going in — see the
Decision log.

## Concrete proof this plan targets

Unlike plan 104/105/106, this proof needs no second process: mDNS is
inherently multicast, so a program can advertise a service and then
discover it via the same real network mechanism a second, independent
process would use — proving the actual wire protocol, not a mocked
loopback shortcut.

```ruby
handle: Int64 = Mdns.advertise("_emerald-demo._tcp", "plan107-proof", 9999)
found: String? = Mdns.discover_one("_emerald-demo._tcp", 3000)
result: String = found ||= "not found"
puts result
Mdns.stop_advertising(handle)
```

Expected output: a string of the shape `plan107-proof@<local-ip>:9999`
(the exact IP depends on the host's own network configuration, so the CI
assertion checks the fixed, deterministic parts — the instance name and
port — rather than a hardcoded IP), proving the advertised service was
genuinely found via a real mDNS query/response cycle within the 3-second
timeout, not a hardcoded loopback shortcut.

## Decision log

- **The crate checked out healthier than the stretch-tier framing
  predicted, verified rather than assumed — say so plainly instead of
  forcing a "niche and weak" narrative onto a crate that isn't.** This
  session's search found `mdns-sd` 0.21.3, released September 8, 2026
  (within the same week as this plan's own authoring date), with 80
  total releases, 33 contributors, and download counts climbing across
  recent weeks (roughly 100,000 to over 200,000 per week per `lib.rs`'s
  own history, a real growth trend, not a stagnant crate coasting on
  past downloads). "Lower priority" in this plan's overview describes
  the *use case*'s narrowness (LAN-only discovery is genuinely a small
  slice of what most Emerald programs need), not a weakness in the
  tooling available for it — those are two different claims, and this
  plan does not conflate them.
- **`mdns-sd` needs no async runtime of its own — a real, disclosed
  simplification versus every other networking plan in this batch that
  touches plan 94.** `mdns-sd`'s own `lib.rs` description states it
  plainly: "mDNS Service Discovery library with no async runtime
  dependency." Verified this session, not assumed from the crate name
  alone. The crate manages its own internal background thread (inside
  `ServiceDaemon`) for the actual multicast send/receive loop, and
  exposes a plain, synchronous, channel-based API to its caller —
  meaning this plan needs none of plan 94's lazy `tokio::runtime::
  Runtime`/`block_on` machinery at all, unlike plan 105/106/108, which
  all bridge into an async crate. This is a genuine, stated point in
  the crate's favor for this project's own "prefer sync" default (plan
  94), not merely a convenient coincidence.
- **`Mdns.discover_one` returns one result within a bounded timeout,
  not an open-ended `Array[String]` of every service found — the same
  `Array[T]`-has-no-length-metadata wall plan 45 already hit and
  disclosed, not a new limitation this plan invents.** A real mDNS
  browse can, in principle, discover an unbounded number of matching
  services over an unbounded time window (services can appear and
  disappear as devices join/leave the network); Emerald's `Array[T]`
  representation, per plan 45's own verified finding, "carries no
  runtime length to iterate against." Rather than force a
  `.browse_count`-style companion scan (plan 45's own workaround for a
  fundamentally *static*, already-fully-received string to split) onto
  a fundamentally *live, open-ended* event stream where "how many will
  there be" has no fixed answer even in principle, this plan narrows
  its v1 surface to the smaller, well-defined question "is there at
  least one match within this timeout" — genuinely useful on its own
  (the common real case: "is there a printer on this network," "am I
  the first instance of this service or is one already running")
  without requiring `Array[T]`'s general dynamic-length redesign plan
  45 explicitly deferred as out of scope for itself.
- **One process-wide `ServiceDaemon`, not one per call — matches the
  crate's own required usage shape, not an Emerald-specific choice.**
  `mdns_sd::ServiceDaemon::new()` starts a real background thread and
  binds a real multicast UDP socket; constructing one per `Mdns.
  advertise`/`Mdns.discover_one` call would both waste real OS
  resources (a fresh thread and socket per call) and risk multiple
  daemons in the same process fighting over the same multicast group on
  some platforms. A single `OnceLock`-guarded daemon, constructed
  lazily on first use and never torn down until process exit, is the
  crate's own documented intended usage pattern, reused here rather
  than reinvented.
- **`Mdns.advertise` returns a handle immediately; the actual
  advertisement is asynchronous (a background thread inside the
  daemon), which this plan's API shape makes honest rather than
  hiding.** `ServiceDaemon::register` itself returns as soon as the
  registration request is *queued*, not once every peer on the network
  has actually seen the announcement (mDNS's own probing/announcing
  handshake, RFC 6762 §8, takes real wall-clock time — typically under
  a second, but not zero). This plan's Concrete Proof's own 3-second
  discovery timeout is set generously above that real handshake latency
  specifically so the proof is not flaky against real network timing,
  a disclosed, deliberate margin rather than an arbitrary round number.
- **Not a DNS resolver — cite plan 97, don't re-derive it.** mDNS
  resolves only `.local.`-suffixed names within one multicast domain,
  using its own wire protocol (UDP port 5353, its own record-caching
  and conflict-detection rules) that shares message *format* with
  ordinary DNS but nothing about *scope* or *transport pattern* with
  it — a real, standard DNS resolver (plan 97's concern) that needs to
  reach `example.com` over the open internet has no use for anything
  this plan builds, and this plan's `Mdns` module makes no attempt to
  generalize into one. An Emerald program wanting both real internet
  DNS resolution and LAN service discovery needs both plans' modules,
  independently.
- **Boolean/handle conventions reused verbatim from plan 59/92/93; no
  new marshaling convention.** `Mdns.advertise`/`.stop_advertising`
  follow the same `Int64`-handle-and-status-code shape every other
  plan in this batch uses; `Mdns.discover_one`'s `String?` return
  reuses plan 43/59's nullable-reference convergence (a real "not
  found within the timeout" outcome maps directly to Emerald `nil`,
  the same way a real C `NULL` return already does for `String.
  from_cstring`).
- **Out of scope.** A general, unbounded multi-result browse API
  (`Array[String]` of every matching service, live-updating as peers
  join/leave) — see the bounded-`discover_one` bullet above; a real,
  substantial follow-up once `Array[T]`'s own dynamic-length
  representation gap (plan 45's own disclosed limitation) is addressed
  project-wide, not something this plan attempts piecemeal. TXT record
  key/value metadata on advertised services (real DNS-SD feature,
  genuinely useful for e.g. advertising a service's own version or
  capabilities) — `mdns-sd`'s own API supports it; this plan's v1
  `Mdns.advertise` signature does not expose it, a small, disclosed,
  non-blocking gap a follow-up leaf could close. IPv6 multicast — this
  plan's proof and v1 scope assume IPv4 multicast (224.0.0.251,
  mDNS's standard IPv4 group); `mdns-sd` itself supports IPv6
  (ff02::fb) but this plan does not commit to testing or guaranteeing
  it. Conflict detection / probing edge cases (two instances
  advertising the same name) — handled internally by `mdns-sd` per its
  own RFC 6762 implementation; this plan does not add any
  Emerald-visible surface for observing or resolving a detected
  conflict.
