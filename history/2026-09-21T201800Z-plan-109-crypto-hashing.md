2026-09-21T20:18:00Z

---
name: Standard Library — Cryptographic Hashing
overview: "Sha256/Sha512 (`sha2` 0.11), Sha3_256/Sha3_512 (`sha3` 0.12), Blake3 (`blake3` 1.8, the BLAKE3 team's own reference crate), and Md5 (`md-5` 0.11, explicitly legacy-interop-only per RustCrypto's own README security warning) exposed as PascalCase pseudo-module intrinsics — `<Algorithm>.hash(bytes): Bytes` one-shot for all six, plus a `<Algorithm>.new()/.update()/.finalize()` incremental handle for Sha256 and Blake3 specifically, chosen as the two structurally distinct streaming shapes (Merkle-Damgård vs. tree hash) worth proving once each — built on a new shared `Bytes` type (a ptr+len fat value, plan 92's binary-safe buffer convention surfaced at the Emerald-source level) and, for the incremental path, an opaque per-algorithm handle whose free/lifetime story plan 93 owns."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-bytes-type-foundation
    content: "New `Type::Bytes` — a two-word {ptr, len: Int64} fat value, sema-level distinct from both `String` (bare null-terminated buffer, no length) and `Array[T]` (bare pointer, also no length, per plan 45's finding) specifically because a hash digest's length is real, runtime-meaningful, and must round-trip exactly. `String.to_bytes(self): Bytes` (zero-copy — a `Bytes` view over the same malloc'd buffer plan 59 found `String` already is, length filled in via the existing `emerald_string_length`/strlen call) and `Bytes.to_hex(self): String` (a small, dependency-free hex-encoder in `emerald-rt`, needed so this plan's own proof output is printable via `puts`). This is shared plumbing plans 110 and 111 both depend on rather than redefining."
    status: pending
  - id: leaf-emerald-rt-hash-deps
    content: "Add `sha2 = \"0.11\"`, `sha3 = \"0.12\"`, `blake3 = \"1.8\"`, `md-5 = \"0.11\"` to `crates/emerald-rt/Cargo.toml`. One `#[no_mangle] pub extern \"C\" fn emerald_rt_<algo>_hash(ptr: *const u8, len: i64, out: *mut u8) -> i64` per algorithm (six total), each body wrapped in `std::panic::catch_unwind` per plan 91's established convention, writing the algorithm's fixed-length digest into a caller-allocated `out` buffer and returning 0/-1 for success/panic, per plan 92's FFI/ABI error-signaling convention."
    status: pending
  - id: leaf-one-shot-intrinsics
    content: "`Sha256.hash`/`Sha512.hash`/`Sha3_256.hash`/`Sha3_512.hash`/`Blake3.hash`/`Md5.hash` — six hardcoded intrinsic arms in `emerald-sema`/`emerald-codegen`, each shaped exactly like plan 45's `File.read`/`File.write` (matched on `Expr::Ident(n) if n == \"Sha256\"` etc., checked before the real module-dispatch arm, never colliding since none of these names ever populate `classes`/`module_names`)."
    status: pending
  - id: leaf-incremental-hashers
    content: "`Sha256Hasher`/`Blake3Hasher` — two new inert sema-only reference types (CString's exact shape per plan 59: no new runtime representation, just an opaque pointer to a boxed Rust `sha2::Sha256`/`blake3::Hasher`). `Sha256.new(): Sha256Hasher`/`.update(self, data: Bytes): Void`/`.finalize(self): Bytes` and the Blake3 equivalent, backed by `emerald_rt_sha256_new/update/finalize` and `emerald_rt_blake3_new/update/finalize`, each catch_unwind-wrapped. `finalize` consumes the handle (frees the `Box` internally); the handle's lifetime/free story if `finalize` is never reached is plan 93's mechanism, not reinvented here."
    status: pending
  - id: leaf-example-and-tests
    content: "`examples/crypto_hashing_proof.em` wired into `emerald-cli/tests/examples.rs`'s checked table. `#[test]`s in `emerald-rt` asserting each wrapper against a real, cited test vector: SHA-256(\"hello world\"), SHA3-256(\"abc\"), MD5(\"hello world\"), BLAKE3(\"\") — all independently verifiable against the algorithms' own primary sources, not invented."
    status: pending
  - id: leaf-dependencies-ledger
    content: "`DEPENDENCIES.md` entries for `sha2`/`sha3`/`blake3`/`md-5` per plan 95's ledger convention: RustCrypto (sha2/sha3/md-5) vs. the BLAKE3 team (blake3) as source, current RustSec advisory status for each (recorded below), and the MD5 legacy-only usage restriction."
    status: pending
