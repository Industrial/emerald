---
name: String Equality Codegen
overview: "String == String and String != String typecheck under emerald-sema (plan 45's own stdlib work implies String is a first-class comparable value) but codegen's comparison lowering has no (Str, Str) arm — it falls through to the catch-all at crates/emerald-codegen/src/lib.rs:5243, `codegen: comparison operands must both be Int64 or both Float64`, and a valid program fails to compile despite sema having already accepted it. This plan adds real string-content comparison (byte-for-byte, not pointer identity) to codegen, mirroring the (Symbol, Symbol) and (Nil-adjacent) arms already sitting right next to the gap."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-runtime-strcmp-intrinsic
    content: "Add (or confirm/reuse, if one already exists under a different name) a runtime comparison primitive in runtime/emerald_runtime.c that compares two Emerald String values by content — byte-for-byte over their known length, not by pointer identity and not by relying on a NUL terminator alone, consistent with how emerald_alloc'd strings are otherwise handled elsewhere in the runtime (check for an existing length-prefixed String representation before assuming C-string semantics are correct here)."
    status: pending
  - id: leaf-codegen-str-str-arm
    content: "Add a (ValKind::Str, ValKind::Str) arm to the comparison match in crates/emerald-codegen/src/lib.rs (immediately before the catch-all at line 5243, alongside the existing (ValKind::Symbol, ValKind::Symbol) arm at line 5200 as the pattern to follow), restricted to CompareOp::Eq/Ne exactly like Symbol's own arm — ordering comparisons (<, <=, >, >=) on String are out of scope, see below — lowering to a call into the new runtime primitive and an integer-zero/nonzero-to-Bool conversion."
    status: pending
  - id: leaf-regression-test
    content: "A compile_link_run test asserting both a true and a false case for == and != on two String values built two different ways (a literal vs. a literal, and a literal vs. a computed/concatenated String) so the fix isn't proven only for the specific shape that happened to be tried first."
    status: pending
isProject: false
---

# Plan 67 — String Equality Codegen

`examples/README.md`'s bug list states this precisely: "sema accepts it,
then codegen rejects it with 'comparison operands must both be Int64 or
both Float64.'" `nullable_safe_nav.em` avoids comparing narrowed `String`
locals for equality specifically because of this gap. Verified directly
this session against `crates/emerald-codegen/src/lib.rs`: the comparison
match's catch-all arm sits at line 5243; the two arms immediately above it
handle `(Symbol, Symbol)` (lines 5200-5209, real int-compare on interned
symbol IDs) and a `Nullable`-vs-`nil` null-pointer test (lines 5223-5242);
no `(Str, Str)` arm exists anywhere in the match.

## Concrete proof this plan targets

```ruby
a: String = "hello"
b: String = "hello"
c: String = "world"

puts a == b   # 1  (true)
puts a == c   # 0  (false)
puts a != c   # 1  (true)

d: String = "hel" + "lo"
puts a == d   # 1  (true — content equality, not pointer identity)
```

Expected output: `1`, `0`, `1`, `1`. Today, this program fails to
compile.

## Decision log

- **Content equality, not pointer identity.** Two separately-allocated
  `String` values with identical bytes must compare equal — `d` above is
  built by concatenation (a fresh `emerald_alloc`) and must still equal
  `a`. A naive pointer-equality lowering (reusing whatever
  `build_is_null`-style pointer comparison plan 43 already added for
  `Nullable`-vs-`nil`) would be a fast but wrong fix — it would pass a
  simplistic test and fail the real case the language needs equality
  for. This plan requires the regression test to include the
  concatenation case specifically so a pointer-identity shortcut cannot
  quietly pass review.
- **`==`/`!=` only, not `<`/`<=`/`>`/`>=` — out of scope, named
  explicitly.** `spec/GRAMMAR.md` marks `Regexp` literals `UNDECIDED`
  and there is no documented lexicographic-ordering semantics for
  `String` anywhere in `TYPE_SYSTEM.md`/`SEMANTICS.md`; adding ordering
  comparisons here would be inventing language semantics inside a
  bug-fix plan rather than deciding them deliberately. The `(Symbol,
  Symbol)` arm this plan mirrors makes the identical restriction
  (line 5210-5214 explicitly errors on any op besides `==`/`!=`) — same
  posture, not a new one.
- **Runtime primitive, not inline byte-loop codegen.** Every other
  cross-string operation (concatenation, `.upcase`, `.length`, etc.) is
  implemented as a `runtime/emerald_runtime.c` C function called from
  codegen, not hand-emitted LLVM IR — following that existing precedent
  keeps this fix consistent with the rest of the `String` intrinsic
  surface rather than introducing a one-off inline-IR code path for
  comparison alone.
