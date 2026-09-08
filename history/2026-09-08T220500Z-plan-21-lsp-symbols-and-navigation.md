---
name: LSP Symbols and Navigation
overview: "A real symbol table exposed from emerald-sema, plus go-to-definition, completion, and symbol-aware semantic tokens in emerald-lsp — the named follow-up to plan 17's own disclosed gap that emerald-sema exposes no symbol table at all."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-symbol-table
    content: "emerald-sema gains a public SymbolTable/collect_symbols(); emerald-driver gains symbols(source, name) so emerald-lsp can reach it without depending on sema/parser directly"
    status: pending
  - id: leaf-go-to-definition
    content: "textDocument/definition in emerald-lsp, scoped to top-level function/class/module names only — a disclosed, regex-based text search, not real span tracking"
    status: pending
  - id: leaf-completion
    content: "textDocument/completion in emerald-lsp: reserved keywords + top-level symbol names, degrading to keywords-only on an unparseable buffer"
    status: pending
  - id: leaf-semantic-tokens
    content: "textDocument/semanticTokens/full in emerald-lsp, using the symbol table to tag user-defined class/function names beyond what the plan-17 TextMate grammar can do statically"
    status: pending
isProject: false
---

# Plan 21 — LSP Symbols and Navigation

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
same posture as [plan 17](./2026-09-08T213000Z-plan-17-ide-integration.md):
new, post-v1 tooling scope. It is the explicit, named follow-up to plan
17's own Decision log entry: "The LSP proves the diagnostics path
end-to-end and stops there — no semantic tokens, go-to-definition,
completion, rename, or formatting... `emerald-sema::check_program`
doesn't return or expose a symbol table... go-to-definition/completion
need real sema surface-area work this plan doesn't do." This plan does
exactly that work, and nothing plan 17 didn't already scope out for a
follow-up. It modifies `crates/emerald-sema`, `crates/emerald-driver`,
and `crates/emerald-lsp` — all three created or extracted in plan 17;
nothing here is a new crate.

Concrete proof this plan targets: in an editor with `emerald-lsp`
running, placing the cursor on `add` in `examples/hello.em`'s
`puts add(20, 22)` and invoking go-to-definition jumps to that file's
own `def add(...)` line; invoking completion mid-edit on
`examples/classes.em` offers `Counter`, `Point`, and the reserved
keywords; and `Counter`/`Point` render as a distinct "class" token
wherever they're used, not just at their declaration, without needing a
tree-sitter grammar or hand-maintained keyword list per class.

## Decision log

- **The symbol table is a thin, disclosed export of information
  `emerald-sema` already computes internally — not new analysis.**
  `check_program`'s two-pass registration already builds
  `HashMap<String, ClassInfo>` (fields + methods, tagged
  `is_module`) and `HashMap<String, FunctionSig>` (verified this
  session, `crates/emerald-sema/src/lib.rs` lines 767–826) — it just
  never returns them. This plan factors that registration into a
  reusable path and exposes a `pub` DTO shape over it
  (`SymbolTable`/`ClassSymbol`/`FunctionSymbol`), keeping the internal
  `ClassInfo`/`FunctionSig` structs private — no encapsulation is given
  up to get navigation working.
- **`collect_symbols` is best-effort and never fails on a body-level
  type error.** It only runs the two *registration* passes (class/module
  names, then field/method signatures, then free-function signatures) —
  the same passes `check_program` already separates from full
  body-checking (verified: `check_program` builds `classes`/`sigs`
  fully before checking any function/method/statement body). A program
  with a live type error somewhere in a method body still yields a
  complete, usable symbol table for every class/function whose
  *signature* resolved cleanly — exactly what an editor needs while a
  user is mid-edit with an unrelated error elsewhere on screen. A
  declaration whose own signature fails to resolve (e.g. a field of an
  unknown type) is simply omitted from the table rather than aborting
  it entirely, the same graceful-degradation shape `check_program`
  already uses when accumulating multiple diagnostics instead of
  stopping at the first one.
