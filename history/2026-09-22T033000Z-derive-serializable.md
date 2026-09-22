2026-09-22T03:30:00Z

---
name: "derive Serializable — to_json_value Synthesis From a Class's Own Fields"
overview: "The last originally-scoped Foundation-tier item from inception-3's own recommendation batch (alongside plan 118's JsonValue and the Iterable[T] investigation, plan 192): `class Point derive Serializable ... end` synthesizes `to_json_value(self): JsonValue`, the same synthesis-from-field-list mechanism plan 61's `derive Comparable` already established for `==`, reusing plan 118's real JsonValue enum as the synthesized method's own return type. Implemented this session with two real, disclosed scope corrections found only by trying to build the reverse direction, not assumed up front — see the Decision log."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-int64-to-f-float64-to-i
    content: "A small, genuinely-needed prerequisite found while designing this leaf, not part of its original scope: `Int64#to_f: Float64` / `Float64#to_i: Int64`, real LLVM `sitofp`/`fptosi` conversions — verified directly that no numeric conversion of any kind existed anywhere in this compiler before this (Int64/Float64 receivers fell straight through `infer_expr_type`'s `Type::Class` fallthrough into a generic \"non-class type\" diagnostic). `JsonNumber` only ever carries a `Float64` (plan 118), so an `Int64`-typed field had no way at all to become one without this."
    status: done
  - id: leaf-expand-derives-serializable-branch
    content: "Extend `crates/emerald-parser/src/ast.rs`'s `expand_derives` (plan 61's own single-target function) to branch on `derive_name`: `\"Comparable\"` keeps its exact existing behavior unchanged; `\"Serializable\"` synthesizes `to_json_value(self): JsonValue`, building a real `JsonObject({...})` `HashLit` from the class's own alphabetically-sorted field list (the identical determinism convention `derive Comparable`'s field-sorted `cmp_chain` already established), each field's own type driving which `JsonValue` variant constructor (`JsonNumber`/`JsonString`/`JsonBool`) wraps it."
    status: done
  - id: leaf-field-type-scope
    content: "Scope field-type support to exactly four types this leaf can correctly lower today — `Int64`, `Float64`, `String`, `Boolean` — rejecting any other field type (a nested class, `Array[T]`, `Hash[K,V]`, `Option[T]`, a newtype, ...) with a real, named `derive`-time compile error identifying the offending class and field, never a silent skip or a wrong/partial `JsonObject`."
    status: done
  - id: leaf-example-and-gate
    content: "`examples/derive_serializable.em` (the Concrete Proof below), wired into `emerald-cli/tests/examples.rs`'s checked table; new `emerald-parser` unit tests covering the synthesized method's exact AST shape, the unsupported-field-type rejection, and the already-defines-`to_json_value` rejection (mirroring `derive Comparable`'s own three analogous tests exactly); full `cargo nextest run --workspace` / `cargo clippy --workspace --all-targets` / `treefmt` gate."
    status: done
  - id: leaf-from-json-value-deferred
    content: "`from_json_value(v: JsonValue): Result[Self, String]` — inception-3's own other named half of `derive Serializable` — is NOT synthesized this session, and this is a verified compiler limitation, not a choice deferred for convenience: constructing `Self` from a parsed `JsonValue` needs a `ClassName.from_json_value(v)` call with no receiver instance in hand, and `emerald-sema`'s own `Expr::New` arm (`crates/emerald-sema/src/lib.rs`, the sole place a class is ever constructed) proves there is no such dispatch path for a real user class today — `.new`/`.spawn`/`.remote`/`.locate` are the only reserved bare-class-name call forms, and `grammar.lalrpop`'s `MethodDef` admits only ordinary, implicitly-`self`-taking methods, no class-level static method at all. Deferred pending a future \"class-level static methods\" prerequisite plan, the identical disclosed-not-dodged discipline plan 118's own deferred `leaf-fix-hash-generic-indexing` item already established."
    status: deferred
isProject: false
---

# `derive Serializable`

