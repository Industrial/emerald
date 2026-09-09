---
name: Result Type and Error Propagation
overview: "`Result[T, E]` as a hardcoded, compiler-native compound type (mirroring plan 42's `Pair[K,V]` precedent) with two constructors, `Ok(...)`/`Err(...)`, a postfix `?` propagation operator restricted to `Let`/`Assign` value positions inside a function whose own declared return type is `Result[T, E]` for an exactly-matching `E`, and a dedicated `case ... when Ok(v) ... when Err(e) ... end` destructuring form for the non-propagating case — closing the batch's Result/`?` gap without generic user-declared enums, without `From`-style error conversion, and without touching plan 38's exception model, which this plan explicitly coexists with rather than replaces."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-parser-result
    content: "Expr::Ok(Box<Expr>), Expr::Err(Box<Expr>), Expr::Try(Box<Expr>), Stmt::MatchResult{scrutinee, ok_var, ok_body, err_var, err_body}; grammar for the `Result[T, E]` compound-type annotation, `Ok(...)`/`Err(...)` constructor expressions, postfix `?`, and the two-armed `case ... when Ok(v) ... when Err(e) ... end` form"
    status: pending
  - id: leaf-sema-result-construction
    content: "Type::Result(Box<Type>, Box<Type>); resolve_type parses `Result[T, E]` exactly like Hash[K,V]'s `, `-split convention; Ok/Err construction is checked only in the three expected-type-providing positions (Let's declared ty, Assign's env-recorded type, Return's threaded return_type) since infer_expr_type carries no expected-type parameter anywhere; two new compiler-intrinsic builtins, is_valid_int/parse_digits, type-checked the same hardcoded way `puts` already is"
    status: pending
  - id: leaf-sema-try-and-match
    content: "check_stmt special-cases `Expr::Try` inside Let/Assign only, requiring the already-threaded `return_type: &Type` to be `Result[_, E]` with E exactly equal (no coercion) to the scrutinee's own E, and the unwrapped Ok type to match the assignment target's declared/existing type; check_stmt also handles Stmt::MatchResult, binding ok_var/err_var into child scopes at the scrutinee's real, statically-known T/E (no bare-rescue-style unbindable case, since Result's payload types are never opaque)"
    status: pending
  - id: leaf-codegen-result-construction
    content: "Ctx gains is_valid_int_fn/parse_digits_fn; Expr::Ok/Expr::Err are ordinary build_expr arms (alloc 16 bytes via ctx.alloc, store an i64 discriminant at offset 0, store the built inner value at offset 8) reusing build_hash_lit's fixed-offset field_ptr idiom; value_kind_for_type's existing `_ => ValKind::Ptr` fallback already covers `Result[T, E]` for free"
    status: pending
  - id: leaf-codegen-try-and-match
    content: "Expr::Try gets dedicated Stmt::Let/Stmt::Assign arms (mirroring Expr::ArrayNew's precedent, since build_expr alone has no declared-type context): load the discriminant, branch; Ok path loads the payload at the target's own already-known ValKind and stores it; Err path returns the SAME Result pointer unchanged via the existing build_return mechanism, no new allocation. Stmt::MatchResult reuses build_case's arm/next_check chaining idiom generalized to a fixed two-way i64 discriminant branch"
    status: pending
isProject: false
---

# Plan 53 — Result[T,E] and Error Propagation

This is plan 53 of the 48–57 batch — ten independent sibling plans
implementing "Beyond the Ceiling" (the follow-up analysis to the
36–47 batch) in full: an actor-model concurrency pillar, algebraic
data types and pattern matching (plan 52), and — this plan — a
`Result[T, E]` error-propagation type, among others, all without
conceding any of Emerald's identity constraints (no `method_missing`/
`eval`/`send`/reflection, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing GC, no runtime
reflection). Like every batch before it, this is post-v1 scope and is
**not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md);
that table is updated separately, once, after all ten 48–57 plans are
authored — this plan does not touch it or any other plan file.

"Beyond the Ceiling" originally floated a much larger idea alongside
Result: an Effect.ts/`id_effect`-style `Effect<A, E, R>` monadic effect
system with a built-in dependency-injection `R`/requirements channel.
That idea was deliberately declined for this batch, after debate, for
four concrete reasons, none of which this plan reopens: it needed
generic data types Emerald doesn't have; composition sugar (do-notation
or generators) Emerald has no mechanism for; a fiber-based structured-
concurrency runtime that would directly compete with this same batch's
actor-model pillar; and a DI/environment channel needing structural or
intersection types and variance rules Emerald has none of. This plan
builds only the smaller idea that survived that debate — `Result[T, E]`
plus `?`-propagation — and introduces no `Effect`, no generic
environment/requirements channel, and no DI machinery anywhere below.

