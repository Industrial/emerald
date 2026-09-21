2026-09-21T20:26:00Z

---
name: Constant-Time Comparison — `subtle`'s `ConstantTimeEq`, and the Project Rule That Secret-Comparing Code Must Route Through It
overview: "A single new stdlib function, `SecureCompare.eq(a: String, b: String): Boolean`, wrapping `subtle` (dalek-cryptography, v2.6.1, the RustCrypto-ecosystem-standard constant-time-operations crate, BSD-3-Clause, zero runtime dependencies) — small in Emerald-visible surface area but cross-cutting in effect, since this plan also states, as an explicit project rule rather than an implicit assumption, which other plans in this batch (112 password verification, 109/110 MAC/hash comparison, 114 JWT signature verification) must route their own secret-equality checks through `subtle::ConstantTimeEq` in `emerald-rt` rather than Rust's ordinary `==`/`PartialEq`, and why Emerald's own `String`/`Array` `==` operators must never be used for this purpose from Emerald source either."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-securecompare-module-scaffold
    content: "Add `SecureCompare` as a compiler-known intrinsic namespace (plan 45's `File`-style dispatch shape). Add `subtle = \"2.6\"` to `crates/emerald-rt/Cargo.toml`, entered into plan 95's `DEPENDENCIES.md` ledger noting its real, verified-this-session fact: zero runtime dependencies (`docs.rs`'s own dependency listing shows only a `dev`-only `rand 0.8`), making it one of the lowest-risk additions to the ledger of any crate in this batch."
    status: pending
  - id: leaf-eq-entrypoint
    content: "`SecureCompare.eq(a: String, b: String): Boolean` via `#[no_mangle] extern \"C\" fn emerald_rt_secure_compare(a: *const c_char, b: *const c_char) -> i64`, converting both `CStr`s to `&[u8]` and calling `a.as_bytes().ct_eq(b.as_bytes())` (`subtle::ConstantTimeEq`'s slice impl), converting the resulting `Choice` via `.into()` to a plain `bool` then `i64` (`0`/`1`) at the boundary, `catch_unwind`-wrapped per plan 91's convention. Document explicitly, in both the doc comment and this plan's Decision log, that differing input lengths still short-circuit in non-constant time — see Decision log for why this is accepted, not a gap."
    status: pending
  - id: leaf-stdlib-internal-routing
    content: "Audit and amend (via a follow-up note in this plan, not a code change this plan performs directly, since plans 109/112/114 are authored in parallel this same session): every `emerald-rt` function in plans 112 (password verification), 109/110 (MAC/hash equality), and 114 (JWT signature verification) that compares secret-derived byte sequences must use `subtle::ConstantTimeEq` (either directly, or by inheriting a crate's own internal use of it/an equivalent RustCrypto-ecosystem constant-time primitive) — recorded here as the cross-cutting requirement those plans' own Decision logs should cite back to this plan number."
    status: pending
  - id: leaf-rust-tests-and-example
    content: "`#[test]` in `emerald-rt`: (1) `emerald_rt_secure_compare` on two identical strings returns `true`; (2) on two different strings of equal length returns `false`; (3) on two different strings of different length returns `false` (exercising the short-circuit path explicitly, so it's asserted as intentional behavior, not accidentally untested). Add `examples/secure_compare_proof.em` to `examples/`, wired into `emerald-cli/tests/examples.rs`'s checked table, matching the Concrete Proof below."
    status: pending
isProject: false
---

# Plan 117 — Constant-Time Comparison

A normal string or byte comparison — Rust's `==`, C's `strcmp`, or
Emerald's own `String` equality (plan 67's string-equality codegen) — is
built to be fast, and "fast" here means "returns as soon as it finds a
difference." Comparing two 32-byte digests byte-by-byte with an early-exit
comparison takes measurably less time when the first byte differs than
when the first 31 bytes match and only the 32nd differs — a timing
difference on the order of nanoseconds per byte position, far below what
a human notices, but not below what a program making thousands or
millions of remote timing measurements and averaging out network jitter
can recover. This is a real, practically-exploited attack class (timing
attacks against MAC/signature verification, HMAC comparison, and API-key
checks have real CVEs and real public write-ups going back over a decade)
— an attacker who can get a server to compare their guessed byte against
a secret digest, one byte at a time, and observe how long each comparison
takes, can recover the secret byte-by-byte far faster than brute force,
turning an exponential search into a linear one. `subtle::ConstantTimeEq`
exists to close exactly that channel: its comparison takes the same time
regardless of *where*, or whether, the inputs differ.

