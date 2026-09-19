---
name: Structured Type Expressions — Parameterized Proc, Multi-Bound Generics, Generic Methods
overview: "Real root cause of plan 70/74's Iterable[T] block, checked against source, not assumed: emerald-sema already has a fully structured, parameterized Proc(Vec<Type>, Box<Type>) in its own Type enum — the gap is that a Proc's signature can only ever be INFERRED from a co-located Expr::Lambda, never WRITTEN as a type annotation, because the parser's TypeName production is a flat String (Array[Int64], Hash[K,V], Result[T,E] are all grammar-level string concatenation, explicitly deferred from a real structured type AST per plan 09's own decision log). This plan replaces TypeName's flat-string convention with a real recursive type-expression AST in emerald-parser, gives Proc a real written form, widens generic-parameter bounds to support conjunction (absorbing plan 75's scope under this number), and lifts the sema restriction that currently rejects any generic method outright. This is the specific, bounded set of type-system gaps every currently-blocked plan (74, and later 81/85) actually needs — not an attempt to reproduce Rust's type system wholesale."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-type-expr-ast
    content: "Replace crates/emerald-parser/src/grammar.lalrpop's TypeName: String production with a real recursive type-expression AST node in emerald-parser (Named(String) for a plain class/interface/primitive name; Generic(String, Vec<TypeExpr>) subsuming Array[T]/Hash[K,V]/Pair[K,V]/Result[T,E]/Option[T] as instances of one general case instead of each needing its own hand-rolled grammar alternative and string format; Tuple(Vec<TypeExpr>); Func(Vec<TypeExpr>, Box<TypeExpr>) for Proc's real parameterized form). This is the single load-bearing change everything else in this plan sits on -- nesting (Proc[Array[Int64], Int64], a generic method returning a Hash[K, Array[V]], etc.) must round-trip correctly through this AST, not just the one-level-deep cases the current string convention happens to handle."
    status: pending
  - id: leaf-resolve-type-consumes-structure
    content: "Rewrite emerald-sema's resolve_type (crates/emerald-sema/src/lib.rs:273) to consume the new TypeExpr directly and recursively, instead of parsing a formatted string. This should simplify resolve_type's existing string-splitting logic, not add to it -- the target sema::Type enum (Array(Box<Type>), Hash(Box<Type>, Box<Type>), Proc(Vec<Type>, Box<Type>), etc., already defined at lib.rs:22) does not change; only how it gets constructed from the parser's output does."
    status: pending
  - id: leaf-parameterized-proc-annotation
    content: "Give Proc a real written-out type-annotation form (decide the exact spelling -- Proc[Args..., Ret] mirroring Hash[K,V]'s bracket convention, or reusing the already-adopted fn(Args): Ret declaration syntax in type position, matching the Sable design brief's own §17 function-type notation more closely -- record which was chosen and why) so a parameter, field, or interface-method-signature type can state a Proc's real parameter/return types directly, not only recover them from an adjacent Expr::Lambda the way plan 10's original design required. The existing lambda-literal-inference path (Let-binding a bare Proc from a co-located lambda) must keep working unchanged."
    status: pending
  - id: leaf-multi-bound-generics
    content: "Widen TypeParam.bound (grammar.lalrpop:352-359 and sema's mirroring Type::Generic bound field) from a single Option<String> to a real bound set, parsed as [T: Bound1 + Bound2 + ...]. Absorbs plan 75's scope under this plan's number rather than leaving that row a separate, now-redundant plan -- mark plan 75 superseded in the plan-of-plans, the same convention plan 70/87 already used."
    status: pending
  - id: leaf-generic-methods
    content: "Lift the explicit 'generic methods are not supported' sema restriction (grammar.lalrpop:382-388's own decision log names this precisely: [T: Bound] is already grammatically reachable on a class method, sema just rejects any non-empty type_params found there). Wire real per-method monomorphization reusing plan 41/58's existing top-level generic-function/class machinery -- this is what actually lets interface Iterable[T]'s map[U](f: Proc[T, U]): Array[U] exist as a real, monomorphizable method signature."
    status: pending
  - id: leaf-codegen-verification
    content: "Verify emerald-codegen's own Proc-call dispatch, which today likely still assumes a Proc's signature is always recoverable from an adjacent Expr::Lambda rather than checked structurally up front. Confirm whether real codegen changes are needed once a Proc's type can be fully known from its written annotation alone (e.g. as an interface method's parameter, with no lambda literal anywhere nearby), and make them if so -- do not assume codegen is unaffected without checking."
    status: pending
  - id: leaf-regression-tests
    content: "Add regression tests proving the actual target capability: a real interface Iterable[T] declaring a generic map[U](f: Proc[T, U]): Array[U]) method that type-checks and monomorphizes correctly for at least two different (T, U) pairs; a multi-bound generic function ([T: Ord + Clone]) correctly rejecting a type argument that satisfies only one of the two bounds; a nested Proc type annotation (e.g. Proc[Array[Int64], Int64]) round-tripping through parsing and sema resolution correctly, proving the AST change actually solved the nesting problem the old string convention couldn't."
    status: pending
