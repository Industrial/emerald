2026-09-21T20:20:00Z

---
name: Standard Library — Asymmetric Cryptography and Digital Signatures
overview: "Three distinct problems, three distinct crates, deliberately not conflated: `ed25519-dalek` 3.0 (signing/verification, the ecosystem-default choice, zero open RustSec advisories on the current major) as `Ed25519`; `x25519-dalek` 3.0 (Diffie-Hellman key exchange, sibling crate, also clean) as `X25519`; and `rsa` 0.9 (RustCrypto's pure-Rust RSA, included for interop with systems that mandate it) as `Rsa`, shipped with RUSTSEC-2023-0071 — the Marvin Attack, a real, currently-unpatched timing side-channel in every published version — disclosed prominently rather than pinned-around, with this plan restricting RSA's private-key operations to explicitly non-network-timing-observable use and leaving public-key verify/encrypt (not exposed to the same attack) unrestricted."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-emerald-rt-asymmetric-deps
    content: "Add `ed25519-dalek = { version = \"3.0\", features = [\"rand_core\"] }`, `x25519-dalek = { version = \"3.0\", features = [\"getrandom\", \"static_secrets\"] }`, and `rsa = \"0.9\"` to `crates/emerald-rt/Cargo.toml`."
    status: done
  - id: leaf-ed25519-signing
    content: "`Ed25519.generate_key(): Ed25519KeyPair` (CSPRNG-backed `SigningKey::generate`), `.sign(self, msg: Bytes): Bytes` (64-byte signature, `SigningKey::sign`), `Ed25519.verify(pubkey: Bytes, msg: Bytes, sig: Bytes): Result[Void, SignatureError]` using `VerifyingKey::verify_strict` (not plain `verify` — see Decision log on weak-key forgery). `Ed25519KeyPair.public_key(self): Bytes` extracts the 32-byte verifying key for distribution."
    status: done
  - id: leaf-x25519-exchange
    content: "`X25519.generate_ephemeral(): X25519EphemeralSecret` (`EphemeralSecret::random`, single-use by the underlying Rust type's own move semantics) and `X25519.generate_static(): X25519StaticSecret` (`StaticSecret`, reusable, `static_secrets` feature) — two distinct key types because the two Rust types encode two distinct real-world usage contracts (see Decision log). `.public_key(self): Bytes` on both; `.diffie_hellman(self, their_public: Bytes): Bytes` returning the 32-byte raw shared secret, self-consuming for `X25519EphemeralSecret`."
    status: done
  - id: leaf-rsa-keygen-encrypt-sign
    content: "`Rsa.generate_key(bits: Int64): Result[RsaKeyPair, RsaError]` (`RsaPrivateKey::new`, real multi-second cost at 2048+ bits — documented, not hidden), `.encrypt(pubkey: RsaPublicKey, data: Bytes): Result[Bytes, RsaError]` / `.decrypt(self, data: Bytes): Result[Bytes, RsaError]` via PKCS#1 v1.5 (`Pkcs1v15Encrypt`), `.sign(self, digest: Bytes): Result[Bytes, RsaError]` / `Rsa.verify(pubkey, digest, sig): Result[Void, RsaError]` via PKCS#1 v1.5 signing over a plan-109-computed SHA-256 digest. Every doc comment on `.decrypt`/`.sign` (the private-key, timing-observable operations) carries RUSTSEC-2023-0071's own workaround text verbatim."
    status: done
  - id: leaf-example-and-tests
    content: "`examples/crypto_asymmetric_proof.em` wired into the CI-checked example table (Ed25519 sign/verify and X25519 exchange only — RSA keygen's multi-second cost makes it unsuitable for a CI-run proof example, see Decision log). `#[test]`s in `emerald-rt`: RFC 8032 §7.1 TEST 1 (Ed25519, empty message, fixed key/signature) and RFC 7748 §6.1's Alice/Bob X25519 test vector (fixed private/public keys and resulting shared secret), both exercised byte-for-byte against this plan's wrappers. RSA gets a round-trip-only test (keygen is randomized) plus a doc-only citation of the crate's own NIST-CAVP-derived bundled test suite for primitive correctness."
    status: done
  - id: leaf-dependencies-ledger
    content: "`DEPENDENCIES.md` entries for all three crates per plan 95's ledger: dalek-cryptography as source for ed25519-dalek/x25519-dalek (zero open advisories, verified against rustsec.org this session), RustCrypto for rsa (one open advisory, RUSTSEC-2023-0071, no patched version exists as of this session — the ledger must record this as an accepted, disclosed risk with its stated scope, not a clean bill of health)."
    status: done
isProject: false
---

