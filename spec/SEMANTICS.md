# Emerald — SEMANTICS.md

**Status:** v1 semantic decisions
**Purpose:** answer every question inception §19 lists, one section per
question group, as numbered decisions — not restated questions. Where
[`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md) already locked part of an answer (Nil,
`Integer`), this document cross-references rather than re-deriving it, per
this plan's no-contradiction requirement.

Every decision below is checked against inception §20's out-of-scope list;
none reintroduces metaprogramming, reflection, monkey patching, or runtime
structural mutation.

---

## 1. Variables

1. **Superseded by the Sable-alignment grammar cutover (plan 72) — local
   variables are now immutable by default.** This point originally read
   "local variables are mutable by default... without a separate `mut`
   keyword," true through v1 and inherited from Ruby having no
   immutable-by-default locals. That decision was reversed, not amended:
   `x: Int64 = 0; x = 1` is now a compile error
   (`"cannot reassign immutable binding `x`, declared without `var`"`).
   Mutation requires an explicit `var` at declaration:
   `var x: Int64 = 0; x = 1` is legal. Class fields (`@x`) and function/
   loop parameters are unaffected by this — they have no `var` form and
   keep their own, separately-decided mutability (fields: freely
   reassignable, unmarked; parameters and loop induction variables:
   never reassignable, no `var` form exists for them at all). See
   `GRAMMAR.md` §2/§3 and `history/2026-09-19T111000Z-plan-72-immutable-
   by-default-bindings.md` for the full decision log.
2. **Constants are truly immutable after their first assignment.** A
   second assignment to a constant identifier (`GRAMMAR.md` §2) is a
   compile error, not Ruby's runtime warning. This is one of the few
   places Emerald is *stricter* than Ruby, justified because static
   analysis (inception §3) requires knowing a constant's value can't
   change out from under later code.
3. **Variables cannot change type after initialization.** Inception §19
   recommends this; adopted as-is. `x: Int64 = 0` fixes `x`'s type for the
   rest of its scope — a later `x = "hello"` is a compile error. This is
   the direct static-typing analogue of "no gradual/dynamic mode"
   (inception §2.3).

---

## 2. Nil

**Superseded by plan 73 — see `TYPE_SYSTEM.md` §4's own superseded
banner.** `T?`/`nil`/`&.`/`||=` are gone; `Option[T]`/`Some`/`None`/
`?.`/`??` replace them outright. The summary below describes the v1
design this replaced, kept for historical accuracy, not current syntax.

Fully decided in [`TYPE_SYSTEM.md` §4](./TYPE_SYSTEM.md#4-nil-and-nullable-types);
summarized here for completeness against inception §19's question list:

1. **`nil` is represented by a `Nil` type** with exactly one value.
2. **`Nil` is not assignable to arbitrary reference types.** Only to `T?`
   (nullable) forms — reference types default non-nullable.
3. **Nullable types are needed, and added:** the `T?` syntax
   (`GRAMMAR.md` cross-references this from safe navigation).

---

## 3. Methods

1. **Methods are not overloaded in v1.** One method name resolves to
   exactly one signature per receiver type/arity combination; defining two
   `def`s with the same name and arity on one class is a compile error.
   Overload resolution (choosing among candidates by static argument
   types) is real complexity inception §22 rule 10 ("keep the initial
   compiler small enough that one person can understand the entire
   pipeline") argues against for v1. Revisit only if a concrete milestone
   in plan-of-plans needs it.
2. **Method resolution is name + arity + static receiver type**, exactly
   as inception §19 poses the question — confirmed, not modified. No
   duck-typed dispatch (inception §9): the receiver's declared type must
   have a matching method.
3. **Keyword arguments are retained**, per `GRAMMAR.md` §6 — same
   type-annotation requirement as positional parameters.
4. **Default arguments are retained**, per `GRAMMAR.md` §6 — the default
   expression's type must match the parameter's declared type.

---

## 4. Classes

1. **Fields are declared explicitly** in the class body with a type
   annotation (`GRAMMAR.md` §8, `TYPE_SYSTEM.md` §5) — matches inception
   §6's `Point` example exactly.
2. **Undeclared instance variables are illegal.** Assigning `@foo` where
   `foo` has no matching field declaration is a compile error. This is
   the direct enforcement mechanism for inception §2.3's "no dynamic
   instance-variable definition."
3. **Multiple inheritance is forbidden.** `class` supports single
   inheritance only (`GRAMMAR.md` §8: REMOVE). `struct` supports no
   inheritance at all (`TYPE_SYSTEM.md` §5).
4. **Modules are namespaces in v1, not mixins.** `module Name; end` can
   group constants/classes/methods under a name and be referenced via
   `Name::Thing`, matching inception §5's "if their static semantics are
   straightforward" qualifier — a namespace has trivial static semantics
   (a compile-time name-resolution prefix). Mixin behavior (`include`,
   `extend`) is addressed separately in §10 below, and is **not** adopted
   in v1: composing method sets from multiple sources per instance adds
   method-resolution-order complexity disproportionate to the milestones
   in inception §17. This resolves `GRAMMAR.md` §8's `UNDECIDED` rows for
   `include`/`extend`: both are `REMOVE` for v1, revisit only when a
   concrete milestone needs cross-cutting method reuse that single
   inheritance can't express.

---

## 5. Blocks

1. **Blocks are closures.** A block captures the local variables of its
   enclosing scope by reference, matching Ruby's closure semantics
   exactly — inception §5 keeps blocks "if they can be represented
   cleanly," and lexical closure is the cleanest representation
   available.
2. **Blocks can escape their defining scope** (e.g. stored in a field,
   returned from a method) **only when their captured variables' types are
   statically known** — which they always are in Emerald, since every
   captured local is already typed at its declaration (§1 above). There is
   no additional restriction beyond ordinary type checking; inception
   §19's question is really asking whether closures need special
   escape-analysis restrictions, and the answer is no — escape analysis is
   an optimization concern (§4 below), not a legality concern.
3. **Block parameter types are inferred from the call site** where the
   receiving method's signature statically pins them (e.g. `xs.each do |x|
   ... end` infers `x: Int64` from `xs: Array[Int64]`); an explicit
   annotation (`GRAMMAR.md` §9's example) is always legal and required
   when the call site can't determine the type unambiguously (e.g. inside
   a generic method body before its type parameter is resolved).
4. **Closures are heap-allocated only when necessary.** A block that does
   not escape its call (the overwhelmingly common case — `each`, `map`,
   `reduce` with an immediately-invoked block) is compiled to a
   stack-passed function value with no heap allocation. A block stored
   past the call's lifetime (assigned to a field, returned) is
   heap-allocated. This is inception §12's "stack allocation / escape
   analysis" list item, scoped down from "investigate later" to "required
   for blocks specifically," because block-heavy code (`each`, `map`) is
   exactly the numeric/collection hot path inception §11 cares about.

---

## 6. Arrays

1. **`Array[T]` is mutable** — matches Ruby's default (`TYPE_SYSTEM.md`
   §8); no separate immutable-array type in v1.
2. **`Array[T]` is homogeneous** — every element has static type `T`;
   confirmed, not modified, from inception §10.
3. **Primitive-element arrays are packed/unboxed.** `TYPE_SYSTEM.md` §8
   states this as a hard representation requirement (not merely
   permitted) for any value-type `T`.

---

## 7. Exceptions

1. **Exceptions are dynamically typed values but statically declared at
   `rescue` sites.** `raise` can construct and throw any subclass of the
   exception hierarchy (dynamic — matches Ruby, and avoids inventing a
   Java-style `throws` clause inception §2.2 would flag as an unwarranted
   import). Each `rescue ExceptionType => e` clause (`GRAMMAR.md` §10)
   statically types `e` as `ExceptionType`. This is the "smallest
   modification" answer: Ruby's own `rescue` clauses are already
   statically named; Emerald just uses that name to type `e` instead of
   leaving it dynamically typed.
2. **`raise` is not arity/type-restricted beyond ordinary method-call
   type checking** — `raise SomeError, "message"` is checked like any
   other call (`SomeError.new` must type-check), not specially
   constrained by a declared "throws" set on the enclosing method. Adding
   checked-exceptions-style method annotations is real complexity
   (Java's own community treats checked exceptions as a wart) and is not
   in inception §5's keep list.
3. **Exceptions are implemented using native unwinding**, matching
   inception §19's suggested direction and inception §3's "no bytecode
   VM, no interpreter" requirement — a native-unwinding implementation
   (e.g. via the codegen backend's own unwind tables, whichever of
   Cranelift/LLVM is chosen in plan-of-plans row `02`) avoids a manual
   error-code-threading calling convention, which would otherwise leak
   into every function signature.

---

## 8. Numeric types

Fully decided in [`TYPE_SYSTEM.md` §3 and §7](./TYPE_SYSTEM.md#3-the-integer-decision);
summarized here for completeness against inception §19's question list:

1. **`Integer` is not arbitrary precision — it does not exist as a
   builtin name in v1.** See `TYPE_SYSTEM.md` §3 for the full rationale.
2. **The default integer literal type is `Int64`.**
3. **Numeric conversions:** implicit widening only, explicit cast required
   for narrowing/precision-losing/sign-changing conversions, no implicit
   promotion in mixed-width arithmetic. Full table in `TYPE_SYSTEM.md` §7.

---

## 9. Strings

Fully decided in [`TYPE_SYSTEM.md` §9](./TYPE_SYSTEM.md#9-strings);
summarized here for completeness against inception §19's question list:

1. **UTF-8**, confirmed.
2. **Mutable** in v1 (matches Ruby's default; no immutable/frozen variant
   introduced yet).
3. **`String` is distinct from byte arrays.** Raw-byte access goes through
   an explicit conversion; `String` itself always represents valid UTF-8
   text, never an arbitrary byte buffer.

---

## 10. Modules

1. **What Ruby module behavior survives:** namespacing only (§4.4 above).
   A module can hold constants, nested classes/modules, and
   namespace-scoped methods (called via `Name.method`, analogous to
   Ruby's `module_function`-style calls, not instance-mixed methods).
2. **`include` is not retained in v1.** Resolves `GRAMMAR.md` §8's
   `UNDECIDED` row — mixin composition is out of scope for the reasons
   given in §4.4 above.
3. **`extend` is not retained in v1.** Same reasoning and same
   `GRAMMAR.md` row, resolved together with `include`.
4. **Modules cannot define fields.** Only classes and structs have
   instance state (`TYPE_SYSTEM.md` §5); a namespace-only module has no
   instances to hold fields on.

---

## Cross-references

- Grammar-level consequences of these decisions (which rows are `KEEP` vs
  `REMOVE` vs `MODIFY`) are recorded in [`GRAMMAR.md`](./GRAMMAR.md).
- Type-level consequences (representations, conversions, nullability) are
  recorded in [`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md).
- Decisions with a direct dependency on toolchain choice (exception
  unwinding mechanism, closure heap-allocation strategy) are flagged for
  plan-of-plans row `02 toolchain-prototype` to confirm feasibility against
  the chosen codegen backend.
