2026-09-21T20:24:00Z

---
name: Key Derivation Functions — `hkdf` for High-Entropy Secrets, `pbkdf2` for Low-Entropy Passwords (Interop Only)
overview: "Two new stdlib entry points backed by two RustCrypto crates with genuinely different jobs: `Kdf.hkdf(ikm, salt, info, length): String` wraps `hkdf` (v0.13.0) for expanding one already-strong secret — a Diffie-Hellman shared value, a master key — into several independent, differently-purposed keys; `Kdf.pbkdf2(password, salt, iterations, length): String` wraps `pbkdf2` (v0.13.0) for stretching a human password into a key, offered here specifically for interop with systems (older standards, existing on-disk formats, hardware/protocol requirements) that already mandate PBKDF2 — plan 112's Argon2id remains the actually-recommended choice for new password-based key derivation, stated here as an explicit steer, not left for the reader to infer from two similarly-shaped functions sitting next to each other."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-kdf-module-scaffold
    content: "Add `Kdf` as a compiler-known intrinsic namespace (plan 45's `File`-style dispatch). Add `hkdf = \"0.13\"` and `pbkdf2 = { version = \"0.13\", features = [\"sha2\"] }` to `crates/emerald-rt/Cargo.toml`, entered into plan 95's `DEPENDENCIES.md` ledger with their real transitive pulls verified this session against docs.rs: `hkdf` depends on `hmac` 0.13; `pbkdf2` depends on `digest` 0.11, `hmac` 0.13 (default feature), and optional `sha2` 0.11 (enabled here, since SHA-256 is this plan's only offered hash)."
    status: pending
  - id: leaf-hkdf-entrypoint
    content: "`Kdf.hkdf(ikm: String, salt: String, info: String, length: Int64): String` via `emerald_rt_hkdf_sha256`, hex-decoding `ikm`/`salt`/`info` from caller-supplied hex text (see Decision log for why hex-in/hex-out, not raw bytes, is this plan's Emerald-facing convention), constructing `Hkdf::<Sha256>::new(Some(salt_bytes), ikm_bytes)` and calling `.expand(info_bytes, &mut okm_buf)` where `okm_buf` is `length` bytes, hex-encoding the result before crossing back into a `String`. `catch_unwind`-wrapped per plan 91; `length > 255 * 32` (SHA-256's `L <= 255*HashLen` ceiling per RFC 5869) rejected as a caught error, not a panic."
    status: pending
  - id: leaf-pbkdf2-entrypoint
    content: "`Kdf.pbkdf2(password: String, salt: String, iterations: Int64, length: Int64): String` via `emerald_rt_pbkdf2_hmac_sha256`, calling `pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt_bytes, iterations as u32, &mut key_buf)`, hex-encoding the result. `iterations < 600_000` (OWASP's current PBKDF2-HMAC-SHA256 minimum, cited alongside plan 112's Decision log making the case for Argon2id instead) does not error — a lower iteration count is a legitimate interop requirement this plan doesn't gate — but the doc comment states the OWASP minimum explicitly at the call site (plan 21's LSP hover mechanism)."
    status: pending
  - id: leaf-rust-tests-and-example
    content: "`#[test]` in `emerald-rt`: (1) HKDF-SHA256 against RFC 5869 Test Case 1's real, published vector; (2) PBKDF2-HMAC-SHA256 against the `pbkdf2` crate's own published doctest vector (`password`/`salt`, 600,000 iterations, 20-byte output). Both cited exactly in the Decision log below, not invented. Add `examples/key_derivation_proof.em` to `examples/`, wired into `emerald-cli/tests/examples.rs`'s checked table, reproducing the HKDF vector end-to-end through the Emerald surface (see Concrete Proof)."
    status: pending
isProject: false
---

# Plan 115 — Key Derivation Functions (HKDF/PBKDF2)

HKDF and PBKDF2 both take some input material and a salt and produce
output that looks, from a distance, like the same kind of thing — a
sequence of bytes derived from another sequence of bytes. They exist to
solve genuinely different problems, and using either one where the other
belongs is a real, documented mistake this plan's own source RFC warns
about directly.

