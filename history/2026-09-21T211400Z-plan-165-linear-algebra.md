2026-09-21T21:14:00Z

---
name: Linear Algebra — `nalgebra`-Backed Fixed-Size Vectors and Matrices
overview: "STRETCH-TIER — flagged plainly, not oversold. Fixed-size `Vector2`/`Vector3`/`Vector4` (compiler-synthesized classes with 2/3/4 named `Float64` fields — no heap allocation, packed exactly the way plan 163's `Decimal` is) and `Matrix3`/`Matrix4` (backed by the `(ptr: *const u8, len: i64)` binary-safe-buffer convention from plan 92, a flat row-major `Float64` buffer, since naming nine or sixteen individual fields is unreasonable), wrapping `nalgebra` (verified active on crates.io as of 2026-05-24, v0.33+). Covers vector add/sub/dot/cross/normalize and matrix-vector multiply for the fixed 3x3/4x4 case common in graphics/physics transforms — needed for scientific/graphics/ML-adjacent use cases, not a general-purpose stdlib essential most Emerald programs will ever touch. No dynamically-sized matrices, no decompositions (SVD/eigen/LU), no generic `N`-dimensional API in v1."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-vendor-nalgebra-and-scaffold-module
    content: "Add `nalgebra = \"0.33\"` to `crates/emerald-rt/Cargo.toml` (verified this session: crates.io last published 2026-05-24; description \"General-purpose linear algebra library with transformations and statically-sized or dynamically-sized matrices\" — this plan uses ONLY the statically-sized half, see Decision log). Create `crates/emerald-rt/src/linalg.rs`, `mod linalg;` from `lib.rs`."
    status: pending
  - id: leaf-fixed-vector-classes
    content: "Register three compiler-synthesized classes, `Vector2{x,y: Float64}`, `Vector3{x,y,z: Float64}`, `Vector4{x,y,z,w: Float64}` — plain named-field classes, zero heap allocation, the same packed-field shape plan 163's `Decimal` and plan 167's `Point` use, chosen over an opaque `nalgebra::Vector3<f64>` handle specifically because a 3-or-4-`Float64` value is exactly as cheap to pass by value through named fields as through a heap pointer, with none of the leak/lifetime cost a handle would add. Add `.add`/`.sub`/`.dot`/`.length` for all three and `.cross` for `Vector3` only (cross product is only defined in 3 and 7 dimensions; `Vector2`/`Vector4` do not get a `.cross` method at all, a real, disclosed mathematical restriction, not an oversight), each decoding into a real `nalgebra::Vector3<f64>` (etc.), calling the matching `nalgebra` operator, and re-encoding the packed result."
    status: pending
  - id: leaf-fixed-matrix-flat-buffer-convention
    content: "Add `Matrix3.from_flat(xs: Array[Float64], n: Int64): Result[Matrix3, String]` and `Matrix4.from_flat(...)` — taking the disclosed explicit-count-parameter shape plan 45's `.split_count` already established for every `Array[T]`-consuming intrinsic in this codebase (`Array[T]` itself carries no runtime length, so the count must travel alongside), `Err` if `n != 9`/`n != 16`. Internally, `Matrix3`/`Matrix4` are represented as an OPAQUE handle (`handle: Int64`, boxed `nalgebra::Matrix3<f64>`/`Matrix4<f64>`) — NOT named fields like the vectors above, since spelling out nine or sixteen individually-named `Float64` fields on a compiler-synthesized class is unreasonable API surface for this plan to hand-maintain; see Decision log for why this is a genuinely different call than the vector classes' packed-field choice, made in the same plan, for a concrete, stated reason."
    status: pending
  - id: leaf-matrix-vector-multiply
    content: "Add `Matrix3.multiply_vector3(self, v: Vector3): Vector3` / `Matrix4.multiply_vector4(self, v: Vector4): Vector4` — the one operation this plan's own worked proof exercises: reconstruct the boxed matrix, call `nalgebra`'s own `Matrix3::mul` against a `nalgebra::Vector3` built from the packed vector's three fields, decode the result back into a fresh `Vector3` class instance. This is deliberately the ONLY matrix operation this plan ships — no matrix-matrix multiply, no inverse, no transpose — see Decision log for the explicit v1 scope line."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/linear_algebra_proof.em` (this plan's Concrete Proof below) to `emerald-cli/tests/examples.rs`'s CI-checked table. Add `#[test]`s in `crates/emerald-rt/src/linalg.rs`: `Vector3.cross` against a hand-computable identity (`(1,0,0) x (0,1,0) = (0,0,1)`); `Vector3.dot`/`.length` against hand-computed values; `Matrix3.from_flat` with `n != 9` asserted against the `Err` path; and an identity-matrix multiply (`I * v == v`) proving the flat-buffer decode and the matrix-vector multiply agree with `nalgebra`'s own semantics, not a custom reimplementation. Run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 165 — Linear Algebra (Stretch Tier)