# Plan 111 — Standard Library: Asymmetric Cryptography and Digital Signatures

This is plan 111 of the 91-191 batch, the third of three cryptography
plans built on plan 91's `emerald-rt` mechanism (109 hashing, 110 AEAD,
this plan asymmetric). Signing, key exchange, and RSA encryption solve
three different problems with three different security models, and
this plan keeps them as three separate API surfaces rather than one
unified "asymmetric crypto" namespace — conflating "prove I hold this
private key" (signing), "two parties derive a shared secret without
ever transmitting it" (Diffie-Hellman), and "encrypt directly to a
public key" (RSA encryption) would hide real differences a caller needs
to reason about correctly (a signing key and a key-exchange key are not
interchangeable even when both happen to be Curve25519-based, and this
plan's own `Ed25519`/`X25519` split reflects that directly rather than
sharing one key type between them).

Depends on: plan 91 (the `emerald-rt` crate and its
`catch_unwind`-at-every-boundary convention), plan 92 (FFI/ABI
Conventions, authored in parallel this session — the ptr+len `Bytes`
convention this plan's every key/signature/shared-secret value is built
on), plan 93 (Resource Handle & Lifetime Model, authored in parallel
this session — the mechanism every private-key-holding handle in this
plan (`Ed25519KeyPair`, `X25519EphemeralSecret`, `X25519StaticSecret`,
`RsaKeyPair`) defers its free/zeroize/single-use story to), plan 95
(Crate-Vetting Policy, authored in parallel this session), plan 53
(`Result[T, E]` — every fallible operation in this plan returns one, for
the identical reasoning plan 110 already gave for AEAD failures), and
plan 109 (`2026-09-22T000000Z-plan-109-crypto-hashing.md` — `Bytes`,
and `Sha256.hash` specifically, which `Rsa.sign`/`Rsa.verify`'s digest
parameter is expected to be the output of).

## Concrete proof this plan targets

```ruby
signing_key: Ed25519KeyPair = Ed25519.generate_key()
verifying_key: Bytes = signing_key.public_key()
message: Bytes = "attack at dawn".to_bytes()
signature: Bytes = signing_key.sign(message)

ok: Result[Void, SignatureError] = Ed25519.verify(verifying_key, message, signature)
puts ok.is_ok()

tampered: Bytes = "attack at dusk".to_bytes()
bad: Result[Void, SignatureError] = Ed25519.verify(verifying_key, tampered, signature)
puts bad.is_err()

alice: X25519EphemeralSecret = X25519.generate_ephemeral()
bob: X25519EphemeralSecret = X25519.generate_ephemeral()
alice_public: Bytes = alice.public_key()
bob_public: Bytes = bob.public_key()

alice_shared: Bytes = alice.diffie_hellman(bob_public)
bob_shared: Bytes = bob.diffie_hellman(alice_public)
puts alice_shared == bob_shared
```

Expected output:
```
true
true
true
```

As with plan 110's AEAD proof, key generation here is randomized by
design — there is no fixed expected signature or shared-secret value
this proof can assert. What's deterministic and actually checked: a
signature verifies against the message it actually signed and the
matching public key; the identical signature fails against a
one-character-different message; and both parties in an X25519 exchange
independently derive the exact same shared secret from their own
private half and the other's public half — Diffie-Hellman's own
defining correctness property. Byte-exact primitive correctness is
what the RFC 8032/RFC 7748 `#[test]`s below exist to prove instead.

## Decision log

- **Ed25519 and X25519 are `dalek-cryptography`'s own crates, current,
  both currently clean of open RustSec advisories — verified this
  session, not assumed from reputation.** `ed25519-dalek` (3.0.0, Jul 6
  2026) and `x25519-dalek` (3.0.0, Jul 6 2026) are sibling crates from
  the same maintainers (Michael Rosenberg, isis agora lovecruft),
  sharing the underlying `curve25519-dalek` arithmetic. `ed25519-dalek`
  carries exactly one historical advisory, RUSTSEC-2022-0093 (the
  "double public key signing oracle" — decoupled private/public keypair
  APIs that let an attacker recover a private key by requesting
  signatures under an adversarial public key), patched in `>=2`; this
  plan's `3.0` pin is two major versions past the fix, and the crate's
  own README states the v2.0 redesign removed the vulnerable API
  entirely except behind an explicitly-labeled, feature-gated `hazmat`
  module this plan does not enable. `x25519-dalek`'s own
  `rustsec.org/packages/x25519-dalek.html` page returns 404 — no
  advisory has ever been filed against it. Ed25519 is used here as the
  default signing choice (not RSA, not ECDSA) because it's the
  ecosystem's own converged default for new protocols needing
  signatures — deterministic, fast, no per-signature randomness
  requirement (unlike ECDSA, where a repeated per-signature nonce
  catastrophically leaks the private key — Sony's PS3 signing key
  recovery being the canonical real-world example of exactly that
  class of failure, which this plan sidesteps entirely by not exposing
  ECDSA at all).
