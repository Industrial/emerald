---
name: Design-by-Contract (Function-Level Pre/Post-Conditions)
overview: "Eiffel-style `requires`/`ensures` clauses on a top-level function's own signature — checked statically only for the narrow, literal-substitution-provable case, otherwise compiled to a runtime check at function entry (`requires`) and at every return point (`ensures`), raising a real, synthetic `ContractViolation` exception via plan 38's already-landed handler-stack mechanism. Explicitly NOT a refinement-type system and NOT SMT-backed verification — see this plan's own Decision log, stated prominently, not as a footnote."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-parser-contracts
    content: "Contract { expr: Spanned<Expr>, text: String }; Function gains requires: Vec<Contract>, ensures: Vec<Contract>; grammar grows `requires`/`ensures` reserved keywords and a `ContractClause*` production between `-> TypeName` and `Stmt*` in FuncDef; text captured via a parse_named-time source-slice pass mirroring plan 47's own @L/@R loc technique"
    status: pending
  - id: leaf-sema-contract-typecheck
    content: "check_function_body type-checks each requires clause against params-only env (must resolve Type::Boolean), each ensures clause against params + a pseudo result: declared_return binding; requires/ensures rejected on class methods and on type_params-bearing generic functions with real diagnostics, not silently ignored"
    status: pending
  - id: leaf-sema-static-provability
    content: "eval_const_bool: a small, new, purpose-built literal-only evaluator (verified: no general constant-folding function exists anywhere in emerald-sema or emerald-codegen today) run at call sites only for requires clauses whose every free parameter-identifier is bound, at that call site, to a bare Int64/Float64/String/Bool literal argument — folds to Some(false) is a real compile-time diagnostic, anything else silently defers to the runtime check"
    status: pending
  - id: leaf-codegen-contracts
    content: "Synthetic ContractViolation class (message: String) injected into the parsed Program whenever it contains a requires/ensures usage, mirroring plan 47's AssertionError precedent exactly; requires-check codegen at function entry (right after bind_params, before the user body); ensures-check codegen at Stmt::Return's codegen site and at the implicit-return fallthrough path; both raise via the exact, unmodified emerald_raise/class_tags/ExceptionRuntimeFuncs plumbing plan 38 already built — no runtime/emerald_runtime.c changes"
    status: pending
isProject: false
---

# Plan 62 — Design-by-Contract (Function-Level Pre/Post-Conditions)

This is plan 62 of the 58–64 batch. A prior debate asked, deliberately
ignoring maturity/ecosystem/stability: what is Emerald's theoretical
technical ceiling against C/C++/Rust/Python/Ruby/TypeScript/Go/Haskell/
Elixir combined? That debate's concrete finding was that Eiffel-style
design-by-contract — `requires`/`ensures` clauses living directly on a
function's own signature, checked by the compiler rather than hand-
written as an `if raise` at the top of a body — is the one feature none
of those nine languages offers as a first-class citizen (Python/Ruby can
approximate it with decorators or gems; none compile it in). This plan
is the direct, disclosed follow-up to that finding. Like plans 36–47
before it, this is post-v1 scope and is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md);
that document is updated separately, once, after every plan in the
58–64 batch is authored — this plan does not touch it or any other plan
file. This plan owns exactly one gap: function-level contracts. It does
not attempt, and explicitly declines (see Decision log), the two much
larger features that "design-by-contract" could otherwise expand into.

**Depends on: plan 38** (`full-exception-model` —
`2026-09-09T104000Z-plan-38-full-exception-model.md`, already landed in
current source: `crates/emerald-codegen/src/lib.rs`'s `ExceptionRuntimeFuncs`
struct, `declare_exception_runtime_funcs`, and `Ctx.class_tags`/
`Ctx.rescue_tag_sets` are real, verified-this-session, currently-compiling
code, not a still-hypothetical design) — this plan's `ContractViolation`
is an ordinary raised-and-rescued exception, riding that exact,
unmodified `emerald_raise(tag: i64, ptr)` / handler-stack mechanism.
**Depends on: plan 41** (`interfaces-and-generics` —
`2026-09-09T107000Z-plan-41-interfaces-and-generics.md`, also already
landed: `Function.type_params: Vec<TypeParam>` and interface/`implements`
machinery are real, current AST/sema surface) — cited here specifically
to draw the line this plan's Decision log draws explicitly: a bound like
`T: Comparable` is a **type-level** constraint (checked once, structurally,
against a class's declared method table, with zero runtime cost and zero
connection to any specific *value*); a contract is a **value-level**
predicate (checked against the actual arguments a specific call passes,
either by a narrow compile-time literal-substitution proof or by a real
runtime branch). The two mechanisms don't overlap and this plan builds
neither on top of the other.

