# `emerald-rt` dependency ledger

Plan 95's own artifact: the running record of every third-party Rust
crate `crates/emerald-rt` depends on, why it was chosen, and which
plan added it. As of this file's own creation (plan 95), `emerald-rt`
has **zero** third-party dependencies — plans 91-93 (the runtime crate
itself, its FFI/ABI conventions, and its resource-handle registry) are
all zero-dependency by their own Decision logs, verified directly
against each plan's own text before this ledger was written.

This file is append-only in spirit, mirroring `history/`'s own
immutability convention: a later plan replacing crate X with crate Y
adds a new row for Y and marks X's own row `superseded by plan N`
rather than deleting it. Never edit an existing row's `Why chosen` or
`Pure-Rust or C-exception` columns to reflect a later plan's own
reasoning — add a new row instead.

## Column contract

- **Crate** — the crate's name exactly as it appears in `Cargo.toml`.
- **Version** — the exact version pinned when the row was added (not
  "latest" — a specific version this project vetted).
- **Why chosen** — a one-line summary only. The full justification
  (alternatives considered, WebSearch evidence, maintenance/advisory
  check) lives in the adding plan's own Decision log, cited by `Plan #`
  below — this ledger is an index into that evidence, not a copy of it.
- **Pure-Rust or C-exception (+ justification)** — either the literal
  word `pure-Rust`, or `C-exception:` followed by a short reason (e.g.
  `links libsqlite3`, `wraps zlib`) — see plan 95's own Decision log
  for the pure-Rust-first preference this column enforces.
- **Date added** — the ISO date of the plan history file that added
  the row, not the date the row itself was last edited.
- **Plan #** — the plan number whose Decision log has the full
  justification, maintenance-bar check, and (for a C-exception) the
  pure-Rust alternatives actually considered and why each was rejected.

## Ledger