- **`emerald_driver` gains exactly one new function, `symbols(source,
  name) -> Result<emerald_sema::SymbolTable, DriverError>`**, sitting
  alongside plan 17's `check`/`compile` — parses via
  `emerald_parser::parse_named`, then calls `collect_symbols`; a parse
  failure reuses the existing `DriverError::Parse` variant rather than
  inventing a new error path. `emerald-lsp` keeps depending only on
  `emerald-driver`, never directly on `emerald-parser`/`emerald-sema` —
  the same boundary plan 17's `leaf-lsp-server` already established.
- **Go-to-definition is a regex/text search over the open buffer, not
  real span tracking — and is scoped to top-level names only
  (functions, classes, modules), explicitly excluding methods.**
  Neither the parser's AST nor `emerald-sema` carry declaration-site
  source positions (confirmed: only `ParseError`, plan 13's parse-time
  type, carries a real `SourceSpan`). `def`/`class`/`module` are all
  LALR(1)-reserved keywords (verified in `grammar.lalrpop`), so a
  `\b(?:def|class|module)\s+NAME\b`-shaped regex search over the raw
  document text is a real, correct position source for *top-level*
  declarations — word-boundary-anchored specifically because a naive
  substring search breaks the moment two top-level names share a
  prefix (`class Point` is a literal substring of a hypothetical
  `class PointCloud`). Methods are excluded because
  `examples/classes.em` already contains a real collision that a
  document-wide text search cannot resolve: both `Counter` and `Point`
  declare a method named `initialize`. Telling which `def initialize`
  a given `c.initialize`-shaped call means requires knowing `c`'s
  static type — real expression-level type inference this leaf doesn't
  add. Go-to-definition on a method-call target returns no location
  (see `leaf-go-to-definition` AC3) rather than guessing wrong.
- **Completion has no ranking, no fuzzy matching, no trigger
  characters** — a flat concatenation of the reserved-keyword list
  (the same ground-truth terminal set plan 17's TextMate grammar
  already uses) and the current buffer's top-level symbol names; the
  editor's own client-side filtering does the rest, the same as any
  minimal LSP completion provider. On an unparseable buffer (the common
  mid-edit case), completion falls back to the keyword-only list rather
  than erroring — `emerald_driver::symbols` failing is an expected,
  handled state here, not a bug.
- **Semantic tokens reuse the symbol table for a real (if modest)
  advance over plan 17's static TextMate grammar — classifying
  user-defined class/function names, not just fixed keywords.** A
  TextMate grammar is per-language and static; it cannot know that
  `Counter` is a class name in *this* file without a general-purpose
  heuristic. This leaf's tokenizer looks up every bare identifier
  against `collect_symbols`'s real table for the open buffer and tags
  known class names / function names accordingly, wherever they occur
  (declaration or use) — genuinely more accurate than a regex grammar
  can be, without requiring full type inference. It does **not**
  distinguish a declaration occurrence from a use occurrence, does not
  type-color local variables, and does not resolve method-call targets
  (same method-disambiguation gap as go-to-definition). Real
  type-aware, per-expression semantic tokens need spans threaded
  through `emerald-sema` — separate, future work, not this plan's job.
- **No multi-file / cross-document symbol resolution.** There is no
  multi-file compilation in this compiler yet (`emerald-cli`/
  `emerald-driver` take exactly one source unit) — a separate,
  not-yet-written plan may add that; this plan does not assume it
  exists and scopes every leaf to a single open document.

## Leaf: leaf-symbol-table

### 1. Context
- Why: `emerald-sema::check_program` builds full class/module/function
  registries internally and discards them the moment type-checking
  finishes — nothing outside the crate can ask "what classes/functions
  does this program declare, and what are their signatures?"
- Target state: `crates/emerald-sema/src/lib.rs` gains `pub struct
  FunctionSymbol { pub params: Vec<Type>, pub return_type: Type }`,
  `pub struct ClassSymbol { pub is_module: bool, pub fields:
  HashMap<String, Type>, pub methods: HashMap<String, FunctionSymbol>
  }`, `pub struct SymbolTable { pub functions: HashMap<String,
  FunctionSymbol>, pub classes: HashMap<String, ClassSymbol> }`, and
  `pub fn collect_symbols(program: &Program) -> SymbolTable`, built by
  factoring `check_program`'s existing two registration loops (class/
  module name pre-registration, then field/method/signature resolution)
  into a shared private helper both `check_program` and
  `collect_symbols` call — `check_program`'s own behavior and existing
  diagnostics are unchanged, only its internal structure is
  reorganized. `crates/emerald-driver/src/lib.rs` gains `pub fn
  symbols(source: &str, name: &str) -> Result<emerald_sema::
  SymbolTable, DriverError>` (parse, then `collect_symbols`; parse
  failure maps to the existing `DriverError::Parse`).