`subtle` — verified this session via `lib.rs` and `docs.rs`, current
stable release `2.6.1` (unchanged since 2024-06-24, a sign of a small,
finished, deliberately minimal API surface rather than neglect — its own
README states MSRV changes only accompany a minor version bump, and none
has been needed), authored by isis agora lovecruft and Henry de Valence
for the `dalek-cryptography` project (the same project behind
`curve25519-dalek`/`ed25519-dalek`, which plan 111 depends on), BSD-3-
Clause licensed, 58M downloads/month, used directly by 2,026 crates. It
has **zero runtime dependencies** — verified against its own `docs.rs`
dependency listing, which shows nothing but a `dev`-only `rand 0.8` — the
smallest possible addition to plan 95's `DEPENDENCIES.md` ledger of any
crate in this entire batch.

## Concrete proof this plan targets

```ruby
secret: String = "s3cr3t-api-key-do-not-leak"
same_value: String = "s3cr3t-api-key-do-not-leak"
different_value: String = "s3cr3t-api-key-do-not-leek"
short_value: String = "too-short"

puts SecureCompare.eq(secret, same_value)
puts SecureCompare.eq(secret, different_value)
puts SecureCompare.eq(secret, short_value)
```

Expected output: `true`, `false`, `false` — `same_value` matches byte for
byte; `different_value` differs in its last two characters only (the
comparison itself takes the same time regardless of where in the string
that difference falls, though this cannot be observed from `puts` output
alone — the Rust-side `#[test]`s below assert the *return value* is
correct, which is what's checkable at this level; the constant-time
*property* is a property of `subtle`'s own implementation, verified by
its own upstream test suite and design, not re-verified by this plan's
tests); `short_value` differs in length and is correctly rejected via the
short-circuit path described below.

## Decision log

- **`subtle`'s slice comparison genuinely short-circuits on length
  mismatch — a real, disclosed exception to "constant time," not a bug
  this plan works around.** Verified directly against the crate's own
  `docs.rs` page for `ConstantTimeEq`'s `[T]` impl: "This function
  short-circuits if the lengths of the input slices are different.
  Otherwise, it should execute in time independent of the slice
  contents." This is accepted, not patched around, for a concrete reason:
  in essentially every real use case this plan's callers have (a MAC tag,
  a password hash, a signature, a fixed-width token), the expected length
  is already public — an HMAC-SHA256 tag is always 32 bytes, a stored
  Argon2 PHC string's length varies only with its own public parameters,
  not with the secret it encodes. The information a length-mismatch
  short-circuit leaks (the actual input's length differs from the
  expected one) is not, in these use cases, itself the secret; content is
  the secret, and content-independence is exactly what `subtle` still
  guarantees once lengths match. A caller in a genuinely different
  situation, where length itself must stay hidden, would need to pad
  both inputs to a fixed size before calling `SecureCompare.eq` — a
  caller-side responsibility this plan's doc comment states explicitly
  rather than leaving as a silent gap.
- **`SecureCompare.eq` takes two Emerald `String`s, not two raw byte
  buffers — a deliberate simplification that costs nothing in this
  plan's actual use cases.** Plan 92's binary-safe `(ptr, len)` buffer
  convention exists for raw bytes that may not be valid UTF-8 or may
  contain embedded NULs; every real caller of `SecureCompare.eq` in this
  batch (a password hash's PHC string, a hex- or base64url-encoded MAC
  tag, a JWT's base64url signature segment, an API token from plan 113's
  `Random.secure_hex`) is already, by construction, ASCII text safely
  representable as an Emerald `String` — plan 59's finding that `String`
  is a bare null-terminated `char*` with zero marshaling cost applies
  directly, and there is no raw-bytes use case in this batch's six plans
  that needs a `(ptr, len)`-based `SecureCompare` variant. Should a
  future plan need to compare genuinely raw, non-text secret buffers,
  extending `SecureCompare.eq` with a `(ptr, len)`-based overload per
  plan 92's convention is a small, additive change, not a redesign — not
  attempted here since nothing in this batch needs it yet.
