---
name: Ranges and Range-Based Iteration
overview: "`a..b` (inclusive) and `a...b` (exclusive) Range syntax over `Int64` endpoints, legal only as a `for...in` scrutinee — extends plan 30's for-in with a second iteration source that lowers into the exact same index-based `while` codegen shape plan 30 already proved, with no first-class `Range`/`Range[T]` value, no `Range#each`, no endless ranges, and no array slicing."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-range
    content: "Stmt::ForRange { var, start, end, exclusive, body } — new `..`/`...` grammar tokens, restricted at the grammar level to a `for...in` scrutinee only, mirroring plan 30's own array-literal-only restriction on `Stmt::For`"
    status: pending
  - id: leaf-sema-range
    content: "Type-check `start`/`end` as Int64 (reject any other type by name), bind `var: Int64` for `body`, thread `in_loop = true` exactly as `Stmt::For`'s existing arm does"
    status: pending
  - id: leaf-codegen-range
    content: "Desugar to the same idx_alloca/cond_blk/body_blk/incr_blk/exit_blk loop skeleton and `LoopTargets` push/pop `build_for` (plan 30) already established, skipping array materialization and per-element GEP/load entirely since a Range's elements are the loop index itself"
    status: pending
isProject: false
---

# Plan 37 — Ranges and Range-Based Iteration

This is plan 37 of the 36-47 follow-up batch — twelve independent sibling
plans whose combined job is closing Emerald's language/stdlib surface
toward its ~45% Ruby-parity ceiling (a prior analysis put plans 01-35's
shipped surface at roughly 10-15% of standard Ruby) without conceding
any of Emerald's identity constraints: no `method_missing`/`eval`/`send`/
reflection, no mixins/open classes/monkey-patching, no dynamic/virtual
dispatch, no tracing GC. Each of the twelve plans owns one distinct,
disjoint gap. Like the 28-35 batch before it, this is post-v1 scope and
is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
that table's own Completion note calls Emerald v1 done at row 15. This
plan does not touch `plan-of-plans.md` or any other plan file;
`plan-of-plans.md` itself is updated separately, once all twelve of
plans 36-47 are authored.

