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
