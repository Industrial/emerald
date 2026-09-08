---
name: Multi-File Compilation
overview: "A `require <path>` top-level item so a real program can span more than one `.em` file — path resolution, cycle detection, and AST-level merging ahead of the existing single-file sema/codegen pipeline."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-grammar-require
    content: "Item::Require(String) + a top-level-only `require <bare/path>` grammar production; `require` reserved"
    status: pending
  - id: leaf-driver-resolution
    content: "crates/emerald-driver gains resolve_program(): DFS require-graph resolution, canonical-path dedup, cycle detection, AST splicing — Item::Require never reaches sema/codegen"
    status: pending
  - id: leaf-cli-lsp-mcp-wiring
    content: "emerald-cli/emerald-lsp/emerald-mcp pass a real filesystem path through so relative requires resolve; emerald-mcp's path-less text tools get a clear, honest error instead of silently ignoring require"
    status: pending
isProject: false
---

# Plan 23 — Multi-File Compilation

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md), for
the same reason plan 17 gave: that table's own Completion note calls
Emerald v1 done as of row 15, and this is new, post-v1 scope. It is not
added to `plan-of-plans.md` or to any other plan's file here, to avoid
colliding with whatever else is concurrently in flight against those
shared documents.

This plan assumes [`plan 17`](./2026-09-08T213000Z-plan-17-ide-integration.md)'s
`leaf-driver-extraction` has already shipped — `crates/emerald-driver`
with `check(source: &str, name: &str)`/`compile(source: &str, name:
&str, output_path: &Path)` existing as that plan describes. Verified
this session: `crates/emerald-driver` does not exist yet in this
checkout — the root `Cargo.toml`'s `[workspace] members` still lists
only the original five crates (`emerald-lexer`, `emerald-parser`,
`emerald-codegen`, `emerald-sema`, `emerald-cli`); plan 16's Cranelift-
vs-LLVM bake-off and plan 17's `emerald-driver` extraction are both,
like this plan, still plan documents rather than executed work. If this
plan is executed before plan 17 lands, extracting the driver is a
prerequisite this plan does not redo — it builds on plan 17's design,
not around its absence.

Concrete proof this plan targets: a real two-file program —
`examples/multi_file/main.em` containing `require helpers` and a call
into a function `helpers.em` defines — compiles, links, and runs for
real through `emerald-cli`, printing the value that function computes,
with no manual concatenation of the two files by hand.

## Decision log