Depends on: **plan 38** (`full-exception-model`,
`2026-09-09T104000Z-plan-38-full-exception-model.md` — real, current
source already implements it: `crates/emerald-parser/src/ast.rs`'s
`Stmt::Begin { body, rescues: Vec<RescueClause>, ensure: Option<Vec<
Stmt>> }` and `Stmt::Retry` are both live AST today, verified this
session) for the exceptions-coexistence framing this plan builds to
(see Decision log); **plan 42** (`enumerable-stdlib`, not yet executed
in real source, but authored and available as a sibling document) for
the `Pair[K, V]`-as-hardcoded-compound-type precedent this plan reuses
for `Result[T, E]`'s own representation — this plan does **not** wait
on plan 42 landing first, it only cites its design. This plan also
does not depend on plan 52 (algebraic-data-types-and-pattern-matching,
authored in parallel this same batch, not on disk as of this session)
for anything load-bearing — see Decision log for why its own
`Ok(v)`/`Err(e)` matching form is a small, dedicated construct instead.

Concrete proof this plan targets — one function that can fail, one
caller that propagates via `?`, one top-level match, run twice:

```ruby
def parse_int(s: String) -> Result[Int64, String]
  if is_valid_int(s)
    return Ok(parse_digits(s))
  end
  return Err("not a number")
end

def try_parse(s: String) -> Result[Int64, String]
  n: Int64 = parse_int(s)?
  return Ok(n * 2)
end

result: Result[Int64, String] = try_parse("21")
case result
when Ok(v)
  puts v
when Err(e)
  puts e
end
```

Run 1, `try_parse("21")`: `is_valid_int("21")` is true, so `parse_int`
returns `Ok(21)`. Inside `try_parse`, `parse_int(s)?` sees a `0` (`Ok`)
discriminant, unwraps `21` into `n`, and execution continues to
`return Ok(n * 2)` — `Ok(42)`. The top-level `case` matches `Ok(v)`
with `v = 42`. **Expected stdout: `42`.**

Run 2, `try_parse("abc")`: `is_valid_int("abc")` is false, so
`parse_int` returns `Err("not a number")`. Inside `try_parse`,
`parse_int(s)?` sees a `1` (`Err`) discriminant and immediately returns
that *same* `Result` value from `try_parse` itself — `return Ok(n * 2)`
never runs. The top-level `case` matches `Err(e)` with
`e = "not a number"`. **Expected stdout: `not a number`.** (A buggy
implementation that always executed `return Ok(n * 2)` regardless of
the `?`'s outcome, or that constructed a fresh, blank `Err` instead of
forwarding the real message, would print something other than
`not a number` here — this is the one line in the whole proof that
actually exercises propagation, not just construction.)

## Decision log

- **Two-channel error model, stated plainly: `Result[T, E]` and plan
  38's `raise`/`rescue`/`ensure` coexist by design, they are not
  redundant and neither replaces the other.** This is Emerald's version
  of Rust's Result-vs-panic split. `Result[T, E]` is for **expected,
  recoverable failures a caller is meant to handle** — this plan's own
  `parse_int` returning `Err("not a number")` on ordinary bad input is
  the canonical case: the caller is expected to check the outcome and
  decide what to do, the same way a real program checks whether a
  lookup or a parse succeeded. Plan 38's `raise`/multi-`rescue`/
  `ensure` model (`2026-09-09T104000Z-plan-38-full-exception-model.md`,
  verified this session against real, currently-shipping source —
  `Stmt::Begin`/`Stmt::Retry`/`RescueClause` all already live in
  `crates/emerald-parser/src/ast.rs`) remains for **exceptional,
  programmer-error conditions** — an assertion failure, an out-of-
  bounds array access, a genuinely unexpected invariant violation —
  conditions a well-behaved caller usually isn't expected to routinely
  check for at every call site. This plan does not add a `Result`-
  returning variant of anything plan 38 already models as a raised
  exception, and it does not add `raise`/`rescue` sugar to anything
  this plan models as a `Result`; the two mechanisms are deliberately
  kept orthogonal, addressing two different failure *kinds*, not two
  competing ways to spell the same failure. Nothing here changes
  `Stmt::Begin`, `class_tags`, or any of plan 38's codegen.
