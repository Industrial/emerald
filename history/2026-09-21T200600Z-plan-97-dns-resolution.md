2026-09-21T20:06:00Z

---
name: DNS Resolution — `hickory-resolver` for DoH/DoT/Custom-Nameserver Lookups
overview: "A new `emerald-rt` module wrapping `hickory-resolver` (the actively maintained, pure-Rust successor to `trust-dns-resolver`, renamed by its own maintainers in October 2023 when the project moved under the `hickory-dns` GitHub organization) to give Emerald a `Dns` module — `Dns.resolve`, `Dns.resolve_all`, and DoH/DoT/custom-nameserver configuration — for the real gap plan 96's `std::net`-based sockets cannot close: `std::net::ToSocketAddrs` already resolves a plain hostname via the OS's own `getaddrinfo`, verified directly, so this plan is not needed merely to make `TcpStream.connect(\"example.com\", 443)` work — it is needed for encrypted resolution (DNS-over-HTTPS/DNS-over-TLS), resolution against a specific nameserver rather than the OS's configured one, and async/batch resolution, none of which `getaddrinfo` offers."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vet-and-pin-hickory-resolver
    content: "Add `hickory-resolver` to `crates/emerald-rt/Cargo.toml` and record it in plan 95's `crates/emerald-rt/DEPENDENCIES.md` ledger with the verified facts this session found: formerly published as `trust-dns-resolver`, renamed `hickory-resolver` when the Hickory DNS project moved to its own `hickory-dns` GitHub organization (announced October 2023, the `bluejekyll.github.io` project blog's own post confirms `trust-dns-resolver`/`trust-dns-proto` becoming `hickory-resolver`/`hickory-proto` under the rename); latest published version `0.26.2` as of this session (`docs.rs/crate/hickory-resolver/latest`), roughly 2.48 million weekly downloads per Socket.dev's package-security profile (classified 'popular'); actively consumed by real, security-sensitive infrastructure — Let's Encrypt's own engineering blog (Dirkjan Ochtman's 2025 maintenance retrospective, published on `dirkjan.ochtman.nl` January 2026) states plainly that ISRG (Let's Encrypt's parent org) is 'working on enabling the use of the Hickory DNS recursive resolver' in production, not merely evaluating it. Confirm no unpatched RustSec advisory applies to the pinned version at implementation time (RustSec has published hickory-resolver/hickory-proto advisories against versions before 0.26.2 for a bogus-DNSSEC-proof-propagation bug per Amazon Linux's own CVE tracking — pin at or above the fixed version, do not pin an older release for compatibility reasons without re-checking this)."
    status: pending
  - id: leaf-dns-resolve-and-resolve-all
    content: "`Dns.resolve(host: String): String` (returns the first resolved address as a dotted-quad/IPv6-literal string, raising plan 92's canonical error shape on NXDOMAIN or a resolver-transport failure) and `Dns.resolve_all(host: String): Array[String]` (every resolved address, working around `Array[T]`'s own no-length-metadata representation the same way plan 45's `.split`/`.split_count` pair already does — ship a companion `Dns.resolve_count(host: String): Int64` rather than inventing a new length-carrying array representation this plan has no mandate to build). Both back onto `hickory_resolver::Resolver`'s synchronous, blocking `lookup_ip` call (the crate ships both a `tokio`-async API and a genuinely separate blocking one built on its own internal executor — see Decision log for why this plan uses the blocking entry point specifically, per plan 94's stated preference for a crate's sync API before reaching for `block_on`)."
    status: pending
  - id: leaf-dns-config-doh-dot-custom-nameservers
    content: "`Dns.configure(mode: String, nameserver: String): Void` (or an equivalent small, fixed set of named constructors — exact shape TBD at implementation, see Not yet decided) selecting among `hickory-resolver`'s own built-in `ResolverConfig` presets (`ResolverConfig::cloudflare_https()`/`cloudflare_tls()`/`google()`/`quad9()` and a custom `NameServerConfigGroup`-built config pointed at an arbitrary IP) — this is the entire reason this plan exists rather than being subsumed into plan 96: DNS-over-HTTPS and DNS-over-TLS both need a real DoH/DoT client implementation (`hickory-resolver`'s own `dns-over-https-rustls`/`dns-over-rustls` Cargo features, both pure-Rust via `rustls` — see plan 99, no OpenSSL pulled in either way), which `getaddrinfo` fundamentally cannot provide since it only ever speaks the OS's own configured plaintext-UDP/TCP resolution path."
    status: pending
  - id: leaf-hickory-resolver-panic-boundary
    content: "Wrap every exported `emerald_rt_dns_*` function's body in `std::panic::catch_unwind` per plan 92, converting `hickory_resolver::ResolveError`'s real variants (NXDOMAIN, timeout, malformed response, no-connections-available) into distinct, real error messages rather than one generic 'DNS lookup failed' string — `ResolveErrorKind`'s own variants are already this granular, and collapsing them loses real diagnostic information a caller doing anything beyond a toy lookup will want."
    status: pending
  - id: leaf-tests-and-example
    content: "Add `#[test]`s in `emerald-rt` covering: a successful `resolve` against a real, stable hostname (network-dependent — gate behind a `#[ignore]`-by-default or an explicit feature flag consistent with however plan 95's mandatory-test-checklist handles network-dependent tests elsewhere in the ledger, not invented fresh here), and a synthetic NXDOMAIN case proving the error path raises rather than panicking. Add `examples/dns_resolution.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli`'s example-conformance table per plan 95's checklist — using a hostname stable and deterministic enough for CI (see Decision log for the specific choice and its tradeoffs), and run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 97 — DNS Resolution

