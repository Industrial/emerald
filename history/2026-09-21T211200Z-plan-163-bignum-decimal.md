2026-09-21T21:12:00Z

---
name: Arbitrary-Precision Integers & Decimals — `num-bigint` and `rust_decimal`
overview: "Two new compiler-synthesized classes with deliberately DIFFERENT representations, chosen per-case rather than by one blanket rule: `BigInt` (`num-bigint`, verified active on crates.io as of 2026-07-04) is a one-field opaque heap handle (`handle: Int64`, a boxed pointer — genuinely unbounded storage cannot fit in any fixed number of scalar fields), while `Decimal` (`rust_decimal`, verified active — `rust_decimal_macros` published 2026-01-13, still the standard pure-Rust fixed-point financial-decimal crate) is a two-field, zero-heap-allocation class (`lo: Int64`, `hi: Int64`) packing `rust_decimal::Decimal`'s own documented 16-byte `serialize()`/`deserialize()` wire format exactly. This plan exists because `Float64` silently loses precision on ordinary decimal arithmetic — `0.1 + 0.2` in IEEE 754 double precision is `0.30000000000000004`, not `0.3` — which is categorically unacceptable for money, and because some integers (factorials, cryptographic moduli, combinatorics) genuinely exceed `Int64`'s 64-bit range."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vendor-crates-and-scaffold-modules
    content: "Add `num-bigint = \"0.4\"` and `rust_decimal = \"1\"` to `crates/emerald-rt/Cargo.toml` (both verified this session: `num-bigint` crates.io last published 2026-07-04; `rust_decimal_macros` — the companion crate to the core `rust_decimal`, tracking its version — last published 2026-01-13; both pure-Rust, no C dependency, satisfying plan 95's pure-Rust-first posture directly). Create `crates/emerald-rt/src/bignum.rs` and `crates/emerald-rt/src/decimal.rs`, `mod bignum; mod decimal;` from `lib.rs`."
    status: pending
  - id: leaf-bigint-opaque-handle-and-arithmetic
    content: "Register a compiler-synthesized `BigInt` class, one field (`handle: Int64`), the same opaque-handle shape plan 161's `CronSchedule` uses, necessary here because `num_bigint::BigInt`'s internal representation is a heap-allocated, variable-length `Vec<u32>` of digits plus a sign — genuinely unbounded, no fixed field count could ever hold it. Add `BigInt.from_i64(n: Int64): BigInt` and `BigInt.from_s(s: String): Result[BigInt, String]` (wrapping `num_bigint::BigInt::parse_bytes`, `Err` on a malformed digit string), `.to_s(self): String` (`Display`, exact, arbitrary-length decimal text — the ONLY lossless way a `BigInt` value can ever reach Emerald's other types, since `Int64` cannot hold an arbitrary-precision value and this plan adds no other conversion), and `.add(self, other: BigInt): BigInt` / `.mul(self, other: BigInt): BigInt` (each allocating a new boxed result — see Decision log for why these leak by the same accepted rule plan 161's handles do)."
    status: pending
  - id: leaf-bigint-worked-precision-proof
    content: "Add `BigInt.factorial(n: Int64): BigInt` (an internal, iterative — not recursive — accumulator loop calling `num_bigint::BigInt`'s own `Mul` impl repeatedly; `n` bounds-checked to a disclosed sane maximum, e.g. reject `n > 10000` via `NativeError`, since an unbounded factorial call is an unbounded-memory footgun this plan does not pretend to protect against beyond one sanity bound) specifically so this plan's own Concrete Proof can compute a value (`20!` = `2432902008176640000`, which fits in `Int64`, versus `25!`, which does not) that makes the actual `Int64`-overflow boundary visible and checkable, not merely asserted in prose."
    status: pending
  - id: leaf-decimal-packed-representation
    content: "Register a compiler-synthesized `Decimal` class, two fields (`lo: Int64`, `hi: Int64`) — NOT an opaque handle. Every native call reconstructs a real `rust_decimal::Decimal` via `Decimal::deserialize([u8; 16])` (the crate's own documented 16-byte wire format — verified this session against `docs.rs/rust_decimal`'s own `Decimal::deserialize` doc: \"the deserialized byte representation must be 16 bytes and adhere to the following convention: bytes 1-4: ...\") by reinterpreting the two `i64` fields as 16 raw bytes, and every construction re-serializes the result back into the same two fields via `.serialize()` before returning. `Decimal` values are therefore `Copy`-cheap, heap-allocation-free, and never leak — a genuinely different, better answer than `BigInt`'s handle, made possible specifically because `rust_decimal::Decimal` (unlike `num_bigint::BigInt`) is a fixed, small, known-width value type."
    status: pending
  - id: leaf-decimal-arithmetic-and-string-io
    content: "Add `Decimal.from_s(s: String): Result[Decimal, String]` (`rust_decimal::Decimal::from_str_exact`, rejecting silent precision loss — `rust_decimal`'s own alternate, lossier `from_str` is deliberately not used here, see Decision log), `.to_s(self): String` (exact `Display`, no exponential notation, no rounding — the classic `0.1 + 0.2` proof in this plan's own Concrete Proof depends on this being exact), and `.add(self, other: Decimal): Decimal` / `.sub`/`.mul`/`.div` (each decode-compute-reencode through the two-field packing above, propagating `rust_decimal`'s own real overflow/divide-by-zero errors as `Result[Decimal, String]` where the underlying crate itself returns a checked-arithmetic `Option`, e.g. `.checked_div`)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/bignum_decimal_proof.em` (this plan's Concrete Proof below) to `emerald-cli/tests/examples.rs`'s CI-checked table. Add `#[test]`s in both new modules: `BigInt.factorial(25).to_s()` against the hand-computable exact value (`15511210043330985984000000`); `Decimal.from_s(\"0.1\").add(Decimal.from_s(\"0.2\")).to_s()` asserted to equal the literal string `\"0.3\"` exactly (the direct, executed refutation of `Float64`'s `0.30000000000000004`); a `Decimal` round-trip through the 16-byte pack/unpack proving no bit is lost; and a `BigInt.from_s` malformed-input `Err` case. Run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 163 — Arbitrary-Precision Integers & Decimals