- **Representation: a tagged union exactly like `Pair[K, V]`'s own
  fixed layout (plan 42's Decision log), generalized from a fixed pair
  to a fixed discriminant-plus-payload — not a novel scheme.** Plan
  42's `Pair[K, V]` (not yet built in real source, but its Decision log
  is explicit) is "a hand-rolled, hard-coded compound `Type` variant...
  with... a fixed 16-byte `[key: 8][value: 8]` layout — deliberately
  the same per-pair byte layout `Hash[K, V]`'s own buffer already
  uses." `Result[T, E]` reuses this exact discipline: a new
  `Type::Result(Box<Type>, Box<Type>)` variant in `emerald-sema`,
  parsed by `resolve_type` with the identical `, `-split convention
  already shipping for `Hash[K, V]` today (verified this session,
  `crates/emerald-sema/src/lib.rs:112-124`), and a fixed 16-byte heap
  layout: `[discriminant: i64 @ offset 0][payload: 8 bytes @ offset
  8]`, `discriminant = 0` for `Ok`, `1` for `Err`. The payload is a
  single 8-byte slot, not a `max(sizeof(T), sizeof(E))`-computed
  variable-width region, because **every value this codegen backend
  ever manipulates already occupies exactly one machine word** —
  verified against `ValKind`'s four storage cases and `local_llvm_type`
  (`crates/emerald-codegen/src/lib.rs:52-101`): `Int64`→`i64`,
  `Float64`→`f64`, `Ptr`/`Str` (covers every class, `Array[T]`,
  `Hash[K,V]`, and now `Result[T,E]` itself)→a pointer, `Bool`→a
  single-bit-but-word-stored value, exactly the same "naively 8 bytes"
  discipline `FieldInfo`'s own doc comment already states for class
  fields. `max(sizeof(T), sizeof(E))` therefore collapses to a
  compile-time constant `8` for every possible `T`/`E` in this
  language — there is no instantiation where the payload could need to
  be wider, so this plan allocates a flat, fixed 16 bytes via
  `ctx.alloc` (the same `emerald_alloc` runtime helper `Pair`/`Hash`/
  `Array` literals already call) every time, with no per-instantiation
  size computation at all.
- **`?` is restricted to exactly two syntactic positions in v1: a
  `Stmt::Let`'s value and a `Stmt::Assign`'s value — not a fully
  general expression-position postfix operator.** This mirrors an
  existing, disclosed restriction already in this exact file for
  exactly the same reason: `Expr::ArrayNew`'s codegen is special-cased
  directly inside `Stmt::Let`'s match arm because, verified this
  session at `crates/emerald-codegen/src/lib.rs:2709-2719`, "`build_expr`
  alone has no declared-type context to draw on" — `build_expr`'s
  signature carries no "expected type" parameter anywhere in this
  codebase (confirmed by inspecting every one of its ~30 call sites).
  `Expr::Try`'s Ok-path unwrap has the identical problem: to load the
  right LLVM type out of the payload slot, it needs to know the
  *target's* declared kind (an `Int64` unwrap loads an `i64`; a
  `String` unwrap loads a pointer) — information only available at the
  `Let`/`Assign` statement that receives the value, never inside a
  bottom-up `build_expr` call on its own. Rather than threading a new
  expected-type parameter through the entire `build_expr`/
  `infer_expr_type` call graph (a materially larger, more invasive
  change touching every recursive call site in both files), this plan
  reuses the exact `Stmt`-level special-casing idiom `ArrayNew` already
  established: `Expr::Try` used anywhere else (nested inside a binary
  operator, as a bare call argument, as a `Return`'s value, etc.) is a
  real, disclosed v1 diagnostic ("`?` is only supported directly as a
  `let` or assignment value in v1"), not a silent partial feature.
- **Sema's exact-`E`-match check reuses `check_stmt`'s already-threaded
  `return_type: &Type` parameter — no new context needs to be plumbed
  in for this half of the feature.** `check_stmt` (`crates/emerald-
  sema/src/lib.rs:893-905`) already carries `return_type: &Type` into
  every statement it checks, the same parameter it already uses to
  validate ordinary `Stmt::Return` values. `Expr::Try(inner)` inside a
  `Let`/`Assign`'s value infers `inner`'s type via the existing
  `infer_expr_type`, requires it to be `Type::Result(t_ty, e_ty)`,
  requires `*return_type` to *itself* be `Type::Result(_, e_ty2)` with
  `e_ty2 == e_ty` by `Type`'s already-derived `PartialEq` (an exact
  structural match — `Type::Class("ParseError")` vs
  `Type::Class("IoError")` fails this check even if both "look like
  errors" to a human; per the critical decision already made for this
  batch, there is no `From<E1> for E2` auto-conversion, full stop, and
  this plan declines to build one), and requires the unwrapped `t_ty`
  to match the assignment target's own type (the `Let`'s `resolve_
  type(ty)` or the `Assign`'s already-recorded `env[name]`, same
  discipline every other `Let`/`Assign` type check in this file already
  uses). A `?` used where `return_type` isn't `Type::Result(_, _)` at
  all — including a bare top-level statement, whose `return_type` is
  never a `Result` — fails this same single check, so "usable only
  inside a function whose own declared return type is itself
  `Result[T, E]`" falls out of one check, not a separate "am I inside a
  function" flag.
