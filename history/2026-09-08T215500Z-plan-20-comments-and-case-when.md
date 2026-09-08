---
name: Comments and case/when
overview: "Real `#` line-comment lexing and a real `case`/`when` value-match statement — two gaps spec/GRAMMAR.md and plan-of-plans both already claim are handled, but grammar.lalrpop implements neither (found while authoring plan 17)."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-comments
    content: "grammar.lalrpop's match block gains a `#`-to-end-of-line skip pattern — no AST/sema/codegen change, purely lexical"
    status: pending
  - id: leaf-case-when
    content: "Stmt::Case { scrutinee, arms: Vec<(Vec<Expr>, Vec<Stmt>)>, else_body } — Int64-scrutinee value match via the same Eq comparison Compare already does, compiled as an icmp/brif chain"
    status: pending
isProject: false
---

# Plan 20 — Comments and case/when

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
for the same reason plan 17 wasn't: that table's Completion note already
calls Emerald v1 done as of row 15. Both gaps this plan closes were
found *while checking* plan-of-plans/`spec/GRAMMAR.md` against the real
parser for plan 17's syntax-coloring work, not invented as new scope:

- `spec/GRAMMAR.md` §12 marks `# line comment` **KEEP**, but
  `crates/emerald-parser/src/grammar.lalrpop`'s `match` block only skips
  whitespace (`r"\s*" => { }`) — there is no comment pattern anywhere in
  the grammar (confirmed by reading the file this session), and no
  `examples/*.em` file contains a `#`. A real `.em` file with a trailing
  comment fails to compile today.
