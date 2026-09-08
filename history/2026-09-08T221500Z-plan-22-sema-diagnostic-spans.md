---
name: Sema Diagnostic Spans
overview: "Thread real byte-offset spans through Expr/Stmt so emerald-sema's diagnostics point at the exact offending sub-expression — the invasive AST change plans 13 and 17 both found and deliberately deferred, done here."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-spans
    content: "Spanned<T> { span: (usize, usize), node: T } wrapping every Expr/Stmt position; grammar.lalrpop threads LALRPOP's own @L/@R byte offsets into it at every production"
    status: pending
  - id: leaf-sema-spans
    content: "emerald_sema::Diagnostic gains a real span field; infer_expr_type/check_stmt/check_args populate it from the Spanned node they're checking instead of leaving it whole-document"
    status: pending
  - id: leaf-span-rendering
    content: "emerald-cli's miette rendering, emerald-lsp's publishDiagnostics, and emerald-mcp's check_source tool all switch from a whole-document range to the new real per-diagnostic span"
    status: pending
isProject: false
---

# Plan 22 — Sema Diagnostic Spans

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
the same post-v1 posture as plan 17: it doesn't touch `plan-of-plans.md`
or any other plan file. It is the named follow-up to a gap two earlier
plans both found and explicitly declined to fix, in their own words:

- Plan 13's Decision log: "adding real spans to sema diagnostics means
  adding a span to `Expr`/`Stmt` itself (every variant, across the
  parser AST, propagated through every sema/codegen call site that
  constructs or matches one), which is an invasive, whole-AST change
  disproportionate to one plan."
- Plan 17's Decision log: "Sema diagnostics get a whole-document
  `Range`, not a precise span — inherited directly from
  `emerald_sema::Diagnostic { message: String }` having no span field at
  all... This plan's LSP surfaces that exact, already-disclosed gap
  faithfully rather than fabricating a fake precise range."

This plan does that invasive work. Concrete proof — the same one
inception itself illustrates, and the same one plan 13 explicitly
declined to use ("a parse error demonstrates the same rendering quality
with a fraction of the plumbing... not a literal requirement that this
exact diagnostic be the one that gets it"):

```ruby
def add(a: Int64, b: String) -> Int64
  a + b
end
```

This is `emerald-sema`'s own existing test fixture
(`rejects_inception_25e_mismatch_with_useful_diagnostic`, verified this
session at `crates/emerald-sema/src/lib.rs:886`) — today it produces one
`Diagnostic` whose `message` names `Int64` and `String` but carries no
position at all. After this plan, that same diagnostic's span points at
the exact byte offset of `b` on line 2 (the mismatched operand of `a +
b`), not line 1's parameter declaration and not the whole function —
verified against the source string's own second `.find("b")` occurrence,
the same discipline plan 13 used for parse-error spans.

## Decision log

- **`Spanned<T> { span: (usize, usize), node: T }`, wrapping every
  `Expr`/`Stmt` position, not a bare `span` field bolted onto each enum
  variant.** Two real designs exist:
  (a) add a trailing `span: (usize, usize)` field to all 11 `Expr`
  variants and all 10 `Stmt` variants (the literal reading of plan 13's
  own "add a span to `Expr`/`Stmt` itself") — touches every constructor
  *and* every match-arm binding pattern across `emerald-parser`,
  `emerald-sema`, and `emerald-codegen`, since a tuple/struct variant's
  field list changes shape everywhere it's destructured;
  (b) an outer `Spanned<T>` wrapper around every `Expr`/`Stmt` position
  (`Box<Expr>` fields become `Box<Spanned<Expr>>`, `Vec<Stmt>` bodies
  become `Vec<Spanned<Stmt>>`), with `infer_expr_type`/`check_stmt`/
  `build_expr`/`build_stmt` changing their *signatures* to take
  `&Spanned<Expr>`/`&Spanned<Stmt>` and destructuring `.span`/`.node`
  once at the top of the function, then matching on `&node` exactly as
  today — recursive calls simply pass the already-`Spanned` sub-node
  through unchanged instead of re-wrapping it.
  (b) is the real choice: `emerald-codegen`'s `build_expr` (cc=37,
  matches every `Expr` variant) and `build_stmt` (cc=50, matches every
  `Stmt` variant) — confirmed this session at 2268 lines total — would
  need every match arm's binding pattern rewritten under (a); under (b)
  they need one new line at the top (`let (span, node) = (&expr.span,
  &expr.node);` or equivalent) and the match arms are otherwise
  untouched, since the values *inside* each arm keep their existing
  shapes. The real cost lands where it should: the parser's grammar
  actions, which are the only place spans are actually known, and which
  must wrap every construction in `Spanned { span, node }`.
