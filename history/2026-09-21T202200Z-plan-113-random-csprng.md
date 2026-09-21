2026-09-21T20:22:00Z

---
name: Cryptographically Secure Random Number Generation — `rand`/`getrandom`-Backed `SysRng`, Distinguished From Fast Non-Cryptographic Shuffling
overview: "A new `Random` stdlib module wrapping the `rand` crate (v0.10.3, `rand_core` 0.10, OS-entropy-backed via `getrandom` 0.4) as two deliberately separated API surfaces: `Random.secure_hex(n)`/`Random.secure_token(n)`, backed by `rand::rngs::SysRng` (the crate's rename, as of 0.10, of what was `OsRng` — a CSPRNG safe for keys, nonces, session tokens, and salts), and `Random.int(min, max)`/`Random.shuffle(arr)`, backed by the crate's fast, non-cryptographic `ThreadRng`, safe only for simulations, games, and sampling. This plan is a hard dependency for plan 112 (Argon2 salt generation), plan 114 (JWT — no direct RNG use, but its RS256/ES256 signing keys route through plan 111, which itself depends on this plan for keypair generation), plan 115 (HKDF/PBKDF2 salt and info generation), plan 117 (no RNG use directly, but its constant-time-comparison discipline protects values this plan generates), and later plans in the batch needing UUIDs and session tokens."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-random-module-scaffold
    content: "Add `Random` as a compiler-known intrinsic namespace to `emerald-sema`/`emerald-codegen`, following plan 45's `File`-style dispatch shape verbatim (an `Expr::Ident(n) if n == \"Random\"` guard checked before the ordinary module-dispatch arm, since `Random` is never declared via a real `ModuleDef` and can never collide with `classes`). Add the crate dependency `rand = { version = \"0.10\", features = [\"sys_rng\", \"thread_rng\"] }` to `crates/emerald-rt/Cargo.toml` per plan 91's scaffolding, entered into plan 95's `DEPENDENCIES.md` ledger with its own transitive pull of `rand_core` 0.10 and `getrandom` 0.4."
    status: pending
  - id: leaf-secure-rng-surface
    content: "`Random.secure_hex(n: Int64): String` and `Random.secure_token(n: Int64): String` (a URL-safe-base64 variant of the same underlying bytes) — both backed by `#[no_mangle] extern \"C\" fn emerald_rt_random_secure_hex(n: i64, out_len: *mut i64) -> *mut c_char`, filling an `n`-byte buffer via `rand::rngs::SysRng` (feeding `rand_core::TryRngCore::try_fill_bytes` or the crate's current fill-bytes entry point) and hex/base64url-encoding it before crossing into an Emerald `String`, wrapped in `std::panic::catch_unwind` per plan 91's mandatory boundary convention. Reject `n <= 0` as a caught, converted error rather than a panic that reaches the boundary."
    status: pending
  - id: leaf-fast-rng-surface
    content: "`Random.int(min: Int64, max: Int64): Int64` (inclusive range) and `Random.shuffle(arr: Array[T]): Void` (in-place Fisher-Yates) backed by `rand::rng()` (`ThreadRng`, the crate's automatically-seeded, non-cryptographic default), each its own `extern \"C\"` export, each independently `catch_unwind`-wrapped. `Random.shuffle` mutates its argument in place via plan 45's existing `Stmt::SetIndex` codegen path, not a new mutation mechanism."
    status: pending
  - id: leaf-doc-distinction
    content: "Add a doc comment directly above both `Random.secure_*` and `Random.int`/`Random.shuffle` declarations in `emerald-sema` stating explicitly, in the generated LSP hover text (plan 21's mechanism), which functions are safe for keys/tokens/nonces and which are not — so the distinction plan 91's Decision log demands is visible at the call site, not only in this plan's prose."
    status: pending
  - id: leaf-rust-tests-and-example
    content: "`#[test]` in `emerald-rt` asserting `emerald_rt_random_secure_hex` returns the requested byte-length (`2*n` hex chars) across repeated calls and that two consecutive calls never produce identical output (a real, if statistically weak, live proof the CSPRNG path is actually wired to a real entropy source and not a fixed buffer) — see the Decision log for why a byte-exact test vector is neither possible nor meaningful for randomness. Add `examples/random_csprng_proof.em` to `examples/` and wire it into `emerald-cli/tests/examples.rs`'s checked table per plan 91's own precedent."
    status: pending
isProject: false
---

# Plan 113 — Cryptographically Secure Random Number Generation