isProject: false
---

# Plan 109 — Standard Library: Cryptographic Hashing

This is plan 109 of the 91-191 batch answering the same 2026-09-21
directive plan 91 states in full: prefer exposing vetted, widely-used
Rust crates as native Emerald functions over hand-rolling more C.
Hashing is the natural first domain plan — every algorithm here is a
pure function of bytes-in, bytes-out (no key material, no randomness,
no network-observable secret), which makes it the least ambiguous case
for the "direct, unmodified passthrough to an established crate, no new
cryptographic construction" posture this whole batch's crypto plans
(109, 110, 111, and 117's later `subtle` work) commit to.

Depends on: plan 91 (`2026-09-21T200000Z-plan-91-rust-native-runtime-crate.md`
— the `emerald-rt` crate, its `catch_unwind`-at-every-boundary
convention, and its build/link mechanism this plan's six new exported
functions ride on, unchanged), plan 92 (FFI/ABI Conventions, authored in
parallel this session — the binary-safe ptr+len buffer convention this
plan's `Bytes` type surfaces at the Emerald-source level, and the
error-signaling convention `emerald_rt_*_hash`'s `-1`-on-panic return
follows), plan 93 (Resource Handle & Lifetime Model, authored in
parallel this session — the mechanism `Sha256Hasher`/`Blake3Hasher`'s
free/lifetime story is deferred to, not reinvented here), plan 95
(Crate-Vetting Policy, authored in parallel this session — the
RustSec-check-mandatory, DEPENDENCIES.md-ledger, RustCrypto-preferred
posture this plan's own crate choices follow), plan 45 (`2026-09-09T111000Z-plan-45-stdlib-strings-and-io.md`
— the `File.read`/`File.write`-shaped hardcoded intrinsic-arm dispatch
this plan's six `<Algorithm>.hash` arms reuse verbatim), and plan 59
(`2026-09-09T133000Z-plan-59-c-ffi.md` — `String`'s real bare
null-terminated-buffer representation this plan's `.to_bytes()`
zero-copy conversion depends on, and `CString`'s "sema-only distinct
type, no new runtime representation" shape this plan's hasher handle
types copy directly).

## Concrete proof this plan targets

```ruby
puts Sha256.hash("hello world".to_bytes()).to_hex()
puts Sha3_256.hash("abc".to_bytes()).to_hex()
puts Blake3.hash("".to_bytes()).to_hex()
puts Md5.hash("hello world".to_bytes()).to_hex()

hasher: Sha256Hasher = Sha256.new()
hasher.update("hello ".to_bytes())
hasher.update("world".to_bytes())
puts hasher.finalize().to_hex()
```

Expected output, in order:
```
b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532
af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262
5eb63bbbe01eeed093cb22bb8f5acdc3
b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
```

Every one of these five values is a real, independently-checkable
digest, not invented for this document: line 1 and line 5 are the
published SHA-256("hello world") value from `sha2`'s own crate-level
README example (`Sha256::digest(b"hello world")`); line 2 is the
published SHA3-256("abc") value from `sha3`'s own README example; line
3 is the `input_len: 0` case's first 32 output bytes from the BLAKE3
project's own canonical `test_vectors.json` (fetched directly from
`github.com/BLAKE3-team/BLAKE3` this session — the file's own comment
states implementations "should also check that the first 32 bytes match
their default-length output"); line 4 is the published MD5("hello
world") value from `md-5`'s own README example. Line 5 re-derives line
1's value via the incremental handle (`"hello " + "world"` fed across
two `.update` calls), proving the streaming path and the one-shot path
agree bit-for-bit.

## Decision log

- **Four crates, two different maintainer stories, both independently
  verified this session against `rustsec.org` and each crate's own
  `lib.rs`/README page, not assumed.** `sha2` (0.11.0, Mar 25 2026),
  `sha3` (0.12.0, May 15 2026), and `md-5` (0.11.0, Mar 27 2026) are all
  RustCrypto, all pure Rust, all currently at zero open RustSec
  advisories (`rustsec.org/packages/{sha3,md-5}.html` both 404 — no
  advisory page exists — and `sha2`'s one historical advisory,
  RUSTSEC-2021-0100, an AVX2-backend miscomputation bug introduced in
  v0.9.7 and fixed in v0.9.8, is nearly five major versions behind the
  0.11.x line this plan pins). `blake3` (1.8.7, Aug 20 2026) is *not* a
  RustCrypto crate — it's the BLAKE3 design team's own reference
  implementation (Jack O'Connor et al., sponsored by Electric Coin
  Company), also currently at zero open RustSec advisories, included
  here specifically because plan 95's crate-vetting policy names
  RustCrypto as the *preferred* source for pure-Rust audited
  primitives, not the *only* one — BLAKE3 has no RustCrypto
  implementation to prefer instead, and the upstream team's own crate is
  the obviously correct choice over any third-party reimplementation.
- **A new `Bytes` type, not `Array[Int64]` and not `String`, is the
  right Emerald-level shape for a digest — a real representation gap
  this plan is the first to hit, not a stylistic choice.** `Array[T]`
  literals carry "no length prefix, no bounds checking" per plan 45's
  own verified finding, and a hash digest's length is only known at
  compile time in the sense that it's fixed *per algorithm* — nothing
  in today's `Type` system lets a function return "an `Array[Int64]` of
  exactly 32 elements" as a distinguishable type from one of 64. `String`
  fails differently: it's real UTF-8 text by `TYPE_SYSTEM.md` §9's own
  contract, and a hash digest is neither valid UTF-8 in general nor
  conceptually text. `Bytes` is a new, minimal two-word `{ptr, len:
  Int64}` value — this plan's Emerald-level surfacing of plan 92's
  binary-safe ptr+len FFI convention, not a separate design. Every
  domain plan needing raw non-UTF8 bytes (this plan's digests, plan
  110's keys/ciphertexts, plan 111's signatures/shared secrets) shares
  this one type rather than each inventing its own.
- **`String.to_bytes()` is zero-copy, `Bytes.to_hex()` is the one new
  piece of actual logic in this plan, and neither is a cryptographic
  construction.** `.to_bytes()` reuses plan 59's own finding — `String`
  is already "a pointer to a null-terminated UTF-8 buffer," bit-for-bit
  what `Bytes`'s `ptr` field wants — and only needs a length, obtained
  the same way `.length` already does (`emerald_string_length`'s
  `strlen`). `.to_hex()` is a dependency-free nibble-to-ASCII loop
  (`emerald-rt`, no crate needed, same "zero-dependency, correctness
  never in question" posture plan 91's own FNV-1a proof function used)
  — necessary purely so this plan's own Concrete Proof output is
  printable through the existing `puts`/`String` machinery, not a
  primitive any of the three crypto plans needed invented.
- **One-shot and incremental are both real, both already how the
  underlying crates work — this plan doesn't pick one and bolt the
  other on.** Verified directly against each crate's own README:
  `sha2`/`sha3` expose the shared RustCrypto `digest::Digest` trait with
  both a one-shot `Sha256::digest(bytes)` and an incremental
  `Sha256::new(); hasher.update(...); hasher.finalize()`; `blake3`
  independently (it doesn't depend on RustCrypto's `digest` crate by
  default) exposes the structurally identical pair, `blake3::hash(bytes)`
  and `Hasher::new(); hasher.update(...); hasher.finalize()`. This
  plan's own `<Algorithm>.hash`/`<Algorithm>.new()...finalize()` split
  is a direct, unmodified mirror of a shape both crate families already
  independently converged on, not a new API this plan is designing from
  scratch. `md-5`'s README example is itself written in the incremental
  form (`Md5::new(); hasher.update(...); hasher.finalize()`) even for a
  one-line input — this plan still only exposes `Md5.hash` one-shot,
  since MD5 has no legitimate incremental-hashing-of-huge-input use case
  distinct from its already-narrow legacy-checksum role.
- **Incremental support ships for exactly two algorithms —
  `Sha256Hasher` and `Blake3Hasher` — chosen because they're
  structurally different constructions, not because the other four
  couldn't support it.** SHA-256 is a classic Merkle-Damgård
  compression-function hash; BLAKE3 is a Merkle-tree hash with a
  genuinely different internal chunking model (its own README:
  "Highly parallelizable... because it's a Merkle tree on the inside").
  Proving the streaming handle pattern once against each is real
  coverage of "does this shape generalize," not redundant repetition;
  `Sha512Hasher`/`Sha3_256Hasher`/`Sha3_512Hasher`/(no `Md5Hasher`, see
  above) are the identical mechanical pattern against the same
  `digest::Digest` trait `Sha256Hasher` already uses, deferred only to
  keep this plan's leaf count and diff size reasonable — a disclosed,
  not a technical, limitation.
- **The hasher handle is an inert, CString-shaped sema type today; its
  real free/lifetime story is plan 93's job, not this plan's.** Per
  plan 59's own precedent for `CString` — "a deliberately inert
  reference type... no `String` method dispatches on it at all" — is
  exactly what `Sha256Hasher`/`Blake3Hasher` need: a sema-visible type
  distinguishable from every other type, backed at codegen by nothing
  more than the bare pointer `CString` already is. What's genuinely new
  and *not* solved by copying `CString`'s shape: `.finalize()` frees the
  boxed Rust state it consumes, but a program that calls `Sha256.new()`
  and never calls `.finalize()` (an early `return`, an exception,
  ordinary control flow) leaks that allocation forever — Emerald has no
  destructor/drop-glue on scope exit for *any* type today (per plan 51/
  59's own "no free, no lifetime tracking" precedent for `emerald_alloc`
  generally), so there is no implicit cleanup path to fall back on. This
  plan does not invent a fix — it names the real gap and defers the
  general mechanism to plan 93, exactly as plan 91 itself deferred its
  own "no resource/handle lifetime model" gap to the same plan.
- **MD5 ships explicitly flagged broken-for-security, kept only for
  legacy interop — RustCrypto's own words, not a paraphrase.** Quoted
  directly from `md-5`'s own README: "This crate is provided for the
  purposes of legacy interoperability with protocols and systems which
  mandate the use of MD5. However, MD5 is cryptographically broken and
  unsuitable for further use... RFC 6151 advises no new IETF protocols
  can be designed MD5-based constructions, including HMAC-MD5." This
  plan reproduces that same warning verbatim in `Md5.hash`'s own Emerald
  stdlib documentation (per plan 95's mandatory-docs checklist) rather
  than softening it — a caller reaching for `Md5.hash` should see the
  same warning RustCrypto's own maintainers put in front of it.
- **This plan invents no new cryptographic construction — every one of
  the six hash functions is a direct, unmodified call into its crate's
  own `digest`/`hash` entry point.** No custom padding, no truncation
  scheme beyond what SHA-384/SHA-512-224/SHA-512-256 would need (none
  of those three variants are exposed by this plan at all — see Out of
  scope), no combining or chaining of algorithms. `Bytes.to_hex()` is an
  encoding, not a cryptographic primitive, and is the one piece of
  genuinely new logic this plan contains.
- **Digest equality is ordinary, non-constant-time `==` — deliberately,
  not by oversight.** Hashing itself has no secret-dependent branch: the
  *data being hashed* isn't secret from the algorithm's own perspective
  in any of this plan's six functions (there's no key, no private
  state). The real timing hazard arrives one layer up, at the call
  site: a program that computes `Sha256.hash(candidate_token)` and
  compares the result against a stored, secret expected digest using
  ordinary byte-array `==` leaks timing information about how many
  leading bytes matched. This plan does not attempt to fix that here —
  `Bytes` gets whatever equality Emerald's existing value-equality
  machinery already gives every other value type, and the constant-time
  comparison a caller doing exactly this needs is plan 117's `subtle`
  crate's job, cited here rather than re-derived.
- **Dispatch reuses plan 45's exact hardcoded-arm mechanism six more
  times, and the duplication this creates is disclosed, not hidden.**
  Six new `Expr::Ident(n) if n == "Sha256"`-shaped arms (plus plans 110
  and 111's own pseudo-modules layered on top over the batch) is real,
  visible repetition of the same `File`-shaped pattern plan 45
  established for exactly one namespace. A general "compiler-known
  intrinsic namespace" table (name string → dispatch entry, looked up
  once instead of matched arm-by-arm) is the obvious refactor once this
  duplication gets large enough to hurt — not attempted here, since
  plan 45's own stated reasoning ("one hard-coded arm is strictly less
  code than a new registration mechanism that would have exactly one
  user") still holds for six more arms; it stops holding once enough of
  this 91-191 batch's later plans land to make the count double-digit,
  and that refactor belongs to whichever future plan first feels that
  pain, not this one speculatively.
- **Out of scope.** HMAC (a distinct MAC construction layered on top of
  a hash, its own plan); password hashing / KDFs (Argon2, scrypt,
  PBKDF2 — deliberately slow constructions serving a different purpose
  than these fast general-purpose hashes, explicitly warned against by
  BLAKE3's own README: "BLAKE3 is not a password hashing algorithm...
  If you hash passwords... we recommend Argon2"); SHA-1 (broken,
  declined entirely, not even for legacy interop the way MD5 is);
  SHA-384/SHA-512-224/SHA-512-256 (real `sha2`-family variants this
  plan simply doesn't wire up — mechanically identical to Sha256/Sha512,
  deferred for the same leaf-count-discipline reason as the extra
  incremental hashers); BLAKE3's `keyed_hash`/`derive_key` modes (real
  BLAKE3 capabilities, but a MAC and a KDF respectively — out of scope
  for the same reason HMAC and password KDFs are); BLAKE3's `rayon`
  multi-threaded-hashing feature (a real performance option for very
  large inputs, not needed to prove this plan's mechanism); constant-
  time digest comparison (plan 117).

## Not yet decided (blocking EXECUTE)

1. `Bytes`'s exact canonical name and struct layout are plan 92's call,
   not this plan's — this plan adopts the provisional `{ptr, len:
   Int64}` shape described above and will reconcile with whatever plan
   92 actually lands if the two diverge.
2. `Sha256Hasher`/`Blake3Hasher`'s exact free/consume mechanism (a
   `.free()` escape hatch for the never-finalized case, `Handle[T]`
   generalized across all of plans 109/110/111's opaque handles, or
   something else) is plan 93's call; this plan's own leaf only commits
   to the handle being *inert and CString-shaped*, not to how its
   lifetime is ultimately managed.