Two genuinely different numeric problems share this plan only because
they're both cases of "`Float64`/`Int64` isn't enough," not because they
share one representation — and the fact that this plan's own two new
classes end up representationally opposite (`BigInt` a heap handle,
`Decimal` two packed scalar fields) is itself the concrete, worked
answer to this batch's recurring representation question, not a
coincidence this plan glosses over.

**Why `Decimal` exists at all — stated concretely, not asserted.**
IEEE 754 double-precision `Float64` cannot exactly represent most
decimal fractions, because decimal fractions like `0.1` and `0.2` are
not finite in binary. The consequence is not theoretical:
`0.1 + 0.2 == 0.30000000000000004` in `Float64` arithmetic, on every
platform this compiler targets, every time. For ordinary scientific or
graphics work, that sixteenth-decimal-digit error is invisible and
irrelevant — nalgebra (plan 165) and statrs (plan 166) both use `Float64`
throughout without a second thought. For money, it is not tolerable: a
financial ledger, an invoice total, a currency conversion computed in
`Float64` accumulates exactly this kind of error across enough
operations to produce a genuinely wrong total, in a domain where "off by
a fraction of a cent, sometimes" is a real, auditable, unacceptable bug
class. `rust_decimal::Decimal` represents a value as an exact integer
mantissa plus a base-10 scale — `0.1` is stored as the exact pair
`(1, scale=1)`, not as an infinite binary fraction truncated to 52 bits
— so `0.1 + 0.2` in `Decimal` arithmetic is exactly `0.3`, not an
approximation of it. This plan's own Concrete Proof computes this
exact comparison and asserts the exact string `"0.3"`, not a
documentation claim.