Plan 96 gave Emerald `TcpStream.connect(host, port)`, and that call
already resolves a plain hostname today with zero help from this plan:
`std::net`'s `ToSocketAddrs` implementation calls the platform's own
`getaddrinfo` internally (confirmed this session against Rust's own
ecosystem discussion — NLnet Labs' own published `IPv6 and Rust`
engineering post states plainly, of `TcpStream::connect`, "under the
hood, the implementation calls `getaddrinfo`"; a `rust-lang/internals`
forum thread titled "Custom Global DNS Resolver" independently confirms
the same point from the opposite direction, an author asking how to
*avoid* `getaddrinfo` precisely because "Rust always uses" it). So the
question this plan has to answer honestly is not "does Emerald need DNS
resolution" — plan 96 already has working resolution — but "what does
`getaddrinfo` not give a program that a dedicated resolver crate does."
The answer is threefold, and none of it is available at the `std::net`
layer no matter how it's called: encrypted transport to the resolver
itself (DNS-over-HTTPS, DNS-over-TLS — `getaddrinfo` always speaks
plaintext UDP/TCP to whatever the OS's `/etc/resolv.conf` or platform
equivalent names), a specific chosen nameserver rather than the OS's
configured one (useful for split-horizon debugging, testing against a
known-good resolver, or avoiding a captive-portal/ISP resolver's own
interception), and structured access to more than just "an address" —
`getaddrinfo` collapses a lookup down to a list of `sockaddr`s with no
way to ask a TTL, an MX/TXT/SRV record, or a DNSSEC validation result.
This plan's worked proof and API surface both stay narrow — A-record
resolution plus the DoH/DoT/custom-nameserver configuration axis that
is this plan's actual reason to exist — and explicitly declines to
build a general DNS record-type API (see Out of scope).

Depends on: plan 91 (`emerald-rt` archive/link mechanism), plan 92
(catch-unwind and canonical error shape for every `Dns.*` call), plan
95 (crate-vetting ledger this plan's `hickory-resolver` entry is
recorded in, and the docs+example+test checklist), plan 96 (the
`TcpStream`/`UdpSocket` types a resolved address ultimately feeds into
— this plan produces address *strings*, plan 96 consumes them, no
tighter coupling than that exists between the two). Plan 99 (TLS) is a
soft, forward dependency in the opposite direction: DoH/DoT resolution
itself needs a TLS client, and `hickory-resolver`'s own `dns-over-
rustls`/`dns-over-https-rustls` Cargo features pull in `rustls`
directly (the crate's own dependency, not plan 99's `emerald-rt`
wrapper) — this plan does not depend on plan 99's Emerald-facing `Tls`
module being built first, only on `rustls` itself existing in the
dependency graph, which `hickory-resolver`'s Cargo features already
guarantee independently.

## Concrete proof this plan targets

```ruby
addr: String = Dns.resolve("one.one.one.one")
puts addr