- `plan-of-plans` row 07 (`control-flow`, status `done`) lists `case` in
  its own scope description ("Locals, `if`, comparisons, loops, `case`,
  `return`/`break`/`next`"). `grammar.lalrpop` has zero `case`/`when`
  productions — a grep for `"case"`, `Case`, `"when"` across
  `crates/emerald-parser/src` returns no matches. `case`/`when` was
  never actually implemented, despite the scope table's own claim.

Concrete proof this plan targets:
```ruby
# classify an integer by a fixed set of buckets
n: Int64 = 2
label: Int64 = 0
case n
when 1
  label: Int64 = 10
when 2, 3
  label: Int64 = 20
else
  label: Int64 = 99
end
puts label
```
Expected output: `20`. The leading `#` comment line must not affect
parsing at all — the same program with every comment stripped must
produce byte-identical `Program` output.

## Decision log

- **`when` matches via the same `CompareOp::Eq` comparison `Expr::
  Compare` already performs — not `spec/GRAMMAR.md` §5's eventual
  method-dispatched `===`.** This compiler has no `===`
  operator-overload dispatch mechanism at all (`SEMANTICS.md`/
  `GRAMMAR.md`'s "statically resolved `===`/`==` on the scrutinee's
  type" describes a protocol that doesn't exist here yet — `Expr::
  Compare`'s `Eq` variant is the only equality primitive this compiler
  has). This plan's `case`/`when` is real value-match, just over that
  smaller, already-existing primitive — the same "prove the core path
  with what already exists" pattern as plan 13 (one diagnostic, not
  `miette`'s whole feature surface) and plan 15 (2 of 9 benchmarks).
- **`case`'s scrutinee and every `when` value must be `Int64` — not
  `Float64`, not `String`, not `Class`.** `emerald_sema::infer_expr_type`
  actually permits `Compare`/`Eq` between any two operands of matching
  type (found this session — it only rejects mismatched types, not
  non-numeric ones), but `emerald-codegen`'s `Expr::Compare` codegen
  unconditionally lowers every comparison operator to Cranelift `icmp`
  (integer compare) regardless of operand type — confirmed this
  session: there is no `fcmp` (float compare) path at all, so comparing
  two `Float64`s today is already a silent, pre-existing miscompilation
  this plan did not introduce and is not the right plan to fix (a
  `Compare`-codegen bug, not a `case`/`when`-scope one; flagged here so
  it isn't rediscovered as if new). Restricting `case`/`when` to `Int64`
  sidesteps inheriting that bug into a brand-new feature; `Boolean` is
  also excluded even though it would `icmp`-compare correctly, since
  matching on a two-valued type is exactly what `if`/`else` already
  does and adds no real proof value.
- **`case`/`when` is a `Stmt`, not an `Expr` — not usable as a value
  the way `spec/GRAMMAR.md` §5 eventually wants (`"if"`/`"unless"` as
  expressions").** Checked this session: `if` itself is *only*
  `Stmt::If` in this AST — there is no `Expr::If` anywhere in `ast.rs`
  — so `spec/GRAMMAR.md`'s "if as a typed expression" is itself still
  aspirational, not something this compiler has ever implemented. This
  plan's `case` deliberately matches `if`'s own actual, current
  statement-only shape rather than leapfrogging it into expression form
  — that leap, if wanted, is a separate `if`-and-`case`-together plan,
  not something `case` alone should do first.
- **Multiple values per `when` clause (`when 1, 2, 3`) are included.**
  Ruby's `case`/`when` is materially less useful without it, and the
  codegen cost is small — one arm becomes an OR-chain of the same
  `icmp`/`brif` pair already needed for a single value, not a new
  mechanism.
- **`case`/`in` pattern matching (`spec/GRAMMAR.md` §5's separate,
  scope-limited, still partly-`UNDECIDED` row) is not this plan.** That
  row covers array/binding-pattern destructuring against
  `deconstruct`/`deconstruct_keys` protocols that don't exist in this
  compiler at all; conflating it with plain value-match `case`/`when`
  would blow this plan's scope past a single provable proof.
- **No `=begin`/`=end` block comments**, despite `spec/GRAMMAR.md` §12
  listing them KEEP alongside `#` line comments. Block comments are
  rare even in idiomatic Ruby and add a second, stateful lexer mode
  (multi-line skip-until-terminator) for a form the `examples/*.em`
  corpus and every prior plan's worked examples have never once needed;
  deferred until a real program wants one.
- **Comments may appear anywhere trailing whitespace already can —
  including at the end of a code line, not just on their own line** —
  the natural reading of "line comment" (Ruby's own `#` behaves this
  way), and it costs nothing extra in a regex-skip rule (`#[^\n]*`
  matches to end-of-line regardless of what precedes it on that line).
- **This plan's comment lexing doesn't special-case string literals.**
  No `.em` program can contain a string literal today (`Type::String`
  exists with no `Expr::StringLit` syntax — a separate, disclosed gap
  this plan doesn't touch). If a future string-literals plan lands
  after this one, whoever writes it must re-verify that a `#` inside a
  `"..."` literal doesn't get eaten by this plan's comment pattern
  (LALRPOP's `match` block is priority-ordered, so a string-literal
  pattern added later needs to be checked against — not assumed
  compatible with — the skip rule this plan adds).

## Leaf: leaf-comments

### 1. Context
- Why: `grammar.lalrpop`'s `match` block has no comment pattern; a `#`
  anywhere in a `.em` file is a lex error today (falls through to the
  grammar's default `_` catch-all as an unrecognized token).
- Target state: the `match` block's whitespace-skip line gains a
  sibling skip pattern for `#[^\n]*` (a `#` through end-of-line,
  newline excluded so line-ending/statement boundaries are unaffected).

### 2. Acceptance Criteria
1. `parse("# just a comment\n")` succeeds and produces `Program { items:
   vec![] }` — a comment-only file is a valid, empty program.
2. Every existing `examples/*.em` file, with a `# ...` comment inserted
   at the end of at least one line and a whole-line comment inserted
   above the first statement, still parses to the exact same `Program`
   value as the unmodified file (a real diff-the-AST regression check,
   not just "still compiles").
3. Every existing test fixture string across
   `crates/emerald-parser/src/lib.rs` continues to parse identically
   (no behavior change for comment-free source).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new comment-regression tests | agent-claimed-locally |

---

## Leaf: leaf-case-when

### 1. Context
- Why: no `case`/`when` AST shape, grammar production, sema rule, or
  codegen exists anywhere in this compiler (verified this session — see
  Decision log).
- Target state: `Stmt::Case { scrutinee: Expr, arms: Vec<(Vec<Expr>,
  Vec<Stmt>)>, else_body: Option<Vec<Stmt>> }` (each arm's `Vec<Expr>`
  holds one or more `when` values, matching if the scrutinee equals
  *any* of them). Grammar: `"case" Expr Stmt* ("when" Args Stmt*)+
  ElseClause? "end"` (reusing the existing `Args` comma-list rule for
  `when`'s value list and `ElseClause` for the trailing `else`, both
  already used elsewhere in this grammar). `case`/`when` become reserved
  keywords, same LALR(1) reason `puts`/`new`/`Array`/`raise`/etc. were
  reserved in plans 07/08/11. Sema: the scrutinee and every `when` value
  must type-check to `Int64` (see Decision log); each arm's body and the
  `else` body type-check as an ordinary `Stmt*` block against the
  enclosing return type/loop context, exactly like `If`'s branches
  already do. Codegen: lowers to the same `icmp`(`Equal`)/`brif` shape
  `Expr::Compare`'s `Eq` arm already emits, chained across arms in
  source order (first matching arm wins, matching Ruby's own `case`
  semantics), OR-ed across a multi-value arm, falling through to
  `else_body` (or nothing) if no arm matches.

### 2. Acceptance Criteria
1. This plan's worked example — including its multi-value `when 2, 3`
   arm — compiled, linked, and run, prints exactly `20`.
2. The same program with `n: Int64 = 1` prints `10`; with `n: Int64 =
   7` (no arm matches) prints `99` (the `else` branch) — proof all three
   branches (single-value match, multi-value match, no-match/`else`)
   are real, distinct, executed code paths, not one path coincidentally
   producing the right number.
3. A `case` with no `else` and no arm matching a given scrutinee value
   compiles and runs to completion (falls through, no diagnostic, no
   crash) — matching `if` with no `else` branch's existing behavior.
4. `case n when "x" ... end` (a `String`/non-`Int64` `when` value) is
   rejected with a diagnostic naming the mismatch, not silently
   accepted or panicking — same standard as every prior type-mismatch
   diagnostic in this project.
5. `case n when 1 ... end` where `n` is `Float64` is rejected with a
   diagnostic (see Decision log's `Int64`-only scope).
6. Regression: every prior plan's worked example still parses and
   type-checks identically (`case`/`when` are new reserved keywords —
   confirm no existing example used `case`/`when` as an identifier).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests),
  `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Parser test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |
| Sema test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |
| Codegen test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `20`/`10`/`99` outputs | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **`case`/`in` pattern matching** (array/binding-pattern
  destructuring) — a separate `spec/GRAMMAR.md` §5 row with its own
  still-`UNDECIDED` protocol questions; see Decision log.
- **Method-dispatched `===`** — no operator-overload dispatch mechanism
  exists in this compiler at all yet; see Decision log.
- **`case`/`when` (or `if`) as an expression returning a value** — `if`
  itself is statement-only in this AST today; see Decision log.
- **`when` over `Float64`, `String`, `Class`, or `Array` scrutinees** —
  restricted to `Int64` this plan; see Decision log (also flags a
  pre-existing `Expr::Compare` codegen gap for `Float64` this plan
  doesn't fix).
- **`=begin`/`=end` block comments** — see Decision log.
- **`when` range patterns (`when 1..10`) or regex patterns (`when
  /foo/`)** — no `Range`/`Regexp` types exist in this compiler
  (`spec/GRAMMAR.md` marks Range literals KEEP and Regexp literals
  UNDECIDED, neither implemented); only literal-value `when` arms are
  in scope here.