Depends on: plan 09 (collections/`Array[T]`, whose index-based codegen
shape this plan's own loop reuses one further level removed), plan 30
(for-in iteration — the literal `Stmt::For` desugaring this plan
extends with a second scrutinee kind), and plan 18 (arithmetic/
comparison operators — `Expr::Sub`/`Mul`/`Div`/`Rem` and `CompareOp`,
which make a Range endpoint a genuinely arbitrary `Int64` expression
like `n - 1`, not just an integer literal).

Concrete proof this plan targets:
```ruby
total: Int64 = 0
for i in 1..5
  total += i
end
puts total

total2: Int64 = 0
for i in 1...5
  total2 += i
end
puts total2
```
Expected output: `15` then `10` — the inclusive range sums `1+2+3+4+5`,
the exclusive range over the identical literal bounds sums `1+2+3+4`,
a real compiled-linked-run proof that `..` and `...` bind their upper
endpoint differently, not just that the syntax parses.

## Decision log

- **`Range` is not a first-class value — `..`/`...` are grammar-
  restricted to appear only directly after a `for <var> in` scrutinee,
  never as a standalone expression.** Verified this session against
  `crates/emerald-sema/src/lib.rs`'s actual `Type` enum (lines 12-38):
  `Int64, Float64, String, Boolean, Void, Nil, Class(String),
  Array(Box<Type>), Hash(Box<Type>, Box<Type>), Proc(Vec<Type>,
  Box<Type>)` — there is no `Range` variant, and adding one would touch
  every exhaustive match already written over `Type` across
  `emerald-sema` and `emerald-codegen`. `spec/GRAMMAR.md` §1 itself
  aspires to more ("Range literals... Statically typed as `Range[T]`
  where `T` is the endpoint type; container type is added to the v1
  universe") — this plan deliberately does not build that. It follows
  plan 30's own precedent instead: plan 30 restricted `for x in [...]`
  to a literal array scrutinee specifically so a non-literal use (`for
  x in arr`) is "a parse error, not deferred to sema" (plan 30's own
  Decision log,
  `history/2026-09-09T092000Z-plan-30-for-in-iteration.md`). This plan
  applies the identical narrowing to Ranges: `r: Range = 1..5`, `(1..5).each`,
  and any other standalone use of `..`/`...` is a parse error, because
  no `Expr::Range` node or `Range`-shaped `PrimaryExpr` alternative
  exists in the grammar at all — verified this session against
  `crates/emerald-parser/src/grammar.lalrpop`'s full `PrimaryExpr`
  production list (`Call`, `New`, `ArrayNew`, `MethodCall`, `ArrayLit`,
  lambda literal, hash literal — no Range alternative) and against
  `crates/emerald-parser/src/ast.rs`'s `Expr` enum (no `Range` variant
  among its sixteen cases). The "value shape" named in this plan's own
  scope (`{start: Int64, end: Int64, exclusive: Bool}`) therefore never
  exists as a runtime value or even a standalone AST node — it is three
  fields living directly on the new `Stmt::ForRange` statement, fully
  consumed by codegen at the one call site that needs them. This is a
  stricter reduction of surface than plan 30's own array-literal for-in
  already achieved (that one still allocates a real heap buffer to
  index into; this one allocates nothing).
- **`Range[T]` generality and float ranges are explicitly declined, not
  merely deferred with a TODO.** A generic `Range[T]` needs a generics
  mechanism this compiler does not have (`Type::Array`/`Type::Hash` are
  each one hand-written, non-generic container type — there is no
  parametric-type facility to hang a user-extensible `Range[T]` off
  of); that is real, disclosed future work for whichever of plans 36-47
  ends up owning generics (plan 41, by this batch's own numbering).
  Float ranges (`1.0..2.0`) are declined outright rather than deferred:
  a float range has no natural "next" successor without an explicit
  step, which is a materially different feature (Ruby itself only
  makes `Float` ranges usable via `Range#step`, not bare iteration) —
  building it would require the very `Range#each`/step-taking
  Enumerable mechanism this plan also declines below, not just a wider
  `Int64` check.
- **Codegen reuses plan 30's exact loop skeleton — this is the core
  claim that makes this plan a real reduction of new codegen surface,
  not a parallel implementation.** Verified this session against
  `crates/emerald-codegen/src/lib.rs`'s current `build_for` (lines
  2085-2220, the function `Stmt::For`'s codegen dispatches to): it
  builds `idx_alloca` (an `Int64` index starting at `0`), four basic
  blocks (`for.cond`, `for.body`, `for.incr`, `for.after`), pushes
  `LoopTargets { header: incr_blk, exit: exit_blk }` onto `loop_stack`
  before emitting `body` via `build_block` (so `break`/`next` resolve
  through the same mechanism every other loop uses), and increments
  `idx_alloca` in `for.incr` before branching back to `for.cond`. This
  plan's new `build_for_range` reuses that entire five-block skeleton
  and the `LoopTargets` push/pop verbatim. Only two things change: (1)
  `idx_alloca` is initialized to the evaluated `start` expression's
  value instead of the literal `0`, and the loop condition compares it
  against the evaluated `end` value with `IntPredicate::SLT` (`..`) or
  `IntPredicate::SLE` (`...`) instead of `build_for`'s fixed `SLT`
  against a compile-time element count; (2) `for.body` stores the
  running index straight into `var_alloca` via `build_store` — there is
  no per-element `arr_ptr`, no `build_in_bounds_gep`, no element load,
  because a Range's "elements" *are* the index. This is a smaller
  codegen surface than `build_for`'s own array-literal case, not a
  bigger one.
- **Range endpoints are arbitrary `Int64` expressions, evaluated once
  each before the loop begins — not restricted to integer literals.**
  This is the concrete reason plan 18 is a real dependency, not a
  formality: `for i in 0..(n - 1)` needs `Expr::Sub` to exist in the
  AST at all, which it does today (verified directly in `ast.rs`:
  `Sub(Box<Expr>, Box<Expr>)` sits alongside `Add`/`Mul`/`Div`/`Rem`,
  all landed by plan 18). `start`/`end` are each run through the same
  `build_expr` codegen entry point every other `Int64`-typed expression
  uses, producing one LLVM SSA value apiece computed once in the loop's
  preheader block — dominance holds across every back-edge into
  `for.cond` without needing a fresh alloca or re-evaluation per
  iteration (only `idx_alloca` itself, the mutable running counter,
  needs a `load`/`store` pair each time round, exactly as `build_for`'s
  own `idx_alloca` already does).
- **`var`'s value kind is always `Int64`, known statically — unlike
  `build_for`'s array-literal case, whose element kind is only known
  once the first element expression is actually built.** Verified this
  session against `collect_lets` (`crates/emerald-codegen/src/lib.rs`,
  lines 479-493): `Stmt::For`'s own comment explains its loop variable
  is deliberately *not* hoisted into this pre-pass "since there's no
  syntactic type annotation to derive its `ValKind` from ahead of
  time" — its `alloca` is instead built inline at the `build_for` call
  site. `Stmt::ForRange` has no such problem: this plan's own Int64-
  only endpoint restriction (Decision log, first bullet) means `var`'s
  kind is `ValKind::Int64` unconditionally, with zero dependence on
  either operand's shape. `collect_lets` therefore gains a genuine new
  arm, `Stmt::ForRange { var, body, .. } => { out.push((var.clone(),
  ValKind::Int64)); collect_lets(body, out); }`, hoisting `var`'s
  `alloca` up front the same way an ordinary `Let` already is — a real
  simplification `Stmt::For`'s own case structurally cannot take.
- **No endless ranges (`5..`).** The grammar production for `for var in
  <start> ".." <end> ... end` requires both a `start` and an `end`
  operand on both sides of the new token — there is no alternative
  production admitting a bare trailing `..` with nothing after it, so
  `for i in 5.. puts i end` is a parse error, structurally, the same
  "enforced by the grammar, not deferred to sema" standard plan 30 used
  for its own array-literal-only restriction (Decision log, first
  bullet, and see `leaf-ast-range`'s AC5).
- **No `Range#each` and no general Enumerable/Iterable mechanism.**
  Ruby's `Range` is real because it participates in `Enumerable`
  (`.each`, `.map`, `.select`, ...) via a shared iterator protocol.
  Emerald has no such protocol and this plan does not add one — that is
  real, disclosed future work explicitly assigned elsewhere in this
  same batch (plan 42, Enumerable/Iterable). Concretely: `Range` never
  reaches `PrimaryExpr` in this plan's grammar (see the first Decision
  log bullet), so `(1..5).each { |i| ... }` has no derivation at all,
  not merely an unimplemented method. This is also where the "no
  dynamic/virtual dispatch" identity constraint bites directly: Ruby's
  `Range#each` calls into a real `Enumerator`/iterator object through
  method dispatch once per element; this plan's `for i in 1..5`
  compiles to pure straight-line index arithmetic with zero method
  calls and zero heap-allocated iterator state per iteration — a
  stricter, more static shape than the feature it's declining to fully
  build, not a compromise on top of it.
- **No array slicing via range index (`arr[0..2]`).** `CallExpr`'s
  existing indexing production (`<base:CallExpr> "[" <idx:Expr> "]" =>
  Expr::Index(...)`, `crates/emerald-parser/src/grammar.lalrpop` line
  604) is untouched by this plan — `idx` still only accepts a plain
  `Expr`, and `Expr::Index`'s codegen still only ever computes a single
  `Int64` offset. Wiring `..`/`...` into that position is deliberately
  out of scope: it would need a `Range`-typed `Expr::Index` overload,
  a new `Array[T]` sub-buffer allocation, and a length-returning slice
  value — three separate design decisions this plan's job (prove the
  Range value shape and the iteration form work at all) doesn't need
  to make. Deferred to whichever future plan actually motivates it.
- **A reverse range (`5..1`, inclusive, or `5...1`, exclusive) is a
  well-typed, zero-iteration loop — not a compile error and not a
  runtime panic.** This mirrors Ruby's own `Range` semantics (`(5..1).to_a
  == []`) directly rather than inventing new behavior: sema only checks
  that both endpoints are `Int64`, it never compares their relative
  order, and codegen's loop condition (`start < end` or `start <= end`,
  evaluated at `for.cond` on entry) is naturally false immediately when
  `start > end`, branching straight to `for.after` without ever
  entering `for.body`. `leaf-codegen-range`'s AC4 is the real compiled-
  and-run proof of this — a defensible, disclosed design choice rather
  than an oversight.

## Leaf: leaf-ast-range

### 1. Context
- Why: no `Range` shape exists anywhere in the source today. Verified
  this session: `crates/emerald-parser/src/ast.rs`'s `Expr`/`Stmt`
  enums contain no `Range` variant; `crates/emerald-parser/src/
  grammar.lalrpop` has no `".."`/`"..."` literal token anywhere (a
  targeted search across the file matched only unrelated uses of a
  literal three-dot ellipsis inside doc comments, e.g. line 29's
  `"..."` inside a code-comment reference to string literals — no
  actual grammar token); and `crates/emerald-parser/src/grammar.
  lalrpop` line 715's `FloatLit` regex is `r"[0-9]+\.[0-9]+"` (a digit
  required immediately after the dot), which means `1..5` can never be
  mis-lexed as a float followed by a stray `.5` — `1` lexes as `Num`,
  `..` lexes as the new token, `5` lexes as `Num`, with no ambiguity.
- Target state: add `Stmt::ForRange { var: String, start: Expr, end:
  Expr, exclusive: bool, body: Vec<Stmt> }` to `ast.rs`, placed
  alongside the existing `Stmt::For` (line 205-209) with a doc comment
  cross-referencing this plan. Add two grammar alternatives to the
  existing `Stmt` production list, directly beside line 157's `for`
  rule:
  ```
  "for" <var:Ident> "in" <start:Expr> ".." <end:Expr> <body:Stmt*> "end" => Stmt::ForRange {
    var, start, end, exclusive: false, body,
  },
  "for" <var:Ident> "in" <start:Expr> "..." <end:Expr> <body:Stmt*> "end" => Stmt::ForRange {
    var, start, end, exclusive: true, body,
  },
  ```
  LALRPOP auto-tokenizes bare quoted literals with longest-match
  semantics, so `"..."` naturally wins over `".."` wins over the
  pre-existing single `"."` token already used by `PrimaryExpr`'s
  `recv "." "new"`/`recv "." method` alternatives (grammar.lalrpop
  lines 630-633) — verified this session that `.` is already a literal
  token there, so no `match { }` block priority reordering is needed;
  LALRPOP's literal-terminal tokenizer resolves this by length alone.

### 2. Acceptance Criteria
1. `Stmt::ForRange` exists with the fields described above.
2. `for i in 1..5 puts i end` parses to `Stmt::ForRange { var: "i",
   start: Expr::Int(1), end: Expr::Int(5), exclusive: false, body:
   [Stmt::Expr(Expr::Call("puts", [Expr::Ident("i")]))] }`.
3. `for i in 1...5 puts i end` parses to the identical shape except
   `exclusive: true` — a direct AST-level proof the two tokens are
   distinguished, not just accepted.
4. `r = 1..5` (a bare Range outside any `for...in` position) is
   rejected as a parse error, not accepted and deferred to sema —
   proving the grammar-level restriction from the Decision log, the
   same standard plan 30's AC3 used for its own array-literal
   restriction.
5. `for i in 5.. puts i end` (an endless range, no `end` operand) is
   rejected as a parse error.
6. Regression: every prior plan's example still parses identically,
   including plan 30's own literal-array `for...in` (`Stmt::For` is
   untouched — this leaf only adds new alternatives).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new AST-shape and rejection tests | agent-claimed-locally |

---

## Leaf: leaf-sema-range

### 1. Context
- Why: once `leaf-ast-range` lands, `Stmt`'s match arms are no longer
  exhaustive anywhere `Stmt` is matched without a wildcard — Rust's
  compiler makes this structurally impossible to miss, and `cargo build
  -p emerald-sema` fails to compile until this leaf adds the missing
  arm. The direct model is `check_stmt`'s existing `Stmt::For` arm
  (`crates/emerald-sema/src/lib.rs`, lines 1047-1061): it infers the
  literal array's unified element type via `infer_array_lit_type`,
  binds `var` at that type in `env`, then calls `check_block(body, ...,
  true)` to thread `in_loop = true` through `body` so `break`/`next`
  are legal inside it.
- Target state: a new `check_stmt` arm for `Stmt::ForRange { var,
  start, end, exclusive: _, body }` that calls `infer_expr_type` on
  `start` and on `end` independently, returns a `Diagnostic` naming the
  offending type and expression if either is not `Type::Int64` (not a
  panic, not a silent coercion), then `env.insert(var.clone(),
  Type::Int64)` and `check_block(body, env, sigs, classes, self_fields,
  return_type, true)` — structurally identical to `Stmt::For`'s own
  arm, just against a fixed `Type::Int64` instead of an inferred
  element type.

### 2. Acceptance Criteria
1. `for i in 1..5 x: Int64 = i + 1 end` type-checks `Ok(())`, with `i`
   genuinely usable at `Int64` inside `body` (not merely accepted
   syntactically).
2. `for i in 0..(n - 1) ... end` (a non-literal, plan-18-shaped
   endpoint expression) type-checks `Ok(())` when `n: Int64` is already
   in scope — proving endpoints are real expressions, not just
   literals.
3. `for i in "a".."z" puts i end` is rejected with a type-mismatch
   diagnostic naming `String` where `Int64` was required — the direct
   negative case proving the Int64-only endpoint restriction is
   actually enforced, not merely documented.
4. `break`/`next` inside a `Stmt::ForRange` body are accepted exactly
   as they already are inside `Stmt::For`/`Stmt::While` bodies (reuses
   the existing `in_loop` flag — no new control-flow-legality logic).
5. Regression: plan 30's literal-array `for...in` example and every
   other prior plan's example still type-check identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. new Int64-endpoint acceptance/rejection tests | agent-claimed-locally |

---

## Leaf: leaf-codegen-range

### 1. Context
- Why: nothing compiles `Stmt::ForRange` to machine code yet, and — as
  with `leaf-sema-range` — Rust's match exhaustiveness makes every site
  that pattern-matches `Stmt` a hard compile error until this leaf adds
  the missing arm. Three such sites exist in
  `crates/emerald-codegen/src/lib.rs`, all verified this session:
  `collect_idents_in_stmt` (lines 298-405, used for lambda free-
  variable capture analysis — plan 10's by-value-capture mechanism),
  `collect_lets` (lines 479-493, which hoists every `Let`-bound name
  into an entry-block `alloca` up front so LLVM's `mem2reg` can promote
  it to an SSA register), and `build_stmt`'s own dispatch match (starting
  line 2227, with `Stmt::For`'s arm around line 2603 calling into
  `build_for`).
- Target state:
  - `collect_idents_in_stmt`: `Stmt::ForRange { var, start, end, body }
    => { bound.insert(var.clone()); collect_idents_in_expr(start,
    referenced); collect_idents_in_expr(end, referenced); for s in body
    { collect_idents_in_stmt(s, referenced, bound); } }` — directly
    mirroring `Stmt::For`'s existing arm, with `start`/`end` filling
    the role `elements` plays there.
  - `collect_lets`: `Stmt::ForRange { var, body, .. } => {
    out.push((var.clone(), ValKind::Int64)); collect_lets(body, out);
    }` — per the Decision log, `var`'s `Int64` kind is knowable
    statically here (unlike `Stmt::For`'s case), so it is hoisted
    directly rather than built inline at the codegen site.
  - `build_stmt`: a new arm dispatching to a new `build_for_range`
    function, built by copying `build_for`'s five-block skeleton
    (`for.cond`/`for.body`/`for.incr`/`for.after`, `LoopTargets` push/
    pop around `build_block`) and replacing the array-materialization
    prologue and the per-iteration GEP/load with: evaluate `start`/
    `end` once via `build_expr` before the loop, store `start`'s value
    into the (now pre-hoisted) `var_alloca`'s own fresh index-carrying
    `alloca`, compare against `end` with `IntPredicate::SLT` (`..`) or
    `IntPredicate::SLE` (`...`), and — inside `for.body` — `build_store`
    the running index directly into `var_alloca` (no `arr_ptr`, no GEP,
    no load).

### 2. Acceptance Criteria
1. This plan's own worked example, compiled, linked, and run, prints
   `15\n10\n` — the real distinguishing proof that `..` and `...` bind
   their upper endpoint differently, not just that both parse and
   run.
2. A real program using `break` inside a `for i in 1..5` loop (e.g.
   `for i in 1..5 if i == 3 break end puts i end` → prints `1\n2\n`) and
   one using `next` (e.g. skipping `i == 3` → prints `1\n2\n4\n5\n`)
   each print the expected, distinct stdout — proving `LoopTargets`
   genuinely works for this new loop shape, not just the array-literal
   one `build_for` already covers.
3. A non-literal endpoint expression (`n: Int64 = 4\nfor i in 0..n
   puts i end` → prints `0\n1\n2\n3\n4\n`) compiles, links, and runs
   correctly — real proof endpoints are evaluated as genuine
   expressions, not baked in as compile-time constants the way
   `build_for`'s array element count is.
4. A reverse range (`for i in 5..1 puts i end`) compiles, links, and
   runs to completion printing nothing (`""`), not a panic and not an
   infinite loop — the real executed proof of the Decision log's
   "reverse range is a well-typed, zero-iteration loop" claim.
5. Regression: every prior plan's linked-and-run example — in
   particular plan 30's own `for_in_sums_a_literal_array`/`for_in_break
   _stops_after_the_second_element`/`for_in_next_skips_one_element`
   tests — still produce byte-identical output after this leaf lands,
   since `Stmt::For`'s own `build_for` function is never modified, only
   called from the same place it already was.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (add
  `build_for_range`; extend `collect_idents_in_stmt`, `collect_lets`,
  and `build_stmt`'s dispatch match; add tests alongside the existing
  "// Plan 30 (for-in iteration)." block, in a new "// Plan 37 (ranges
  and range-based iteration)." block)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run inclusive/exclusive/break/next/non-literal-endpoint/reverse-range proofs | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