**Why `BigInt` exists at all.** `Int64`'s range is roughly ±9.2×10^18.
`20!` (`2432902008176640000`) fits; `21!` (`51090942171709440000`)
already overflows a signed 64-bit integer. Combinatorics, cryptographic
modular arithmetic, and any exact large-integer computation a real
program might want (this batch's own factorial worked proof is a
deliberately simple, checkable instance of the category) need a value
that can grow without a fixed bound — which is precisely the property
`Int64` cannot offer by construction, and precisely the property
`num_bigint::BigInt`'s own heap-allocated digit vector provides.

## Concrete proof this plan targets

```ruby
twenty_factorial: BigInt = BigInt.factorial(20)
puts twenty_factorial.to_s

twenty_five_factorial: BigInt = BigInt.factorial(25)
puts twenty_five_factorial.to_s

bad_bigint: Result[BigInt, String] = BigInt.from_s("not a number")
case bad_bigint
when Ok(n)
  puts n.to_s
when Err(e)
  puts e
end

point_one: Decimal = Decimal.from_s("0.1")!
point_two: Decimal = Decimal.from_s("0.2")!
sum: Decimal = point_one.add(point_two)
puts sum.to_s

float_sum: Float64 = 0.1 + 0.2
puts float_sum
```

Expected output, in order: `2432902008176640000` (`20!`, which — not
coincidentally — is exactly what `Int64` arithmetic would also produce,
since it still fits; the proof this plan's own `BigInt` is doing real
work is the NEXT line); `15511210043330985984000000` (`25!`, a
27-digit value that `Int64` arithmetic cannot represent at all — this is
the line that actually proves arbitrary precision, not the first);
`num-bigint`'s own real parse-error text for the malformed digit string;
`0.3` (`Decimal` arithmetic, exact); and `0.30000000000000004`
(`Float64` arithmetic, the classic, real IEEE 754 rounding artifact,
printed deliberately alongside the `Decimal` result so the contrast is
executed and visible in the same program's own output, not merely
described in this plan's prose).

## Decision log

- **`BigInt` and `Decimal` get deliberately different representations —
  the type-representation question this batch keeps asking is answered
  per-case here, with the reasoning shown, not by one rule applied
  twice.** `num_bigint::BigInt` is fundamentally unbounded — its own
  internal `Vec<u32>` digit storage can be arbitrarily long, so no
  fixed number of `Int64` fields could ever hold every possible value;
  an opaque boxed-pointer handle (the same one-field `handle: Int64`
  shape plan 161's `CronSchedule` already established for a different,
  also-heap-shaped value) is the only representation that actually
  fits. `rust_decimal::Decimal`, by contrast, is fixed at exactly 16
  bytes — `docs.rs/rust_decimal`'s own page states this explicitly
  ("Decimal's alignment to 16 bytes (128 bits)") and documents a real,
  stable `serialize()`/`deserialize([u8; 16])` byte-for-byte contract.
  Two plain `Int64` fields (16 bytes total) hold that exactly, with
  zero heap allocation, zero pointer indirection, and — unlike `BigInt`
  — zero leak on every arithmetic operation. Stating both reasons next
  to each other, for two crates chosen in the same plan, is the
  strongest way to show this was reasoned per-case rather than answered
  once by reflex.
- **`Decimal.from_s` uses `from_str_exact`, not `rust_decimal`'s own
  looser `from_str` — a deliberate, disclosed choice against the
  crate's own default-feeling entry point.** `rust_decimal` itself
  offers both: `from_str` accepts scientific notation and silently
  rounds beyond the type's maximum representable scale, while `from_
  str_exact` rejects anything it cannot represent exactly. For a plan
  whose entire justification is "financial arithmetic must not
  silently lose precision," accepting the looser parser and inheriting
  its silent-rounding behavior would directly undercut the plan's own
  stated purpose — this is not a hypothetical risk avoided out of
  caution, it is the literal failure mode (silent precision loss) this
  plan exists to prevent, so the stricter constructor is the only
  defensible default here.
- **`BigInt` arithmetic leaks, by the same accepted, disclosed rule
  plan 161's `CronSchedule` handles already leak by — not a new
  omission this plan introduces independently.** Every `.add`/`.mul`
  call allocates a fresh boxed `num_bigint::BigInt` and returns a new
  handle; the previous operand handles are never freed. This follows
  directly from Emerald's own existing, disclosed memory model (plan
  45's citation of `emerald_alloc`: "no free... no lifetime tracking"),
  the identical situation plan 161's Decision log names explicitly and
  defers to plan 93 (resource/handle lifetime model) rather than
  solving unilaterally here. A tight loop computing many large `BigInt`
  values will leak proportionally to iteration count — a real,
  practical limitation this plan states plainly rather than discovering
  silently in a future program's memory profile.
- **`BigInt.factorial`'s input is bounds-checked against a disclosed
  sanity ceiling, not left fully unbounded.** `num_bigint::BigInt`
  itself places no upper bound on magnitude — but an unbounded `n` in
  `BigInt.factorial(n)` is an unbounded-memory-allocation footgun with
  no natural backpressure (a caller passing `n = 10_000_000` would
  allocate gigabytes before this plan's own proof of arbitrary
  precision ever gets to run). This plan's own `#[test]`s exercise
  `25!`, deliberately small and exactly hand-verifiable; the disclosed
  `n > 10000` rejection (routed through `NativeError`, since this is a
  genuine misuse of the function's documented contract, not an
  anticipated `Result`-worthy input) exists so this plan's worked proof
  and any real caller both stay inside a bound this plan can actually
  reason about, not so this plan can claim unlimited scale it hasn't
  tested.
- **`.to_s()` is the only way a `BigInt` value ever becomes visible to
  the rest of Emerald's type system — there is no `.to_i64()`
  "best-effort truncate" escape hatch in v1.** A `BigInt` that
  genuinely exceeds `Int64`'s range has no honest `Int64`
  representation at all; adding a truncating or saturating conversion
  method would create exactly the kind of silent-precision-loss trap
  this plan's own `Decimal` half explicitly rejects (`from_str_exact`
  over `from_str`, above) — for consistency, and because no real use
  case in this plan's own worked proof needs it, this plan declines to
  add one. A future plan may add a checked `.to_i64(self): Result[Int64,
  String]` if a real need appears; it is not invented speculatively
  here.