HKDF's job, per RFC 5869 itself (fetched and read in full this session):
"take some source of initial keying material and derive from it one or
more cryptographically strong secret keys." Its whole design assumes the
input is *already* a good source of entropy — a Diffie-Hellman shared
secret (which plan 111's `x25519-dalek` produces), a symmetric master
key, anything an attacker cannot enumerate by brute force. HKDF's job is
then to concentrate and re-shape that entropy into one or more
independent keys bound to different purposes via its `info` parameter —
it is fast by design, because slowing it down would defend against
nothing: there's no dictionary attack against a 256-bit
uniformly-random Diffie-Hellman output to slow down.

PBKDF2's job is the opposite: RFC 5869 itself names this explicitly in
its own "Applications" section — "One significant example is the
derivation of cryptographic keys from a source of low entropy, such as a
user's password... In the case of password-based KDFs, a main goal is to
slow down dictionary attacks... Applications interested in a
password-based KDF should consider whether [PKCS5, i.e. PBKDF2] meets
their needs better than HKDF." A human password has far less real entropy
than its character count suggests, and is exactly the kind of input an
attacker *can* enumerate — PBKDF2 exists to make each guess expensive via
a caller-chosen iteration count, the exact opposite performance goal from
HKDF. **Plan 112's Argon2id is the modern, OWASP-preferred choice for
deriving a key from a password today** — Argon2id is memory-hard (GPU/ASIC
resistant in a way PBKDF2's pure-CPU cost is not) as well as
time-hard. This plan's `Kdf.pbkdf2` exists for one honest reason: interop
with systems, formats, or standards that already mandate PBKDF2
specifically (WPA2/WPA3 Wi-Fi key derivation, many existing encrypted
file formats, FIPS-140 contexts that permit PBKDF2 but not yet Argon2)
— not because it is this project's recommendation for new work.

## Concrete proof this plan targets

```ruby
ikm: String = "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"
salt: String = "000102030405060708090a0b0c"
info: String = "f0f1f2f3f4f5f6f7f8f9"

okm: String = Kdf.hkdf(ikm, salt, info, 42)
puts okm

pw_key: String = Kdf.pbkdf2("password", "salt", 600000, 20)
puts pw_key
```

Expected output, both lines real, independently-verifiable hex, not
invented (see Decision log for sourcing): first line
`3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865`
(RFC 5869 Appendix A.1's own published OKM for this exact IKM/salt/info/
length); second line `669cfe52482116fda1aa2cbe409b2f56c8e45637` (the
`pbkdf2` crate's own published doctest expected value for this exact
password/salt/iteration-count/length).

## Decision log

- **Both functions take and return hex-encoded `String`s, never raw
  bytes, for the same reason plan 113's `Random.secure_*` does.** Plan
  59's finding that Emerald's `String` is a bare null-terminated `char*`
  makes it unsafe to carry arbitrary binary IKM/salt/info/output directly
  — HKDF's own real inputs (a Diffie-Hellman shared secret, in
  particular) are guaranteed *not* to be valid UTF-8 or NUL-free. Hex
  text is this plan's chosen Emerald-facing encoding, matching plan 113's
  own choice exactly, so a value produced by `Random.secure_hex` can be
  passed directly as `Kdf.hkdf`'s `salt` argument with no re-encoding
  step. Plan 92's binary-safe `(ptr, len)` convention is what the Rust-
  side implementation uses internally to move the decoded bytes between
  `hex::decode`, the `hkdf`/`pbkdf2` crate calls, and `hex::encode` — it
  never needs to surface at the Emerald boundary because, as with plan
  113, nothing in this plan's proof needs raw bytes visible to Emerald
  source itself.
- **The RFC 5869 Test Case 1 vector is quoted directly from the RFC's own
  published text, cross-checked against the `hkdf` crate's own README
  doctest, which reproduces the identical numbers as a compiling,
  crate-maintainer-verified example — not derived or estimated by this
  plan.** Both sources, fetched independently this session, agree
  byte-for-byte: `IKM = 0x0b0b...0b` (22 octets), `salt =
  0x000102...0c0c` (13 octets — see IETF text for exact value used
  above), `info = 0xf0f1...f9` (10 octets), `L = 42`, `OKM =
  0x3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf3400
  7208d5b887185865`. Using an RFC's own published test vector, corroborated
  by the crate's own doctest, is the strongest form of "not invented" this
  plan can offer — no plausible-looking value was computed or guessed.
- **The PBKDF2 vector is quoted directly from the `pbkdf2` crate's own
  current (`0.13.0`) `docs.rs` documentation page, fetched this session —
  a compiling doctest the crate's own CI runs on every release, not a
  hand-copied external value.** `password = b"password"`, `salt =
  b"salt"`, `n = 600_000` iterations, `Sha256`, `expected =
  669cfe52482116fda1aa2cbe409b2f56c8e45637` (20 bytes). RFC 6070's own
  published PBKDF2 test vectors use HMAC-SHA1, not SHA-256 (this plan's
  only offered hash, per the leaf below) — rather than adapt an SHA-1
  vector to a different PRF by hand (a real risk of introducing an error
  this plan's own test would then silently bake in), this plan cites the
  crate's own SHA-256 doctest directly, which is both more directly
  applicable and independently checkable by any reader with `cargo doc
  --open -p pbkdf2` and no RFC cross-referencing required. The 600,000
  iteration count is not arbitrary either — it is OWASP's own current
  documented PBKDF2-HMAC-SHA256 minimum, verified this session, and
  reused here for both the crate's chosen doctest value and this plan's
  own `iterations` argument documentation.
- **This plan offers SHA-256 only, not a hash-generic interface, even
  though both underlying crates are generic over any `Digest`.** `hkdf`
  and `pbkdf2` are both written generic over RustCrypto's `Digest` trait
  (`Hkdf<Sha256>`, `Hkdf<Sha1>`, etc.) — this plan's Emerald-visible
  surface fixes `Sha256` for both, matching plan 109's own presumed
  general-hashing default and avoiding an Emerald-level algorithm-choice
  parameter for a decision that has one clearly correct answer today
  (SHA-1 is cryptographically broken for collision resistance and has no
  legitimate reason to be newly offered in either KDF context; SHA-512 or
  SHA-3 variants are real, reasonable future additions declined here only
  because nothing in this batch's proof needs them). A future
  `Kdf.hkdf_sha512`/`Kdf.pbkdf2_sha512` pair is additive, not a redesign.
- **`Kdf.hkdf`'s three-argument shape (`ikm`, `salt`, `info`) mirrors RFC
  5869's own extract-then-expand two-step process collapsed into one
  call, not exposed as two separate `extract`/`expand` steps.** The
  `hkdf` crate itself offers both a combined `Hkdf::new` + `.expand()`
  entry point and a lower-level `Hkdf::extract`/`Hkdf::from_prk` split
  for callers who need to inspect or cache the intermediate PRK. This
  plan's single `Kdf.hkdf` call always performs both steps together —
  the PRK is not itself a byte sequence any use case in this batch's six
  plans needs to inspect or persist independently, so exposing it as a
  second Emerald-visible function is deferred, not designed away
  permanently.
- **`salt` is a required, non-optional argument for both functions in
  this plan's v1 surface, even though both underlying algorithms
  technically permit an empty or absent salt.** RFC 5869 itself: "if not
  provided, [salt] is set to a string of HashLen zeros" — a real, legal
  configuration, but one the same RFC's own "Notes to HKDF Users" section
  discourages ("the use of salt adds significantly to the strength of
  HKDF"). Requiring `salt` as a plain (non-nullable) `String` parameter
  rather than a `String?` with an implicit all-zero default nudges every
  caller toward supplying a real salt (ideally from plan 113's
  `Random.secure_hex`) without making the RFC-legal empty-salt case
  impossible — a caller who genuinely wants RFC 5869's zero-salt behavior
  can still pass an all-zero hex string of the correct length explicitly,
  visibly, rather than by omission.
- **Out of scope.** No `Kdf.hkdf_extract`/`Kdf.hkdf_expand` split entry
  points (see above) — additive, not required for this plan's proof. No
  hash-algorithm choice beyond SHA-256 (see above). No integration with
  plan 111's X25519 Diffie-Hellman output beyond stating the intended
  data flow in this plan's own overview — plan 111 is authored in
  parallel this session and this plan does not assume or depend on its
  exact key-material representation beyond "a hex-encoded shared secret
  `String`," which is this plan's own `ikm` parameter type regardless of
  where it came from. No PBKDF2 variants using a PRF other than
  HMAC-SHA256 (the `pbkdf2` crate supports arbitrary keyed PRFs via its
  lower-level `pbkdf2` function; this plan exposes only the common,
  default `pbkdf2_hmac` entry point). No caller-facing warning or hard
  error when `Kdf.pbkdf2`'s `iterations` argument is set below OWASP's
  600,000 minimum — documented as a recommendation, not enforced, since
  legitimate interop targets may mandate a specific, possibly lower,
  count this plan must not silently override.