| Crate | Version | Why chosen | Pure-Rust or C-exception (+ justification) | Date added | Plan # |
|---|---|---|---|---|---|
| `serde_json` (+ `preserve_order` feature, pulling in `indexmap`) | 1.0.151 | #1 in crates.io's Encoding category, 116M downloads/month, no serious pure-Rust competing choice for JSON; `serde_json::Value` lowers directly to Emerald's own `JsonValue` enum layout | pure-Rust | 2026-09-22 | 118 |
| `tracing` | 0.1.44 | #1 in crates.io's Debugging category, 67M downloads/month, tokio-rs-owned; industry-standard structured-event macros/dispatch | pure-Rust | 2026-09-22 | 168 |
| `tracing-subscriber` (+ `registry` feature) | 0.3.23 | Subscriber/Layer composition utilities for `tracing`, same owners; backs `Log`'s custom JSON `Layer`. RUSTSEC-2025-0055 (ANSI-escape injection) checked — patched in >=0.3.20, this pin resolves well above it | pure-Rust | 2026-09-22 | 168 |
| `base64` | 0.23.1 | #4 in crates.io's Encoding category, 123.7M downloads/month, used in 95,248 crates; `Engine`-trait API's four predefined `general_purpose` constants map 1:1 onto `Base64`'s own four variant pairs. No RustSec advisory found | pure-Rust (default-on `simd-unsafe` feature is internal-only `unsafe`, invisible at this API level) | 2026-09-22 | 123 |
| `hex` | 0.4.3 | #52 in crates.io's Encoding category, 53.9M downloads/month, used in 44,269 crates; stable/essentially unchanged since 2021, case-insensitive `decode` matching this plan's own wrapper design. No RustSec advisory found | pure-Rust | 2026-09-22 | 123 |
| `regex` | 1.13.1 | #1 in crates.io's Text processing category, 91.9M downloads/month, used in 105,388 crates, owned directly by rust-lang; RE2-derived finite-automata matching gives a real, verified worst-case `O(m*n)` time guarantee with no catastrophic-backtracking/ReDoS class at all. No RustSec advisory found | pure-Rust | 2026-09-22 | 122 |
| `libm` | 0.2.16 | #2 in crates.io's No-std category, 41.8M downloads/month, used in 36,355 crates, owned by rust-lang-owner — the exact fallback `core`'s own float math already uses on targets with no OS-provided math library; portable to `wasm32-wasip1` with zero linker flag, unlike calling libc math via `extern "C"`. No RustSec advisory found | pure-Rust | 2026-09-22 | 164 |
| `humantime` | 2.4.0 | #2 in crates.io's Date and time category, 22.0M downloads/month, used in 14,490 crates, "No runtime deps"; stable, unchanged API surface for years. No RustSec advisory found | pure-Rust | 2026-09-22 | 162 |
| `sysinfo` | 0.39.6 | #1 in crates.io's #process category, 16.8M downloads/month, used in 6,472 crates, MSRV 1.95; abstracts over `/proc`, `sysctl`, and Windows performance-counter APIs behind one cross-platform interface. No RustSec advisory found | C-exception: links libc and platform-native APIs (`ntapi` on Windows, `objc2`/IOKit on macOS) — no pure-Rust alternative exists for cross-platform system introspection | 2026-09-22 | 152 |
| `sha2` | 0.11.0 | #4 in crates.io's Cryptography category, 90.8M downloads/month, RustCrypto-owned; `Sha256`/`Sha512.hash`'s direct backing via the shared `digest::Digest` trait. RUSTSEC-2021-0100 (AVX2 miscomputation, fixed in 0.9.8) checked — nearly five major versions behind this pin. No open RustSec advisory found | pure-Rust | 2026-09-22 | 109 |
| `sha3` | 0.12.0 | RustCrypto-owned SHA-3/Keccak family; `Sha3_256`/`Sha3_512.hash`'s direct backing, same `digest::Digest` trait shape as `sha2`. No RustSec advisory found | pure-Rust | 2026-09-22 | 109 |
| `md-5` | 0.11.0 | RustCrypto-owned; `Md5.hash`'s direct backing — kept explicitly legacy-interop-only per the crate's own README security warning, reproduced verbatim in `hashing.rs`'s own `md5_hash` doc comment. No RustSec advisory found | pure-Rust | 2026-09-22 | 109 |
| `blake3` | 1.8.7 | #3 in crates.io's Cryptography category, 16.8M downloads/month; the BLAKE3 design team's own reference implementation (not a RustCrypto crate — no RustCrypto BLAKE3 exists to prefer instead), `Blake3.hash`'s direct backing. No RustSec advisory found | pure-Rust | 2026-09-22 | 109 |
| `aead` | 0.6.1 | RustCrypto-owned trait crate (`Aead`/`KeyInit`/`Payload`) both `aes-gcm`/`chacha20poly1305` build on; added directly (not just transitively) so this plan's own dispatch code can name these traits without reaching through either cipher crate's own re-export | pure-Rust | 2026-09-22 | 110 |
| `aes-gcm` | 0.11.1 | RustCrypto-owned (`github:rustcrypto:aeads`); one NCC Group audit (2020, funded by MobileCoin, no significant findings). RUSTSEC-2023-0096 (`decrypt_in_place_detached` exposing unauthenticated plaintext) checked — this plan's own `.decrypt` is built on the safe, `Vec`-returning `Aead::decrypt` only, never affected regardless of version. `AesGcm256`'s direct backing | pure-Rust | 2026-09-22 | 110 |
| `chacha20poly1305` | 0.11.0 | RustCrypto-owned, same audit as `aes-gcm`. `XChaCha20Poly1305`'s direct backing — chosen alongside AES-GCM for its own hardware-portability reason (fast in pure software, no AES-NI/CLMUL dependency) | pure-Rust | 2026-09-22 | 110 |
| `zeroize` | 1.9.0 | RustCrypto-owned; both AEAD crates above already pull it in transitively via their own `zeroize` feature — added directly so `AeadKey`'s own key-material storage in `emerald-rt` can wrap its bytes in `Zeroizing<[u8; 32]>` itself, giving `AeadKey#free` a real, working zero-on-drop guarantee | pure-Rust | 2026-09-22 | 110 |
| `getrandom` | 0.4.3 | rust-random-owned, already pulled in transitively by both AEAD crates' own `getrandom` feature; added directly so this plan's nonce/key generation can call `getrandom::fill` directly rather than fight either crate's own generic `Generate` trait API (found, empirically, to need type-level plumbing this plan's plain byte-buffer needs don't warrant) | pure-Rust | 2026-09-22 | 110 |
| `ed25519-dalek` | 3.0.0 | dalek-cryptography-owned, ecosystem-default Ed25519 signing choice; one historical advisory (RUSTSEC-2022-0093, "double public key signing oracle") patched in >=2, this plan's 3.0 pin two majors past the fix. `Ed25519.generate_key`/`.verify`, `Ed25519KeyPair#sign`/`#public_key`'s direct backing — `.verify` calls the crate's own `verify_strict`, not plain `verify` (rules out weak-key forgery) | pure-Rust | 2026-09-22 | 111 |
| `x25519-dalek` | 3.0.0 | dalek-cryptography-owned sibling crate to `ed25519-dalek`; own `rustsec.org` page 404s — no advisory ever filed. `X25519.generate_ephemeral`/`.generate_static`, `X25519EphemeralSecret`/`X25519StaticSecret#public_key`/`#diffie_hellman`'s direct backing | pure-Rust | 2026-09-22 | 111 |
| `rsa` | 0.9.10 | RustCrypto's pure-Rust RSA, included for interop with systems mandating it. **Carries a real, currently unpatched RustSec advisory: RUSTSEC-2023-0071 (Marvin Attack, a timing side-channel in private-key operations, `patched = []` — no fixed version exists).** Accepted deliberately per this plan's own Decision log, not overlooked — `Rsa.decrypt`/`RsaKeyPair#sign` (the affected operations) carry the advisory's own stated workaround verbatim in their doc comments; `Rsa.encrypt`/`Rsa.verify` (public-key-only) are unaffected and unrestricted. `cargo audit --ignore RUSTSEC-2023-0071` (see `AGENTS.md`'s own gate command and its comment) is required for this repo's gate to pass with this dependency present | pure-Rust | 2026-09-22 | 111 |
| `rand` | 0.8.8 | rust-random-owned; added specifically because `rsa` 0.9's own `rand_core` dependency is pinned to `^0.6.4`, one generation behind the `getrandom`/`rand_core 0.10` ecosystem every other crate in this file uses — `rand::rngs::OsRng` is the compatible CSPRNG source for that older trait generation, backing `Rsa.generate_key`/`.encrypt`. A real, disclosed extra dependency this plan's own text didn't anticipate | pure-Rust | 2026-09-22 | 111 |
| `subtle` | 2.6.1 | dalek-cryptography-owned (isis agora lovecruft, Henry de Valence), unchanged since 2024-06-24, 58M downloads/month, used in 2,026 crates. **Zero runtime dependencies** (only a `dev`-only `rand` 0.8, verified against its own `docs.rs` listing) — the smallest addition to this ledger of any crate in the 91-191 batch. `SecureCompare.eq`'s direct backing (`ConstantTimeEq`'s slice impl); this plan's own central, cross-cutting requirement is that every secret-comparison in this stdlib batch route through this (or an equivalent RustCrypto-ecosystem constant-time primitive), never Rust's `==`/Emerald's own `String`/`Array` equality | pure-Rust | 2026-09-22 | 117 |
| `url` | 2.5.8 | servo-owned — the reference-grade WHATWG URL Standard implementation the rest of the Rust web ecosystem (`reqwest`, `hyper`, `cargo` itself) already depends on. #8 in Parser implementations, 64.9M downloads/month, used in 77,921 crates. Pure Rust (`form_urlencoded`/`idna`/`percent-encoding`, no C dependency). `Url.parse`/`.build`/`#scheme`/`#host`/`#port`/`#path`/`#query`/`#fragment`/`#with_path`/`#with_query`/`#with_port`'s direct backing — the shared foundation plans 100/101 (HTTP client/server) both parse a URL through. No RustSec advisory found | pure-Rust | 2026-09-22 | 98 |
| `hickory-resolver` (+ `tls-ring`/`https-ring`/`webpki-roots` features) | 0.26.3 | Actively maintained, pure-Rust-first successor to `trust-dns-resolver`, owned by `bluejekyll`. Pinned >=0.26.2 for a real, disclosed advisory (Amazon Linux CVE tracking: pre-0.26.2 fails to propagate a bogus DNSSEC proof correctly). `Dns.resolve`/`.resolve_all`/`.resolve_count`/`.configure`'s direct backing. `webpki-roots` (bundled Mozilla CA list) is required, not optional in practice — verified live: without it, every TLS-based lookup (DoT/DoH) fails with `invalid peer certificate: UnknownIssuer` regardless of the actual CA, since no trust anchors are loaded by default. `tls-ring`/`https-ring` chosen over the `-aws-lc-rs` variants: `ring` is the smaller, more established C-exception (hand-optimized assembly, RustCrypto-adjacent), `aws-lc-rs` vendors a full AWS-LC (BoringSSL fork) C codebase for the same requirement | C-exception: `ring`'s TLS crypto backend is not pure Rust (the plan's own "stays pure-Rust end to end" claim does not hold once TLS is actually needed — a real, disclosed correction) | 2026-09-22 | 97 |
| `tokio` (+ `rt-multi-thread` feature) | 1.53.1 | The first genuine (non-illustrative) use of plan 94's own documented shared-runtime bridging pattern (`crate::tokio_rt()`, `lib.rs`) — required because `hickory-resolver` 0.26.x's `Resolver<P>` is async-only in the actual pinned version (verified false against the plan's own "genuinely separate, real synchronous `Resolver` type" claim). `rt-multi-thread`, not `rt` alone: `Resolver`'s own docs warn its lookup futures and the background tasks they spawn must run concurrently on the same executor, which a single-threaded `block_on` cannot guarantee | pure-Rust | 2026-09-22 | 97 |
| `ureq` | 3.4.2 | #1 in lib.rs's own HTTP-client category, 22.2M downloads/month, used in 8,525 crates. Chosen over `reqwest` (69.4M downloads/month, more widely used in raw numbers) specifically because `reqwest::blocking` still drives an embedded `tokio` `Runtime` under the hood, while `ureq` is genuinely, natively blocking I/O with no embedded async runtime at all — the strongest fit for plan 94's "prefer a crate's sync API" rule, a real, disclosed trade-off against raw popularity. `Http.get`/`.post`'s direct backing. Default features (`gzip` + `rustls`, the latter already pulling in its own `rustls-webpki-roots` trust-anchor feature by default — unlike plan 97's `hickory-resolver`, no extra feature needed). No RustSec advisory found | C-exception: `rustls`'s own crypto backend is not pure Rust (the same real caveat plan 97's `hickory-resolver` row already discloses) | 2026-09-22 | 100 |
| `tiny_http` | 0.12.0 | Real-reconfirmed at this plan's own execution time (2026-09-22, via docs.rs) — STILL the latest crates.io release (published Oct 6, 2022), the plan's own "no newer release since 2022" staleness caveat found still true, unlike every other crate this session touched. 100% blocking, no embedded async runtime; its own internal per-connection thread pool already solves concurrency, matching plan 94's sync-first posture with zero bridging. `Http.serve`/`HttpResponse.build`/`HttpRequest#method`/`#path`/`#body`'s direct backing. No RustSec advisory found | pure-Rust | 2026-09-22 | 101 |
| `libc` | 0.2.189 | rust-lang-owned (`github:rust-lang:libc`), "Raw FFI bindings to platforms' system libraries" — the single most foundational, near-ambient FFI crate in the Rust ecosystem (`std`'s own platform layer is itself built on it). Backs `TcpListener.bind`'s own `SO_REUSEADDR` fix in `net.rs` (a raw `getaddrinfo`/`socket`/`setsockopt`/`bind`/`listen` sequence, socket2-free per this plan's own instruction) — `std::net::TcpListener::bind` does not set `SO_REUSEADDR` by default, a real divergence from `emerald_tcp_listen`'s own explicit C-side behavior (`runtime/emerald_runtime.c:1506`) verified this session, not assumed. No RustSec advisory found against the crate itself | C-exception: raw libc FFI is the point of the crate — no pure-Rust alternative exists for a socket option `std::net` itself declines to expose | 2026-09-22 | 96 |

| `hkdf` | 0.13.0 | RustCrypto-owned, pure Rust. `Kdf.hkdf`'s direct backing. A rare row in this ledger where the plan's own cited version/API needed zero correction — its own `docs.rs` usage example, fetched live this session, matches the plan's own cited RFC 5869 Test Case 1 vector byte-for-byte. No RustSec advisory found | pure-Rust | 2026-09-22 | 115 |
| `pbkdf2` (+ `sha2` feature) | 0.13.0 | RustCrypto-owned, pure Rust. `Kdf.pbkdf2`'s direct backing (`pbkdf2_hmac::<Sha256>`). Its own `docs.rs` usage example matches the plan's own cited doctest vector byte-for-byte, same as `hkdf` above. No RustSec advisory found | pure-Rust | 2026-09-22 | 115 |
| `csv` | 1.4.0 | BurntSushi-owned (author of `ripgrep`), the canonical, official Rust CSV reader/writer, #6 in Encoding, 17.9M downloads/month, used directly in 3,392 crates. `Csv.parse`/`.parse_with_headers`/`.write`'s direct backing — deliberately does NOT reuse plan 118's `JsonValue` tree (a CSV document is a flat table, never nested). No RustSec advisory found | pure-Rust | 2026-09-22 | 121 |
| `toml` | 1.1.6+spec-1.1.0 | Already an in-tree, already-vetted dependency — `crates/emerald-cli/Cargo.toml` pins `toml = "0.8"` separately, parsing every `emerald.toml` manifest since plan 46. This crate's own copy is pinned at the crate's real, current major (a full major ahead, now targeting TOML spec 1.1.0 vs 0.8.x's spec 1.0.0) — a real, disclosed version skew two workspace members carry independently with no conflict. `Toml.parse`/`JsonValue.to_toml`'s direct backing, reusing plan 118's `JsonValue` enum verbatim as the dynamic-value representation. No RustSec advisory found | pure-Rust | 2026-09-22 | 119 |

## Pre-existing transitive dependencies (out of scope, triaged)

Not `emerald-rt` dependencies at all, and predating this batch — noted
here only because `cargo audit` (wired into the gate by this same
plan) surfaces them and a future reader should not mistake silence for
an unnoticed finding. `emerald-driver` depends on `id_effect`, which
pulls in `im` 15.1.0 (transitively `bitmaps` 2.1.0 and `sized-chunks`
0.6.5) — as of plan 95's own authoring, all three carry a real RustSec
`unmaintained` advisory, and `im`/`sized-chunks` additionally carry a
real `unsound` advisory (RUSTSEC-2023-0126, RUSTSEC-2026-0255). `cargo
audit` reports these as warnings, not hard failures (its default exit
code is 0 for advisories at `unmaintained`/`unsound` severity, only
nonzero for an actual `vulnerability`-class finding) — triaged here as
a pre-existing, out-of-scope condition; fixing `id_effect`'s own
dependency choice is not this plan's mandate and is left for whichever
future plan (or a dedicated maintenance pass) actually touches
`emerald-driver`'s own dependency tree.