- **Interaction with plan 41 (interfaces-and-generics): `?`'s `E` must
  be a concrete type, never a plan-41 type parameter placeholder — a
  real, disclosed v1 narrowing, not an oversight.** Plan 41's own
  Decision log (`2026-09-09T107000Z-plan-41-interfaces-and-generics.md`)
  checks a generic function's body exactly once, before any concrete
  type is known, using a `Type::Generic("T")` placeholder substituted
  for `Self`. If a plan-41 generic function declared, say, `def
  attempt[T: Comparable](x: T) -> Result[T, String]` and used `?`
  inside its own body, this plan's exact-match check above would need
  to compare against `Type::Generic("T")` rather than a concrete
  `Type`, and — more fundamentally — decide whether two *different*
  call-site instantiations of the same generic function are allowed to
  produce genuinely different concrete `Result[T, E]` shapes from one
  shared, once-checked body. That is a real, separate design question
  neither plan 41 nor this plan resolves here; this plan's `?` support
  is scoped to functions whose declared return type resolves to a
  fully concrete `Type::Result(Type::Class(_) | Type::Int64 | ...,
  Type::Class(_) | ...)` with no `Type::Generic` anywhere in it —
  `check_stmt`'s exact-match comparison already rejects a `Type::
  Generic` on either side via the same `PartialEq` check, so this
  narrowing needs no extra guard code, just this explicit disclosure
  that it is a real, intentional v1 boundary and not a coincidence of
  what happens to type-check today.
- **`Ok`/`Err` construction is restricted to the same three
  expected-type-providing positions `ArrayNew` already established the
  precedent for (`Let`), generalized to the two other spots this
  plan's own worked example actually needs (`Assign`, `Return`) — never
  a fully general, freestanding expression.** `infer_expr_type` has no
  expected-type parameter anywhere in its signature (re-verified this
  session across the whole function, `crates/emerald-sema/src/
  lib.rs:361-664`), so `Ok(21)` on its own has no way to know what `E`
  is (nothing about `21` says whether the surrounding type is
  `Result[Int64, String]` or `Result[Int64, IoError]`) — the mirror
  image of `Err("not a number")` not knowing what `T` is. Both
  constructors are therefore checked only where an expected
  `Result[T, E]` type is already on hand: a `Stmt::Let`'s declared
  `ty`, a `Stmt::Assign`'s already-recorded `env[name]`, or a
  `Stmt::Return`'s already-threaded `return_type` — exactly this plan's
  own `parse_int` body (`return Ok(parse_digits(s))`, `return
  Err("not a number")`). `Ok(...)`/`Err(...)` used anywhere else (e.g.
  as a bare call argument, or the scrutinee of an ordinary `if`) is a
  real, disclosed diagnostic ("cannot infer `Result[T, E]`'s type
  parameters without a declared expected type here"), not a silent
  best-effort guess.
- **`Ok`/`Err` codegen, unlike `Try`, needs no `Stmt`-level special
  case — they're ordinary `build_expr` arms.** The asymmetry is real,
  not an inconsistency: `Ok(inner)`/`Err(inner)`'s payload kind comes
  from evaluating `inner` itself (`build_expr(inner)`, bottom-up, no
  ambiguity — exactly like `Expr::HashLit`'s key/value builds already
  work), so there's nothing a `Let`/`Assign`/`Return` wrapper needs to
  supply that `build_expr` doesn't already have on its own. `Try`'s
  Ok-path unwrap is the opposite direction — top-down, needing the
  *consumer's* declared kind — which is exactly why only `Try` needs
  the `ArrayNew`-style dedicated arm (see above).
- **`?`'s `Err` path forwards the exact same heap pointer, with zero
  new allocation — a real, provable optimization, not a shortcut that
  happens to work by luck.** Because `Result[T, E]`'s 16-byte layout
  (`[discriminant][payload]`) never depends on `T` at all — only `E`
  ever occupies the payload slot when `discriminant == 1` — a
  `Result[Int64, String]` value in the `Err` state and a
  `Result[Boolean, String]` value in the `Err` state are bit-for-bit
  identical whenever their `E` matches (which sema's exact-match check
  already guarantees before this codegen path is ever reached). `?`'s
  Err-path codegen is therefore just: load the discriminant, branch,
  and on the `Err` side call the *exact same* `builder.build_return
  (Some(&result_ptr))` this codegen backend already uses for an
  ordinary `Stmt::Return(Some(e))` (verified this session, `crates/
  emerald-codegen/src/lib.rs:2908-2919` — the exact call this plan
  reuses unchanged), passing the untouched pointer straight through.
  No new `Result` object is built, no bytes are copied, and this is
  only correct because sema, not codegen, is the thing enforcing that
  `E` genuinely matches — codegen leans on that guarantee rather than
  re-deriving it.
- **Pattern-matching the non-propagating case: a small, dedicated
  `Stmt::MatchResult`, not a dependency on plan 52's mechanism —
  a deliberate choice, not a default because plan 52 wasn't available
  to read.** Two independent reasons hold even setting aside plan 52's
  absence from disk this session: first, these are ten *independent*
  sibling plans in the same batch with no defined execution order —
  making this plan's own worked example depend on plan 52 landing
  first would be a real, unnecessary coupling this batch's own
  structure is designed to avoid. Second, and more fundamentally, this
  compiler's existing `case`/`when` (plan 20) is a flat, `Int64`-only
  value-equality dispatch with **zero destructuring or variable-binding
  capability at all** — verified this session directly against its own
  doc comment in `crates/emerald-parser/src/ast.rs`: "value-match via
  the same `CompareOp::Eq` `Expr::Compare` already performs, over an
  `Int64`-only scrutinee." There is no mechanism anywhere in this
  compiler today by which a `when` arm extracts a payload out of a
  scrutinee and binds it to a fresh name — that capability doesn't
  exist independent of whether plan 52 exists, so "reuse plan 52's
  mechanism" was never actually available to reuse even in principle;
  plan 52, when it lands, will have to build exactly this "an arm binds
  a payload variable" capability from scratch for user-declared enums,
  and `Result[T, E]`'s needs are a fixed, tiny special case of that
  (exactly two constructor names, fixed arity 1 each, discriminant
  already `0`/`1`) — small enough that inventing plan 52's general
  mechanism just to serve this one plan would be substantially more
  work than a dedicated form. `Stmt::MatchResult`'s *codegen*, however,
  does reuse an existing idiom at the LLVM level: plan 38's Decision
  log already establishes that multiple `rescue` clauses reuse
  `build_case`'s own "`arm_blk`/`next_check_blk` chained in source
  order" shape (`crates/emerald-codegen/src/lib.rs:2678-2795`); this
  plan's two-armed, fixed-order (`Ok` always first, `Err` always
  second, both mandatory, no `else`) discriminant branch is the
  smallest possible instance of that same chaining shape, generalized
  from "match one of N arbitrary literal values" to "branch on one of
  two known integers" — reusing the *codegen pattern*, not plan 52's
  (not-yet-existing) AST.
- **`Ok`/`Err` are recognized as literal keyword-headed grammar
  productions, not a fourth `interface`-style general mechanism —
  consistent with how this exact grammar already treats `Array`,
  `Hash`, and `Proc`.** Plan 41's own Decision log states this
  precedent outright: "`Array`... `Hash`... `Proc`... are three
  individually hand-written, literal-keyword grammar alternatives."
  `Ok(<Expr>)`/`Err(<Expr>)` as expressions, and the fixed `case
  <Expr> when Ok(<Ident>) <Block> when Err(<Ident>) <Block> end` form,
  are additions of exactly that same kind — two more hardcoded,
  literal-keyword productions, not a general "any two-constructor tag
  union" mechanism a user program could define more of. Building that
  general mechanism is explicitly plan 52's job, declined here on
  purpose (see above and the assumed contract in this batch's shared
  brief: plan 52 declines generic enums entirely, and this plan does
  not build one either, hardcoded or otherwise, beyond this one
  compiler-native type).
- **`is_valid_int`/`parse_digits` are two new, minimal, plan-53-owned
  compiler intrinsics — deliberately not a dependency on plan 45's
  stdlib string parsing.** Plan 45 (`stdlib-strings-and-io`) has not
  landed in real source as of this session (verified: no numeric-
  parsing runtime helper of any kind exists in `crates/emerald-codegen/
  src/lib.rs` or a `runtime/emerald_runtime.c` string-parsing symbol
  today), and — same reasoning as plan 52 above — this plan does not
  want a hard ordering dependency on another same-batch sibling with no
  defined execution order. Rather than reusing an unbuilt `.to_i`, this
  plan adds exactly the two small primitives its own worked example
  needs, checked and compiled the same hardcoded way `puts` itself
  already is: verified this session, `crates/emerald-sema/src/
  lib.rs:467-470`'s own comment states plainly that "`puts` is a
  compiler intrinsic, not an overloaded function... checked here
  directly rather than via a `FunctionSig` in `sigs`." `is_valid_int(s:
  String) -> Boolean` and `parse_digits(s: String) -> Int64` are
  type-checked by the same kind of hardcoded `Expr::Call` name match
  inside `infer_expr_type`, and compiled by two new small C helpers in
  `runtime/emerald_runtime.c` (`emerald_is_valid_int`, backed by a
  plain ASCII-digit scan with an optional leading `-`; `emerald_parse_
  digits`, backed by `strtoll`) plus two new `FunctionValue<'ctx>`
  fields on `Ctx`, declared the same way `print_str`/`string_concat`
  already are. This is a real, disclosed narrowing versus a general
  numeric-parsing stdlib surface — it validates and parses `Int64`
  only, nothing else — left for plan 45 (or a future plan) to broaden
  when it actually lands.