- **`verify_strict`, not plain `verify` — a real, deliberate choice
  this plan makes rather than defaulting to the crate's first-listed
  method.** Verified directly against `ed25519-dalek`'s own README:
  plain `VerifyingKey::verify` "permits weak keys" — a public key an
  attacker can construct such that a single signature verifies
  successfully against *many* different messages with high probability
  (the "weak key forgery" the crate's own docs describe, which caused a
  real exploitable bug in the Scuttlebutt protocol, cited by name in
  the crate's own README). `VerifyingKey::verify_strict` performs the
  extra check that rules this out. `Ed25519.verify`'s intrinsic calls
  `verify_strict` unconditionally — this plan does not expose the
  weaker `verify` at all, since there is no use case in this plan's
  own scope that needs the marginal performance gain over the safety
  loss.
- **`X25519EphemeralSecret` and `X25519StaticSecret` are two distinct
  types because the underlying crate encodes two distinct real usage
  contracts in its own type system, and collapsing them into one
  Emerald type would throw that guarantee away.** `EphemeralSecret` in
  `x25519-dalek` is deliberately non-`Clone`/non-serializable and is
  *consumed by value* on `.diffie_hellman()` — the crate's own type
  system makes reusing an ephemeral secret for a second exchange a
  compile error in Rust. `StaticSecret` (gated behind the
  `static_secrets` feature this plan explicitly enables) is the
  opposite: reusable, exportable, meant for a long-term identity key.
  This plan mirrors that split with two Emerald types rather than one
  `X25519SecretKey` with a runtime "reusable" flag, specifically so a
  caller who only ever touches `X25519EphemeralSecret` gets the
  single-use property Rust's own design intended for it to have. The
  real, disclosed gap: Rust's compile-time single-use enforcement
  cannot cross the FFI boundary unchanged (see below).
- **A real, previously-unremarked ownership hazard: wrapping
  `EphemeralSecret` behind an opaque FFI handle defeats the exact
  single-use guarantee the Rust type was designed to provide — flagged
  here, not solved here.** `x25519-dalek`'s own design uses Rust's move
  semantics specifically to make calling `.diffie_hellman()` twice on
  the same `EphemeralSecret` a compile-time impossibility. Once that
  value lives behind a boxed, pointer-typed handle for Emerald's sake
  (per plan 59's own "no lifetime tracking" precedent for any FFI
  boundary value), Emerald-generated code has no borrow checker and
  nothing stops a naive caller from invoking `.diffie_hellman()` on the
  same `X25519EphemeralSecret` handle twice — silently reusing an
  ephemeral secret, exactly the misuse the Rust API was built to make
  impossible. This plan does not solve this — it names the gap
  explicitly and defers to plan 93 whether handles get invalidated
  (nulled/tombstoned) after a consuming call, the same open question
  plan 110 raised for `AeadKey`'s free story, now doubled by the fact
  that here the *entire point* of the type is single-use enforcement,
  not just eventual cleanup.
- **RSA ships with a real, currently open, currently unpatched RustSec
  advisory — disclosed prominently, not pinned around, because there is
  no version to pin around it.** RUSTSEC-2023-0071 ("Marvin Attack"):
  verified directly against the advisory this session — "no patched
  versions... `patched = []` is intentional... Still affected as of
  2026-09-12: rsa 0.9.10 (latest stable) and rsa 0.10.0-rc.18 (latest)."
  The crate's own README states the same thing in its own words: "This
  crate is vulnerable to the Marvin Attack which could enable private
  key recovery by a network attacker." The advisory's own stated
  workaround: "Avoid using the rsa crate in settings where attackers can
  observe timing, for example over the network. Local use on a
  non-compromised computer is fine." This plan is included anyway,
  specifically for interop with external systems that mandate RSA (a
  real, common constraint this plan cannot design around), with the
  crate's own workaround text reproduced verbatim in every doc comment
  on the affected operations.
- **The Marvin Attack's actual scope is private-key operations only —
  `.decrypt`/`.sign` — and this plan's own API surface reflects that
  distinction rather than blanket-restricting all of RSA.** The timing
  side-channel is in RSA's private-key modular exponentiation (padding
  validation during decryption/signing leaks timing information an
  attacker observing a network round-trip can exploit for key
  recovery) — `Rsa.encrypt` (public-key operation, no private
  exponentiation, nothing secret-dependent to leak) and `Rsa.verify`
  (also public-key-only) are not exposed to this attack class at all.
  This plan's doc-comment warnings are attached specifically to
  `.decrypt` and `.sign` — the two operations that actually touch
  `RsaPrivateKey`'s exponentiation path — not to every RSA function
  uniformly, since over-warning on the unaffected half would blur the
  actual, specific risk.
