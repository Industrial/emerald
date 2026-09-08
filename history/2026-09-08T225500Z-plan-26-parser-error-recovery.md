---
name: Parser Error Recovery
overview: "Multi-error parse recovery at top-level Item boundaries — a file with two independent syntax errors reports both in one pass instead of stopping at the first, so crates/emerald-lsp's live diagnostics show every problem in a buffer at once."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-parser-recovery
    content: "grammar.lalrpop's Item production gains a `!` recovery alternative producing Item::Error; parse_named/parse return Result<Program, Vec<ParseError>>, capped at 50 recovered errors"
    status: pending
  - id: leaf-consumer-updates
    content: "emerald-driver's DriverError::Parse, emerald-cli's rendering loop, and emerald-lsp's diagnostic conversion all updated from one ParseError to Vec<ParseError>"
    status: pending
isProject: false
---

# Plan 26 — Parser Error Recovery

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
same posture as
[plan 17](../../history/2026-09-08T213000Z-plan-17-ide-integration.md):
new, post-v1 tooling scope, not an inception §§ item. `plan-of-plans.md`
is left untouched here.

Concrete proof: today, `emerald_parser::parse_named` on a file containing
two independent, unrelated top-level syntax errors reports only the
first — the second is never seen until the first is fixed and the file
is reparsed. This plan makes it report both in one pass:

```ruby
def broken_one(a: Int64 -> Int64
  a + 1
end

def broken_two(b: Int64) -> Int64
  b +
end
```

(`broken_one` is missing the closing `)` before `->`; `broken_two`'s body
ends on a dangling `+` with nothing after it — two unrelated mistakes, in
two unrelated functions.) This directly serves `crates/emerald-lsp`
(plan 17's `leaf-lsp-server`, and any future `21`-numbered LSP-v2 work):
`textDocument/publishDiagnostics` already accepts an array of
diagnostics — plan 17's sema path already sends more than one this way
(`emerald_sema::check_program` already returns `Vec<Diagnostic>`,
verified this session) — but the parse-error path has never had more
than one to send, because `emerald_parser::parse_named` returns
`Result<Program, ParseError>`, a single error, and LALRPOP stops at the
first syntax error it hits. No LSP-side plumbing changes: only the
parser needs to start *producing* more than one error; the LSP already
knows how to *consume* an array.

## Decision log

- **LALRPOP's own built-in error-recovery mechanism is used, not a
  hand-rolled resync-by-skipping-tokens scheme.** `crates/emerald-parser/
  Cargo.toml` pins `lalrpop-util = "0.22"` (verified this session) —
  LALRPOP has shipped a `!`-marked error-recovery mechanism since its
  0.19 series; its own manual describes it as an experimental but real,
  usable feature: mark a production alternative with `!`, and on failure
  LALRPOP skips tokens in panic-mode fashion until it can resynchronize
  at that production's boundary, collecting an `ErrorRecovery` value
  instead of aborting the whole parse. This is the "second, parallel
  parser implementation" risk plan 17 explicitly avoided for tree-sitter
  — but doesn't apply here, since this is the *same* LALRPOP grammar
  gaining a feature it already ships, not a second grammar to maintain.
- **Recovery is scoped to top-level `Item` boundaries only — not inside
  a function/class/method body's `Stmt*` list.** LALRPOP's `!` marker is
  added per-production; the natural, narrow first cut is the `Item`
  production itself (`Item: Item = { FuncDef, ClassDef, ModuleDef, Stmt,
  ! => Item::Error }`), which resynchronizes at the start of the next
  top-level construct. Adding the same marker inside every `Stmt*`
  occurrence (`If`'s `then_branch`/`else_branch`, `While`'s `body`,
  `Begin`'s `body`/`rescue_body`, `FuncDef`'s `body`) is a much larger,
  separately-riskier change — each is a different recovery boundary with
  its own resynchronization token set, and getting panic-mode recovery
  wrong inside a statement list risks silently swallowing real code, not
  just a malformed one. This plan's concrete proof (two broken top-level
  `def`s) only needs the `Item`-level cut; an error occurring *inside* a
  function body still hard-stops the whole parse with a single
  `ParseError`, exactly as today. Within-body recovery is real,
  legitimate follow-up work, not solved here.