- **The parser gains real position data via LALRPOP's own `<l:@L> ...
  <r:@R>` markers** — confirmed unused anywhere in `grammar.lalrpop`
  this session (`grep '@L|@R'` returns zero matches today), even though
  `lalrpop_util::ParseError` already proves LALRPOP tracks byte offsets
  internally (plan 13 relied on exactly this for parse errors). No new
  dependency, no lexer change — every grammar production that currently
  produces an `Expr`/`Stmt` value is rewritten to wrap it:
  `<l:@L> <e:AddExprInner> <r:@R> => Spanned { span: (l, r), node: e }`.
- **Diagnostic precision is real but not uniform — disclosed exactly per
  diagnostic class, not claimed as blanket precision:**
  - Every statement gets at least a real, correct span for free, just
    from `Stmt` becoming `Spanned<Stmt>` — even `Stmt::Break`/`Stmt::Next`
    (which carry no sub-expression to blame more precisely) point at
    their own real statement span, a strict improvement over today's
    whole-document range even where no better answer exists.
  - Type-mismatch diagnostics inside `infer_expr_type` (the `Add`/
    `Compare`/method-call-argument branches) point at the actual
    mismatched sub-expression's own span, not the enclosing statement —
    this plan's concrete proof (`b`'s span inside `a + b`) is exactly
    this class.
  - `check_args`' arity-mismatch diagnostic ("expects N argument(s)")
    points at the whole call expression's span — arity is a property of
    the call, not of any one argument, so a per-argument span would be
    dishonest precision. A per-argument *type* mismatch inside the same
    function, once one is looped over per-arg, points at that specific
    argument's span.
  - `Expr::Call(String, _)`/`Expr::New(String, _)`/`Expr::Ident(String)`
    carry their callee/name as a bare `String` with no separate
    sub-span for just the identifier token — "undefined function
    `foo`"/"undefined variable `foo`" diagnostics point at the whole
    `Call`/`Ident` node's span (which, for a bare identifier, already is
    just the identifier's own span; for a call, is the whole `foo(...)`
    expression) — a real, disclosed granularity limit, not silently
    presented as pointing at just the name.
- **Codegen is unaffected in behavior, only in signature shape.** This
  plan doesn't add anything codegen needs (it has never produced a
  diagnostic — its errors are all internal `Result<_, String>` "should
  never happen" guards) — `build_expr`/`build_stmt` gain the one-line
  `Spanned` unwrap described above purely to keep compiling against the
  new AST shape, then proceed exactly as before. No codegen test's
  expected output changes.
- **Follow-up, not redone here:** plan 21's LSP go-to-definition (a
  separate plan in this batch) uses a source-text-search shortcut for
  declaration positions specifically because no real span data existed;
  once this plan lands, that shortcut could be upgraded to use
  `ClassInfo`/`FunctionSig`'s declaration-site `Spanned<Function>`/
  `Spanned<ClassDef>` directly — noted as a real, natural follow-up, not
  attempted in this plan.

## Leaf: leaf-ast-spans

### 1. Context
- Why: `Expr`/`Stmt` (`crates/emerald-parser/src/ast.rs`) carry zero
  positional information today — confirmed this session, every one of
  the 11 `Expr` and 10 `Stmt` variants is bare data. `grammar.lalrpop`
  never uses LALRPOP's `@L`/`@R` position markers (confirmed: zero
  matches).
- Target state: `pub struct Spanned<T> { pub span: (usize, usize), pub
  node: T }` (in `emerald-parser::ast`, deriving `Debug`/`Clone`/
  `PartialEq`, plus `Deref`/`DerefMut` to `T` for ergonomic field
  access) with `Expr`'s recursive fields (`Add`, `Compare`, `MethodCall`,
  `Index`, `ArrayLit`'s elements, `Lambda`'s body, etc.) and every
  `Vec<Stmt>` body (function/method/lambda bodies, `if`/`while`/
  `begin`/`rescue` blocks) changed to hold `Spanned<Expr>`/`Spanned<Stmt>`
  instead of the bare type. `grammar.lalrpop`'s productions for `Expr`
  and `Stmt` are each wrapped with `<l:@L> ... <r:@R> => Spanned { span:
  (l, r), node: ... }`.

### 2. Acceptance Criteria
1. `parse("def add(a: Int64, b: String) -> Int64\n  a + b\nend")`
   succeeds and the resulting `Program`'s `Stmt::Expr(Spanned { node:
   Expr::Add(lhs, rhs), .. })` inside `add`'s body has `rhs.span` equal
   to the real byte offset of the *second* occurrence of `b` in the
   source (verified against `src.match_indices("b").nth(1)`, not just
   "some span exists").
2. Every span is non-degenerate for real source (`span.1 > span.0` for
   every multi-character token span; single-character tokens like `+`
   have `span.1 == span.0 + 1`).
3. Regression: every existing `emerald-parser` test — every prior plan's
   worked example (`hello.em`, milestone-2, `Point`, arrays, lambdas,
   exceptions, modules) — still parses to an AST equal to before modulo
   the new `Spanned` wrapper (i.e. `.node` matches the old value
   exactly); `PartialEq` on `Spanned<T>` compares both `span` and `node`
   deliberately (not `node`-only) so this is checked structurally, not
   asserted by inspection.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (adds `Spanned<T>`,
  rewraps `Expr`/`Stmt` field types), `crates/emerald-parser/src/
  grammar.lalrpop` (every `Expr`/`Stmt` production gains `<l:@L> ...
  <r:@R>`), `crates/emerald-parser/src/lib.rs` (tests updated for the
  new AST shape)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALRPOP conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new span-offset assertions | agent-claimed-locally |

---

## Leaf: leaf-sema-spans

### 1. Context
- Why: `emerald_sema::Diagnostic { message: String }` has no span field;
  its 40 construction call sites (confirmed this session, across
  `resolve_type`, `infer_expr_type`, `infer_array_lit_type`,
  `check_args`, `check_set_index`, `check_stmt`, `check_begin`,
  `check_implicit_return`) all report a bare message with no position.
- Target state: `Diagnostic` gains `pub span: (usize, usize)`.
  `infer_expr_type`/`check_stmt`/etc. change their signatures to accept
  `&Spanned<Expr>`/`&Spanned<Stmt>` (leaf 1) and populate each
  `Diagnostic`'s span from the specific node the Decision log assigns it
  to — the mismatched sub-expression for type errors, the whole call for
  arity errors, the enclosing statement as the fallback for
  no-better-candidate diagnostics (`break`/`next` outside a loop, etc.).

### 2. Acceptance Criteria
1. `check_program`, run on this plan's concrete-proof program, returns
   exactly one `Diagnostic` whose `span` equals the real byte offset of
   the second `b` (the same assertion as leaf 1 AC1, now observed at the
   `Diagnostic` level, proving the span actually threads from the parser
   through sema, not just that the AST carries one internally).
2. `rejects_arity_mismatch_at_call_site`'s existing fixture (`puts
   add(20)`) produces a `Diagnostic` whose span covers the whole
   `add(20)` call expression, not just `add` or just `20` — verified
   against the real byte range of the substring `"add(20)"`.
3. `rejects_break_outside_loop`'s existing fixture produces a
   `Diagnostic` whose span equals the `break` statement's own span
   (the honest fallback case) — still a real, correct, non-degenerate
   span, not a leftover whole-document range.
4. Regression: every existing `emerald-sema` test's message-content
   assertions (`msg.contains("Int64")`, etc.) keep passing unmodified —
   this plan adds a field, it doesn't change any diagnostic's wording.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. new span assertions | agent-claimed-locally |
| Workspace (codegen compiles against new AST shape) | `cargo build --workspace` | clean | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass, no codegen test output changes | agent-claimed-locally |

---

## Leaf: leaf-span-rendering

### 1. Context
- Why: three call sites currently render sema diagnostics with no
  position at all: `emerald-cli`'s sema-error branch (`eprintln!("error:
  {}", d.message)`, plain text, no miette involved), `emerald-lsp`'s
  `publishDiagnostics` (plan 17: whole-document `Range` — its own
  disclosed gap), and `emerald-mcp`'s `check_source` tool (plan 17: same
  whole-document caveat). All three become real per-diagnostic ranges
  now that leaf 2 provides one.