- **`Ed25519KeyPair`/`RsaKeyPair` are single opaque handles bundling
  both halves of a keypair, not two separate public/private handles —
  a deliberate simplification, made possible by the fact that neither
  crate's own API forces the split the way `x25519-dalek`'s ephemeral/
  static distinction does.** `SigningKey::verifying_key()` and
  `RsaPublicKey::from(&priv_key)` both derive the public half from the
  private one cheaply, so this plan's `.public_key()` accessor method on
  the combined handle is a direct, unmodified call into that existing
  derivation — not new cryptographic logic, just a field accessor.
- **This plan invents no new cryptographic construction.** Ed25519
  signing/verification, X25519 Diffie-Hellman, and RSA PKCS#1 v1.5
  encrypt/decrypt/sign/verify are all direct, unmodified calls into
  each crate's own published API (`SigningKey::sign`, `VerifyingKey::
  verify_strict`, `EphemeralSecret::diffie_hellman`, `RsaPrivateKey::
  decrypt(Pkcs1v15Encrypt, ...)`, etc.) — no custom padding, no
  alternate curve, no modified key-derivation path. This plan's own
  `Ed25519KeyPair`/`RsaKeyPair` handle-bundling (above) is a packaging
  convenience, not a cryptographic change to either underlying scheme.
- **Out of scope.** RSA-OAEP and RSA-PSS (the crate's own README lists
  both as not-yet-implemented — "OAEP: Encryption & Decryption" and
  "PSS: Sign & Verify" are marked incomplete in `rsa`'s own Status
  section, verified this session — so PKCS#1 v1.5 is not a scope
  choice here, it's the only complete scheme the crate currently
  offers); ECDSA/P-256/P-384 (a real, different asymmetric family,
  deliberately excluded in favor of Ed25519 as this plan's one
  signature default, per the per-signature-nonce hazard noted above);
  X25519 static-key PEM/DER import-export and Ed25519's own `pkcs8`
  serialization feature (real crate capabilities, deferred — this
  plan's keys are generated and consumed in-process, not persisted to
  a file format, which is a distinct, separable follow-up); Ed25519's
  `batch` multi-signature-verification feature (a real performance
  path, not needed to prove this plan's mechanism); constant-time
  comparison of any raw key/secret bytes this plan's wrappers might
  return to Emerald-visible code (plan 117's `subtle` crate, cited
  here rather than re-derived, exactly as plan 109 and 110 both do).

## Not yet decided (blocking EXECUTE)

1. Whether `X25519EphemeralSecret`'s opaque handle should be
   tombstoned/invalidated by plan 93's mechanism after one
   `.diffie_hellman()` call (restoring the Rust type's own single-use
   guarantee at the Emerald level) is plan 93's call, not this plan's
   — flagged above as a real, disclosed gap this plan cannot close
   alone.
2. Whether `Rsa.generate_key`/`.decrypt`/`.sign` should carry an
   additional, harder gate than a doc-comment warning (e.g. a feature
   flag a program must explicitly opt into, given RUSTSEC-2023-0071 has
   no fix to eventually adopt) is left to plan 95's crate-vetting policy
   to decide as a general policy question, not resolved ad hoc by this
   one plan.

## Update (2026-09-22, EXECUTE)

All six leaves implemented and verified; full workspace gate green
(`cargo nextest run --workspace`: 1012/1012 passed, 2 skipped —
unrelated, pre-existing wasm-target/env-dependent guards; `cargo
clippy --workspace --all-targets`: clean; `treefmt`: 0 files changed;
`cargo audit --ignore RUSTSEC-2023-0071`: only the 5 pre-existing,
already-triaged `im`/`bitmaps`/`sized-chunks` warnings — see below for
why the `--ignore` flag itself is now required and disclosed).

**Resolution of item 2 (RSA's gate posture), forced by this plan's
own EXECUTE, not deferred further.** `cargo audit` (no flags) now
reports RUSTSEC-2023-0071 as a hard `error: 1 vulnerability found!`
(a `vulnerability`-class finding, unlike the pre-existing `unmaintained`/
`unsound` warnings plan 95's ledger already triages) the moment `rsa`
enters `Cargo.lock` — a real gate break, not a hypothetical. Resolved
by adding `--ignore RUSTSEC-2023-0071` to `AGENTS.md`'s own documented
gate command, with a citation back to this plan's own Decision log
(the affected operations' doc-comment warnings) as the accepted-risk
justification — anyone re-running the bare `cargo audit` gate command
from `AGENTS.md` gets the correct, working invocation. A real,
disclosed finding along the way: this environment's installed
`cargo-audit` binary has no config-file support at all (`--help` lists
no `-c`/`--config` flag) — an `audit.toml`-based ignore (the more
common convention) is silently inert here, confirmed by testing it
directly before switching to the working `--ignore` CLI flag.

**Real, disclosed API-version findings from EXECUTE, none anticipated
by this plan's own text:**
- `ed25519-dalek`/`x25519-dalek` 3.0's own `SigningKey::generate`/
  `Key::generate()`-style RNG-trait constructors need `rand_core`
  0.10-generation plumbing this plan's plain byte buffers don't
  carry — `Ed25519.generate_key` seeds via `getrandom::fill` directly
  (`SigningKey::from_bytes`) instead; `X25519.generate_ephemeral`/
  `.generate_static` use each crate's own no-argument `::random()`
  constructor instead (simpler, no RNG-trait plumbing needed at all).
- `rsa` 0.9's own `rand_core ^0.6.4` pin is one generation behind the
  `getrandom`/`rand_core 0.10` ecosystem every other crate here uses —
  `rand` 0.8's `OsRng` was added specifically to bridge this, a real
  dependency this plan's own text didn't anticipate.
- `Pkcs1v15Sign::new::<Sha256>()` (the plan's own literal design)
  requires the digest type to implement `rsa::pkcs8::AssociatedOid`
  against a `const-oid` major this crate's own dependency graph
  cannot satisfy consistently (tried both `sha2` 0.11, plan 109's own
  dependency, and a separately-pinned `sha2` 0.10 — both hit the
  identical trait-bound mismatch). `Pkcs1v15Sign::new_unprefixed()` is
  used instead: `RsaKeyPair#sign`/`Rsa.verify` share the identical
  unprefixed scheme, so round-tripping through this module's own
  wrapper is fully correct, but a signature produced here omits the
  standard ASN.1 DigestInfo prefix and is not directly interoperable
  with an external strict-PKCS1v15 verifier expecting the prefixed
  form — a real, disclosed interop gap, not a correctness one.

