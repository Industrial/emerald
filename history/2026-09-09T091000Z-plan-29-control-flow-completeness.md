---
name: Control-Flow Completeness
overview: "`elsif` chains and `unless`/`until` — three of spec/GRAMMAR.md §5's KEEP control-flow forms that were never wired into the grammar; modifier/postfix forms are investigated and explicitly declined."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-elsif
    content: "elsif chains via pure parse-time desugaring into nested Stmt::If inside the existing else_branch — no new AST variant, no sema/codegen changes"
    status: pending
  - id: leaf-unless-until
    content: "unless/until via parse-time desugaring to Stmt::If/Stmt::While wrapping the condition in plan 18's Expr::Not — a real, disclosed dependency on plan 18 landing first"
    status: pending
isProject: false
---

# Plan 29 — Control-Flow Completeness

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md), for
the same reason plan 17 wasn't: that table's Completion note calls
Emerald v1 done as of row 15. This is new, post-v1 language-completeness
scope — not touching `plan-of-plans.md` or any other existing plan file.
It's one of eight sibling plans (28–35) written in the same batch, each
covering one distinct, independently-requested language-completeness
gap; this one owns `elsif`/`unless`/`until`/modifier-forms only.

Concrete proof this plan targets — two real programs, compiled, linked,
and run, exact stdout asserted:

**Example A (`elsif` chain):**
```ruby
def grade(score: Int64) -> Int64
  if score >= 90
    return 4
  elsif score >= 80
    return 3
  elsif score >= 70
    return 2
  else
    return 1
  end
end

puts grade(95)
puts grade(85)
puts grade(72)
puts grade(50)
```
Expected: `4\n3\n2\n1\n`.

**Example B (`unless`/`until`):**
```ruby
def describe(x: Int64) -> Int64
  unless x > 0
    return 0
  end
  return 1
end

puts describe(-5)
puts describe(5)

i: Int64 = 0
until i >= 3
  puts i
  i: Int64 = i + 1
end
```
Expected: `0\n1\n0\n1\n2\n` (`describe(-5)` prints `0`, `describe(5)`
prints `1`, then the `until` loop prints `0`, `1`, `2`).

## Note on current codegen state

`crates/emerald-codegen/src/lib.rs` is, as of this session, the subject
of unrelated, concurrent, in-progress work by a different agent
executing plan 16 (the Cranelift-vs-LLVM bake-off) — its on-disk state
right now is transient and mid-migration (at one point during this
session `src/lib.rs` was absent entirely). This plan cites the last
documented, stable shape of that file from earlier in this session: a
Cranelift-based `build_expr`/`build_stmt`/`Ctx` implementation where
`Stmt::If` codegen already branches via `create_block`/`brif` into a
then-block, an optional else-block, and a merge block. This plan's own
leaves don't touch codegen at all (see Decision log), so plan 16's
outcome doesn't block them either way.

## Decision log

- **`elsif` is a pure parse-time desugaring, not a new AST shape.**
  `grammar.lalrpop`'s `ElseClause` rule today is exactly `"else" Stmt*`,
  feeding `Stmt::If`'s `else_branch: Option<Vec<Stmt>>` (verified this
  session directly against `crates/emerald-parser/src/grammar.lalrpop`
  and `ast.rs`). Adding a second `ElseClause` alternative —
  `"elsif" Expr Stmt* ElseClause?` — that builds a single-element
  `vec![Stmt::If { cond, then_branch, else_branch: <recurse> }]` gives
  real `elsif`-chain *semantics* (each `elsif` is checked in order,
  first match wins, trailing `else` still works) using the AST this
  compiler already has and already type-checks/codegens correctly
  (a nested `If` inside an `else_branch` is just another statement in
  that block, walked by the same `check_block`/`build_block` loops every
  other nested statement already goes through). This is the same
  "grammar-level construction, not a new AST node" move plans 09
  (`Array[Elem]` as a string) and 11 (exact-tag `rescue` matching) both
  already used where a real new AST variant would have been
  disproportionate to the semantics gained.