Dns.configure("cloudflare_tls", "")
secure_addr: String = Dns.resolve("one.one.one.one")
puts secure_addr
```

Expected output: two IP address strings (in practice, both resolving to
one of Cloudflare's own well-known DNS-over-anything anycast addresses,
`1.1.1.1` or `1.0.0.1`, or their IPv6 equivalents — the exact literal
is not asserted verbatim in CI, only that it is a well-formed address
and that both calls succeed without raising). `one.one.one.one` is
chosen deliberately over an arbitrary third-party domain: it is
Cloudflare's own self-hosted vanity hostname for its `1.1.1.1` resolver
service, stable by the operator's own design commitment, and — not
incidentally — resolving it is a real, meaningful proof exactly because
the second call routes the query itself through DNS-over-TLS to that
same Cloudflare service rather than the OS's plaintext resolver,
demonstrating the DoT configuration path actually changes transport
behavior, not just an inert config flag.

## Decision log

- **`hickory-resolver` is the current, correctly-named, actively
  maintained choice — verified this session, not assumed from
  pre-rename memory.** The `trust-dns-resolver` -> `hickory-resolver`
  rename is real and already fully in effect: the project's own
  announcement post (`bluejekyll.github.io/blog/posts/announcing-
  hickory-dns/`) states the crates "will become `hickory-resolver` and
  `hickory-proto`"; `reqwest`'s own CHANGELOG (a large, widely-depended
  crate) documents removing its long-deprecated `trust-dns` feature
  "which was renamed hickory-dns a while ago"; NixOS's own release
  notes independently confirm "`trust-dns` has been renamed to
  `services.hickory-dns`". The crate is not merely renamed-and-
  abandoned — `hickory-resolver 0.26.2` is a current release
  (`docs.rs/crate/hickory-resolver/latest`, `hickory-resolver-0.26.1`
  itself dated 01 May 2026 per docs.rs's own version page), carries
  ~2.48M weekly downloads (Socket.dev), and Let's Encrypt's own
  engineering organization (ISRG) is actively integrating Hickory DNS's
  recursive resolver into production per its January 2026 maintenance
  retrospective — a security-sensitive CA choosing to depend on this
  project is a strong, concrete adoption signal, not marketing
  copy. This plan cites the real rename date and real current version
  rather than assuming either from pre-rename training-data familiarity
  with "trust-dns-resolver," per this task's own stated verification
  requirement.
- **A real RustSec-relevant advisory exists and must be checked against
  the pinned version, not glossed over.** Amazon Linux's own CVE
  tracking (`explore.alas.aws.amazon.com`) lists a real advisory:
  "hickory-resolver versions before 0.26.2 fail to propagate bogus
  DNSSEC proof" correctly. This plan pins at `0.26.2` or later
  specifically because of this, and `leaf-vet-and-pin-hickory-resolver`
  requires re-confirming no advisory applies at actual implementation
  time (this session's own finding will be stale by then) rather than
  trusting this plan document's own snapshot indefinitely — exactly the
  kind of check plan 95's ledger exists to make routine.
- **`getaddrinfo` (via plan 96's `std::net`) is not replaced by this
  plan — it remains the default, correct choice for the common case,
  and this plan's own worked proof deliberately does not duplicate
  it.** Verified precisely, not asserted loosely: `std::net::
  ToSocketAddrs`'s blanket impl for `(&str, u16)` and `SocketAddr`-
  producing string parsing both route through the platform resolver
  (`getaddrinfo` on Unix), meaning `TcpStream.connect("example.com",
  443)` from plan 96 already performs real hostname resolution with no
  help from this plan at all. A user who wants plain DNS resolution and
  nothing more should keep using plan 96's connect calls directly —
  reaching for `Dns.resolve` first, only to feed the result into
  `TcpStream.connect`, is strictly more code for no behavior change
  over just calling `TcpStream.connect(hostname, port)`. This module
  exists for the cases `getaddrinfo` cannot serve: DoH/DoT transport,
  explicit nameserver selection, and (deferred, see Out of scope)
  structured record-type queries.
- **The blocking API, not `tokio::net::TcpStream`-style async, is
  chosen for the same reason plan 94 states generally.**
  `hickory-resolver` ships a genuinely separate, real synchronous
  `Resolver` type built on its own internal, self-contained executor —
  not a thin `block_on` wrapper this plan would have to write itself.
  Per plan 94's own stated preference ("prefer a crate's sync API; else
  one lazy `tokio::runtime::Runtime` behind `block_on`"), this plan
  uses that synchronous entry point directly. `hickory-resolver`'s
  async `TokioResolver` variant exists and is a legitimate future
  option if a later plan needs concurrent batch resolution at a scale
  where blocking calls on the actor worker pool (the same hazard plan
  96 documents for sockets) becomes the binding constraint — not
  needed for this plan's narrow surface, and not built speculatively.
- **DoH/DoT selection reuses `hickory-resolver`'s own built-in
  `ResolverConfig` presets rather than this plan inventing its own
  protocol-selection enum.** `ResolverConfig::cloudflare_https()`/
  `cloudflare_tls()`/`google()`/`quad9()` are real, maintained-upstream
  constructors already shipped by the crate, each wired to the correct
  well-known IP/hostname/certificate-validation combination for that
  provider's DoH or DoT endpoint — reimplementing that mapping in
  `emerald-rt` would be strictly more code with strictly more chances
  to get a well-known provider's endpoint or cert-pinning detail wrong,
  for zero benefit over calling the crate's own constructor.
- **Both DoH and DoT stay pure-Rust end to end, keeping this plan
  inside plan 95's pure-Rust-first policy despite pulling in real
  cryptographic/TLS machinery.** `hickory-resolver`'s DoH/DoT Cargo
  features are named `dns-over-https-rustls`/`dns-over-rustls`
  specifically (not `-openssl` or `-native-tls` variants) — the crate's
  own maintainers made the same pure-Rust choice this batch's plan 99
  makes independently for its own TLS story. This plan's
  `DEPENDENCIES.md` entry should record enabling exactly these feature
  flags, not a broader default feature set that might pull in a
  C-backed TLS backend this project's policy would otherwise flag.
- **A record-type-general API (MX/TXT/SRV/CNAME, raw DNSSEC validation
  results, TTL access) is explicitly out of scope.** `hickory-resolver`
  supports all of this through its `lookup` (generic-over-`RecordType`)
  API, but Emerald's own type system has no natural representation for
  a heterogeneous DNS record beyond the plain address-string case this
  plan's `Dns.resolve`/`Dns.resolve_all` already cover — designing a
  `DnsRecord` class hierarchy (or an enum, if `Enum` per plan 59's
  verified `Type` list is expressive enough) for six-plus real record
  types is substantial, separate scope no current plan in this batch
  needs to unblock. `Array[String]`'s own no-length-metadata limitation
  (plan 45's already-documented gap, worked around here identically via
  a companion `.resolve_count`) is inherited, not re-litigated.
- **Out of scope.** Reverse DNS (PTR lookups), a general record-type
  query API (see above), DNSSEC validation result surfacing to Emerald
  code (the crate can validate internally; this plan does not expose a
  pass/fail signal for it), resolver-level caching configuration beyond
  whatever `hickory-resolver`'s own defaults provide, and async/batch
  resolution via `TokioResolver` (deferred pending an actual need, per
  plan 94's sync-first preference stated above).

## Not yet decided (blocking EXECUTE)

1. The exact shape of `Dns.configure` — a single `(mode, nameserver)`
   pair as sketched in `leaf-dns-config-doh-dot-custom-nameservers`, or
   a small set of dedicated zero/one-argument calls
   (`Dns.use_cloudflare_tls()`, `Dns.use_custom_nameserver(ip)`, ...)
   more consistent with the fixed-arity-intrinsic style plan 45
   established for `String`. Either is implementable on the same
   `hickory-resolver` foundation; this plan does not commit to one over
   the other before implementation-time API-surface review.
2. Whether `Dns.configure` is process-global (a single lazily-
   initialized `Resolver` behind a `OnceLock`, reconfigured in place —
   simplest, but a reconfigure call affects every subsequent `Dns.*`
   call program-wide, including ones made from unrelated actors) or
   produces a distinct, independently held resolver handle under plan
   93's opaque-handle convention (more consistent with this batch's
   general resource model, but turns a currently two-call worked proof
   into a three-value one). The Concrete Proof above is written
   assuming the simpler global-reconfiguration model; if implementation
   settles on the handle-based alternative, the proof's `Dns.configure`
   call becomes a `Dns.connect_via("cloudflare_tls"): DnsResolver`
   returning a handle, and both `resolve` calls move to instance
   methods on it.