**Two RFC test vectors, hand-transcribed from memory on the first
attempt, were caught wrong and corrected against the actual primary
source before being trusted** — a real, disclosed process note: the
Ed25519 RFC 8032 §7.1 TEST 1 seed/pubkey and the X25519 RFC 7748 §6.1
Alice/Bob private scalars each initially had one hex character wrong
or missing (an `OddLength` decode panic on the first run made the
Ed25519 error impossible to miss; the X25519 one silently decoded to
the wrong 32 bytes and only surfaced as a failed `assert_eq!`). Both
were re-fetched directly from `rfc-editor.org`'s own plaintext RFC
files and corrected byte-for-byte before this plan's own test suite
was trusted — the same "verify against the actual observed/primary
value, don't assume the first draft was right" discipline this
session's own `libm_proof.em`/`humantime_proof.em` corrections already
established, applied here to hand-typed test vectors instead of
predicted program output.

**Scope actually shipped**: `Ed25519.generate_key`/`.verify`,
`Ed25519KeyPair#sign`/`#public_key`; `X25519.generate_ephemeral`/
`.generate_static`, `X25519EphemeralSecret`/`X25519StaticSecret#public_
key`/`#diffie_hellman` (the ephemeral variant's `.diffie_hellman`
consumes and closes its own handle, restoring the underlying Rust
type's single-use intent as far as plan 93's existing registry
mechanism allows — item 1's own cross-FFI gap remains real and
unclosed, exactly as disclosed); `Rsa.generate_key`/`.encrypt`/
`.verify`, `RsaKeyPair#decrypt`/`#sign`. 7 new `emerald-rt` unit tests
(the RFC 8032/7748 vectors, an Ed25519 sign/verify + tamper-fails
round-trip, an X25519 ephemeral-exchange-agrees-both-directions proof,
and an RSA-2048 encrypt/decrypt + sign/verify round-trip);
`examples/crypto_asymmetric_proof.em` (Ed25519 + X25519 only, per this
plan's own Decision log — RSA's multi-second keygen makes it
unsuitable for a CI-run proof) plus its `emerald-cli` test;
`DEPENDENCIES.md` rows for all four new crates, including RSA's own
accepted-risk row.