## CRITICAL SCOPING DECISION — stated here, prominently, not as a footnote

**This plan builds function-level contracts only.** It does **not** build:

1. **A refinement-type system.** `type PositiveInt = Int64 where self > 0`
   — a genuinely distinct nominal type that propagates through arithmetic
   and is usable everywhere `Int64` is — is a much larger, separate
   type-system feature. It would need a new `Type` variant, a subtyping/
   coercion rule interacting with every arithmetic operator's existing
   type-checking (`check_numeric_binop`, `resolve_type`, `is_assignable`,
   all verified against real `emerald-sema/src/lib.rs` this session), and
   a decision about whether refined-ness survives being stored in a
   field, passed through a generic, or returned from a lambda. None of
   that is attempted here. It is real, disclosed, permanent future work,
   not an oversight this plan works around.
2. **SMT-solver-backed full static verification** (à la SPARK/Dafny/
   Why3). This plan's "static" checking (`leaf-sema-static-provability`)
   is limited to one narrow, concrete, enumerable case: a call site
   passing a bare literal argument that substitutes directly into the
   contract expression and constant-folds to `false`. It is not a
   theorem prover, has no notion of loop invariants, symbolic ranges, or
   path-sensitive reasoning, and does not attempt to prove a contract
   *holds* — only, in one narrow case, that it provably does not.

Concrete proof this plan targets — the primary program (`contracts.em`)
compiles, links, and runs, exercising proof points (a) and (c); a second,
separate snippet (`contracts_reject.em`) demonstrates proof point (b),
which by definition cannot share a successful compilation with (a)/(c):

```ruby
# contracts.em
def divide(a: Int64, b: Int64) -> Int64
  requires b != 0
  ensures result * b <= a
  return a / b
end

y: Int64 = 10
z: Int64 = 2
puts divide(y, z)

x: Int64 = 3
w: Int64 = x - 3
begin
  puts divide(20, w)
rescue ContractViolation => e
  puts e.message
end
```

Expected stdout:
```
5
contract violation: `divide`'s requires `b != 0` failed (declared at contracts.em:2)
```

Trace: (a) `divide(y, z)` — `z` is `2`, a value only known at run time
from this compiler's point of view (it flows through a `Let` local, not
a literal argument at the call site) — `requires b != 0` and `ensures
result * b <= a` (`5*2=10 <= 10`) both hold; the call succeeds normally
and prints `5`. (c) `w` is computed at run time (`x - 3`, `x` itself a
runtime local) and happens to evaluate to `0`; `divide(20, w)`'s call
site has no literal argument for sema's static check to substitute (see
`leaf-sema-static-provability`'s exact boundary below), so it compiles
cleanly — the violation surfaces only when the compiled program actually
runs, `requires b != 0` fails at function entry, `ContractViolation` is
raised, propagates via plan 38's real handler-stack `longjmp` to the
enclosing `begin`'s `rescue ContractViolation => e`, and `e.message` is
printed.

Second snippet, proof point (b) — a literal `0` argument, REJECTED AT
COMPILE TIME, never producing a binary:

```ruby
# contracts_reject.em
def divide(a: Int64, b: Int64) -> Int64
  requires b != 0
  return a / b
end

puts divide(10, 0)
```

Expected compiler output (non-zero exit, no object file emitted):
```
error: contract violation provable at compile time: `divide`'s requires `b != 0` is false for the literal arguments given at this call site (b = 0) [contracts_reject.em:6]
```

## Decision log