**This plan is stretch-tier, and is labeled that way in its own
overview rather than dressed up as a stdlib essential.** Most Emerald
programs — string processing, file I/O, HTTP services, CLI tools — will
never call a single function this plan adds. The real audience is
graphics/physics/scientific/ML-adjacent code, a genuine but narrow
slice of what Emerald programs will realistically be written to do. It
is written at real depth because a stretch-tier plan still deserves a
concrete, defensible design — not because it is claimed to matter as
much as plan 160's date/time gap or plan 163's decimal-precision gap,
both of which affect ordinary, everyday programs in a way this plan's
own audience does not represent.

`nalgebra` is the obvious, uncontested choice here: verified this
session, crates.io shows it last published 2026-05-24 at v0.33+,
described plainly as a "general-purpose linear algebra library with
transformations and statically-sized or dynamically-sized matrices" —
the dominant, mature Rust linear-algebra crate, with no live rivalry
this session's research surfaced the way `chrono`/`jiff` had one. The
scope decision this plan actually has to make is not "which crate" but
"how much of nalgebra's own enormous surface to expose in v1" — and the
answer, stated plainly, is: fixed-size vectors up to 4 dimensions,
fixed 3x3/4x4 matrices, and exactly one matrix operation
(matrix-vector multiply). Nothing else.

## Concrete proof this plan targets

```ruby
a: Vector3 = Vector3.new(1.0, 0.0, 0.0)
b: Vector3 = Vector3.new(0.0, 1.0, 0.0)
cross: Vector3 = a.cross(b)
puts cross.x
puts cross.y
puts cross.z

identity_flat: Array[Float64] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
identity_result: Result[Matrix3, String] = Matrix3.from_flat(identity_flat, 9)
case identity_result
when Ok(identity)
  transformed: Vector3 = identity.multiply_vector3(a)
  puts transformed.x
  puts transformed.y
  puts transformed.z
when Err(e)
  puts e
end

bad_result: Result[Matrix3, String] = Matrix3.from_flat(identity_flat, 4)
case bad_result
when Ok(m)
  puts "unexpected"
when Err(e)
  puts e
end
```

