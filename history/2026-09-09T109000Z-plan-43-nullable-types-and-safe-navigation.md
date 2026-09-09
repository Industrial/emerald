---
name: Nullable Types and Safe Navigation
overview: "Finishes the `T?` nullable-type system spec/TYPE_SYSTEM.md §4 already designs but plan 25 deliberately stopped short of: sema tracks nilness as a real type-level distinction (scoped to reference types — Class/String/Array/Hash — not value types), a direct method/field access on a `T?` value is a compile-time diagnostic, `obj&.method` compiles to a real conditional branch, and `||=`/`&&=` (blocked by plan 31 for exactly this reason) become buildable."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-nullable-type-system
    content: "Type::Nullable(Box<Type>), TypeName's `?` suffix, a shared is_assignable() widening rule threaded through every existing assignability check-site, and rejecting a direct .method/.field on a T? receiver as a compile-time diagnostic"
    status: pending
  - id: leaf-safe-navigation
    content: "`obj&.method(...)` — Expr::SafeCall, sema dispatch scoped to class-typed nullable receivers with a pointer-representable return type, codegen as a real is-nil branch + PHI (reusing build_short_circuit's existing pattern)"
    status: pending
  - id: leaf-or-and-assign
    content: "`x ||= default` (assign-if-nil, then sema narrows x's tracked type to non-null) and `x &&= value` (assign-only-if-non-nil, no narrowing) — the two compound-assignment forms plan 31 explicitly deferred to this plan"
    status: pending
isProject: false
---

# Plan 43 — Nullable Types and Safe Navigation

This is plan 43 of the 36-47 batch — twelve independent sibling plans,
each closing one distinct, identity-preserving gap in Emerald's
language/stdlib surface, whose combined purpose (per this batch's
shared brief) is moving Emerald from ~10-15% of standard Ruby's surface
toward its ~45% strategic ceiling without conceding any of Emerald's
locked identity constraints (no `method_missing`/`eval`/`send`/
reflection, no mixins/open classes, no dynamic dispatch/vtables, no
tracing GC). It is post-v1 scope, same posture as plans 25/31 before
it: this plan does not touch `plan-of-plans.md` or any other plan file
— `plan-of-plans.md` itself is updated separately once all twelve
36-47 plans are authored.