## Leaf: leaf-ast-parser-result

### 1. Context
- Why: none of `Result[T, E]`'s surface syntax exists today — no
  `Result[...]` type annotation, no `Ok`/`Err` constructor expressions,
  no postfix `?`, and no `Ok(v)`/`Err(e)` matching form.
- Target state: `crates/emerald-parser/src/ast.rs` gains `Expr::Ok
  (Box<Expr>)`, `Expr::Err(Box<Expr>)`, `Expr::Try(Box<Expr>)`, and
  `Stmt::MatchResult { scrutinee: Expr, ok_var: String, ok_body:
  Vec<Stmt>, err_var: String, err_body: Vec<Stmt> }`.
  `crates/emerald-parser/src/grammar.lalrpop` gains: a `Result[T, E]`
  alternative in the `TypeName` production, formatted as a plain
  `"Result[T, E]"` string exactly like `Hash[K, V]`'s own compound-
  string convention (`emerald-sema`'s `resolve_type` is the only thing
  that ever interprets it structurally); `"Ok" "(" <e:Expr> ")" =>
  Expr::Ok(Box::new(e))` and the `Err` analogue at expression
  precedence; `<e:Expr> "?" => Expr::Try(Box::new(e))` at the tightest
  (postfix) precedence tier, alongside method-call/index; and a new
  `case <Expr> when "Ok" "(" <Ident> ")" <Block> when "Err" "("
  <Ident> ")" <Block> end` alternative alongside the existing `Case`
  production, distinguished by the literal `Ok`/`Err` tokens
  immediately following `when` (neither is a valid arm-head `Expr` in
  the existing value-equality `case`, so there's no shape this could
  be confused with).