- **A new `Item::Error` AST variant represents a recovered, unusably
  malformed top-level item — and it never reaches `emerald-sema` or
  `emerald-codegen`.** `parse_named` returns `Ok(Program)` only when
  *zero* items were recovered (i.e. `Program.items` contains no
  `Item::Error` at all — behaviorally identical to today's all-or-
  nothing success case); the moment one or more top-level items fail,
  the whole call returns `Err(Vec<ParseError>)` instead, the same
  short-circuiting shape `emerald-driver`'s pipeline (plan 17) already
  assumes for a parse failure. This sidesteps the harder question of
  "should sema try to check the parts that *did* parse around a broken
  item" entirely — it doesn't, by construction, in this plan. The real,
  mechanical cost this still imposes: adding a variant to `Item` breaks
  every *exhaustive* `match` over it, so `emerald-sema::check_program`
  and `emerald-codegen`'s program-iteration code each need one new arm —
  verified via `cargo build --workspace`, not assumed. Each new arm is
  `unreachable!("Item::Error never survives into a returned Ok(Program)")`,
  consistent with the guarantee above, rather than a real defensive
  `Err` — there is no reachable input that hits it.
- **Recovered errors are capped at 50 per parse call.** Panic-mode
  recovery on a badly malformed file is a known way to produce a flood
  of low-quality, cascading secondary errors (one real mistake can
  desynchronize the parser badly enough to misreport many downstream
  non-problems as separate errors). Fifty is an arbitrary but generous
  ceiling — this plan's own concrete proof uses exactly two errors, far
  under it, so the cap is never exercised by the acceptance criteria
  themselves; it exists purely as a disclosed robustness bound on
  worst-case output, not a limit this plan's own proof depends on.
- **`ParseError`'s own shape (one `message`/`#[source_code]`/`#[label]`
  span, plan 13) is unchanged.** Only the *return type wrapping it*
  changes, from `ParseError` to `Vec<ParseError>` — each recovered
  `lalrpop_util::ErrorRecovery`'s inner `lalrpop_util::ParseError` is
  converted through the exact same `to_parse_error` helper plan 13
  already wrote, called once per recovered error instead of once per
  parse call. `emerald-parser`'s own existing single-error tests
  (`missing_end_errors`, `parse_error_span_points_at_the_offending_token`)
  keep their exact assertions, just against `errs[0]` instead of a bare
  `e` — a mechanical, disclosed test update, not a behavior change.

## Leaf: leaf-parser-recovery

### 1. Context
- Why: `grammar.lalrpop`'s `Item` production has no `!` alternative —
  confirmed this session (full grammar file re-read: no `!` marker
  anywhere) — so LALRPOP aborts the whole parse at the first syntax
  error, exactly as `to_parse_error`/`parse_named`'s current
  `Result<Program, ParseError>` signature implies.
- Target state: `Item: Item = { ...existing alternatives..., ! =>
  Item::Error }` in `grammar.lalrpop`; `Item::Error` added to
  `ast.rs`'s `Item` enum; `parse_named`'s LALRPOP call becomes
  `grammar::grammar::ProgramParser::new().parse(&mut errors, src)` (the
  `&mut Vec<lalrpop_util::ErrorRecovery<usize, T, String>>` parameter
  LALRPOP requires once any `!` exists in the file); `parse_named`
  returns `Ok(program)` when `errors.is_empty() && program.items`
  contains no `Item::Error`, otherwise `Err(vec_of_parse_errors)` — built
  by mapping every recovered `ErrorRecovery.error` plus (if the parse
  itself also hard-failed outside any recovery point) the original
  single error, all through `to_parse_error`, capped at 50 (see Decision
  log). `parse_named`/`parse`'s public signature becomes `Result<Program,
  Vec<ParseError>>`.

### 2. Acceptance Criteria
1. `parse_named` on this plan's two-broken-`def` fixture returns
   `Err(errors)` with `errors.len() == 2`, `errors[0]`'s span pointing at
   `broken_one`'s actual malformed token and `errors[1]`'s span pointing
   at `broken_two`'s — each verified against the source string's own
   token offsets, not just "two errors exist somewhere."
2. A syntax error occurring inside a function body (not at an `Item`
   boundary — e.g. today's plan 13 fixture, `x: Int64 = +`, at the top
   level is actually an `Item::Stmt` parse failure, still covered by the
   `Item`-level `!`; a fixture broken *inside* a `def`'s body instead)
   still returns `Err(vec![single_error])` — length 1, same span/message
   as today, proving within-body behavior is genuinely unchanged, not
   silently altered by adding recovery elsewhere.
3. `cargo build --workspace` succeeds after `Item::Error` is added —
   real proof every exhaustive `match` over `Item` across the workspace
   (`emerald-sema`, `emerald-codegen`) was updated, not just
   `emerald-parser` itself.
4. Regression: `emerald-parser`'s existing single-error tests
   (`missing_end_errors`, `parse_error_span_points_at_the_offending_token`)
   pass against `errs[0]`; every other existing `.expect("...should
   parse")` call site in the crate still parses identically (`Vec<
   ParseError>` still satisfies `Result::expect`'s `E: Debug` bound via
   `ParseError`'s own existing `#[derive(Debug)]`, so those call sites
   need no changes at all).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/ast.rs`, `crates/emerald-parser/src/lib.rs`
  (incl. its own test module)
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs` (one new, `unreachable!`-bodied
  match arm each for `Item::Error` — see Decision log)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (workspace-wide) | `cargo build --workspace` | clean, incl. `Item::Error`'s new match arms | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. the new 2-error and updated single-error cases | agent-claimed-locally |