This closes out the original Foundation-tier batch inception-3 recommended
alongside plan 118 (`JsonValue`) and the `Iterable[T]` investigation (plan
192): a class opting into `derive Serializable` gets a real,
compiler-synthesized `to_json_value(self): JsonValue` method, built
directly from its own field list — no schema declared up front, no
runtime reflection, the same "synthesize from what the class already
declares" mechanism plan 61's `derive Comparable` established for `==`.

## Concrete proof this plan targets

```ruby
class Person derive Serializable
  name: String
  age: Int64
  gpa: Float64
  active: Boolean

  fn initialize(name: String, age: Int64, gpa: Float64, active: Boolean): Void do
    @name = name
    @age = age
    @gpa = gpa
    @active = active
  end
end

p: Person = Person.new("Ada", 36, 3.9, true)
doc: JsonValue = p.to_json_value
puts doc.to_s
```

Expected output (verified, not assumed):
```
{"active":true,"age":36.0,"gpa":3.9,"name":"Ada"}
```

Field order is alphabetical (`active`, `age`, `gpa`, `name`), not
declaration order — the identical determinism convention `derive
Comparable`'s own field-sorted `cmp_chain` already established, reused
verbatim rather than invented fresh for this leaf. `age` (`Int64`)
widens to `36.0` through this leaf's own new `.to_f` conversion; `gpa`
(already `Float64`) does not — the same disclosed JSON-numeric-widening
behavior plan 118 already established for `Json.parse`, now also true
of the encoding direction.

## Decision log

- **`Int64#to_f` / `Float64#to_i` are a real, previously-nonexistent
  addition, found necessary rather than assumed.** Checked directly
  against `emerald-sema`'s `infer_expr_type`: before this session, an
  `Int64`- or `Float64`-typed receiver calling ANY method fell straight
  through every existing receiver-type-gated dispatch arm (`String`,
  `JsonValue`, `Pair`, newtype, `Array`/`Hash`) into the generic
  `Type::Class` fallthrough, producing `"method call \`.foo\` on
  non-class type Int64"` — there was no numeric conversion of any kind.
  Since `JsonNumber` only ever carries a `Float64` (plan 118's own
  disclosed design), an `Int64` field had no path to becoming a
  `JsonValue` at all without this. Added as a narrow, real LLVM
  `sitofp`/`fptosi` cast (`build_signed_int_to_float`/
  `build_float_to_signed_int`), not a runtime call — there is no
  `emerald-rt` symbol for this, nor should there be one.
  **Real, disclosed side effect found by running the full workspace
  gate, not assumed:** two pre-existing `emerald-sema` tests
  (`rejects_each_on_a_non_iterable_receiver`,
  `rejects_method_call_on_non_class_receiver`) asserted the OLD generic
  `"non-class type"`/`".each"` substring for a `.each`/`.sum` call on a
  plain `Int64` receiver. Since `Int64` now has its own dedicated
  dispatch arm (mirroring `String`/`JsonValue`/newtype's own established
  "receiver's real inferred type wins" precedent), that same call is now
  rejected by the new arm's own, more specific `"Int64 has no method
  \`each\`"`/`"Int64 has no method \`sum\`"` message instead — a real,
  disclosed, and arguably-improved wording change, not a regression in
  what gets rejected. Both tests were updated to assert the new message
  rather than loosened or deleted.
- **Only four field types are supported — `Int64`, `Float64`, `String`,
  `Boolean` — a deliberate, narrow v1 scope, not an oversight.** A field
  of any other type (a nested class, `Array[T]`, `Hash[K,V]`,
  `Option[T]`, another newtype, ...) is a real `derive`-time compile
  error naming the offending class and field, never a silent skip or a
  partially-correct `JsonObject`. Recursing into a field's own type
  (calling ITS `to_json_value` in turn for a nested class, or lowering
  an `Array[T]` element-wise into a `JsonArray`) is a straightforward,
  real follow-up this leaf does not attempt — scoped out the same way
  plan 118 itself scoped out fixing `Hash[K,V]`'s generic indexing
  defect rather than let scope creep block shipping `Json.parse`.