Expected output, in order: `0`, `0`, `1` (the real cross product
`(1,0,0) x (0,1,0) = (0,0,1)` — the standard right-hand-rule identity,
computed by `nalgebra`'s own cross-product implementation, not
hand-rolled here); `1`, `0`, `0` (the identity matrix applied to `a`
returns `a` unchanged, proving `Matrix3.from_flat`'s row-major decode
and `nalgebra`'s own matrix-vector multiply agree); and a real, disclosed
error string for the deliberately wrong element count (`4`, not `9`).

## Decision log

- **Flagged stretch-tier in the overview, not buried in a caveat —
  stated up front because the task's own instructions require it, and
  because pretending otherwise would misrepresent this plan's real
  priority relative to, say, plan 160's date/time gap or plan 163's
  decimal-precision gap.** A linear-algebra primitive is valuable for a
  real but narrow slice of Emerald programs; it is not a general
  stdlib essential the way `String`/`File`/`DateTime` are. This plan's
  own place in this batch's rough priority ordering reflects that
  honestly.
- **Fixed-size vectors get named `Float64` fields; fixed-size matrices
  get an opaque handle over a flat buffer — two different
  representations chosen in the SAME plan, for a concrete, stated
  reason, not an inconsistency.** A `Vector3` has exactly three
  components — naming them `x`/`y`/`z` as ordinary class fields (the
  same zero-heap-allocation packed-field pattern plan 163's `Decimal`
  and plan 167's `Point` already use) is entirely reasonable API
  surface. A `Matrix3` has nine components and a `Matrix4` has sixteen
  — naming sixteen individual fields (`m00`...`m33`) on a
  compiler-synthesized class is unreasonable to hand-maintain and
  unreasonable for a caller to construct field-by-field, so this plan
  instead takes the flat-buffer convention plan 92 already established
  for binary-safe data, paired with the disclosed explicit-count
  parameter plan 45's `.split_count` already established for
  `Array[T]`'s own missing length metadata — and stores the DECODED
  matrix behind an opaque `handle: Int64`, the same heap-handle shape
  plan 161's `CronSchedule` and plan 163's `BigInt` use, because
  `nalgebra::Matrix3<f64>` genuinely is copy-cheap in principle (it's
  `Copy` in `nalgebra`'s own implementation) but re-encoding sixteen
  floats back into sixteen named fields on every single operation is
  needless ceremony this plan declines to add when a handle serves
  just as well and is simpler to implement uniformly with the matrix
  operations this plan actually ships.
- **Exactly one matrix operation ships in v1: matrix-vector multiply —
  no matrix-matrix multiply, no inverse, no transpose, no
  determinant.** This is the single operation this plan's own worked
  proof needs to demonstrate the flat-buffer decode is correct
  end-to-end (an identity-matrix multiply is the simplest possible
  check that composes decode-then-operate-then-encode correctly), and
  it is also the single most common operation a graphics/physics
  caller actually reaches for (applying a transform to a point).
  Every additional operation `nalgebra` offers is real, available, and
  deliberately deferred — this plan's own job is proving the
  representation choices work, not exhausting `nalgebra`'s API surface.
- **No dynamically-sized matrices (`nalgebra::DMatrix`) at all.**
  `nalgebra`'s own crates.io description explicitly advertises both
  "statically-sized" and "dynamically-sized" matrices — this plan takes
  only the static half. A dynamically-sized matrix has the identical
  representation problem `Array[T]` itself already has (no fixed field
  count, no natural opaque-handle boundary that doesn't also need a
  disclosed dimension pair threaded through every call) and is a
  meaningfully larger design this stretch-tier plan does not attempt to
  fold in as a side effect of proving the fixed-size case works.
- **No decompositions (SVD, eigenvalue, LU, Cholesky) — real,
  substantial `nalgebra` features this plan declines outright, not
  partially attempts.** These are the operations that make `nalgebra`
  genuinely valuable for serious scientific computing, and also the
  operations with the most numerically delicate edge cases (near-
  singular matrices, convergence behavior, condition numbers) — a
  correct, trustworthy wrapping of even one of these would be a
  substantial plan on its own, not a leaf inside this already-narrow
  stretch-tier plan. Deferred entirely, not attempted at reduced
  fidelity.
- **`Vector2`/`Vector4` get no `.cross` method — a real mathematical
  restriction stated explicitly, not a missing feature.** The cross
  product is only defined (in the usual sense) in 3 and 7 dimensions;
  `nalgebra` itself does not offer `.cross` on its own `Vector2`/
  `Vector4` types either. This plan mirrors that restriction exactly
  rather than inventing a 2D "perpendicular vector" pseudo-cross-product
  some libraries offer under the same method name — a real, disclosed
  choice to match the underlying crate's own semantics rather than
  Emerald-specific convenience.
- **Out of scope.** No dynamically-sized matrices (above); no
  decompositions (above); no `Vector5`+/arbitrary-dimension vectors; no
  matrix-matrix multiply, inverse, transpose, or determinant; no
  quaternion/rotation-specific types (`nalgebra::UnitQuaternion` and
  similar — a real, separate, graphics-specific follow-up); no SIMD
  acceleration beyond whatever `nalgebra` does internally by default; no
  integration with plan 163's `Decimal`/`BigInt` (this plan's vectors
  and matrices are `Float64`-only, matching `nalgebra`'s own primary,
  best-supported scalar type).