Every one of this batch's crypto plans needs unpredictable bytes somewhere:
plan 112 needs a fresh salt per password, plan 114's HS256 path needs a
strong shared secret if the caller generates rather than supplies one,
plan 115's HKDF needs salt/info material, and plan 111 (asymmetric crypto,
written in parallel this session) needs key material for Ed25519/RSA/X25519
keypairs. All of that unpredictability has to come from one place, and it
has to come from the *right kind* of randomness — this plan is that place,
and its central job is making the right kind the only kind that's easy to
reach.

`rand` — verified this session via `lib.rs`, current release `0.10.3`
(2026-09-20), 166M downloads/month, used directly by 32,735 crates — is
explicit, in its own README, about what it is and isn't: "Rand **is not**:
... Primarily a cryptographic library. `rand` does provide some generators
which aim to support unpredictable value generation under certain
constraints... Users are expected to determine for themselves whether
`rand`'s functionality meets their own security requirements." That
sentence is the whole reason this plan exists as a separate design
decision rather than "just call `rand::random()`" — the crate ships both a
CSPRNG and a fast PRNG behind names similar enough that picking the wrong
one by accident is a real, historically common mistake (predictable
session tokens, guessable password-reset codes), and this plan's job is to
make that mistake structurally harder to make in Emerald than it is in
raw Rust.