isProject: false
---

# Plan 88 — Structured Type Expressions

This plan exists because plan 70 could not build `Iterable[T]` as a real
generic interface, and the reason turned out to be more specific and
more bounded than "the type system isn't powerful enough" — checked
directly against `emerald-sema`'s own source, not assumed. `sema::Type`
already has `Proc(Vec<Type>, Box<Type>)`, a fully structured,
parameterized function type. The actual gap is narrower: that structure
can currently only be *inferred* from a co-located `Expr::Lambda`, never
*written* as a type annotation — because the parser hands `resolve_type`
a flat, pre-formatted `String` (`"Array[Int64]"`, `"Hash[K, V]"`), a
convention plan 09's own decision log already flagged as a deferred
simplification, not a permanent design. This plan pays that deferral
down, specifically for the cases the currently-blocked plans (74, and
later 81/85) actually need — not as a general "make the type system
better" exercise.

## Concrete proof this plan targets

```ruby
interface Iterable[T]
  fn map[U](f: Proc[T, U]): Array[U]
end

class Numbers
  implements Iterable[Int64]

  values: Array[Int64]

  fn initialize(values: Array[Int64]): Void do
    @values = values
  end

  fn map[U](f: Proc[T, U]): Array[U] do
    result: Array[U] = Array.new(0)
    for x in @values
      result = result + [f.call(x)]
    end
    result
  end
end

fn largest[T: Comparable + Cloneable](values: Array[T]): T do
  # multiple bounds, conjunction — plan 75's own ask, absorbed here
  ...
end

n: Numbers = Numbers.new([1, 2, 3])
doubled: Array[Int64] = n.map(->(x: Int64): Int64 { x * 2 })
puts doubled[0]   # 2
```

None of this parses today: `Proc[T, U]` as a written type annotation
does not exist, a generic method on a class is grammatically reachable
but explicitly sema-rejected, and `[T: Comparable + Cloneable]` has no
grammar for a second bound at all.

## Decision log

- **Why sema's `Type` enum is not being redesigned.** It already
  correctly models everything this plan needs (`Proc`, `Array`, `Hash`,
  `Generic` with a bound). The problem is entirely upstream, at the
  parser's `TypeName` production and the string-formatting/re-parsing
  boundary `resolve_type` currently has to bridge. Fixing that boundary
  is a smaller, more precise change than replacing a type-checking
  model that already works.
- **Why this is scoped to exactly these four things, not "a proper
  Rust-like type system" taken literally.** Explicitly declined, named
  here so a future session doesn't wonder whether they were forgotten:
  higher-kinded types, associated types, const generics, trait objects/
  dynamic dispatch (a permanent, day-one decision — inception's
  no-vtables stance), a general variance system (already declined for
  borrows specifically in `spec/OWNERSHIP.md` §9, extended here to
  generics generally — invariant everywhere, simplest rule, revisit only
  if a real program needs otherwise), and macro-based trait deriving
  (inception §22's standing rule against a macro system at all). This
  project's own inception engineering rule #4 — "do not import another
  language's type system wholesale" — is the standard this plan is held
  to, the same one `spec/OWNERSHIP.md` was held to for the borrow
  checker.
- **Absorbing plan 75.** Multi-bound generics and this plan's own
  `TypeParam.bound` widening are the exact same change — leaving plan 75
  as a separate row would mean redoing this plan's own work under a
  different number. Marked superseded in the plan-of-plans, matching
  the convention plan 70 (superseding 42) and plan 87 (superseding 86)
  already established.
- **One atomic plan, not staged.** The `TypeExpr` AST change is a
  single load-bearing representation shift every other leaf in this
  plan depends on — there's no independently-provable intermediate
  stage the way the actor-concurrency chain (54→55→56→57) had one.
  This mirrors plan 71's own precedent for exactly this kind of
  foundational, single-file-anchored change.
- **Relationship to plan 74.** Once this ships, `Iterable[T]` becomes
  buildable for real, closing plan 74's actual blocker — this plan does
  not build `Iterable[T]` itself, plan 74 (or a revival of plan 70's
  scope) still does that work, now against a type system that can
  express it.
- **Relationship to plans 81 and 82-85.** Domain-type/newtype support
  (plan 81) and ownership qualifiers (`own`/`borrow`/`borrow var`, plan
  82's design) are natural future consumers of a real `TypeExpr` AST —
  a newtype needs a `TypeExpr::Named` wrapper with nominal-not-
  structural identity, and an ownership qualifier needs a clean
  attachment point on a type rather than another bolt-on string
  convention. Neither is implemented here; this plan only ensures the
  representation they'll need already exists rather than requiring its
  own redesign later.
