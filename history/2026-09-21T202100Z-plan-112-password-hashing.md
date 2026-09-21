2026-09-21T20:21:00Z

---
name: Password Hashing — `argon2` (RustCrypto, Argon2id) as a Structurally Separate Surface From General-Purpose Hashing
overview: "A new `Password` stdlib module wrapping `argon2` (RustCrypto, v0.6.0, the pure-Rust, PHC-award-winning Argon2 implementation defaulting to the Argon2id variant) with exactly two Emerald-visible entry points — `Password.hash(plaintext): String` and `Password.verify(plaintext, stored_hash): Boolean` — deliberately namespaced away from plan 109's `Hash` module so that no plausible autocomplete or copy-paste path leads a caller from `Hash.sha256(password)` to a stored password column. Salt generation is automatic and internal, using plan 113's CSPRNG surface via the `argon2`/`password-hash` crate's own `getrandom`-backed salt generator; memory/time/parallelism parameters default to OWASP's own current second-tier recommendation (`m=19456 KiB, t=2, p=1`) and are never required at the call site."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-password-module-scaffold
    content: "Add `Password` as a compiler-known intrinsic namespace (plan 45's `File`-style `Expr::Ident(n) if n == \"Password\"` dispatch, checked before the ordinary module-dispatch arm). Add `argon2 = { version = \"0.6\", features = [\"password-hash\"] }` to `crates/emerald-rt/Cargo.toml`, entered into plan 95's `DEPENDENCIES.md` ledger with its real transitive pulls (`base64ct`, `blake2` 0.11, `cpufeatures`, `password-hash` 0.6) verified this session against the crate's own docs.rs dependency listing."
    status: pending
  - id: leaf-hash-entrypoint
    content: "`Password.hash(plaintext: String): String` returning a full PHC string (`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, RFC-less but industry-standard Password Hashing Competition string format) via `#[no_mangle] extern \"C\" fn emerald_rt_password_hash(plaintext: *const c_char) -> *mut c_char`, using `Argon2::default()` (Argon2id, v19, per the crate's own doc default) with `Params` overridden to OWASP's `m=19456,t=2,p=1` tier and `PasswordHasher::hash_password` for random-salt generation, `catch_unwind`-wrapped per plan 91's boundary convention, empty-string input rejected as a caught error rather than hashed."
    status: pending
  - id: leaf-verify-entrypoint
    content: "`Password.verify(plaintext: String, stored_hash: String): Boolean` via `emerald_rt_password_verify`, parsing `stored_hash` back into a `password_hash::PasswordHash` and calling `Argon2::default().verify_password(plaintext.as_bytes(), &parsed)`, mapping `Ok(())` to `true` and every `Err` variant (including a malformed/unparseable `stored_hash`) to `false` rather than propagating a distinguishable error — see Decision log for why a caller must never be able to distinguish \"wrong password\" from \"corrupt hash\" from the return value alone. The comparison itself is the `argon2`/`password-hash` crate's own internal constant-time check, not a hand-rolled one — plan 117 is cited, not re-implemented, for why this matters."
    status: pending
  - id: leaf-params-not-caller-tunable-in-v1
    content: "No Emerald-visible parameter for memory/time/parallelism in this plan's two entry points — the OWASP-recommended tier is hardcoded Rust-side in `emerald-rt`. Document in the doc comment (plan 21's LSP hover mechanism) that raising the cost parameters requires a source change to `emerald-rt`, not an Emerald-level call, and why (see Decision log)."
    status: pending
  - id: leaf-rust-tests-and-example
    content: "`#[test]` in `emerald-rt`: (1) round-trip `emerald_rt_password_hash` then `emerald_rt_password_verify` on the same plaintext returns `true`; (2) verify against a deliberately wrong plaintext returns `false`; (3) verify against a hand-corrupted PHC string (a flipped character in the base64 hash segment) returns `false`, not a panic or an error propagated to the caller. Add `examples/password_hashing_proof.em` to `examples/`, wired into `emerald-cli/tests/examples.rs`'s checked table, asserting `Password.verify(pw, Password.hash(pw))` is `true` and `Password.verify(\"wrong\", Password.hash(pw))` is `false` — no fixed hash string is asserted since `Password.hash` output is salted and non-deterministic by design (see Decision log)."
    status: pending
isProject: false
---

# Plan 112 — Password Hashing