### 2. Acceptance Criteria
1. `Result[Int64, String]` parses as a `TypeName` and round-trips
   through `resolve_type` (exercised once `leaf-sema-result-
   construction` lands) — a parser-level test asserting the raw string
   shape alone is sufficient for this leaf.
2. `Ok(42)`, `Err("bad")`, and `x?` (where `x` is any parsed `Expr`)
   each parse to their respective new `Expr` variant — AST-shape
   assertions, no compile-and-run required yet.
3. This plan's own worked `case result when Ok(v) ... when Err(e) ...
   end` parses to `Stmt::MatchResult` with `ok_var == "v"`, `err_var ==
   "e"`, and both bodies populated.
4. Regression: every prior plan's example (including plan 38's own
   `begin`/`rescue`/`ensure`/`retry` example and plan 20's ordinary
   `case`/`when`) still parses identically — this leaf only adds new
   alternatives, it must not change how `Stmt::Case`/`Stmt::Begin`/any
   existing `Expr` shape is parsed.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new AST-shape tests | agent-claimed-locally |

---

## Leaf: leaf-sema-result-construction

### 1. Context
- Why: `emerald-sema` has no `Type::Result` variant and no way to
  check `Ok`/`Err` construction against an expected type; `is_valid_
  int`/`parse_digits` don't exist as callable names at all.
- Target state: `Type::Result(Box<Type>, Box<Type>)` added to
  `crates/emerald-sema/src/lib.rs`'s `Type` enum; `resolve_type` gains
  a `"Result["`/`"]"` branch parsing `T`/`E` via the identical `,
  `-split already shipping for `"Hash["`/`"]"` (see Decision log);
  `Expr::Ok`/`Expr::Err` are checked only inside `check_stmt`'s
  `Stmt::Let`/`Stmt::Assign`/`Stmt::Return` arms, against the relevant
  expected `Type::Result(t_ty, e_ty)` (see Decision log for exactly
  which three positions and why); `infer_expr_type`'s `Expr::Call`
  handling gains two new hardcoded intrinsic cases, `is_valid_int`
  (one `String` argument, `Type::Boolean` result) and `parse_digits`
  (one `String` argument, `Type::Int64` result), mirroring `puts`'s own
  existing hardcoded case exactly.

### 2. Acceptance Criteria
1. `r: Result[Int64, String] = Ok(5)` type-checks `Ok(())`; `resolve_
   type("Result[Int64, String]")` returns `Type::Result(Type::Int64,
   Type::String)` directly (a unit test against `resolve_type`, not
   just an end-to-end program).
2. `return Ok(42)` inside a function declared `-> Result[Int64,
   String]` type-checks; `return Ok("wrong")` inside the same function
   is rejected with a type-mismatch diagnostic naming the declared `T`.
3. `return Err("msg")` inside the same function type-checks; `return
   Err(42)` (wrong `E`) is rejected the same way.
4. `Ok(1)` used as a bare call argument (no enclosing `Let`/`Assign`/
   `Return` to supply an expected type) is rejected with the "cannot
   infer `Result[T, E]`'s type parameters" diagnostic, not a panic and
   not a silent guess.
5. `is_valid_int("21")` type-checks to `Type::Boolean`; `parse_digits
   ("21")` type-checks to `Type::Int64`; either called with a non-
   `String` argument is rejected with a diagnostic, not a panic.
6. Regression: every prior plan's example still type-checks
   identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-sema-try-and-match

### 1. Context
- Why: even once `Type::Result` exists, nothing checks `?`'s legality/
  exact-`E`-match, and nothing checks `Stmt::MatchResult`.
- Target state: `check_stmt`'s `Stmt::Let`/`Stmt::Assign` arms special-
  case a `value: Expr::Try(inner)` per the Decision log's exact-match
  rule, using the already-threaded `return_type: &Type` parameter — no
  new parameter added to `check_stmt`'s signature. `Expr::Try` reached
  anywhere else (via the ordinary `infer_expr_type` fallback) is a
  fixed diagnostic. `check_stmt` (or a new `check_match_result` helper
  alongside `check_case`) handles `Stmt::MatchResult`: infers the
  scrutinee's type, requires `Type::Result(t_ty, e_ty)`, binds
  `ok_var: t_ty` into a child `env` scope for `ok_body` and `err_var:
  e_ty` into a separate child scope for `err_body` (both real,
  usable bindings — unlike plan 38's bare-`rescue` case, `Result`'s
  `T`/`E` are always statically known here, so there's no "no universal
  root class" problem to work around).

### 2. Acceptance Criteria
1. `n: Int64 = parse_int(s)?` inside a function declared `-> Result
   [Int64, String]` type-checks `Ok(())`.
2. The same `?` inside a function declared `-> Result[Int64,
   IoError]` (a different, non-matching `E`) is rejected with a
   diagnostic naming the mismatch — proving no `From`-style
   auto-conversion happens.
3. `?` used inside a function whose declared return type is not
   `Result[_, _]` at all (including a bare top-level statement) is
   rejected with a diagnostic, not a panic.
4. `?` used anywhere other than directly as a `Let`/`Assign` value
   (e.g. `1 + parse_int(s)?`) is rejected with the "only supported
   directly as a `let`/assignment value" diagnostic.
5. `case result when Ok(v) ... when Err(e) ... end` type-checks when
   `result: Result[Int64, String]`, with `v` usable as `Int64` inside
   `ok_body` and `e` usable as `String` inside `err_body` (and each
   name is *not* visible in the other arm's body or after the `case`).
6. A `case` whose scrutinee is not `Result[_, _]`-typed is rejected
   with a diagnostic.
7. Regression: every prior plan's example still type-checks
   identically, including plan 38's own `retry`/`ensure` example (this
   leaf never touches `in_loop`/`in_rescue`/`check_begin`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-result-construction

### 1. Context
- Why: no codegen exists for `Expr::Ok`/`Expr::Err`, and the two new
  intrinsic builtins have no compiled runtime backing at all.
- Target state: `runtime/emerald_runtime.c` gains `emerald_is_valid_
  int(const char*) -> i1`-shaped and `emerald_parse_digits(const
  char*) -> i64`-shaped C helpers (a plain ASCII-digit scan with an
  optional leading `-`; `strtoll` for the value — no dependency on
  plan 45, see Decision log). `Ctx` (`crates/emerald-codegen/src/
  lib.rs`) gains `is_valid_int_fn: FunctionValue<'ctx>` and `parse_
  digits_fn: FunctionValue<'ctx>`, declared the same way `print_str`/
  `string_concat` already are. `build_expr` gains ordinary arms for
  `Expr::Ok(inner)`/`Expr::Err(inner)`: allocate 16 bytes via `ctx.
  alloc` (same helper `build_hash_lit`/`build_array_lit` already call),
  store the `i64` discriminant (`0`/`1`) at offset `0` via `field_ptr`,
  build `inner` and store its value at offset `8` — the same fixed-
  offset `field_ptr` idiom `build_hash_lit`'s key/value stores already
  use. `build_expr` also gains hardcoded `Expr::Call("is_valid_int",
  [s])`/`Expr::Call("parse_digits", [s])` arms emitting a direct call
  to the new `Ctx` function values, mirroring `build_puts`'s own
  existing hardcoded-name dispatch.