As of `rand` 0.10 (released Feb 2026), the crate renamed its
OS-entropy-backed generator: verified against the crate's own CHANGELOG,
"Rename `os_rng` -> `sys_rng`, `OsRng` -> `SysRng`, `OsError` -> `SysError`"
— what was `rand_core::OsRng`/`rand::rngs::OsRng` in every pre-0.10 rand
tutorial is now `rand::rngs::SysRng`, backed by the `getrandom` crate
(bumped to `getrandom` 0.4 as part of the same 0.10 update, per the Rand
Book's "Updating to 0.10" page: "`rand_core::OsRng` has been replaced with
`getrandom::SysRng` (also available as `rand::rngs::SysRng`)"). This plan
targets that current name; the task's requested `OsRng`/`getrandom`
framing and the crate's actual current `SysRng`/`getrandom` reality are
the same underlying mechanism under a new name, stated here so a future
reader isn't confused by pre-0.10 tutorials still in wide circulation.

## Concrete proof this plan targets

```ruby
token: String = Random.secure_hex(16)
puts token.length

low: Int64 = 1
high: Int64 = 6
roll: Int64 = Random.int(low, high)
puts roll >= low && roll <= high

a: Array[Int64] = [1, 2, 3, 4, 5]
Random.shuffle(a)
puts a[0] >= 1 && a[0] <= 5
```

Expected output: `32` (16 bytes hex-encoded is 32 ASCII characters —
`token`'s literal *value* is not asserted, since it must be different on
every run; `.length` is), `true` (the die roll lands in the inclusive
`[1,6]` range every run), `true` (`shuffle` reorders `a` in place without
losing or duplicating elements — the fixed range check on `a[0]` is what's
run-to-run stable, not `a`'s new order).

## Decision log

- **Two backing generators, two names, deliberately far apart in the
  method-name alphabet and in this plan's own prose — not one generator
  with a "secure mode" flag.** A boolean flag (`Random.bytes(n, secure:
  true)`) is one easy-to-drop keyword away from silently downgrading; two
  entirely separate top-level method names (`secure_hex`/`secure_token` vs
  `int`/`shuffle`) require an affirmative, visible choice of the wrong one
  to get it wrong, and grepping an Emerald codebase for `Random.int` next
  to a `password_reset_token` variable name is a far more obvious code-
  review smell than a flag buried in an argument list. This mirrors, at
  the API-naming level, the same "structural fence, not a convention"
  posture plan 59's Decision log used for `unsafe extern "C" { ... }`.
- **`SysRng` (née `OsRng`) is the only generator this plan wires to
  anything security-sensitive, and it never gets a seed parameter.**
  `SysRng` draws directly from the OS's own CSPRNG (`getrandom(2)` on
  Linux, `BCryptGenRandom` on Windows, `getentropy` on macOS/BSD) on every
  call — verified via the crate's own feature-flag description, "`sys_rng`
  enables `rand::rngs::SysRng` (uses the `getrandom` crate)". No Emerald-
  visible API accepts a caller-supplied seed for `Random.secure_*` — a
  seedable "secure" RNG is a contradiction (reproducibility and
  unpredictability are opposite goals), so the seeding surface plan 91's
  own FNV-1a proof needed none of and this plan doesn't invent one either.
- **`ThreadRng` is real, automatically seeded, and reasonably strong for
  its stated purpose — not a toy — but it is still explicitly out of
  bounds for secrets, per the crate's own disclaimer above.** `rand::rng()`
  returns `ThreadRng`, described in the crate's own README as "an
  asymptotically-fast, automatically-seeded and reasonably strong
  generator available on all `std` targets" — good enough that
  `Random.int`/`Random.shuffle` never need to expose seeding or algorithm
  choice to Emerald source, but never promoted to `Random.secure_*`'s
  status. A future contributor tempted to have `Random.int` "just also
  work for tokens since it's already reasonably strong" is exactly the
  drift this plan's naming split exists to block.
- **Raw bytes never cross the Emerald FFI boundary as raw bytes — they are
  hex-encoded on the Rust side before becoming a `String`, per plan 92's
  binary-safe convention applied to the one case where it matters least
  and the escape hatch is cheapest.** Plan 59 verified Emerald's `String`
  is a bare null-terminated `char*`; arbitrary random bytes can contain an
  embedded `0x00` or invalid UTF-8, so they can never be handed to Emerald
  as a `String` directly — this is exactly the hazard plan 92's binary-
  safe `(ptr, len)` buffer convention exists to fence. This plan's own
  `emerald_rt_random_secure_hex` never exposes a raw `(ptr, len)` pair to
  Emerald source at all: the Rust-side implementation fills a byte buffer
  using that convention internally, then hex-encodes it into a guaranteed-
  ASCII, guaranteed-non-null buffer before ever constructing the `String`
  the caller receives. A future plan needing genuinely raw bytes at the
  Emerald boundary (a binary file write, a network payload) is the one
  that should reach for plan 92's `(ptr, len)` surface directly instead of
  a hex string — this plan's own two methods don't need it because their
  whole purpose is producing a value immediately usable as a token, and
  hex text is strictly more convenient for that than a raw buffer would
  be.
- **No byte-exact `#[test]` vector exists for this plan, and that absence
  is itself the honest, correct choice — not a gap.** Plan 91's own FNV-1a
  proof and plan 115's HKDF/PBKDF2 tests below both lean on fixed,
  independently-published test vectors precisely because those algorithms
  are required to be deterministic. A CSPRNG is required to be the
  opposite — asserting a fixed output for `Random.secure_hex(16)` would be
  asserting a bug (a hardcoded or badly-seeded generator), not a
  correctness proof. This plan's `#[test]` instead asserts the two
  properties that *are* checkable without contradicting the primitive's
  own definition: correct output length, and two consecutive calls never
  colliding (weak evidence of real entropy, strong evidence against an
  all-zero-buffer regression, the actual bug class this kind of test
  exists to catch).
- **`Random.shuffle` mutates its argument, breaking with plan 45's own
  "every String/Array method returns a new value, never mutates" rule —
  disclosed here as a deliberate, narrow exception, not an oversight.**
  Plan 45's Decision log states plainly that no method on any type
  mutates its receiver in place today, and that a mutating method "would
  need a genuinely new capability... with no existing precedent". This
  plan doesn't invent that capability from nothing, though: `Stmt::
  SetIndex` (`arr[i] = ...`) already exists as an ordinary statement-level
  mutation path, and `Random.shuffle`'s Rust-side implementation performs
  its in-place Fisher-Yates by issuing exactly the sequence of index
  writes an Emerald `while`-loop full of `arr[i] = arr[j]` swaps would —
  it is legitimate reuse of an existing mutation mechanism through a
  library function, not a new one. An `Enumerable`-style non-mutating
  `Random.shuffled(arr): Array[T]` (returning a new array, leaving the
  input untouched) is a reasonable future companion this plan does not
  add, since nothing in this batch's proof needs it yet.
- **Out of scope.** No seedable/reproducible RNG variant (`rand_chacha`
  with an explicit seed) for deterministic testing — a real, legitimate
  need, but a distinct feature with its own API surface this plan's
  two-tier split doesn't need to design alongside it. No `Random.float`,
  `Random.choice(array)`, `Random.bool`, or other convenience wrappers
  beyond `int`/`shuffle` — trivially addable later on the same `ThreadRng`
  path, not required to prove the CSPRNG/fast-RNG split works. No UUID
  generation (a later plan's job, built on `Random.secure_hex`'s same
  `SysRng` foundation but needing its own version/variant-bit formatting
  this plan doesn't attempt). No WASM-target entropy story (`getrandom`'s
  own WASM support requires target-specific feature configuration plan
  91's own "Not yet decided" section already flagged as unresolved for
  the whole `emerald-rt` crate, not something this plan resolves alone).