- **New keyword is `require`, not `module`.** This language already
  reserves `module` for an unrelated, existing in-language construct — a
  namespace-only, no-file-relation grouping of static methods (plan 12;
  verified this session against `crates/emerald-parser/src/grammar.
  lalrpop`'s `ModuleDef` production: `"module" <name:Ident> <methods:
  FuncDef*> "end"`). Overloading it for file inclusion would conflate
  two unrelated concepts under one keyword. `require` names the Ruby
  concept this most resembles and isn't taken by anything in this
  grammar.
- **The require path is a bare, unquoted token
  (`require foo/bar`), not a string literal (`require "foo/bar"`).**
  This language has no string literal syntax at all yet — `Type::String`
  exists with no `Expr::StringLit` to construct one (plan 15's finding,
  reconfirmed by plan 17's Decision log). A sibling plan in this same
  batch (string literals) may add one, but making *this* plan depend on
  that one landing first is an unforced, avoidable coupling — a bare
  path token (`r"[A-Za-z_][A-Za-z0-9_/]*"`, no extension, matching how
  `Ident` is already just a regex terminal in this grammar) needs
  nothing this plan doesn't already have. If/when string literals land,
  switching `require`'s argument to a real string is a small, isolated
  follow-up, not blocked by anything this plan does.
- **`require` is a top-level `Item`, not a `Stmt`.** Modeled as
  `Item::Require(String)`, reachable only from `Program`'s `Item*`
  rule — not from any `Stmt*` block — so `require` inside a function
  body, an `if`, or a `while` is a real parse error, not silently
  accepted and ignored. Ruby's `require` is technically legal anywhere
  at runtime; restricting this to top-of-file-equivalent position is a
  deliberate, disclosed narrowing that matches every real use this
  compiler needs to support and keeps resolution a simple pre-pass over
  `Program.items`, never a walk into statement bodies.
- **Resolution is relative to the file *containing* the `require`, not
  the entry file.** (Rust's `mod` and Python's relative imports both
  make this same choice; Ruby's plain `require` — load-path-relative —
  is the one call this project does *not* copy, since there is no
  load-path/stdlib-search-path concept here at all — see Out of scope.)
  This is what makes `require` transitively well-behaved: a required
  file's own `require`s resolve against *its* directory, so moving a
  subtree of files around doesn't silently break requires two hops away
  from the file you moved. `.em` is always implied and appended; there
  is no extension-less-vs-with-extension ambiguity to resolve.
- **Compilation model: parse each file separately (so parse-error spans
  stay real, file-accurate `miette` diagnostics — see plan 13), then
  splice the resolved dependency graph's items into one flat `Program`
  *before* sema or codegen ever runs.** Not genuine separate compilation
  units with cross-object linking (symbol visibility across `.o` files,
  name mangling, multiple objects handed to `cc`) — that's real,
  substantial machinery this compiler has no foundation for yet (no
  incremental compilation, no existing per-unit symbol table; plan 13
  and plan 17 both already declined `salsa` for the same reason). AST
  merging reuses `emerald_sema::check_program` and
  `emerald_codegen::compile_to_object` completely unchanged in their
  actual type-checking/codegen logic — to that code, a merged multi-file
  `Program` looks exactly like one big file, which is the entire point
  of this design. Real separate compilation units (for incremental
  build times at scale) are future work, not required to prove a
  program can span files.
- **`Item::Require` still needs one trivial match arm in
  `emerald-sema` and `emerald-codegen`, not zero changes.** Both crates'
  `Item` matches are exhaustive; adding a new `Item` variant means both
  need an arm for it even though a correctly-resolved `Program` (one
  that went through `emerald-driver`'s new resolution step) never
  contains one by the time either crate sees it. Sema's arm is a no-op;
  codegen's arm defensively returns
  `Err("unresolved require — internal driver bug")`, since reaching it
  means the driver's resolution step was skipped or is broken, not a
  normal user-facing failure. This is a small, real, disclosed touch —
  not the "zero sema/codegen changes" claim this plan would otherwise be
  tempted to make.
- **Diamond dependencies are deduplicated by canonicalized path,
  cycles are detected and rejected.** `resolve_program` tracks two sets
  keyed by `std::fs::canonicalize`d paths: `visited` (fully resolved
  already — load its items at most once, so `main` requiring both `b`
  and `c`, which both require `d`, doesn't duplicate `d`'s
  definitions into the merged `Program` and trip sema's "already
  defined" checks) and an in-progress `stack` (detects `a` requiring `b`
  requiring `a` and returns a real `DriverError` naming the cycle by
  path, rather than recursing until the stack overflows).
- **`emerald_driver::check`/`compile`'s existing `name: &str` parameter
  becomes load-bearing, not cosmetic.** Plan 17 introduced it purely as
  a `miette` source label. This plan requires it to be a real,
  resolvable filesystem path — the base every `require` in that file
  resolves against. A truly location-less in-memory check (a scratch
  buffer with no file on disk) can no longer resolve its own
  `require`s; a real, narrow, disclosed regression for that one edge
  case, acceptable because every real caller (`emerald-cli`'s argument,
  an LSP-open document's URI, an MCP tool call operating on a real
  file) already has an actual path to give it.
- **`emerald-mcp`'s `check_source`/`compile_and_run` tools (plan 17),
  which take bare source text with no filesystem location at all, can't
  resolve a `require` in that text — there is no directory to resolve
  it against.** Rather than silently ignoring the `require` or crashing
  on a missing base path, this plan's `leaf-cli-lsp-mcp-wiring` makes
  that tool call return one clear, documented error
  ("`check_source`/`compile_and_run` do not support `require` — no file
  path context; use a real file via `emerald-cli`/`emerald-lsp`
  instead"). A real, disclosed capability gap for those two tools, not
  a silent correctness hole.
- **Cross-file sema diagnostic attribution in `emerald-lsp` is a
  disclosed, deferred gap, not solved here.** Once files are merged into
  one flat `Program`, a sema diagnostic about something a *required*
  file declared has no item-level "which source file did this come
  from" tag to route it back to the right editor tab/URI —
  `emerald_sema::Diagnostic` has no span *or* source-file field at all
  (plan 13's own already-disclosed gap, restated here in a new context).
  Parse-time diagnostics are unaffected (each file is parsed
  separately, so a syntax error in a required file already renders with
  that file's own real path and snippet). Precise per-file sema
  attribution needs the same span-threading work plan 13/17 already
  deferred; this plan doesn't re-solve it.

## Leaf: leaf-grammar-require

### 1. Context
- Why: no AST shape or grammar production for cross-file references
  exists at all — verified this session against
  `crates/emerald-parser/src/grammar.lalrpop` (no `require`/`import`/
  `use` production of any kind) and `crates/emerald-cli/src/main.rs`
  (`args.get(1)` is the one and only source path an invocation accepts).
- Target state: `Item::Require(String)` in `crates/emerald-parser/src/
  ast.rs`; a grammar production `"require" <path:RequirePath> => Item::
  Require(path)` reachable only from `Program`'s top-level `Item*` rule
  (see Decision log); `require` reserved the same LALR(1) way `puts`/
  `new`/`Array`/`Proc`/`raise`/`begin`/`rescue` already are.

### 2. Acceptance Criteria
1. `require helpers` parses to `Item::Require("helpers".to_string())`.
2. `require utils/math` parses to `Item::Require("utils/math".to_string())`
   — multi-segment paths work, proving this isn't single-identifier-only.
3. `require` written inside a function body (`def f -> Void\n  require
   helpers\nend`) is a real parse error, not silently accepted — proves
   the top-level-only restriction is structural, not just documented.
4. Regression: every existing `examples/*.em` file and every existing
   `emerald-parser` test still parses identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new require/negative tests | agent-claimed-locally |

---

## Leaf: leaf-driver-resolution

### 1. Context
- Why: `emerald-driver` (plan 17) reads/checks/compiles exactly one
  source string; nothing walks a `require` graph, dedups shared
  dependencies, or detects cycles.
- Target state: `crates/emerald-driver` gains `resolve_program(entry:
  &Path) -> Result<Program, DriverError>`: parses `entry`, then for each
  `Item::Require(path)` in order, resolves `path` relative to `entry`'s
  parent directory with `.em` appended, recursively resolves *that*
  file the same way (relative to *its own* directory — see Decision
  log), splices its resolved items in at that position (skipping a path
  already in `visited`), and drops the `Item::Require` marker itself
  from the output — so the returned `Program` contains only
  `Function`/`Class`/`Module`/`Stmt` items, never a `Require`, by the
  time it's handed to `emerald_sema::check_program`. A path already on
  the in-progress `stack` (see Decision log) produces
  `DriverError::Require(String)` naming the cycle
  (e.g. `"require cycle: a.em -> b.em -> a.em"`); a path that doesn't
  exist on disk produces a descriptive `DriverError::Require`, not a
  panic. `check`/`compile` are extended (or given path-taking siblings)
  to call `resolve_program` first when the entry has any `Item::
  Require`, so a single-file program with none pays no new cost.

### 2. Acceptance Criteria
1. `examples/multi_file/main.em` (`require helpers` + a call into a
   function `helpers.em` defines) resolved via `resolve_program`,
   compiled, linked, and run for real prints the expected value —
   proof the whole splice-then-existing-pipeline path genuinely works
   end to end, not just that the merged `Program` value looks right.
2. A three-file diamond (`main.em` requires both `b.em` and `c.em`;
   both `b.em` and `c.em` require the same `d.em`) resolves without a
   duplicate-definition sema error — real proof of path-canonicalized
   dedup, not just "no crash."
3. A real two-file cycle (`a.em` requires `b.em`; `b.em` requires
   `a.em`) returns `Err(DriverError::Require(_))` naming both files,
   and the test asserts this returns promptly (bounded time/no stack
   growth) rather than merely not crashing by luck.
4. `require nonexistent` (no such file on disk) returns a descriptive
   `Err(DriverError::Require(_))`, not a panic or an `io::Error` leaking
   raw through.

### 3. File & Module Structure
- **Create:** `examples/multi_file/main.em`, `examples/multi_file/
  helpers.em`, `crates/emerald-driver/tests/multi_file.rs` (or fixtures
  under a `tests/fixtures/` subdirectory for the diamond/cycle cases,
  kept out of the polished `examples/` corpus)
- **Modify:** `crates/emerald-driver/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass: 2-file real run, diamond dedup, cycle rejection, missing-file error | agent-claimed-locally |
| Sema/codegen touch-point | `cargo test -p emerald-sema -p emerald-codegen` | pass with the one new `Item::Require` match arm each (see Decision log) | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-cli-lsp-mcp-wiring

### 1. Context
- Why: `resolve_program` (leaf 2) needs a real filesystem path to
  resolve relative `require`s against; `emerald-cli` already has one,
  but `emerald-lsp`'s open-document model and `emerald-mcp`'s bare-text
  tools (both plan 17) need explicit wiring — or an explicit, honest
  refusal — to supply one.
- Target state: `emerald-cli`'s existing `source_path` argument now
  flows into `resolve_program` instead of a plain single-file read —
  its own behavior for a `require`-free program is unchanged (verified
  by regression, see AC1). `emerald-lsp` converts a
  `textDocument/didOpen`/`didChange` notification's URI via
  `Url::to_file_path()` and passes that real path through, so an
  open document's own `require`s resolve against its real location on
  disk (its *unsaved* edits are still what's checked — only files it
  `require`s are read from disk, per plan 17's existing in-memory-buffer
  model). `emerald-mcp`'s `check_source`/`compile_and_run` (which take
  bare text, no path) detect any `Item::Require` in the parsed result
  and return the clear, documented error from the Decision log instead
  of attempting resolution against a nonexistent base directory.

### 2. Acceptance Criteria
1. `emerald-cli examples/multi_file/main.em` compiles, links, and runs
   for real end to end through the actual CLI binary (not just
   `emerald-driver`'s own unit tests) and prints the expected value;
   every one of `emerald-cli`'s existing single-file tests
   (`tests/hello_em.rs`) still passes unmodified.
2. `emerald-lsp`, given a real `file://` URI for `examples/multi_file/
   main.em`, publishes an empty `publishDiagnostics` array — proof the
   LSP's own path plumbing resolves `require`s too, not only the CLI's.
3. `emerald-mcp`'s `check_source`, called with source text containing
   `require helpers`, returns the documented "`require` not supported
   without a file path" error as a real tool result — not a crash, not
   a silently-wrong empty-diagnostics response.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs`,
  `crates/emerald-lsp/src/main.rs`, `crates/emerald-mcp/src/main.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| CLI | `cargo test -p emerald-cli` | 3/3 existing tests pass + new multi-file test | agent-claimed-locally |
| LSP | `cargo test -p emerald-lsp` | existing + new multi-file-open test pass | agent-claimed-locally |
| MCP | `cargo test -p emerald-mcp` | existing + new require-without-path test pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **A package manager, version resolution, or any standard-library
  search path** — there is no standard library beyond this language's
  own builtins today; `require` only ever resolves a real relative path
  on disk, nothing more.
- **Explicit export/visibility annotations on required declarations** —
  every top-level `def`/`class`/`module` in a required file becomes
  visible to whatever requires it, the same all-or-nothing visibility
  Ruby's own plain `require` has; a real access-control mechanism is
  future work, not needed to prove multi-file compilation works.
- **Genuine separate compilation units / cross-object linking** — see
  Decision log; this plan merges ASTs ahead of a single sema/codegen
  pass, not real per-file object code with cross-unit symbol
  resolution.
- **Incremental recompilation of unchanged required files** — every
  `emerald-cli`/`emerald-lsp`/`emerald-mcp` invocation re-parses and
  re-resolves the whole require graph from scratch, consistent with
  this compiler having no incremental-compilation infrastructure at all
  (`salsa` explicitly declined in plans 13 and 17).
- **`require` anywhere but top-level** (inside a function body, an
  `if`, a loop) — see Decision log; a real, structural restriction, not
  an oversight.
- **Cross-file sema diagnostic attribution in `emerald-lsp`** — see
  Decision log; a disclosed, deferred gap tied to `emerald_sema::
  Diagnostic` having no span or source-file field at all yet (plan 13).
- **`require` support in `emerald-mcp`'s path-less text tools** — see
  Decision log; those tools get a clear, documented error instead,
  not a workaround that invents a fake base directory.