Plan 109 (general-purpose cryptographic hashing — SHA-256/SHA-3/BLAKE3,
written in parallel this session) and this plan look, on the surface, like
they overlap: both take a string and return a string that looks like a
hash. They solve opposite problems and this plan exists specifically to
keep them from ever being reached for interchangeably. A general-purpose
hash function's entire design goal is to be *fast* — SHA-256 on modern
hardware runs at multiple gigabytes per second, and that speed is exactly
what makes it catastrophic for password storage: an attacker who steals a
database of SHA-256(password) values can brute-force billions of guesses
per second per GPU, because the hash function itself puts up no
resistance — there is no salt requirement built into the primitive, no
tunable work factor, nothing standing between a stolen hash and a
dictionary attack except the password's own entropy. A password hashing
function's entire design goal is the opposite: to be *slow and
memory-hungry on purpose*, tunably so, so that the exact same brute-force
attack that takes seconds against unsalted SHA-256 takes years against a
correctly-parameterized Argon2id hash of the same password. **`Password`
and plan 109's `Hash` module must never be the same namespace, must never
share an implementation, and this plan states as a hard project rule:
plan 109's SHA-256/SHA-3/BLAKE3 functions must never be documented,
suggested, or used for password storage anywhere in Emerald's own stdlib
or its examples.**

`argon2` — verified this session via `lib.rs` and `docs.rs`, current
release `0.6.0` (2026-08-27), a RustCrypto/`password-hashes` project crate,
pure Rust, `no_std`-capable — implements all three Argon2 variants
(Argon2d, Argon2i, Argon2id) and defaults to Argon2id: the crate's own docs
state plainly, "**Argon2id**: (default) hybrid version combining both
Argon2i and Argon2d." Argon2 won the Password Hashing Competition in July
2015, and OWASP's own Password Storage Cheat Sheet (fetched this session)
names Argon2id first: "Out of the three Argon2 versions, use the Argon2id
variant since it provides a balanced approach to resisting both
side-channel and GPU-based attacks." This plan's default matches both the
crate's own default and OWASP's recommendation with no divergence.

## Concrete proof this plan targets

```ruby
password: String = "correct horse battery staple"
hash: String = Password.hash(password)
puts Password.verify(password, hash)
puts Password.verify("wrong password", hash)
puts hash.length > 0
```

Expected output: `true`, `false`, `true`. The actual value of `hash`
cannot be asserted as a fixed string — see the Decision log's explanation
of why a correct implementation must produce a different hash on every
call to `Password.hash` even for the identical input, which is the
opposite property a deterministic-hash `#[test]` vector would need.

## Decision log

- **Namespace separation from plan 109's `Hash` module is the primary
  safety mechanism this plan relies on, not a footnote.** A shared
  namespace (`Hash.argon2(pw)` sitting next to `Hash.sha256(pw)`) invites
  exactly the confusion this plan exists to prevent — a caller skimming
  autocomplete sees two equally-plausible-looking hash functions and picks
  whichever one they've heard of. `Password.hash`/`Password.verify` living
  in their own namespace, named for the *task* ("hash a password") rather
  than the *mechanism* ("Argon2id"), is a deliberate API-design choice
  that plan 91's Decision log's "structural fence, not prose convention"
  posture applies to directly.
- **Salt generation is automatic, mandatory, and internal — there is no
  Emerald-visible salt parameter at all.** `PasswordHasher::hash_password`
  (the crate's own high-level API, verified against its docs.rs usage
  example) generates a fresh random salt internally via the
  `password-hash` crate's `getrandom`-backed `SaltString::generate`,
  which this plan's Rust-side implementation invokes with no caller input.
  This is a deliberate simplification relative to a hypothetical
  `Password.hash(pw, salt)` signature: every historical password-hashing
  vulnerability class involving salts (reused salts across users, salts
  too short to matter, salts the developer forgot to generate at all) is
  a caller-facing mistake this design makes structurally impossible by
  never giving the caller the chance to make it. Plan 113 is the
  CSPRNG this salt generation ultimately rests on, transitively, via the
  `argon2`/`password-hash` crate's own `getrandom` dependency — this
  plan's Rust code never calls plan 113's `emerald_rt_random_*` functions
  directly, since the crate's own internal salt generator is already
  correct and duplicating it would be redundant, not safer.