Concrete proof this plan targets — one program, two calls, both paths:
```ruby
class Greeter
  name: String

  def initialize(name: String) -> Void
    @name = name
  end

  def shout -> String
    @name + "!"
  end
end

def find_greeter(id: Int64) -> Greeter?
  if id == 1
    return Greeter.new("ada")
  end
  return nil
end

def greet(id: Int64) -> String
  g: Greeter? = find_greeter(id)
  message: String? = g&.shout
  message ||= "nobody here"
  return message
end

puts greet(1)
puts greet(2)
```
Expected output: `ada!`, `nobody here` — `greet(1)`'s `g` is a real,
non-nil `Greeter`, so `g&.shout` calls `shout` for real and produces
`"ada!"` wrapped as `String?`; `message ||= "nobody here"` finds
`message` already non-nil and no-ops. `greet(2)`'s `g` is `nil`
(`find_greeter`'s "not found" path), so `g&.shout` short-circuits to
`nil` without ever calling `shout`; `message ||= "nobody here"` then
assigns the default. Both paths return through `return message`, which
type-checks against `greet`'s declared `String` return type only
because `||=` narrowed `message`'s tracked type from `String?` to
`String` earlier in the same function body — the exact mechanism that
makes `||=` buildable at all, not just parseable (see the Decision
log).

## Decision log

- **`spec/TYPE_SYSTEM.md` §4 already specifies exactly this feature and
  was never built.** Verified this session by reading it in full: "A
  **nullable type** is written `T?`... the union of `T` and `Nil`...
  Safe navigation (`&.`)... is legal only on a `T?` receiver and
  produces a `U?` result from a method returning `U`." §10's
  assignability table already specifies `nil → T?` ✓ and `T → T?` ✓
  (widens), `nil → T` ✗. None of this exists in the compiler today —
  verified against `crates/emerald-sema/src/lib.rs`'s actual `Type`
  enum (L12-38): it has `Type::Nil` (plan 25) but no `Nullable`
  variant, and `resolve_type` (L83-127) has no case for a trailing `?`
  in a type-name string — `"Greeter?"` falls straight through every
  arm to the final `other => Err("unknown type")` at L125. This plan
  adds `Type::Nullable(Box<Type>)` and a `TypeName` grammar suffix,
  finishing exactly the gap §4 already names, not inventing new spec.
- **Plan 25's `leaf-nil-type` explicitly, precisely declined this.**
  Verified against `history/2026-09-08T224500Z-plan-25-stdlib-
  expansion.md`'s own Decision log: "`nil` gets the narrowest possible
  scope, deliberately short of `TYPE_SYSTEM.md` §4's actual... design
  ... Building `T?` for real means a new type-syntax form, safe
  navigation (`&.`), and reworking every assignability check in
  `emerald-sema` to understand a type union — genuinely comparable in
  size to this compiler's entire existing type-checking surface... `T?`
  ... remain real, substantial, separate future work." Plan 25's own
  `Type::Nil`/`Expr::Nil` (still a fixed `i64` `0` sentinel per
  `emerald-codegen`'s `ValKind::Nil` doc comment) is unchanged and
  fully reused here — a bare `Nil`-typed value is still exactly what
  plan 25 built; this plan adds `Nullable(T)` *alongside* it, as the
  wrapper a `nil` literal can also inhabit when the declared type
  admits it.
  Today, `x: Int64 = nil` is rejected — but *only* because `Type::Nil
  != Type::Int64` under `Stmt::Let`'s plain equality check (L887,
  `if actual != declared`); there is no nullability concept at all
  driving that rejection, just an unrelated primitive-type mismatch
  that happens to produce the right answer by coincidence. This plan
  replaces that accidental correctness with a real, checkable
  type-level property.
- **`T?` is scoped to reference types only — `Type::Class(_)`,
  `Type::String`, `Type::Array(_)`, `Type::Hash(_, _)` — not value
  types (`Int64`, `Float64`, `Boolean`, `Nil` itself, `Proc`).**
  `TYPE_SYSTEM.md` §1's own value/reference split already draws this
  exact line (value types: unboxed, no allocation, no identity; reference
  types: heap-allocated). Verified against `emerald-codegen/src/lib.rs`:
  every reference type here lowers to `ValKind::Ptr`/`ValKind::Str`
  (`value_kind_for_type`, L64-86), both backed by an LLVM `ptr` — which
  has a spare bit pattern (`null`) to spend on nilness for free. A
  value type like `Int64` is an unboxed `i64` with no spare
  representation to steal; making `Int64?` real would require boxing it
  (a heap allocation + a tag check on every use) — the exact
  boxed-representation cost `TYPE_SYSTEM.md` §3 already refuses to pay
  for arbitrary-precision `Integer`, for the identical reason (inception
  §11's numeric-performance goal). `resolve_type`'s new `?`-suffix
  handling enforces this directly: it only constructs `Type::Nullable`
  when the resolved inner type is one of the four reference kinds above;
  `Int64?`/`Float64?`/`Boolean?`/`Proc?` are rejected with a diagnostic
  naming this exact reason, at the type-annotation boundary — a
  compile-time refusal, not a codegen crash discovered later.
- **A direct `.method`/`.field` access on a `T?` receiver is a
  compile-time diagnostic — this is the plan's actual payoff, and it
  makes Emerald strictly better than Ruby on this one axis, not just at
  parity.** In Ruby, `user.name` on a `nil` `user` raises
  `NoMethodError: undefined method 'name' for nil:NilClass` — at
  runtime, however deep into a program's execution that `nil` traveled
  from. In Emerald, once `find_greeter`'s return type is declared
  `Greeter?`, writing `find_greeter(id).shout` anywhere (no `&.`, no
  prior `== nil` check) is rejected by `emerald-sema` before the
  program ever runs. This is a genuine case where static analyzability
  — the whole reason this project accepts no reflection, no
  `method_missing`, no dynamic dispatch — buys something Ruby
  structurally cannot: the exact class of bug Ruby programmers hit
  constantly (a `nil` where an object was expected) moves from
  "discovered in production" to "does not compile." Implemented in
  `infer_expr_type`'s existing `Expr::MethodCall` arm (L523-539): when
  the receiver's inferred type is `Type::Nullable(_)`, this plan returns
  a distinct diagnostic ("use safe navigation `&.` or an explicit `==
  nil` check") *before* falling into the arm's existing `Type::Class`
  match, instead of the generic "non-class type" message that arm
  already produces for other type mismatches.
- **`is_assignable(actual, declared)` replaces the raw `actual !=
  declared` equality check at every existing assignability
  check-site** — not just the two or three this plan's own worked
  example touches. Verified and enumerated this session, each a
  distinct `if actual != declared`/`if t != *return_type`-shaped check
  in `crates/emerald-sema/src/lib.rs`: `Stmt::Let` (L887), `Stmt::Assign`
  (L926), `Stmt::SetField` (L903), `check_args` (L774, function/method/
  lambda/module call arguments), `check_set_index` (L813, array/Hash
  element writes), `check_multi_assign` (L850), and `Stmt::Return`
  (L980). `is_assignable` is exact-equality for every non-nullable
  `declared` (so all seven sites' behavior on existing, non-`T?`
  programs is provably unchanged — same predicate, same answer), and
  additionally accepts `Type::Nil` or the unwrapped inner type into a
  `Type::Nullable(inner)` `declared`, per `TYPE_SYSTEM.md` §10's table.
  Routing every site through one shared helper — rather than hand-
  patching only `Let`/`Return` for this plan's own example — is what
  makes `T?` actually compose with the rest of the language (a
  `Greeter?` field, a `Greeter?` function argument, a `Greeter?` array
  element write) instead of only working in the one shape this plan
  happened to demonstrate.
- **`x ||= default` unblocks exactly the dependency plan 31 named.**
  Verified against `history/2026-09-09T093000Z-plan-31-compound-and-
  multiple-assignment.md`'s own Decision log: "`||=`/`&&=` conditional
  compound assignment are out of scope. `spec/GRAMMAR.md` §3 itself
  ties `||=` to 'x's type already admitting Nil'... Building `||=`
  correctly needs that fuller optional-type system, not just a `nil`
  literal — this plan doesn't redo plan 25's scope call." That fuller
  system is exactly `leaf-nullable-type-system` above. `x ||= default`
  desugars to a real conditional (is `x` nil? if so, store `default`)
  — see `leaf-or-and-assign` — but the reason it's *useful*, not just
  legal, is a second move: after `check_stmt` processes an `OrAssign`,
  it updates `env[name]` from `Type::Nullable(inner)` to `*inner`
  directly (not flow-sensitive branch narrowing — `emerald-sema`'s
  `env` is a single, linear, forward-updated map the exact same way
  `Stmt::Let`/`Stmt::Assign` already mutate it at every statement, so
  this is the same mechanism, not a new one). This is sound *by
  construction* of `||=`'s own semantics, independent of which runtime
  branch executes: if `x` was already non-nil, it stays whatever
  non-nil value it had (type `inner`); if it was nil, it is now
  `default`, whose checked type is also `inner`. Either way, immediately
  after `x ||= default`, `x` is unconditionally `inner`-typed — which is
  exactly what lets `return message` in this plan's worked example
  type-check against `greet`'s declared `String` return type, even
  though `message` was declared `String?` two lines earlier.
- **`x &&= value` deliberately does *not* narrow `x`'s tracked type —
  the asymmetric twin of `||=`'s narrowing, not an oversight.** `x &&=
  value` only assigns when `x` is currently non-nil; when `x` is nil, it
  is skipped and `x` stays nil. Unlike `||=`, there is no runtime branch
  after which `x` is unconditionally non-nil — the nil-and-skipped case
  leaves it exactly as nilable as before. `check_stmt`'s `AndAssign` arm
  therefore requires `value`'s type be assignable to `x`'s *full*
  declared `Nullable(inner)` type (via the same `is_assignable` helper,
  so widening a plain `inner`-typed `value` into the nullable slot still
  works, matching this plan's own `g &&= Greeter.new("upgraded")`
  acceptance case) and leaves `env` untouched.
- **Safe navigation is scoped to a class-typed nullable receiver whose
  dispatched method's return type is itself one of the four
  pointer-representable kinds** (`Class`/`String`/`Array`/`Hash`) —
  not `Int64`/`Float64`/`Boolean`/`Void`/`Nil`. This is the same
  boxing-avoidance line drawn above, applied to `&.`'s *result*: `obj&.
  method` where `method` returns `Int64` would need to produce
  `Int64?`, which this plan has already declined to represent. Rejected
  at the sema level with a diagnostic naming the concrete return type,
  not a codegen panic. `String?`/`Array[T]?`/`Hash[K,V]?` receivers are
  grammar- and sema-legal for `&.` in principle but practically inert
  today — verified this session that no method table exists anywhere in
  `emerald-sema` for `String`/`Array`/`Hash` receivers (`infer_expr_
  type`'s `MethodCall` arm requires `Type::Class`); a class-typed
  nullable receiver (this plan's `Greeter?`) is the only shape with any
  real methods to dispatch, so it's the only shape this plan's
  acceptance criteria exercise.
- **`&.` is illegal on a non-nullable receiver — not silently accepted
  as a no-op `.`.** `spec/GRAMMAR.md` §4 states this directly: safe
  navigation is "Only legal on a receiver whose type admits `Nil`."
  `x&.shout` where `x: Greeter` (not `Greeter?`) is therefore a
  diagnostic ("use `.` — `x` is never nil"), not a permissive synonym
  for `.` the way real Ruby treats it (Ruby's `&.` is legal, if
  pointless, on a known-non-nil receiver). A real, disclosed narrowing
  of Ruby's actual behavior, justified by the spec text already on
  file, not invented for this plan.
- **`nil` as a codegen value is context-dependent, and this plan fixes
  exactly the four call sites where that context is locally available
  — `Let`, `Assign`, `MultiAssign`'s per-value build, and `Return` —
  not a general type-directed codegen refactor.** Verified this
  session: `Expr::Nil`'s existing `build_expr` arm
  (`emerald-codegen/src/lib.rs` L1449) unconditionally emits
  `context.i64_type().const_int(0, false)` typed `ValKind::Nil` — a
  fixed `i64` bit pattern, plan 25's design. A `Greeter?`-typed local's
  pre-allocated storage, by contrast, is a `ptr` slot (`value_kind_for_
  type` falls through any non-primitive-named string, including
  `"Greeter?"`, to `ValKind::Ptr`, verified at L64-86). Storing the
  literal `i64` `0` into that `ptr`-typed alloca is a real LLVM type
  mismatch, not a hypothetical one. `Stmt::Let`'s generic arm (L2304),
  `Stmt::Assign` (L2342), `Stmt::MultiAssign`'s per-value loop (L2363),
  and `Stmt::Return(Some(e))` (L2444) already have the expected
  `ValKind` available locally without new plumbing — `Let` from
  `value_kind_for_type(ty)`, `Assign`/`MultiAssign` from the target's
  already-recorded `(ptr, ValKind)` in `vars`, `Return` from `ret_kind`
  (already a `build_stmt` parameter) — so each site gets a small,
  local special case: when the value expression is literally
  `Expr::Nil` and the expected kind is `Ptr`/`Str`, build a null pointer
  constant directly instead of calling the generic `Expr::Nil` codegen
  path. `Stmt::Let`'s existing `local_classes.insert` (which records a
  local's class name for later method dispatch) is adjusted to strip a
  trailing `?` before its `ctx.classes.contains_key` check, so a
  `Greeter?` local is still recorded as class `Greeter` for `&.`'s
  dispatch to find — `local_classes` only ever needs a bare class name,
  independent of nullability, exactly the way it already doubles up to
  hold `"Hash[K, V]"` strings per plan 25's own Decision log.
- **A bare `nil` literal used directly as a function/method/lambda call
  argument remains rejected, even into a `T?`-typed parameter — out of
  scope, declared, with a workaround.** Fixing the four codegen sites
  above (Let/Assign/MultiAssign/Return) covers this plan's own worked
  example; `check_args`'s argument-checking loop (shared by ordinary
  functions, methods, module calls, and `.call` on a `Proc`) is a fifth
  call site this plan does not also special-case, to keep the codegen
  surface this plan touches small and mechanically verifiable rather
  than threading expected-kind context through every call-argument
  builder in `build_expr`/`build_method_call`/`Expr::New`. `foo(nil)`
  where `foo`'s parameter is `Greeter?` is therefore still rejected —
  bind the `nil` to a `T?`-typed local first (`g: Greeter? = nil;
  foo(g)`), which works today with no further codegen changes, since a
  loaded `T?` local is an ordinary already-correctly-typed pointer value
  by the time it reaches an argument list. This mirrors plan 25's own
  `Array.new(...)` restriction ("may only appear as a top-level `Let`'s
  value") — a real, disclosed narrowing with a working escape hatch,
  not a silent gap.
- **Comparing a `T?` value against `nil` (`g == nil`) is scoped to
  exactly that shape — a `Nullable(_)` operand against the `Nil`
  literal — not general `Nullable == Nullable` comparison between two
  different nilable values.** `infer_expr_type`'s `Expr::Compare` arm
  gains one new case (alongside its existing `lt != rt` strict-equality
  rule, unchanged for every other pair): a `Nullable(_)` operand against
  a `Type::Nil` operand — either order — always type-checks to
  `Boolean`, since this is precisely the "explicit nil check" alternative
  to `&.`. Codegen lowers this as a genuine null-pointer test (comparing
  the pointer-backed side against LLVM's `null`, `Eq`/`Ne` selecting
  which of "is null"/"is not null" is emitted) rather than a general
  pointer-equality comparison — the only case this plan needs, and the
  only case its acceptance criteria exercise. Two *different* `T?`
  values compared against each other (`g1 == g2`, both `Greeter?`) is a
  real, smaller possible follow-on this plan does not build.
- **Declined: a general nil-coalescing expression operator (an "elvis"
  `?:`/`x || default`-as-an-expression form) beyond exactly `T?`,
  `&.`, `||=`, and `&&=`.** Ruby itself has no single canonical
  construct beyond plain `||` for this (which Emerald's own `||` cannot
  reuse here regardless — plan 18 already typed `&&`/`||` as
  `Boolean`-only operators, verified against `check_boolean_binop`; a
  general nil-coalescing `||` over arbitrary `T?` values was never on
  the table), so there is nothing further to match Ruby's surface with.
  `||=`/`&&=` as statement-level compound assignments and `&.` as a
  navigation operator are the complete, disclosed feature set this plan
  ships; a value-producing "give me `x` or a default, in one expression,
  anywhere" form is real, larger, separate future work.

## Leaf: leaf-nullable-type-system

### 1. Context
- Why: `spec/TYPE_SYSTEM.md` §4's `T?` design has no implementation at
  all — verified this session (`Type` enum, `resolve_type`, every
  assignability check-site) — see the Decision log for the precise gap
  and the exact seven check-sites this leaf touches.
- Target state: `crates/emerald-sema/src/lib.rs` gains `Type::
  Nullable(Box<Type>)`; `resolve_type` gains a `?`-suffix case
  (rejecting non-reference inner types with a named diagnostic, per the
  Decision log); a new `fn is_assignable(actual: &Type, declared: &Type)
  -> bool` replaces the raw `!=` check at `Stmt::Let`, `Stmt::Assign`,
  `Stmt::SetField`, `check_args`, `check_set_index`, `check_multi_
  assign`, and `Stmt::Return`; `infer_expr_type`'s `MethodCall` arm
  rejects a `Type::Nullable(_)` receiver with a `&.`/nil-check-naming
  diagnostic; `infer_expr_type`'s `Compare` arm accepts a
  `Nullable(_)`-vs-`Nil` pair. `crates/emerald-parser/src/grammar.
  lalrpop`'s `TypeName` production gains `<base:TypeName> "?" =>
  format!("{base}?")`. `crates/emerald-codegen/src/lib.rs` gets the
  four `Expr::Nil`-into-`ptr`-slot special cases (`Let`/`Assign`/
  `MultiAssign`/`Return`), the `local_classes` trailing-`?` strip, and
  the `Compare`-vs-nil null-pointer-test codegen case.

### 2. Acceptance Criteria
1. `g: Greeter? = Greeter.new("ada")` and `g: Greeter? = nil` both
   type-check `Ok(())` (a real class value widens into `T?`; `nil`
   assigns into it directly) — and, compiled, linked, and run, this
   plan's own top-level worked example (`greet(1)`/`greet(2)`) prints
   `ada!` then `nobody here`, proving both a real non-null pointer and a
   real null pointer round-trip correctly through a `Greeter?`-typed
   local without crashing or misreading the wrong bit pattern.
2. `x: Int64 = nil` is still rejected — the pre-existing plan-25
   regression test, now passing for the *right* reason (nilness/type
   mismatch on a genuinely non-nullable type), not the old accidental
   `Type::Nil != Type::Int64` coincidence.
3. `y: Int64? = 5` is rejected with a diagnostic naming the
   nullable-value-type restriction (not a panic, not silently accepted)
   — the concrete, executed proof of this plan's value-type decline.
4. `g.shout` where `g: Greeter?` (no `&.`, no nil check) is rejected
   with a diagnostic naming `&.`/an explicit nil check as the fix — the
   plan's headline "compile-time, not `NoMethodError`" proof.
5. `g == nil` where `g: Greeter?` type-checks to `Boolean`, and — as
   part of AC1's compiled-and-run proof — evaluates correctly for both
   a nil and a non-nil `g` at runtime.
6. Regression: every prior plan's example (`hello.em`, `Point`,
   collections, exceptions, modules, inheritance, field-access sugar,
   `for`-`in`, compound/multiple assignment) still parses, type-checks,
   and links-and-runs identically — `is_assignable`'s exact-equality
   fallback for every non-nullable `declared` type must not change any
   existing accept/reject outcome.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop` (`TypeName`
  `?` suffix), `crates/emerald-sema/src/lib.rs` (`Type::Nullable`,
  `resolve_type`, `is_assignable`, the seven check-sites, `MethodCall`
  and `Compare` arms), `crates/emerald-codegen/src/lib.rs` (`Expr::Nil`
  special-casing at four `build_stmt` sites, `local_classes` suffix
  strip, `Compare`'s new codegen case)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Sema unit tests | `cargo test -p emerald-sema` | all pass, incl. AC2-AC5 | agent-claimed-locally |
| Workspace (real linked-and-run) | `cargo test --workspace` | all pass, incl. AC1's `ada!\nnobody here\n` | agent-claimed-locally |

---

## Leaf: leaf-safe-navigation

### 1. Context
- Why: `spec/GRAMMAR.md` §4 marks safe navigation `&.` KEEP with a
  worked example (`user&.name`) and `TYPE_SYSTEM.md` §4 specifies its
  typing rule, but no `&.` token, AST node, or codegen exists anywhere
  in this compiler today (verified this session against `grammar.
  lalrpop` and `ast.rs`'s `Expr` enum). Depends on `leaf-nullable-
  type-system` for `Type::Nullable` and `is_assignable`.
- Target state: a new `"&."` grammar token, distinct from the existing
  `"."`, `"&"` (bitwise/block-param marker), and `"&&"` tokens; new
  `StmtPrimaryExpr`/`PrimaryExpr` alternatives mirroring the existing
  `<recv:Ident> "." <method:Ident> ["(" Args ")"]` shapes exactly, but
  producing a new `Expr::SafeCall(Box<Expr>, String, Vec<Expr>)` AST
  node (`crates/emerald-parser/src/ast.rs`) instead of reusing `Expr::
  MethodCall` — kept distinct because its sema dispatch and codegen are
  both genuinely different (nil-check branch, `U?` result type), not
  just an evaluation-order variant of an ordinary call. `emerald-sema`'s
  `infer_expr_type` gains a `SafeCall` arm per the Decision log's
  scoping rules (class-typed nullable receiver only; dispatched method's
  return type must itself be pointer-representable). `emerald-codegen`
  gains `build_safe_call`, reusing `build_short_circuit`'s existing
  is-null-guarded-basic-blocks-plus-PHI pattern (`emerald-codegen/src/
  lib.rs` L835-919, the codebase's own precedent for "conditionally
  compute one of two values and merge them into one SSA result").

### 2. Acceptance Criteria
1. `g&.shout` where `g: Greeter?` and `shout -> String` type-checks to
   `Type::Nullable(Box::new(Type::String))`.
2. This plan's own top-level worked example, compiled, linked, and run,
   prints `ada!` then `nobody here` — real executed proof the
   nil-check branch actually skips the call on the nil receiver (never
   invoking `shout` on a null pointer) and actually performs it on the
   non-nil receiver, merging both paths into one well-typed `String?`
   result via a real LLVM `phi`.
3. `x&.shout` where `x: Greeter` (non-nullable) is rejected with a
   diagnostic naming that `&.` requires a nullable receiver (per
   `GRAMMAR.md` §4) — proving `&.` isn't silently accepted as a `.`
   synonym.
4. `g&.some_method_returning_int64` (a class method whose return type
   is `Int64`) is rejected with a diagnostic naming the
   pointer-representable-return-type restriction — the value-type
   decline's negative proof for `&.` specifically.
5. Regression: every prior plan's example still parses/type-checks
   identically; ordinary `.` method calls (plan 08/32/33) are
   unaffected — `&.`'s new grammar alternatives must not shadow or
   collide with the existing `"." Ident` productions (verify no LALR(1)
   conflict at build time).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Expr::SafeCall`),
  `crates/emerald-parser/src/grammar.lalrpop` (`"&."` token + two new
  `StmtPrimaryExpr`/`PrimaryExpr` alternatives), `crates/emerald-
  parser/src/lib.rs` (tests), `crates/emerald-sema/src/lib.rs`
  (`infer_expr_type`'s `SafeCall` arm), `crates/emerald-codegen/src/
  lib.rs` (`build_safe_call`, `Expr::SafeCall` dispatch in `build_expr`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser -p emerald-sema` | all pass, incl. AC1, AC3, AC4 | agent-claimed-locally |
| Workspace (real linked-and-run) | `cargo test --workspace` | all pass, incl. AC2's `ada!\nnobody here\n` | agent-claimed-locally |

---

## Leaf: leaf-or-and-assign

### 1. Context
- Why: `spec/GRAMMAR.md` §3 marks `x ||= expr` KEEP, tied explicitly to
  "`x`'s type already admitting `Nil`"; plan 31 built every other
  compound-assignment form (`+= -= *= /= %=`) but named `||=`/`&&=` as
  the two it could not build yet, for exactly this reason (see the
  Decision log's precise citation). Depends on `leaf-nullable-type-
  system` for `Type::Nullable`/`is_assignable` and reuses `leaf-safe-
  navigation`'s `Greeter`/`find_greeter` scaffold for its own compiled
  proof.
- Target state: two new grammar tokens `"||="`/`"&&="` and two new
  `Stmt`-initial alternatives (mirroring plan 31's `<lhs:StmtExpr>
  "+=" <rhs:Expr> =>? match lhs { Expr::Ident(name) => ..., other =>
  Err(...) }` shape exactly, restricted to a plain-local target the
  same way); two new `Stmt` variants in `crates/emerald-parser/src/
  ast.rs`, `Stmt::OrAssign { name: String, default: Expr }` and `Stmt::
  AndAssign { name: String, value: Expr }` — **not** desugared into
  `Stmt::Assign` the way `+=`/etc. are, because their semantics are
  genuinely conditional (assign-only-if), unlike `+=`'s unconditional
  `x = x + 1` rewrite. `emerald-sema`'s `check_stmt` gains an arm for
  each, per the Decision log's narrowing/non-narrowing asymmetry.
  `emerald-codegen`'s `build_stmt` gains an arm for each: a real
  is-nil-guarded conditional store (two basic blocks plus a merge,
  no `phi` needed since neither statement produces a value). Two
  existing exhaustive `match stmt` functions in `emerald-codegen/src/
  lib.rs` need a new arm each for both variants or the crate fails to
  compile: `collect_idents_in_stmt` (L298-405, no wildcard arm —
  verified this session; mirror its existing `Stmt::Assign` arm at
  L316-321, pushing `name` to `referenced` since this is always a
  reassignment of an outer name, never a fresh declaration) and, for
  completeness, `collect_lets` (L487-533, which *does* have a trailing
  `_ => {}` wildcard, verified — so `OrAssign`/`AndAssign` need no new
  arm there, since neither declares a fresh local).

### 2. Acceptance Criteria
1. `message: String? = nil` followed by `message ||= "nobody here"`
   type-checks `Ok(())`, and after it, `message`'s tracked type is
   `Type::String` (not `Type::Nullable(String)`) — verified directly by
   a follow-up statement in the same test that only type-checks if the
   narrowing actually happened (e.g. `return message` against a
   declared `String` return type, exactly this plan's own `greet`).
2. This plan's own top-level worked example, compiled, linked, and run,
   prints `ada!` then `nobody here` — real executed proof of `||=`'s
   both branches: `greet(1)`'s `message` is already non-nil (the
   assignment is skipped), `greet(2)`'s `message` is nil (the default
   is assigned) — verified as two genuinely different `puts` outputs
   from the same compiled function called with two different arguments.
3. A second compiled-and-run program exercises `&&=`'s both branches:
   ```ruby
   def upgrade(id: Int64) -> String
     g: Greeter? = find_greeter(id)
     g &&= Greeter.new("upgraded")
     message: String? = g&.shout
     message ||= "still nobody"
     return message
   end

   puts upgrade(1)
   puts upgrade(2)
   ```
   prints `upgraded!` then `still nobody` — `upgrade(1)`'s `g` is
   non-nil, so `&&=` assigns (`g` becomes the `"upgraded"` `Greeter`,
   whose `shout` produces `"upgraded!"`); `upgrade(2)`'s `g` is nil, so
   `&&=` is skipped (`g` stays nil, `g&.shout` short-circuits, `||=`
   supplies `"still nobody"`) — real, executed proof `&&=` only assigns
   when its target is non-nil, in both directions.
4. `s ||= "y"` where `s: String` (declared non-nullable) is rejected
   with a diagnostic naming that `||=` requires a nullable declared
   type — same for `&&=` on a non-nullable target.
5. `message ||= other` where `other: String?` (itself nullable, not the
   unwrapped `String`) is rejected — `||=`'s default must be the
   unwrapped, non-nullable type, or the narrowing postcondition
   wouldn't be sound (see the Decision log); this is a real, disclosed
   restriction, not an accidental gap.
6. Regression: every prior plan's example still parses/type-checks
   identically; plan 31's existing `+=`/`-=`/`*=`/`/=`/`%=`/multiple-
   assignment tests are unaffected (`Stmt::Assign`/`Stmt::MultiAssign`
   are untouched by this leaf — `OrAssign`/`AndAssign` are new,
   parallel `Stmt` variants, not a change to the existing ones).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Stmt::OrAssign`,
  `Stmt::AndAssign`), `crates/emerald-parser/src/grammar.lalrpop`
  (`"||="`/`"&&="` tokens + two `Stmt` alternatives), `crates/emerald-
  parser/src/lib.rs` (tests), `crates/emerald-sema/src/lib.rs`
  (`check_stmt`'s `OrAssign`/`AndAssign` arms), `crates/emerald-
  codegen/src/lib.rs` (`build_stmt`'s `OrAssign`/`AndAssign` arms,
  `collect_idents_in_stmt`'s two new arms)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser -p emerald-sema` | all pass, incl. AC1, AC4, AC5 | agent-claimed-locally |
| Workspace (real linked-and-run) | `cargo test --workspace` | all pass, incl. AC2's `ada!\nnobody here\n` and AC3's `upgraded!\nstill nobody\n` | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