- **`unless`/`until` desugar to `Stmt::If`/`Stmt::While` with the
  condition wrapped in `Expr::Not`** — `"unless" Expr Stmt* ElseClause?
  "end"` builds `Stmt::If { cond: Expr::Not(Box::new(cond)), ... }`;
  `"until" Expr Stmt* "end"` builds `Stmt::While { cond:
  Expr::Not(Box::new(cond)), body }`. This is a **real, disclosed
  dependency on plan 18** (`2026-09-08T213500Z-plan-18-arithmetic-and-
  logical-operators.md`), which is what actually adds `Expr::Not` to
  the AST — unlike plan 21's deliberate avoidance of a hard dependency
  on plan 22 (where a throwaway workaround was honest and easy), there
  is no honest lighter-weight way to negate an arbitrary `Expr` here
  without either reusing plan 18's real `Not` node or inventing a
  second, parallel negation mechanism just for this plan — reusing the
  real one is more honest than that. **This leaf cannot land before
  plan 18 does.**
- **Modifier/postfix forms (`stmt if cond`, `stmt unless cond`, `stmt
  while cond`) are investigated and explicitly declined, not
  implemented.** `spec/GRAMMAR.md` §5 calls them "Sugar over the block
  form; no new semantics," but this grammar's `Stmt*` repetitions have
  no statement terminator at all (plan 04's original "no significant
  newlines" simplification) — the same root cause plans 04, 07, and 09
  each already hit their own concrete shift/reduce conflicts over
  ("bare `return`/`raise` needing a mandatory value," "array-literal
  statement colliding with indexing's `[`"). A postfix modifier
  production (`Stmt "if" Expr` as an alternative `Stmt`) sits inside the
  same `Stmt*` list a fresh block-form `if`/`unless`/`while` can also
  start — after a complete statement, LALR(1) has exactly one token of
  lookahead to decide whether a following `if` extends *this* statement
  as a modifier or begins the *next* one, and both are genuinely valid
  continuations with no separator to disambiguate them. This wasn't
  empirically confirmed with an actual LALRPOP build in this session
  (see the codegen note above for why builds are being avoided this
  session — unrelated concurrent work has the workspace in a transient
  state), but it is the identical structural shape as three already-
  documented, already-real conflicts in this exact grammar, not a
  speculative concern. Landing modifier forms for real needs either a
  significant-newline/explicit-terminator change to the grammar (a much
  larger, cross-cutting redesign well beyond this plan) or restricting
  them to contexts provably free of the ambiguity — neither is
  attempted here; see Out of scope.
- **`case`/`when`'s own condition/branch semantics are sibling plan 20's
  job** (`2026-09-08T215500Z-plan-20-comments-and-case-when.md`) — not
  duplicated or extended here.
- **`redo` and `throw`/`catch` are not implemented** — `spec/
  GRAMMAR.md` §5 itself marks both REMOVE, so there is nothing for this
  plan (or any future one) to add here.

## Leaf: leaf-elsif

### 1. Context
- Why: `grammar.lalrpop`'s `ElseClause` rule has exactly one shape,
  `"else" Stmt*` — there is no `elsif` production at all; the only way
  to express an `elsif`-shaped chain today is manually nesting a fresh
  `if...end` inside an `else` block, with one extra required `end` per
  level (confirmed: no `"elsif"` token appears anywhere in
  `crates/emerald-parser/src/grammar.lalrpop`).
- Target state: a second `ElseClause` alternative,
  `"elsif" <cond:Expr> <body:Stmt*> <rest:ElseClause?> => vec![Stmt::If
  { cond, then_branch: body, else_branch: rest }]`, reusing the existing
  `Stmt::If`/`ElseClause` shapes exactly as they are today. `elsif`
  becomes a reserved keyword (same LALR(1) reason `puts`/`new`/`Array`
  were reserved in plans 07/08/09).

### 2. Acceptance Criteria
1. Example A parses into a `Stmt::If` whose `else_branch` is
   `Some(vec![Stmt::If { ... else_branch: Some(vec![Stmt::If { ...
   else_branch: Some(vec![...]) }]) }])` — i.e. a real, correctly nested
   chain, not a flattened or lossy representation.
2. A **trailing plain `else` after one or more `elsif`s** still works
   (Example A's final `else` clause) — the `ElseClause?` recursion
   terminates in the existing `"else" Stmt*` alternative unchanged.
3. An `elsif`-free `if`/`else` (every prior plan's existing examples)
   parses identically to before — zero regression, since the new
   alternative is strictly additive to `ElseClause`, not a replacement.
4. `emerald_sema::check_program` and `emerald_codegen::compile_to_object`
   require **no changes at all** to correctly handle Example A — proven
   by writing no sema/codegen code for this leaf and having Example A's
   `Ok(())`/correct-output acceptance criteria (leaf 2's shared quality
   gate, since this leaf alone only needs a parser-level test) still
   pass once plan 18 and this plan are both applied. (This leaf's own
   quality gate below is parser-only; the full compiled-and-run proof is
   folded into this plan's `Total quality gate` once both leaves exist.)

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new `elsif`-chain shape test | agent-claimed-locally |

---

## Leaf: leaf-unless-until

### 1. Context
- Why: there is no `unless` or `until` keyword anywhere in
  `grammar.lalrpop` today (confirmed this session) — `spec/GRAMMAR.md`
  §5 marks both KEEP ("Same `Boolean`-only condition rule as `if`" for
  `until`).
- Target state: `"unless" <cond:Expr> <body:Stmt*> <rest:ElseClause?>
  "end" => Stmt::If { cond: Expr::Not(Box::new(cond)), then_branch:
  body, else_branch: rest }` (reuses leaf-elsif's `ElseClause` rule
  as-is, so `unless ... else ... end` and even `unless ... elsif ...
  end` fall out for free — `unless`'s condition is only ever the
  *first* branch's guard, so chaining an `elsif` after it is legal
  grammar but unusual style; not specifically tested here beyond what
  falls out naturally). `"until" <cond:Expr> <body:Stmt*> "end" =>
  Stmt::While { cond: Expr::Not(Box::new(cond)), body }`. Both
  `unless`/`until` become reserved keywords. **Depends on plan 18's
  `Expr::Not` existing** (see Decision log) — this leaf cannot be
  implemented before plan 18 lands.

### 2. Acceptance Criteria
1. Example B parses `unless x > 0 ... end` into `Stmt::If { cond:
   Expr::Not(Box::new(Expr::Compare(x, Gt, 0))), ... }` and `until i >=
   3 ... end` into `Stmt::While { cond: Expr::Not(Box::new(Expr::Compare
   (i, Ge, 3))), ... }` — real proof of the desugaring shape, not just
   that it parses.
2. Example B, compiled, linked, and run (once plan 18's `Expr::Not`
   codegen exists — this is exactly why this leaf's compiled-and-run
   proof lives in this plan's `Total quality gate`, run only after both
   plan 18 and this plan are applied, not as a standalone claim this
   leaf can prove in isolation), prints exactly `0\n1\n0\n1\n2\n`.
3. Regression: every prior plan's `if`/`while` example still parses and
   (once compiled) still runs identically — the new keywords are
   additive reservations, not changes to `if`/`while` themselves.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test (parse-shape only) | `cargo test -p emerald-parser` | all pass, incl. new `unless`/`until` desugaring-shape tests | agent-claimed-locally |
| Full compiled-and-run proof | `cargo test --workspace` (after plan 18 lands) | Example A and Example B both print their exact expected stdout | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
(The second command's Example A/Example B assertions are only
meaningful once plan 18 has landed — see leaf-unless-until's Decision
log entry. `leaf-elsif` alone has zero dependency on plan 18 and can
land, build, and prove itself independently.)

## Out of scope / deferred
- **Modifier/postfix `if`/`unless`/`while` forms** (`stmt if cond`) —
  investigated and declined; see Decision log's LALR(1)-ambiguity
  analysis. Real follow-up work, blocked on a significant-newline or
  explicit-statement-terminator grammar redesign this plan doesn't
  attempt.
- **`case`/`when`** — sibling plan 20's scope, not duplicated here.
- **`redo`, `throw`/`catch`** — `spec/GRAMMAR.md` §5 itself marks both
  REMOVE; nothing to add.
- **Ternary (`cond ? a : b`)** — a separate KEEP row in `spec/
  GRAMMAR.md` §4, not requested as part of this batch.
- **`begin ... end while` (do-while)** — a separate KEEP row in `spec/
  GRAMMAR.md` §5, not requested as part of this batch; would need its
  own leaf given it evaluates the body before the first condition
  check, a genuinely different control-flow shape from everything in
  this plan.
