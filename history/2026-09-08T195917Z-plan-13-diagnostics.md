---
name: Diagnostics
overview: "miette-rendered parse errors with a source snippet and a caret span — inception §14.4's quality bar, proven on a real invalid program."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-parser-spans
    content: "emerald_parser::ParseError carrying a byte-offset SourceSpan + the original source, built from lalrpop_util::ParseError's own location info"
    status: pending
  - id: leaf-cli-miette
    content: "emerald-cli renders ParseError via miette's fancy GraphicalReportHandler instead of a bare eprintln! string"
    status: pending
isProject: false
---

# Plan 13 — Diagnostics

This is `diagnostics`, row `13` of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §14.4's `miette` investigation. Inception frames
this as "the compiler should **eventually** produce diagnostics of
roughly this quality" — an explicit eventual/exploratory bar, not an
immediate hard requirement on every diagnostic path — and gives one
worked example (a `+` type mismatch rendered with a source snippet, a
caret-underlined span, and a help line). This plan's concrete proof,
self-designed since inception gives no parse-error-specific example:

```ruby
x: Int64 = +
```

Run through `emerald-cli`, this should render a `miette`-formatted
diagnostic — file-relative source snippet, line/column, and a caret
pointing at the exact offending token (`+`) — not the bare
`parse error: Unrecognized token '+' found at 11:12...` string
`emerald-cli` prints today.

## Decision log

- **Scope: parse-time diagnostics only. Sema/codegen diagnostics stay
  text-only in this plan.** `emerald_sema::Diagnostic` is already
  documented as having "no source-span tracking yet... Line/column-precise
  diagnostics are `13 diagnostics`'s job" — but adding real spans to sema
  diagnostics means adding a span to `Expr`/`Stmt` itself (every variant,
  across the parser AST, propagated through every sema/codegen call
  site that constructs or matches one), which is an invasive, whole-AST
  change disproportionate to one plan, and risks regressing the 12 prior
  plans' worth of working AST-consuming code for a diagnostics-quality
  improvement. Parse errors, by contrast, get real spans essentially for
  free: LALRPOP already tracks byte offsets internally for every token
  and captures them in `lalrpop_util::ParseError`'s own variants — no AST
  changes needed at all, just a richer error type at the parser's public
  boundary. This is consistent with `spec/COMPILER.md`'s posture of
  deferring `emerald-ir`/typed-IR work: sema-level span-aware diagnostics
  are real, valuable follow-up work, explicitly not this plan's job.
  Inception's own example (a `+` type mismatch) is illustrative of the
  *target rendering quality*, not a literal requirement that this exact
  diagnostic be the one that gets it — a parse error demonstrates the
  same rendering quality with a fraction of the plumbing.
- **No `rowan` lossless syntax tree, no `salsa` incremental
  compilation.** Both are inception §14.3/§14.5's *own* explicitly-gated
  investigations ("do not introduce solely for theoretical purity" /
  "do not add unless it materially simplifies the architecture") — this
  plan doesn't need either: `miette` only needs a byte-offset span and
  the original source text, both of which LALRPOP and the CLI's own file
  read already provide directly.
- **`miette`'s `fancy` feature** (the graphical, colored,
  box-drawing `GraphicalReportHandler`) is enabled in `emerald-cli` only
  — the crate that actually renders diagnostics to a terminal.
  `emerald-parser` only needs the `derive` feature (to build `ParseError`
  values), not a renderer.
- **One `lalrpop_util::ParseError` variant (`User`, the fallible
  `=>?` assignment-target check) has no location info to report** — its
  span defaults to byte `0` rather than threading a location marker
  through that one action. A real, small, disclosed gap (that one error
  message just won't point at anything useful yet); every other parse
  error variant (`InvalidToken`, `UnrecognizedEof`, `UnrecognizedToken`,
  `ExtraToken`) carries a genuine byte offset already.

## Leaf: leaf-parser-spans

### 1. Context
- Why: `emerald_parser::parse` returns `Result<Program, String>` —
  `lalrpop_util::ParseError`'s own byte-offset location info is
  discarded via `.to_string()` before it ever reaches a caller.
- Target state: a new `pub struct ParseError` (in `emerald-parser`)
  deriving `thiserror::Error` + `miette::Diagnostic`, carrying the
  original source (`#[source_code]`) and a `SourceSpan` (`#[label]`)
  built from whichever byte offset(s) the matched `lalrpop_util::
  ParseError` variant carries. `parse`'s signature changes to
  `Result<Program, ParseError>`.

### 2. Acceptance Criteria
1. `parse("x: Int64 = +\n")` returns `Err(ParseError)` whose span points
   at the `+` token's actual byte offset (verified against the source
   string's own `.find('+')`, not just "some span exists").
2. `ParseError` implements `miette::Diagnostic` and `std::error::Error`
   — `miette::Report::new(err)` is constructible, proving it satisfies
   the trait bound `leaf-cli-miette` needs.
3. Regression: `cargo test -p emerald-parser` — every existing
   `.expect("... should parse")` call site still compiles (`Result::
   expect` only requires `E: Debug`, satisfied by `#[derive(Debug)]` on
   `ParseError`) and every existing parse still succeeds identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/lib.rs`,
  `crates/emerald-parser/Cargo.toml` (adds `miette`, `thiserror`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |

---

## Leaf: leaf-cli-miette

### 1. Context
- Why: `emerald-cli`'s parse-error branch does `eprintln!("parse error:
  {e}")` on the old bare-`String` error — no source snippet, no caret.
- Target state: the parse-error branch instead prints
  `miette::Report::new(err)`'s `{:?}` (Debug) rendering, which — with
  `emerald-cli`'s `miette` dependency's `fancy` feature enabled —
  produces the graphical, source-snippet-and-caret rendering inception
  §14.4 asks for.

### 2. Acceptance Criteria
1. Running `emerald-cli` on a file containing `x: Int64 = +\n` prints a
   diagnostic containing the source line, a caret/underline at the `+`
   token, and the file path — not the old bare one-line string. Verified
   by actually running the built `emerald-cli` binary against a real
   temp file and inspecting its captured stderr (real executed proof,
   not simulated).
2. `emerald-cli` still exits non-zero on a parse error (unchanged
   behavior, just better rendering).
3. Regression: `emerald-cli`'s existing `hello_em.rs` integration test
   (successful compile-and-run path) still passes unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs`,
  `crates/emerald-cli/Cargo.toml` (adds `miette` with `fancy`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-cli` | all pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- Span-aware sema/codegen diagnostics (inception's own literal `+` type
  mismatch example) — see Decision log; requires threading spans through
  the whole AST.
- `rowan` lossless syntax trees, `salsa` incremental compilation — both
  explicitly gated by inception itself; not needed for this plan's proof.
- `help:`-style secondary annotations, multi-label diagnostics, warning
  (non-error) diagnostics — this plan proves the core "real span,
  real rendering" claim with a single-label error; richer diagnostic
  shapes are natural, low-risk follow-ups once spans exist at all.