- Target state: `emerald-cli` renders sema diagnostics through `miette`
  too (matching plan 13's parse-error rendering quality, finally closing
  inception §14.4's own worked example for *both* diagnostic sources,
  not just parse errors) — `Diagnostic` gains a small
  `miette::Diagnostic`-deriving wrapper carrying the byte span + source
  text, mirroring `emerald_parser::ParseError`'s existing shape.
  `emerald-lsp` and `emerald-mcp` both convert `Diagnostic::span` (byte
  offsets) to a real line/column `Range`/`(line, column)` pair instead
  of the whole-document fallback.

### 2. Acceptance Criteria
1. Running `emerald-cli` on this plan's concrete-proof program prints a
   `miette`-rendered diagnostic with a caret pointing at `b` on line 2 —
   real executed proof, the same standard plan 13 used for parse errors,
   now met for a sema error too.
2. `emerald-lsp`'s existing diagnostics integration test (plan 17) is
   extended: opening a document with this plan's concrete-proof program
   produces a `publishDiagnostics` notification whose `range.start`
   column equals `b`'s real column on line 2 (UTF-16-converted) — not
   `(0, 0)` anymore.
3. `emerald-mcp`'s existing `check_source` tool test (plan 17) is
   extended the same way: the returned diagnostic's line/column matches
   `b`'s real position, not a whole-document placeholder.