- **Neither crate takes a `serde` dependency, and this plan does not
  add one either.** Both `num-bigint` and `rust_decimal` support
  optional `serde` integration behind feature flags this plan does not
  enable — no consuming leaf in this batch currently needs JSON/binary
  serialization of a `BigInt`/`Decimal` value, and enabling an unused
  feature would be exactly the kind of unjustified dependency-surface
  growth plan 95's vetting checklist exists to catch.
- **Out of scope.** No arbitrary-precision RATIONAL type
  (`num_bigint`'s sibling crate `num-rational`, `BigRational` — a real,
  separate follow-up this plan does not fold in); no arbitrary-precision
  FLOAT (`rust_decimal`'s own scale is capped at 28-29 significant
  digits by its fixed 96-bit mantissa, which is not the same thing as
  true arbitrary decimal precision — a `BigDecimal`-style unbounded
  crate is a distinct, larger dependency this plan declines); no
  `BigInt`/`Decimal` interop with `Float64` beyond what each already
  exposes for construction (no `Decimal.from_f64` in v1 — constructing
  a `Decimal` FROM an already-imprecise `Float64` would reintroduce the
  exact problem this plan exists to solve, so this plan requires
  string- or integer-based construction only); no handle-freeing
  mechanism for `BigInt` (plan 93, above); no modular-exponentiation or
  other cryptography-flavored `BigInt` operations beyond `.add`/`.mul`
  (a real cryptographic bignum use case wants `crypto-bigint`'s
  constant-time guarantees specifically, a materially different crate
  with a materially different threat model this plan does not attempt
  to also serve).