### 2. Acceptance Criteria
1. `emerald_is_valid_int("21")` (a direct C-level unit test against the
   new runtime helper) returns true; `emerald_is_valid_int("abc")`
   returns false; `emerald_parse_digits("21")` returns `21`.
2. A minimal program `x: Result[Int64, String] = Ok(5) ... ` compiled,
   linked, and run (via a temporary `case`-free debug path, or in
   concert with `leaf-codegen-try-and-match` if sequenced together)
   produces the right discriminant/payload bytes — verified by reading
   back through a `MatchResult` once that leaf lands, or by a direct
   in-process buffer inspection test if this leaf lands first.
3. `is_valid_int`/`parse_digits`, called from compiled Emerald source,
   compile and link cleanly and return the values the runtime helpers
   themselves already prove correct in (1).
4. Regression: `cargo test --workspace` — every existing compiled-and-
   run example (plan 08's `Point`, plan 09's collections, plan 25's
   `Hash[K, V]`, plan 38's exception example) still passes unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`,
  `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. new runtime-helper unit tests | agent-claimed-locally |

---

## Leaf: leaf-codegen-try-and-match

### 1. Context
- Why: this is the leaf that makes the plan's own worked example
  actually run — `?`'s early-return propagation and the top-level
  `Ok(v)`/`Err(e)` match.
- Target state: `build_stmt` gains dedicated `Stmt::Let { value:
  Expr::Try(inner), .. }` and `Stmt::Assign { value: Expr::Try(inner),
  .. }` arms (positioned before their generic counterparts, exactly
  where `Expr::ArrayNew`'s own dedicated arm already sits — see
  Decision log): build `inner` to get the `Result` pointer; load the
  `i64` discriminant at offset `0`; `build_conditional_branch` on
  `== 0`; on the `Ok` side, `field_ptr` to offset `8`, `build_load`
  using the *target's* own already-known `ValKind` (the `Let`'s
  declared `ty` via `value_kind_for_type`, or the `Assign` target's
  existing stored `ValKind` from `vars`), and `build_store` into the
  target's pre-allocated slot; on the `Err` side, `builder.build_
  return(Some(&result_ptr))` — the exact same call `Stmt::Return(Some
  (e))` already uses (`lib.rs:2908-2919`) — immediately followed by
  `build_unreachable`. `build_stmt` also gains a `Stmt::MatchResult`
  arm reusing `build_case`'s `arm_blk`/`next_check_blk` chaining
  idiom: load the discriminant, branch to an `ok_blk` (loads the
  payload at the scrutinee's own `T`'s `ValKind`, binds it to a fresh
  alloca for `ok_var`, then `build_block(ok_body)`) or an `err_blk`
  (the `E`-typed analogue for `err_var`/`err_body`).

### 2. Acceptance Criteria
1. This plan's own worked example, compiled, linked, and run with
   `try_parse("21")`, prints `42` — the full good-input trace (Ok
   construction, `?` unwrap, re-wrap, top-level match).
2. The same program with `try_parse("abc")` prints `not a number` —
   the full bad-input trace, proving `?` genuinely short-circuits
   `try_parse` (the "never runs `return Ok(n * 2)`" claim in this
   plan's own worked-example trace) rather than merely type-checking.
3. A direct unit test on the `Err` propagation path confirms the
   pointer identity claim in the Decision log: the `Result` value
   returned by the *outer* function via `?` is bit-identical (same
   allocated address, or, at minimum, same discriminant+payload bytes)
   to the one produced inside the *inner* function that first
   constructed the `Err` — proving no copy/reconstruction happened.
4. Regression: `cargo test --workspace` passes in full, including
   every existing compiled-and-run example.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test (real linked-and-run proof) | `cargo test --workspace` | all pass, incl. `try_parse("21")` printing `42` and `try_parse("abc")` printing `not a number` | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