4. Regression: plan 17's own "happy path produces zero diagnostics"
   tests (`examples/hello.em` through both the LSP and the MCP server)
   keep passing unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs` (and/or `crates/
  emerald-driver/src/lib.rs`, wherever plan 17 landed the rendering
  call), `crates/emerald-lsp/src/main.rs`, `crates/emerald-mcp/src/
  main.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| CLI | `cargo test -p emerald-cli` | all pass, incl. new miette sema-rendering assertion | agent-claimed-locally |
| LSP | `cargo test -p emerald-lsp` | all pass, incl. updated span assertion | agent-claimed-locally |
| MCP | `cargo test -p emerald-mcp` | all pass, incl. updated span assertion | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **Warnings / non-error diagnostics** — `Diagnostic` stays
  error-only, same as every prior plan; a severity field is a natural,
  separate follow-up.
- **Multi-label diagnostics** (pointing at more than one span per
  diagnostic — e.g. "expected type declared here, found type here") —
  this plan gives every diagnostic exactly one span, the same
  single-label standard plan 13 set for parse errors.
- **Codegen-time diagnostics** — codegen has never produced a
  diagnostic (only internal `Result<_, String>` guards); out of scope,
  same line plan 13 drew between parse-time and everything-else.
- **Upgrading plan 21's go-to-definition to use real declaration
  spans** — a real, natural follow-up once this lands; not attempted
  here (see Decision log).
- **Per-name sub-spans inside `Call`/`New`/method-call nodes** (pointing
  at just the callee identifier rather than the whole call) — `Call`/
  `New` store their name as a bare `String`; a real future refinement,
  not required by this plan's concrete proof.