- **Work-factor parameters (memory, time, parallelism) default to OWASP's
  own current second recommendation and are not exposed as an Emerald-
  level tuning knob in this plan.** OWASP's Password Storage Cheat Sheet
  (fetched this session) lists five equal-strength configurations trading
  memory for CPU; this plan hardcodes the second, `m=19456` KiB (19 MiB),
  `t=2` iterations, `p=1` degree of parallelism — a real, currently-
  recommended, non-invented number, not a guess. Making this caller-
  tunable (`Password.hash(pw, memory_kib: ..., iterations: ...)`) is a
  reasonable future extension but is declined here deliberately: a
  parameter surface invites a well-meaning caller to weaken it for
  perceived performance reasons in exactly the way a hardcoded, security-
  reviewed default cannot be weakened by accident. Raising the default as
  hardware gets faster remains possible via an ordinary `emerald-rt`
  source change and version bump, which is the same mechanism any other
  stdlib security default would use.
- **`Password.verify`'s failure modes are deliberately collapsed to a
  single `Boolean`, never a distinguishable error.** A `stored_hash` that
  fails to parse as a valid PHC string (truncated, corrupted, or simply
  not an Argon2 hash) and a `stored_hash` that parses correctly but simply
  doesn't match `plaintext` both return `false` — the Rust-side
  implementation matches on `Result<(), password_hash::Error>` and maps
  every `Err` variant, parse errors included, to `false`, never
  propagating which failure occurred. This is a deliberate security
  choice, not a missed error-handling case: an API that let a caller
  distinguish "your stored hash is corrupted" from "the password didn't
  match" via a different return value or exception type would leak
  information about the stored-hash format/corruption state to anything
  that can call `Password.verify` in a loop, a real (if narrow) oracle
  this design closes by construction.
- **The equality check inside `verify_password` is the crate's own
  problem to get right, and it already does — this plan does not
  re-implement or wrap it in a second comparison.** Plan 117 (constant-
  time comparison, this same session) establishes the project-wide rule
  that hand-written Emerald or `emerald-rt` code comparing secrets must
  route through `subtle::ConstantTimeEq` rather than `==`. This plan
  satisfies that rule by inheritance rather than duplication: `argon2`'s
  own `PasswordVerifier::verify_password` performs its digest comparison
  internally using the same RustCrypto-ecosystem constant-time discipline
  plan 117 documents, and this plan's Rust code never re-extracts the raw
  hash bytes to compare them a second time itself — doing so would be
  strictly more risk (a second, hand-rolled comparison site plan 117's
  audit would need to catch) for zero benefit.
- **No deterministic `#[test]` hash-value vector exists for this plan,
  for the same reason plan 113 has none — verified as `argon2`'s own
  documented behavior, not assumed.** Because `Password.hash` generates a
  fresh random salt on every call (per the point above), hashing the same
  plaintext twice produces two different PHC strings by design — this is
  the entire point of salting. This plan's Rust `#[test]`s therefore
  assert behavior (`verify` round-trips correctly, wrong passwords and
  corrupted hashes are rejected) rather than a fixed byte-for-byte output,
  the same honest constraint plan 113's Decision log states for CSPRNG
  output.
- **`argon2`'s own `hash_password_into` low-level key-derivation API is
  explicitly not exposed by this plan.** The crate documents a second,
  lower-level usage mode ("Key Derivation... useful for transforming a
  password into cryptographic keys for e.g. password-based encryption")
  separate from its PHC-string password-hashing mode. This plan's two
  entry points cover password *storage/verification* only; using Argon2
  as a raw KDF for encryption-key derivation is a distinct use case plan
  115 (HKDF/PBKDF2) is the closer fit for discussing, and is not folded
  into this plan's narrower scope.
- **Out of scope.** No configurable "pepper" (a server-side secret mixed
  into every hash via `Argon2::new_with_secret`, which the crate does
  support) — a real, legitimate hardening technique, but one requiring
  its own secret-management story (where does the pepper live, how is it
  rotated) this plan's two-function surface doesn't attempt to design.
  No password-strength/complexity validation (`Password.hash` accepts any
  non-empty `String` and hashes it as given) — a distinct, policy-level
  concern, not a hashing-mechanism one. No migration/upgrade path for
  re-hashing an existing password at login time when OWASP's recommended
  parameters change in the future (`Password.needs_rehash(stored_hash):
  Boolean`, comparing the parsed hash's params against the current
  default) — a reasonable, real feature, but additive and not required to
  prove hashing and verification work correctly today.