- **This plan's central, cross-cutting requirement is a project rule
  stated in prose, not (yet) a static check the compiler enforces — and
  that gap is named explicitly rather than glossed over.** Plan 112's
  password verification, plan 109/110's MAC/hash-equality checks, and
  plan 114's JWT signature verification must all route their own
  secret-comparison logic through `subtle::ConstantTimeEq` (or inherit it
  transitively from a RustCrypto-ecosystem crate that already does, as
  plan 112's Decision log documents `argon2`/`password-hash` doing
  internally) rather than Rust's `==` or Emerald's own `String`/`Array`
  equality operators (plan 67's codegen) reaching a secret value. Nothing
  in `emerald-sema` or `emerald-codegen` today can detect "this string
  holds a secret" as a type-level fact — Emerald's type system has no
  secret/tainted-value tracking (a real, larger feature no plan in this
  batch adds) — so this rule is enforced by code review and this plan's
  own explicit statement, the same posture plan 91 took toward its own
  `catch_unwind`-at-every-boundary convention before plan 92 generalized
  it. A future static-analysis pass flagging `==` on a value that flows
  from `Password.hash`/`Random.secure_*`/an HMAC output is a real,
  valuable follow-up this plan does not attempt to build.
- **Emerald's own `==` on `String`/`Array` is not modified, deprecated,
  or warned against for its ordinary, non-secret uses — this plan adds a
  parallel, narrowly-scoped function rather than changing existing
  semantics.** Plan 67's string-equality codegen exists for entirely
  legitimate, non-secret comparisons (`if name == "admin"`, `if status ==
  "done"`) that have no timing-attack surface worth defending, since
  nothing about those comparisons is meant to be secret in the first
  place; adding constant-time overhead to every `String` `==` in Emerald
  would be a real, measurable performance regression for zero security
  benefit in the overwhelming majority of comparisons. `SecureCompare.eq`
  is deliberately a separate, explicitly-named function precisely so a
  reader can tell, at the call site, that a comparison is
  security-sensitive — the same "visible at the point of use, not buried
  in a general mechanism" posture plan 59 used for `CString`/`String.
  from_cstring` at every FFI trust boundary.
- **`Choice`, `subtle`'s own boolean-like return type, is converted to a
  plain `Boolean` at the FFI boundary — deliberately, even though this
  reintroduces an ordinary (non-constant-time) branch one level up.**
  `subtle::Choice` is designed to be used *without* branching on it inside
  a larger constant-time computation (via `Choice`'s own
  `conditional_select`/bitwise composition operators) — but this plan's
  `SecureCompare.eq` is a standalone comparison returning directly to
  Emerald caller code that will inevitably branch on the result (`if
  SecureCompare.eq(a, b) then ... end`) regardless of what representation
  crosses the FFI boundary. Converting `Choice` to a `bool`/`i64` at the
  return-value boundary loses no protection this design was already
  relying on — the comparison itself, the part with genuine timing risk,
  has already completed in constant time before the conversion happens;
  branching on the final yes/no answer is unavoidable and not the
  channel `subtle` protects.
- **No `SecureCompareBytes` distinct from `SecureCompare.eq` for numeric
  or `Boolean` comparisons.** `subtle::ConstantTimeEq` is implemented for
  every Rust integer width, and a hypothetical `SecureCompare.int_eq`
  wrapping it is possible, but no plan in this batch (or any Emerald use
  case this session identified) compares a secret `Int64` for equality —
  every secret this batch handles (passwords, keys, tokens, MAC tags,
  signatures) is fundamentally a byte sequence represented as `String`.
  Declined as unneeded scope, not as a design rejection — trivially
  addable later if a real use case appears.
- **Out of scope.** No `subtle::ConditionallySelectable`/`CtOption`-based
  constant-time branching primitives beyond equality — `subtle` offers a
  broader toolkit for building entire constant-time algorithms (not just
  comparing two finished values), which this plan's single `SecureCompare.
  eq` function doesn't need and doesn't expose; a future plan implementing
  a constant-time algorithm from scratch in `emerald-rt` (rather than
  relying on a vetted crate that already handles its own timing safety,
  per plan 95's vetting policy) could reach for the fuller `subtle` API
  directly in Rust without any new Emerald-visible surface. No compiler-
  enforced secret-tracking/taint system forcing `==` on sensitive values
  to be a type error — named above as a real, valuable, larger follow-up
  this plan does not build.