- **Attachment point: verified against the real, current `FuncDef`
  grammar production** (`crates/emerald-parser/src/grammar.lalrpop`):
  ```
  FuncDef: Function = {
    "def" <name:Ident> <type_params:TypeParamClause?> <parts:ParenParams>
      "->" <return_type:TypeName> <body:Stmt*> "end" => Function { ... }
  };
  ```
  and the real, current `Function` struct (`crates/emerald-parser/src/
  ast.rs:477-508`): `name, params, return_type, body, block_param,
  splat_param, type_params` — no contract-shaped field exists today.
  `requires`/`ensures` clauses attach in the one open slot this
  production has: between `<return_type:TypeName>` and `<body:Stmt*>`,
  via a new `<contracts:ContractClause*>`. This is the same "reserve a
  new keyword the same LALR(1) way `class`/`def`/`interface`/`actor`
  already are" idiom the grammar file's own comments describe for every
  prior reserved word — `requires`/`ensures` are simply two more, and
  they occupy a position (`-> TypeName` has just been consumed, `Stmt*`/
  `end` haven't started) with no FIRST-set overlap to disambiguate.
  Function struct gains `pub requires: Vec<Contract>, pub ensures:
  Vec<Contract>` — empty `Vec` for every function that doesn't declare
  one, additive and source-compatible with every prior plan, matching
  `type_params`' own precedent exactly.
- **`Contract` is a new, small struct — not a bare `Spanned<Expr>` —
  because the runtime violation message needs the clause's actual source
  text, and this codebase has no unparser.** Verified this session (and
  already disclosed by plan 47's own Decision log): "No unparser/pretty-
  printer exists for this AST... nothing in `emerald-parser` renders
  `Expr`/`Stmt` back to source." `Contract { expr: Spanned<Expr>, text:
  String }` — `text` is populated by a small, targeted post-parse pass
  over the freshly-built `Program`, slicing `source[expr.span.0..
  expr.span.1]` — the *exact* technique plan 47 already established for
  its own `loc` (`file:line`) capture ("a small, targeted post-parse
  pass... not a generic `Spanned` walk"). This plan's pass additionally
  computes the clause's declaration-site `file:line` the same way plan
  47's `loc` does (counting `\n` bytes in `source[..offset]`), giving a
  real "declared at `contracts.em:2`" string with zero new general
  source-position machinery — plan 22's own general `Spanned<T>`-
  everywhere overhaul (still `status: pending`) is not a prerequisite,
  matching plan 47's identical reasoning for declining it.
- **`result` needs no grammar change at all — it is an ordinary
  `Expr::Ident("result")`, special-cased only in sema's `env`
  construction while checking an `ensures` clause.** This is the
  "grammar stays general, sema narrows" discipline plan 31's Decision
  log already established for its own restricted-shape leaves, applied
  here for the opposite reason (adding meaning to an existing shape
  rather than restricting one). Concretely: `check_function_body`
  (verified, `crates/emerald-sema/src/lib.rs:4215-4261`) already builds
  `env` from `f.params` before checking anything; this plan's sema leaf
  clones that `env`, inserts one extra entry — `"result" ->
  declared_return` — and type-checks each `ensures` clause's `expr`
  against *that* clone via the existing, unmodified `infer_expr_type`,
  requiring `Type::Boolean` back, the same requirement `requires`
  clauses get against the unmodified params-only `env`. **A real,
  disclosed edge case, deliberately not special-cased further:** a
  function with a parameter literally named `result` would have that
  binding shadowed by the pseudo-`result` while its `ensures` clauses
  are checked. No evidence anywhere in this project's eight prior
  worked examples that reference a parameter named `result` suggests
  this matters in practice; this plan treats it the same way plan 38
  treated a bare `rescue`'s unbindable `e` — a named, disclosed
  narrowing, not a new reserved-word diagnostic invented for a case with
  no observed cost.
- **`old(x)` (a pre-call snapshot of a parameter's value, for an
  `ensures` clause to compare against) is explicitly declined, not
  silently omitted.** It is a real, larger feature: unlike `result`
  (which needs one new `env` entry, populated from a value codegen
  already has in hand at the return site), `old(x)` needs codegen to
  capture and preserve `x`'s *pre-call* value across the entire
  function body's execution — a second, shadow local per contract-
  referenced parameter, allocated at function entry before the body (or
  a mutation to it) can run, alive until the matching `ensures` check.
  That is a real, tractable feature, but a distinct one from this plan's
  "one new `env` entry" `result` mechanism, and not required to prove
  function-level contracts work — declined here, not attempted narrowly.
- **Static provability, concretely, and the honest correction of this
  plan's own starting assumption:** the task brief for this plan asked
  to "reuse whatever constant-folding the compiler already has." Verified
  this session, directly: a full function-signature scan of both
  `crates/emerald-sema/src/lib.rs` (7514 lines) and `crates/emerald-
  codegen/src/lib.rs` (13964 lines) turns up nothing named `fold`,
  `const_eval`, `eval_const`, or `try_eval` anywhere in either crate —
  **no general constant-folding mechanism exists in this compiler
  today.** The one real, existing precedent this plan *can* reuse is
  narrower: plan 39's `DefaultLit` grammar production (`crates/emerald-
  parser/src/grammar.lalrpop`), which already restricts a parameter
  default to exactly `Expr::Int | Float | StringLit | Bool | Nil` —
  literal tokens, never an arbitrary expression, never a reference to
  another parameter (verified, `FuncParam`'s own comment: "`DefaultLit`
  admits only literal tokens... a default referencing another parameter
  is a parse error here, at the grammar level"). That is this codebase's
  own working definition of "compile-time-evaluable," and this plan's
  `eval_const_bool` reuses exactly that definition rather than inventing
  a broader one: it only ever substitutes a call-site argument into a
  `requires` clause when that argument is, itself, a bare `Expr::Int`/
  `Float`/`StringLit`/`Bool` literal — never a `Let`-bound local, never a
  computed expression, no matter how "obviously" constant a human reader
  might judge it. (LLVM's own backend will separately, silently constant-
  fold arithmetic at the IR level as an ordinary optimizer pass — that is
  real but orthogonal: it can shrink emitted code, but it never produces
  a *source-level diagnostic* or refuses to emit a binary, which is the
  entire point of this leaf.)
- **`eval_const_bool`'s enumerated scope — catches vs. does not catch,
  stated exhaustively, not just by example.** Catches: every `requires`
  clause whose free parameter-identifiers are *all* bound, at a specific
  call site, to bare literal arguments, evaluated through a small,
  closed evaluator over `Expr::Int/Float/StringLit/Bool`,
  `Expr::Compare` (`== != < <= > >=`), `Expr::And/Or/Not`, and
  `Expr::Add/Sub/Mul/Div/Rem/Neg` — e.g. `divide(5, 0)` against
  `requires b != 0` (`0 != 0` folds to `false`), or `requires a > b`
  against `check(3, 10)` (`3 > 10` folds to `false`). Does **not**
  catch, and this plan does not pretend to: `divide(5, x)` where `x` is
  a `Let`-bound local whose value happens to always be `0` at run time
  (even if a human — or a more sophisticated future dataflow pass —
  could prove it); a literal argument combined with a *non*-literal one
  in the same clause (`requires a + b > 0` called as `f(5, y)` — `a` is
  literal, `b` isn't, so the whole clause is skipped, not partially
  folded); any clause whose predicate depends on a field, a method call,
  or an array/hash lookup, all of which are outside `eval_const_bool`'s
  closed grammar by construction. Every one of these silently defers to
  the runtime check — never a false compile-time accept, and never a
  panic; the worst case is simply "not caught until run time," which is
  always still caught.
- **Contrast with plan 41's `T: Comparable`, spelled out because both
  mechanisms sit in the same "checked, not duck-typed" territory but do
  genuinely different jobs.** Verified against plan 41's own, already-
  landed source: an interface bound is a **type-level, structural**
  check — `check_interface_conformance` (`crates/emerald-sema/src/
  lib.rs:5234-5279`) verifies once, at class-declaration time, that a
  class's method table matches an interface's required signature; it
  never inspects an actual runtime value and costs nothing at either a
  specific call site or run time (monomorphization emits a direct call,
  full stop). A contract is a **value-level, per-call** check — it says
  nothing about `Int64`'s type-level shape and everything about whether
  *this specific argument value* satisfies a predicate; it either
  resolves at compile time via the narrow literal-substitution proof
  above, or it costs a real branch and (on failure) a real raised
  exception at run time. A generic function bounded by an interface
  (`max[T: Comparable]`) and a function with a contract (`divide`
  `requires b != 0`) can coexist in the same program without
  interacting at all — this plan does not attempt to let a `requires`
  clause reference a generic type parameter's own bound-derived methods,
  and see the next bullet for why type-parameterized functions are out
  of v1 scope entirely.
- **Scoped to top-level, non-generic functions only — not class methods,
  not `type_params`-bearing generic functions.** Methods: `emerald-sema`
  already has a real, disclosed precedent for narrowing a feature to
  free functions only and rejecting it on methods with a diagnostic
  rather than silently ignoring it — verified, `check_method_body`
  (`crates/emerald-sema/src/lib.rs:4263-4340`) already does exactly this
  for default parameter values, splat parameters, and tuple return
  types ("default parameter values are not supported on methods yet"),
  each a real `Err(Diagnostic)`, not a panic. This plan's `leaf-sema-
  contract-typecheck` adds a fourth: a method (or module function) with
  a non-empty `requires`/`ensures` is rejected the same way. Generic
  functions: `type_params`-bearing functions are compiled via whole-
  program monomorphization (plan 41's Decision log, verified) — a
  `requires`/`ensures` clause referencing a type parameter's own bound
  method (e.g. `requires a.compare_to(b) >= 0` inside `max[T:
  Comparable]`) is a real, coherent, larger feature (each monomorphized
  specialization would need its own re-checked, re-compiled contract
  instance), genuinely additive on top of this plan rather than a
  precondition for it — declined here, real disclosed future work, not
  attempted narrowly.
- **`requires` is checked at call sites (only for the literal-provable
  case); `ensures` never is.** A `requires` clause is a property of the
  *arguments a caller supplies* — a caller-visible thing sema can, in
  the narrow literal case, inspect before the call ever runs. An
  `ensures` clause is a property of the *callee's own computation*
  (`result`, derived from the callee's body) — there is nothing at a
  call site for sema to substitute into it at all; it is purely a
  runtime-checked invariant on the function's own implementation.
- **Multiple `requires` (or multiple `ensures`) clauses on one function
  compose as an implicit AND**, checked/raised in source declaration
  order — the same "first thing that's wrong wins" behavior `check_args`
  and every other multi-condition checker in this codebase already
  exhibits, not a new composition rule.
- **`ContractViolation` is a synthetic class the compiler injects into
  the parsed `Program`, never user-declared — the exact mechanism plan
  47 already built and named for `AssertionError`.** Verified against
  plan 47's Decision log: "A synthetic `AssertionError` class (`message:
  String`, one `initialize`) is injected into the parsed `Program`...
  whenever it contains a `test`/`assert`/`assert_eq` usage — the user
  never declares it... mirrors an existing pattern in this codebase:
  `define_main` already synthesizes structure... around user code that
  never declares it itself." This plan's `ContractViolation` (`message:
  String`, one `initialize`) is injected whenever the `Program` contains
  any non-empty `requires`/`ensures` — same shape, same injection
  timing, deliberately a *distinct* class from `AssertionError` (not a
  reuse of it) because a contract violation is a language-level
  correctness failure, not a test-framework assertion failure, and
  keeping them distinct lets a `rescue AssertionError` and a `rescue
  ContractViolation` be told apart by a program that legitimately wants
  to handle them differently (e.g. a test runner that treats one as a
  test failure and the other as a bug it should not silently swallow).
- **Runtime enforcement reuses plan 38's raise plumbing completely
  unmodified — no `runtime/emerald_runtime.c` changes.** Verified: `Ctx.
  exc_funcs: ExceptionRuntimeFuncs` (`crates/emerald-codegen/src/
  lib.rs:2033-2042, 2646-2828`) already exposes `raise: FunctionValue`
  bound to the real, external `emerald_raise(tag: i64, ptr)` symbol, and
  `Ctx.class_tags: &HashMap<String, i64>` already assigns every declared
  class (in declaration order) a stable integer tag purely from the
  class's presence in the compiled `Program` — since `ContractViolation`
  is injected into that same `Program` before class-tag assignment runs
  (matching `AssertionError`'s own injection timing), it gets a real tag
  for free, with zero new runtime C code and zero new codegen plumbing
  beyond what `Stmt::Raise`'s own existing codegen path already does for
  every other raised class today (construct the instance, call
  `emerald_raise` with its tag and pointer). A `rescue ContractViolation
  => e` clause is, from the compiler's point of view, ordinary,
  unmodified plan-38 `rescue` codegen — it needs no special case at all.
- **Two fixed codegen injection sites per function — not a stack, unlike
  plan 38's `ensure_stack`.** Plan 38's `ensure` needed a stack
  (`ensure_stack: &mut Vec<...>` threaded through `build_stmt`/
  `build_block`) because a `begin`/`ensure` construct can nest inside
  another, and each exit must run only its own innermost enclosing
  `ensure` bodies, in order. A function's own `requires`/`ensures` are
  not nested at all — exactly one function is being compiled at a time,
  so this plan needs no stack: `requires` codegen is emitted exactly
  once, immediately after `bind_params` runs and before the user body's
  first statement compiles (the same "known, fixed point in `build_
  function_body`" every function already has); `ensures` codegen is
  emitted at exactly two fixed sites reused from existing, unmodified
  code paths — the codegen arm that already handles `Stmt::Return`
  (wherever in the body it textually occurs, however deeply nested
  inside `if`/`while`/`case` — that arm already runs once per `Return`,
  regardless of nesting) and the codegen path that already handles a
  function's implicit-return-at-block-end fallthrough (`check_
  implicit_return`'s codegen-side counterpart). Both sites already know
  the function's own declared return kind (needed today to emit a
  correctly-typed `ret` instruction) — this plan's addition is: bind
  that same about-to-be-returned value to a local under the name
  `result`, evaluate each `ensures` clause's `expr` against it via the
  ordinary `build_expr` path, and branch to a `ContractViolation`-raise
  block on any `false` before the real `ret` executes.
- **The violation message names the function and the contract's own
  declaration site — deliberately not the call site.** `Contract.text`
  (source-sliced, see above) plus the enclosing function's `Function.
  name` plus the clause's own `file:line` (computed the same way plan
  47's `loc` is) together produce, e.g., `` contract violation: `divide`'s
  requires `b != 0` failed (declared at contracts.em:2) ``. Naming the
  *call* site instead would need a hidden, propagated caller-location
  argument threaded through every call in the program — a calling-
  convention change with project-wide reach, wildly disproportionate to
  one plan in this batch. This is a real, disclosed narrowing versus a
  hypothetical fuller design: the exception itself is still an ordinary,
  fully catchable value, so a caller that wants call-site context can
  already get it the ordinary way — by wrapping the call in its own
  `begin`/`rescue`, exactly as this plan's own worked example does.

## Leaf: leaf-ast-parser-contracts

### 1. Context
- Why: no grammar shape exists today for a clause attached to a function
  signature at all (verified this session against the real, current
  `FuncDef` production — see Decision log).
- Target state: `Contract { expr: Spanned<Expr>, text: String }` in
  `crates/emerald-parser/src/ast.rs`; `Function` gains `requires:
  Vec<Contract>, ensures: Vec<Contract>`; `grammar.lalrpop` gains
  `"requires"`/`"ensures"` reserved terminals and a `ContractClause*`
  production consumed between `-> TypeName` and `Stmt*` in `FuncDef`
  only (not `MethodDef` — methods reject a non-empty list in sema, per
  the Decision log, so there is no need to make the grammar accept it
  there at all — a stricter, simpler cut than `type_params`' own
  "grammatically reachable on methods, sema-rejected" precedent);
  `emerald_parser::parse_named` gains a small, targeted post-parse pass
  (mirroring plan 47's `loc` pass) that fills in each `Contract.text`
  and stores its computed `file:line` alongside it (either as a second
  field, e.g. `line: usize`, or folded directly into `text`'s own
  format — implementer's call, either is a small, self-contained
  addition).

### 2. Acceptance Criteria
1. `def divide(a: Int64, b: Int64) -> Int64 requires b != 0 ensures
   result * b <= a return a / b end` parses to a `Function` with
   `requires.len() == 1` (`expr` structurally equal to `Expr::Compare
   (Ident("b"), Ne, Int(0))`) and `ensures.len() == 1`, `text` equal to
   the clause's real source substring (`"b != 0"`, `"result * b <= a"`).
2. A function declaring neither clause parses identically to before this
   leaf — `requires`/`ensures` both empty `Vec`s, zero regression on any
   prior plan's worked example.
3. `requires`/`ensures` are reserved the same way `class`/`def`/
   `interface` already are — a program using `requires` or `ensures` as
   an ordinary variable/parameter name is a real parse error, not
   silently accepted.
4. A `MethodDef` (inside `class`/`module`/`actor`) does not accept a
   `requires`/`ensures` clause at the grammar level at all — a parse
   error, not a sema diagnostic, for this leaf's own narrower cut (see
   Context above).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (the `parse_named` post-parse pass
  and its tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new contract-parsing and `text`-capture tests | agent-claimed-locally |

---

## Leaf: leaf-sema-contract-typecheck

### 1. Context
- Why: parsed `requires`/`ensures` expressions are not yet type-checked
  against anything — a clause referencing an undeclared name or
  resolving to a non-`Boolean` type must be a real diagnostic, not
  silently accepted or a panic.
- Target state: `check_function_body` (`crates/emerald-sema/src/
  lib.rs:4215-4261`) type-checks each `f.requires[i].expr` against the
  existing params-only `env` (built exactly as it is today, unmodified)
  via the existing, unmodified `infer_expr_type`, requiring `Type::
  Boolean` back — a non-Boolean result (e.g. `requires b` where `b:
  Int64`) is a real diagnostic naming the clause. Each `f.ensures[i].
  expr` is checked against a *clone* of that `env` with one extra entry,
  `"result" -> declared_return`, inserted first (see Decision log) —
  same `Type::Boolean` requirement. `check_method_body` (`lib.rs:4263-
  4340`) gains the same "not supported on methods yet" diagnostic
  pattern it already uses for default parameters/splats/tuple returns,
  triggered when `m.requires`/`m.ensures` is non-empty (defense in depth
  even though `leaf-ast-parser-contracts` already makes this
  grammatically unreachable). A `type_params`-bearing function
  (`f.type_params.is_empty() == false`) with a non-empty `requires`/
  `ensures` is likewise a real, named diagnostic — "contracts are not
  supported on generic functions yet."

### 2. Acceptance Criteria
1. `divide` (this plan's worked example) type-checks `Ok(())`.
2. `def bad(a: Int64) -> Int64 requires a return a end` (a non-Boolean
   `requires` expression) is rejected with a diagnostic naming the
   clause and its actual inferred type, not a panic.
3. `def bad2(a: Int64) -> Int64 ensures unknown_name return a end` (an
   `ensures` clause referencing an undeclared identifier) is rejected
   with the existing, unmodified undefined-variable diagnostic.
4. `def ok(a: Int64) -> Int64 ensures result >= 0 return a end` type-
   checks `Ok(())` — proving `result` resolves to the function's own
   declared return type inside `ensures` only.
5. A `requires`/`ensures` clause attached (via a hand-constructed AST,
   since the grammar leaf already blocks the source-level path) to a
   class method is rejected with the "not supported on methods yet"
   diagnostic, mirroring the existing default-parameter-on-methods case
   exactly.
6. A `requires`/`ensures` clause on a `type_params`-bearing function is
   rejected with the "not supported on generic functions yet"
   diagnostic.
7. Regression: every prior plan's example still type-checks identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-sema-static-provability

### 1. Context
- Why: the batch's headline claim for this plan is that *some* contract
  violations are caught before a binary is ever produced — a real,
  narrow mechanism, not a theorem prover (see Decision log's exhaustive
  catches/does-not-catch enumeration).
- Target state: `eval_const_bool(expr: &Expr, bindings: &HashMap<&str,
  &Expr>) -> Option<bool>` — a small, new, purpose-built evaluator (see
  Decision log: no existing constant-folding mechanism was found to
  reuse) over exactly `Int/Float/StringLit/Bool` literals, `Compare`,
  `And/Or/Not`, `Add/Sub/Mul/Div/Rem/Neg`; returns `None` the instant it
  hits anything outside that closed grammar (an `Ident` not present in
  `bindings`, a method/field/index access, etc.) — never a panic, never
  a guessed answer. Wired in at every ordinary call-site check (wherever
  `check_call_args`/`check_args` already validate an `Expr::Call`'s
  arguments against a callee's `FunctionSig`, `crates/emerald-sema/src/
  lib.rs:2520-2587` and `:2471-2509`): after the existing arity/type
  checks pass, for a callee whose `FunctionSig` carries a non-empty
  `requires` list (`FunctionSig` gains a `requires: Vec<Contract>`
  field, populated by `function_signature`, `lib.rs:452-490`, from the
  same `Function.requires` this plan's other leaves already parse and
  type-check), build `bindings` from exactly the call's literal-typed
  argument expressions (positional-name matched against `param_names`,
  already threaded through `FunctionSig` for keyword-argument resolution
  — reused unchanged here), and call `eval_const_bool` on each `requires`
  clause. `Some(false)` is a real `Diagnostic`, using the call
  expression's own span (matching plan 22's own "arity diagnostic's span
  is the whole call expression" precedent, `check_call_args`'s doc
  comment, reused for the same reason here — a contract violation is a
  property of the whole call, not one argument). `Some(true)` or `None`:
  no diagnostic, proceed exactly as before this leaf.

### 2. Acceptance Criteria
1. `divide(10, 0)` (both arguments literal) against `requires b != 0` is
   rejected at compile time with a diagnostic naming the function, the
   clause's own text, and the offending literal value — the exact
   `contracts_reject.em` proof point.
2. `divide(y, z)` where `y`/`z` are `Let`-bound locals (this plan's
   primary worked example) type-checks `Ok(())` at this call site — no
   compile-time rejection, even though both happen to be initialized
   from literals one line earlier; `eval_const_bool` never looks past
   the call site's own argument expressions.
3. `divide(20, w)` where `w: Int64 = x - 3` (a computed, non-literal
   expression) type-checks `Ok(())` at this call site — proving the
   check is skipped, not incorrectly evaluated, the moment an argument
   isn't a bare literal.
4. `f(5, y)` against a hypothetical `requires a + b > 0` (one literal
   argument, one non-literal) type-checks `Ok(())` — proving the whole
   clause is skipped rather than partially folded, per the Decision
   log's explicit enumeration.
5. Regression: every prior plan's call-site example still type-checks
   identically — this leaf only *adds* a rejection path for a genuinely
   new, always-false case; it never changes an existing `Ok(())` result
   to `Err`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. the literal-vs-non-literal boundary tests above | agent-claimed-locally |

---

## Leaf: leaf-codegen-contracts

### 1. Context
- Why: a contract that survives sema unrejected still needs to do
  something at run time — this leaf is where the majority of contracts
  (anything not literally provable false, which is most of them, by
  design) actually get enforced.
- Target state: `ContractViolation` (`message: String`, one
  `initialize`) injected into the `Program` exactly like plan 47's
  `AssertionError`, whenever `requires`/`ensures` is non-empty anywhere
  in the compiled program. `build_function_body` (the pipeline plan 41's
  Decision log already names — `bind_params`, then the user body) gains
  one fixed injection point immediately after `bind_params`: for each
  `f.requires[i]`, `build_expr` the clause against the just-bound
  parameter values; on `false`, construct a `ContractViolation` instance
  (`message` built from `Contract.text` + `f.name` + declared `file:
  line`, per the Decision log's exact format) and call `ctx.exc_funcs.
  raise` with `class_tags["ContractViolation"]`, then `build_unreachable`
  — the identical shape `Stmt::Raise`'s own existing codegen already
  uses for every user-written `raise`. `Stmt::Return`'s existing codegen
  arm, and the existing implicit-return-fallthrough codegen path, each
  gain the mirror-image check for `f.ensures`: bind the about-to-be-
  returned value under the name `result` in a scratch `vars` entry,
  `build_expr` each `ensures` clause against `vars` (now including
  `result`), same false-branch raise shape, only *then* emit the real
  `ret`.

### 2. Acceptance Criteria
1. `contracts.em` (this plan's own worked example), compiled, linked,
   and run, prints exactly:
   ```
   5
   contract violation: `divide`'s requires `b != 0` failed (declared at contracts.em:2)
   ```
   — real, executed proof of proof points (a) and (c) together: a
   runtime-only-determinable divisor succeeding normally, and a
   runtime-only-determinable zero divisor raising a real exception
   caught by an ordinary `rescue`.
2. A direct call to `divide` with a non-zero divisor and a body that
   violates `ensures` (e.g. a deliberately-wrong `divide2` returning
   `a / b + 1`) raises `ContractViolation` at the `ensures` site, not the
   `requires` site — proving the two checks are wired to their own,
   correct injection points independently.
3. `ContractViolation` is never emitted into the compiled module at all
   for a program containing no `requires`/`ensures` usage anywhere —
   zero-cost, zero-regression for every prior plan's existing example
   (verified by confirming those examples' compiled output is
   byte-identical before/after this leaf, the same regression bar plan
   47 already set for its own `AssertionError` injection).
4. `runtime/emerald_runtime.c` is unmodified by this leaf — the whole
   feature rides plan 38's existing `emerald_push_handler`/`setjmp`/
   `emerald_raise`/`emerald_pop_handler` C surface unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`
- **Do not modify:** `runtime/emerald_runtime.c` (see Decision log and
  AC4)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test (real linked-and-run trace) | `cargo test --workspace` | all pass, incl. `contracts.em`'s real compiled-and-run output matching the exact trace above | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