### 2. Acceptance Criteria
1. `collect_symbols` on `examples/hello.em`'s real contents returns a
   `SymbolTable` whose `functions` contains `"add"` with `params ==
   [Type::Int64, Type::Int64]` and `return_type == Type::Int64`.
2. `collect_symbols` on `examples/classes.em`'s real contents returns
   `classes` containing both `"Counter"` (field `value: Int64`,
   methods `initialize`/`value`/`add`) and `"Point"` (fields `x`/`y:
   Float64`, methods `initialize`/`sum`) — both classes, both method
   sets, proving the two-pass registration survived the refactor
   intact.
3. `collect_symbols` on a program with one class whose field names an
   unknown type still returns a table containing every *other*
   class/function that resolved cleanly — proof the degrade is
   per-declaration, not all-or-nothing (see Decision log).
4. `emerald_driver::symbols` returns `Err(DriverError::Parse(_))` for
   an unparseable buffer and `Ok(SymbolTable)` for `examples/hello.em`
   and `examples/classes.em`'s real contents, called directly on
   in-memory `&str` with no filesystem write — same in-memory
   discipline as plan 17's `leaf-driver-extraction` AC2.
5. Regression: `cargo test -p emerald-sema` — every existing
   `check_program`-based test still passes unmodified (the
   registration refactor is behavior-preserving).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-driver/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. existing suite unmodified | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | new `symbols` tests + existing `check`/`compile` tests pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-go-to-definition

### 1. Context
- Why: `leaf-symbol-table` exposes *what* is declared but not *where* —
  nothing in `emerald-lsp` answers `textDocument/definition` yet.
- Target state: `crates/emerald-lsp` adds a `textDocument/definition`
  handler: extract the identifier under the request's cursor position
  from the document's own text (a small `word_at_position` helper,
  reusing the UTF-16 `Position`-to-byte-offset conversion plan 17's
  `leaf-lsp-server` already built for diagnostics); look the word up in
  `emerald_driver::symbols(text, uri)`'s table; if it names a top-level
  function/class/module, run a word-boundary-anchored regex
  (`\bdef\s+NAME\b`, `\bclass\s+NAME\b`, or `\bmodule\s+NAME\b`
  depending on which map it was found in) over the raw text, convert
  the first match's byte offset to a UTF-16 `Position`, and return a
  `Location` spanning just `NAME` at that declaration site. Any other
  case (a keyword, a method name, an unresolved identifier) returns an
  empty response — see Decision log.

### 2. Acceptance Criteria
1. Real test: a `textDocument/definition` request at `add`'s position
   in `examples/hello.em`'s `puts add(20, 22)` line returns a
   `Location` whose range start matches the real byte offset of `add`
   in `"def add(a: Int64, b: Int64) -> Int64"` (verified against the
   source string's own `.find("def add")`, not just "some location").
2. Real test: a request at `Point`'s position in `examples/classes.em`'s
   `p: Point = Point.new(2.0, 3.0)` line returns the location of the
   *second* `class` declaration in the file (`class Point`), not the
   first (`class Counter`) — proof the search finds the right
   declaration, not just the first one in the file.
3. Real test: a request at `value`'s position in `c.value` (a method
   call on a `Counter` instance) returns an empty/null response, not a
   location — proof the method-go-to-def scope cut (Decision log) fails
   safely rather than silently pointing at the wrong class's method.
4. A request on a keyword (`if`) or an identifier with no matching
   declaration returns an empty/null response, not an error or a panic.

### 3. File & Module Structure
- **Modify:** `crates/emerald-lsp/src/main.rs`
- **Create:** `crates/emerald-lsp/tests/definition.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-lsp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-lsp` | all 4 new tests pass + plan 17's existing diagnostics tests unmodified | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-completion

### 1. Context
- Why: nothing in `emerald-lsp` answers `textDocument/completion` —
  every editor's autocomplete is silent for `.em` files today.