| Sema/codegen unaffected | `cargo test -p emerald-sema -p emerald-codegen` | all pass unmodified | agent-claimed-locally |

---

## Leaf: leaf-consumer-updates

### 1. Context
- Why: three real call sites assumed exactly one parse error —
  `crates/emerald-driver`'s `DriverError::Parse(ParseError)` and its
  `check`/`compile` pipeline (plan 17's `leaf-driver-extraction`),
  `emerald-cli`'s `eprintln!("{:?}", miette::Report::new(e))` rendering
  branch, and `emerald-lsp`'s parse-error-to-`Diagnostic` conversion
  (plan 17's `leaf-lsp-server`) — each needs updating now that
  `emerald_parser::parse_named` returns `Vec<ParseError>` on failure.
- Target state: `DriverError::Parse(Vec<ParseError>)`;
  `emerald-cli`'s rendering branch loops over the vec, printing one
  `miette::Report` per entry (still exits non-zero once, after printing
  all of them — unchanged overall exit-code behavior); `emerald-lsp`'s
  conversion maps every entry to its own LSP `Diagnostic` and passes the
  resulting `Vec<Diagnostic>` to `publishDiagnostics` in one call — no
  change to the publish call's own shape, which already accepted a
  `Vec` for the sema-diagnostics case.

### 2. Acceptance Criteria
1. Running the real `emerald-cli` binary against this plan's
   two-broken-`def` fixture prints two distinct miette-rendered
   diagnostics (two separate source snippets/carets, one per function)
   to stderr and exits non-zero — verified against the binary's real
   captured stderr, not simulated.
2. `emerald-lsp`'s integration test suite (plan 17's `tests/
   diagnostics.rs`) gains a case: opening the two-broken-`def` fixture
   produces one `publishDiagnostics` notification with
   `diagnostics.len() == 2`, each `range` matching its own error's real
   span.
3. Regression: plan 13's original single-parse-error fixture
   (`x: Int64 = +`) still renders/publishes exactly one diagnostic
   through both `emerald-cli` and `emerald-lsp` — no accidental
   double-reporting or off-by-one introduced by the `Vec`-ification.

### 3. File & Module Structure
- **Modify:** `crates/emerald-driver/src/lib.rs`,
  `crates/emerald-cli/src/main.rs`, `crates/emerald-lsp/src/main.rs`,
  `crates/emerald-lsp/tests/diagnostics.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver -p emerald-cli -p emerald-lsp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver -p emerald-cli -p emerald-lsp` | all pass, incl. new 2-diagnostic cases | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **Recovery inside a function/class/method body's `Stmt*` lists** — see
  Decision log; only top-level `Item` boundaries resynchronize. A single
  malformed statement inside an otherwise-fine function still hard-stops
  the whole parse today, unchanged by this plan.
- **Sema-checking the parts of a file that parsed fine around a broken
  item** — by construction, any recovered error makes the whole call
  `Err`, so `emerald-sema`/`emerald-codegen` never see a partially-valid
  `Program`; see Decision log's `Item::Error`-never-reaches-sema
  guarantee.
- **A configurable or higher error cap** — 50 is a fixed, disclosed
  constant; making it configurable is unnecessary machinery for a
  bound that exists only for worst-case robustness.
- **Improving cascading-error quality** (panic-mode recovery
  occasionally attributing a secondary error to a less-than-ideal
  location after a bad desync) — a known, general limitation of
  syntax-error recovery, not specific to or solved by this plan.