- **`from_json_value` is NOT synthesized, and this is a verified
  compiler limitation, not a scope choice made for convenience.**
  Checked directly against `crates/emerald-sema/src/lib.rs`'s
  `Expr::New` arm (the ONLY place a class is ever constructed in this
  compiler) and `grammar.lalrpop`'s `MethodDef` production: there is no
  class-level static-method dispatch anywhere in this language today.
  `.new`/`.spawn`/`.remote`/`.locate` are the sole reserved bare-
  class-name call forms, each hardcoded in sema/codegen; a real user
  class has no way to declare or dispatch a method that doesn't take an
  implicit `self` receiver. Synthesizing `ClassName.from_json_value(v)`
  would need that prerequisite first. Deferred, not silently dropped —
  the identical "disclosed, not dodged" discipline plan 118's own
  deferred `leaf-fix-hash-generic-indexing` item already established
  for its own out-of-scope-but-real finding.
- **`to_json_value`'s own body needs no accessor synthesis, unlike
  `derive Comparable`'s `==`.** `Comparable`'s synthesized `==` calls
  `other.field()` on a DIFFERENT instance, so it needs a real public
  accessor to exist (synthesizing one when missing). `to_json_value`
  only ever reads `self`'s own fields via `@field` (`Expr::InstanceVar`),
  which is always legal inside a method body regardless of whether an
  accessor exists — so this leaf adds no accessor-synthesis step at
  all, a real, simpler case than `Comparable`'s.

## Update (2026-09-22, same-day session): implemented, four of five leaves done, one deferred by design

`crates/emerald-sema/src/lib.rs`'s `infer_expr_type` gained two new
receiver-type-gated arms (`Type::Int64` → `.to_f`, `Type::Float64` →
`.to_i`), placed immediately after the existing `JsonValue`-receiver
arm, before the `Type::Class` fallthrough. `crates/emerald-codegen/src/
lib.rs`'s `build_method_call` gained a matching, receiver-representation-
scoped check for `.to_f`/`.to_i` placed BEFORE the `Expr::Ident`-only
receiver guard (not after, unlike a first draft) — `derive
Serializable`'s own synthesized body calls `.to_f` on a bare `@field`
(an `InstanceVar`, never a named local), the identical "found by running
it, not assumed" fix pattern plan 93/168's own `@field.value`/`@field.
call` InstanceVar-receiver bugs needed.

`expand_derives` now branches on `derive_name`: the `"Comparable"` path
is completely unchanged (verified via the full pre-existing test suite
still passing unmodified except for the two message-wording updates
above); a new `else` (`"Serializable"`) branch builds the `JsonObject
({...})` `HashLit` from `field_names`/`fields` (already shared,
pre-computed state both branches now consume) and pushes the
synthesized `to_json_value` method directly.

Full concrete proof verified end to end via `examples/
derive_serializable.em`, producing exactly the predicted
`{"active":true,"age":36.0,"gpa":3.9,"name":"Ada"}` — confirmed by
actually running the compiled binary via `cargo nextest`, not assumed
from the buffer-layout reasoning alone (the same "verify, don't assume"
discipline plan 118 itself demonstrated for its own `.to_s` round-trip).

New `emerald-parser` unit tests: `expand_derives_synthesizes_to_json_
value_from_alphabetized_fields` (exact synthesized-AST-shape assertion,
mirroring `derive Comparable`'s own analogous test), `expand_derives_
rejects_a_serializable_field_of_unsupported_type`, `expand_derives_
rejects_a_class_that_already_hand_writes_to_json_value`. The pre-
existing `expand_derives_rejects_an_unknown_derive_target` test's own
fixture used `derive Serializable` as its example of an unsupported
target — now a real, supported one — so it was updated to use a
genuinely nonexistent name (`Frobnicatable`) instead, rather than
deleted.

Full workspace gate: `cargo nextest run --workspace` (960/960, 2
skipped, all passing — including the two updated pre-existing
`emerald-sema` message-wording assertions), `cargo clippy --workspace
--all-targets` (clean), `treefmt` (0 changed).