- Target state: a `textDocument/completion` handler returning a flat
  list combining the reserved-keyword set (`class`, `module`, `def`,
  `end`, `if`, `else`, `while`, `return`, `break`, `next`, `puts`,
  `raise`, `begin`, `rescue`, `new`, `Array`, `Proc` — the same
  ground-truth terminal list plan 17's TextMate grammar already uses)
  with `emerald_driver::symbols`'s top-level function/class/module
  names for the current buffer. On a parse failure, returns the
  keyword-only list (see Decision log) instead of an error response.

### 2. Acceptance Criteria
1. A completion request against `examples/hello.em`'s real contents
   returns a list including `add` (the real declared function) and at
   least `def`, `end`, `if`, `class` from the keyword set.
2. A completion request against plan 13's own known-bad buffer
   (`x: Int64 = +`) still returns the non-empty keyword-only list —
   the graceful degrade path, not an error.
3. Regression: `crates/emerald-lsp/tests/diagnostics.rs` (plan 17) and
   `tests/definition.rs` (this plan's `leaf-go-to-definition`) both
   still pass unmodified — completion is an additive handler.

### 3. File & Module Structure
- **Modify:** `crates/emerald-lsp/src/main.rs`
- **Create:** `crates/emerald-lsp/tests/completion.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-lsp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-lsp` | all pass, incl. regressions | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-semantic-tokens

### 1. Context
- Why: `emerald-lsp` publishes no `textDocument/semanticTokens`, and
  plan 17's TextMate grammar — necessarily static — has no way to know
  that `Counter` is a class name in this particular file.
- Target state: `emerald-lsp` advertises a `semanticTokensProvider`
  legend (`keyword`, `type`, `class`, `function`, `variable` for
  instance vars, `number`) and implements `textDocument/
  semanticTokens/full`: a line-by-line regex pass (the same lexical
  categories as plan 17's TextMate grammar) that additionally
  cross-references every bare identifier against
  `emerald_driver::symbols`'s table for the open buffer, tagging a
  match against `classes` as `class` and a match against `functions`
  as `function` wherever it occurs — the real advance over static
  TextMate coloring (see Decision log). Tokens are emitted in the LSP
  spec's required delta-line/delta-start-char relative encoding.

### 2. Acceptance Criteria
1. Real test: semantic tokens for `examples/classes.em` tag `Counter`
   as a `class` token both at its `class Counter` declaration and at
   its later use as a type annotation (`c: Counter = ...`) — proof the
   symbol-table cross-reference fires at every occurrence, not just the
   declaration.
2. Real test: semantic tokens for `examples/hello.em` tag `add` as a
   `function` token at its call site (`puts add(20, 22)`) and tag
   `def`/`end`/`puts` as `keyword` tokens.
3. The test decodes the response's delta-encoded `data` array back into
   absolute `(line, character, length)` triples and checks them against
   the real expected positions (found via `.find()` on the source
   string) — real proof the relative encoding is correct, not just that
   some numbers came back.

### 3. File & Module Structure
- **Modify:** `crates/emerald-lsp/src/main.rs`
- **Create:** `crates/emerald-lsp/tests/semantic_tokens.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-lsp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-lsp` | all pass, incl. all prior leaves' regressions | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **Rename, find-all-references, workspace-wide symbol search** — all
  need either cross-file resolution (no multi-file compilation exists;
  see Decision log) or real reference-tracking this plan's textual
  go-to-definition doesn't build.
- **Method go-to-definition and member/method completion on a
  receiver** — both need expression-level type inference at the
  cursor to know a receiver's static type; see Decision log's
  `examples/classes.em` `initialize`-collision evidence.
- **True type-aware semantic tokens** (per-local-variable inferred
  type, declaration-vs-use distinction) — needs spans threaded through
  `emerald-sema`, a separate, not-yet-written future plan; this leaf's
  symbol-table cross-reference is a real but modest step, not that.
- **Formatting (`textDocument/formatting`)** — untouched by this plan;
  no formatter exists anywhere in this project yet.
- **Incremental re-parsing on every keystroke** — unchanged from plan
  17: full-buffer re-check via `emerald_driver`, no incremental
  compiler exists (inception §14.5's `salsa` deferral still applies).
- **Wiring this plan into `plan-of-plans.md`** — same deferral as plan
  17, until the concurrently-authored plan 16 lands.
