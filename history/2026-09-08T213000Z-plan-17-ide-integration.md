---
name: IDE Integration
overview: "Syntax coloring, a Language Server Protocol server, and an MCP server so Emerald source gets live diagnostics and highlighting in an editor and structured tool access for AI coding agents — new, post-v1 tooling scope, not an inception §§ row."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-driver-extraction
    content: "crates/emerald-driver — a library extracting emerald-cli's parse->check->codegen->link pipeline behind an in-memory check()/compile() API, the second caller plan 06 said would justify it"
    status: pending
  - id: leaf-textmate-grammar
    content: "editors/vscode/syntaxes/emerald.tmLanguage.json + language-configuration.json, grounded in grammar.lalrpop's actual terminals, not spec/GRAMMAR.md's aspirational KEEP marks"
    status: pending
  - id: leaf-lsp-server
    content: "crates/emerald-lsp — a synchronous lsp-server/lsp-types binary publishing real diagnostics (miette spans for parse errors, whole-document range for sema errors) over textDocument/didOpen+didChange"
    status: pending
  - id: leaf-vscode-extension
    content: "editors/vscode/{package.json,extension.js} wiring the grammar + emerald-lsp into a minimal VSCode extension"
    status: pending
  - id: leaf-mcp-server
    content: "crates/emerald-mcp — an rmcp stdio server exposing check_source/compile_and_run/list_examples tools over emerald-driver"
    status: pending
  - id: leaf-editor-docs
    content: "editors/README.md — VSCode/Neovim/Helix/Zed setup and MCP client config, stating plainly what works per editor and what doesn't"
    status: pending
isProject: false
---

# Plan 17 — IDE Integration

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md).
That table's own Completion note says Emerald v1 "is complete when every
row above is `done`" — true as of row 15. This plan is new, post-v1
tooling scope requested directly ("integrate Emerald into IDEs: syntax
coloring, LSP, MCP server, and whatever else optimum adoption needs"),
not an inception §§ item. Plan 16 is being authored concurrently by a
separate agent against `plan-of-plans.md` itself; editing that shared
index here would race that work, so this plan stands alone — folding it
in as a numbered row (17, or whatever number is free once 16 lands) is a
follow-up, not this plan's job.

Concrete proof this plan targets: opening `examples/hello.em` in an
editor shows real keyword/type/literal coloring and zero diagnostics;
opening a copy with `x: Int64 = +` (plan 13's own worked example) shows
one live, correctly-positioned red squiggle without running
`emerald-cli` by hand; and an MCP client (Claude Code itself, e.g.) can
call a tool to check or compile Emerald source instead of shelling out
to the CLI binary and scraping stderr text.

## Decision log

- **Ground truth for every keyword/operator/literal this plan touches is
  `crates/emerald-parser/src/grammar.lalrpop`'s literal terminals — not
  `spec/GRAMMAR.md`'s KEEP marks, not `plan-of-plans`' scope prose.**
  Both have already been shown, twice, to diverge from what's actually
  implemented: plan-of-plans row 07 lists `case` as in-scope and done,
  but `grammar.lalrpop` has no `case`/`when` production at all; `spec/
  GRAMMAR.md` §12 marks `# line comment` KEEP, but the grammar's `match`
  block never wires a comment pattern in, and no `examples/*.em` file
  uses one; plan 15 separately found `Type::String` exists with no
  `StringLit` expression syntax to write one. This plan's syntax
  coloring, LSP diagnostics, and MCP tools all reflect the compiler's
  actual current surface, verified against `grammar.lalrpop` and the
  real `examples/*.em` corpus, not its documented eventual one.
- **`#` line comments are added to the syntax grammar anyway**, per
  `spec/GRAMMAR.md` §12's KEEP mark — a purely editorial, zero-risk,
  forward-compatible addition (a TextMate grammar has no dependency on
  what the compiler accepts). Disclosed clearly: a real `.em` file
  containing a `#` comment today still fails to compile — a real,
  pre-existing compiler gap (unrelated to this plan) that this tooling
  faithfully surfaces rather than papers over.
- **No tree-sitter grammar.** Tree-sitter needs a from-scratch grammar
  authored in its own DSL and generated into a separate C parser — a
  second, parallel-maintained grammar implementation with its own drift
  risk against `grammar.lalrpop`, not a derivation of it (LALRPOP's
  LR(1) tables and tree-sitter's GLR-style incremental parsing are
  architecturally unrelated; no tool converts one into the other). A
  TextMate grammar reuses nothing from the real parser either, but it's
  an order of magnitude smaller and is what VSCode, GitHub Linguist, and
  most terminal highlighters (`bat`, etc.) already consume directly —
  the right first investment for a language with one working parser and
  no editor support at all yet. Zed and Neovim's native (as opposed to
  LSP-fed) highlighting are the concrete casualties of this call; see
  `leaf-editor-docs`.
- **The LSP proves the diagnostics path end-to-end and stops there — no
  semantic tokens, go-to-definition, completion, rename, or
  formatting.** The same "prove the core path before generalizing"
  shape as plan 13 (one single-label parse diagnostic) and plan 15 (2 of
  9 benchmarks): `emerald-sema::check_program` doesn't return or expose
  a symbol table (its `HashMap<String, Type>`/`HashMap<String,
  FunctionSig>` environments are function-local to the type-checking
  walk, never returned), so go-to-definition/completion need real sema
  surface-area work this plan doesn't do. Diagnostics need nothing new
  from sema beyond what it already returns.
- **Sema diagnostics get a whole-document `Range`, not a precise
  span** — inherited directly from `emerald_sema::Diagnostic { message:
  String }` having no span field at all, a gap plan 13 itself already
  found and explicitly deferred ("sema-level span-aware diagnostics are
  real, valuable follow-up work, explicitly not this plan's job"). This
  plan's LSP surfaces that exact, already-disclosed gap faithfully
  rather than fabricating a fake precise range. Parse errors, by
  contrast, get a real, byte-accurate `Range` for free — `ParseError`
  already carries a genuine `SourceSpan` (plan 13).
- **Full-document `textDocumentSync`, no incremental diffing.**
  Consistent with inception §14.5's own `salsa` deferral ("do not add
  unless it materially simplifies the architecture") — there is no
  incremental compiler to synchronize incrementally against; re-running
  `emerald_driver::check` on the whole buffer on every keystroke is the
  honestly-scoped behavior for a language this size, not a shortcut.
- **`lsp-server` + `lsp-types` (synchronous, channel-based — the same
  scaffold rust-analyzer itself uses), not `tower-lsp`/`async-lsp`.**
  Confirmed live on crates.io this session (`lsp-server` 0.10.0,
  `lsp-types` 0.97.0). Avoids pulling a `tokio` async runtime into the
  workspace for what is, per-request, a single synchronous
  parse-then-check call — the same reasoning plan 14 already applied to
  `emerald-cli` ("no async, no fibers/concurrency... none of
  `id_effect`'s async strata apply"), now applied to the same
  pipeline's second consumer.
- **`emerald-mcp` uses `rmcp` 3.2.0** (official Rust MCP SDK, confirmed
  live on crates.io this session), whose stdio transport is
  `tokio`-based — a deliberate, disclosed exception to the "no async"
  call above. Plan 14's reasoning was scoped explicitly to
  `emerald-cli`'s own single-shot pipeline orchestration, not to every
  future crate in this workspace; `emerald-mcp` is a new, separate
  binary that gains nothing from staying synchronous once its one SDK
  dependency already requires an async runtime.
- **`crates/emerald-driver` is the "second caller" plan 06's own
  Decision log said would justify extracting a shared library**
  ("`emerald-cli` orchestrates the pipeline directly... rather than in a
  separate `emerald-driver` crate — extracted when a second caller needs
  the same pipeline"). `AGENTS.md`'s own documented crate layout already
  names `driver` as a pipeline stage slot that's never been created.
  This plan's LSP and MCP server are that second and third caller
  arriving; `leaf-driver-extraction` is the trigger, not new scope
  invented for its own sake.
- **No `npm`/`vsce`-driven packaging or Extension Development Host
  launch is automated in this session.** Confirmed this session: `node`
  v24.18.1 is present and the npm registry is reachable
  (`registry.npmjs.org` returns `200`), but `npm`, `npx`, `yarn`, and
  `pnpm` are all absent from `PATH` — a real environment constraint, not
  a corner cut in the extension's own code (same shape as plan 15's
  "`ruby` not installed"). `leaf-vscode-extension`'s automated gates use
  only `node` directly; installing `vscode-languageclient` and
  live-testing in a real VSCode window is a disclosed manual step.

## Leaf: leaf-driver-extraction

### 1. Context
- Why: the parse→check→codegen→link pipeline exists in exactly one
  place — `emerald-cli/src/main.rs`, a binary crate with no library
  target (plan 06's Decision log: deferred "until a second caller needs
  the same pipeline"). `emerald-lsp` and `emerald-mcp` are both that
  second caller, and both need to check/compile an in-memory buffer that
  was never written to disk — something today's pipeline can't do since
  `main` reads the path itself.
- Target state: new library crate `crates/emerald-driver` holding the
  `id_effect`-orchestrated pipeline (plan 14's `CliError`/stage
  functions, renamed `DriverError`/kept as-is internally) behind two
  entry points: `pub fn check(source: &str, name: &str) -> Result<(),
  DriverError>` (parse + sema only, no codegen/link — what live
  diagnostics need) and `pub fn compile(source: &str, name: &str,
  output_path: &Path) -> Result<(), DriverError>` (the full pipeline,
  what the CLI and `compile_and_run` need). `link_stage`'s
  `env!("CARGO_MANIFEST_DIR")`-relative lookup of `runtime/
  emerald_runtime.c` moves with it unchanged — `crates/emerald-driver`
  sits at the same depth under `crates/` as `emerald-cli` did, so
  `../../runtime/emerald_runtime.c` still resolves correctly.
  `emerald-cli/src/main.rs` shrinks to argument parsing, reading the
  source file, calling `emerald_driver::compile`, and rendering the
  resulting `DriverError` — unchanged rendering logic, moved call site.

### 2. Acceptance Criteria
1. `crates/emerald-cli/tests/hello_em.rs`'s three existing tests pass
   unmodified — proof the extraction is behavior-preserving, not a
   rewrite (same discipline as plan 14 AC1).
2. `emerald_driver::check` returns `Ok(())` for `examples/hello.em`'s
   real contents and `Err(DriverError::Sema(_))`/`Err(DriverError::
   Parse(_))` for real known-bad programs, called directly on an
   in-memory `&str` with no filesystem write involved — proof the
   concrete need (check an unsaved buffer) is actually satisfied.
3. `DriverError`'s four variants each still carry the original stage
   error value unchanged (no information lost in the rename from
   `CliError`) — `emerald-cli`'s existing stderr-content assertions keep
   passing without loosening them.

### 3. File & Module Structure
- **Create:** `crates/emerald-driver/Cargo.toml`,
  `crates/emerald-driver/src/lib.rs`
- **Modify:** `crates/emerald-cli/src/main.rs` (shrinks to CLI-only
  concerns), `crates/emerald-cli/Cargo.toml` (drops direct
  `emerald-parser`/`emerald-sema`/`emerald-codegen`/`id_effect` deps in
  favor of `emerald-driver`), root `Cargo.toml` (adds the new member)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | new `check`/`compile` unit tests pass | agent-claimed-locally |
| CLI unaffected | `cargo test -p emerald-cli` | 3/3 existing tests pass unmodified | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-textmate-grammar

### 1. Context
- Why: no syntax-coloring artifact exists anywhere in the repo; every
  `.em` file renders as plain text in every editor and on GitHub.
- Target state: `editors/vscode/syntaxes/emerald.tmLanguage.json`
  (`source.emerald` scope) covering exactly `grammar.lalrpop`'s real
  terminals — keywords (`class`, `module`, `def`, `end`, `if`, `else`,
  `while`, `return`, `break`, `next`, `puts`, `raise`, `begin`,
  `rescue`, `new`), the two reserved type-position keywords (`Array`,
  `Proc`), operators/punctuation (`->`, `=>`, `.`, `,`, `:`, `=`, `+`,
  `>`, `<`, `>=`, `<=`, `==`, `!=`), integer/float literals, plain
  identifiers, instance variables (`@name`), and `#` line comments (see
  Decision log). Plus `editors/vscode/language-configuration.json`
  (line comment `#`, bracket pairs `()[]{}`, auto-closing pairs).

### 2. Acceptance Criteria
1. `editors/vscode/check-grammar.mjs` — a dependency-free Node script
   (only `node:fs`/built-in `RegExp`, no `npm install`; see Decision
   log) — loads the tmLanguage JSON's patterns and tokenizes every file
   in `examples/*.em`, asserting every keyword in `grammar.lalrpop`'s
   terminal list is matched by a keyword scope at least once across the
   real corpus, with zero unmatched non-whitespace residue on any line —
   real proof the patterns fire on real Emerald source.
2. The same script spot-checks `classes.em`'s `@value = start` line:
   `@value` gets a distinct instance-variable scope, `Int64`/`Float64`
   get a distinct type scope, and `10`/`2.0` get distinct
   integer/float-literal scopes.

### 3. File & Module Structure
- **Create:** `editors/vscode/syntaxes/emerald.tmLanguage.json`,
  `editors/vscode/language-configuration.json`,
  `editors/vscode/check-grammar.mjs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Grammar check | `node editors/vscode/check-grammar.mjs` | prints PASS, all `examples/*.em` files fully tokenized | agent-claimed-locally |

---

## Leaf: leaf-lsp-server

### 1. Context
- Why: `emerald_driver::check` (leaf 1) exists but nothing speaks LSP —
  an editor has no way to get live diagnostics without a human re-running
  `emerald-cli` by hand after every edit.
- Target state: `crates/emerald-lsp`, a binary crate on `lsp-server`
  0.10 + `lsp-types` 0.97 (see Decision log). Implements `initialize`
  (advertises `textDocumentSync: Full`), `textDocument/didOpen` and
  `textDocument/didChange` (both call `emerald_driver::check` on the
  full buffer text) and publish `textDocument/publishDiagnostics`: a
  `ParseError`'s real `SourceSpan` converts to a precise `Range`; a
  sema `Diagnostic` (no span — disclosed gap, see Decision log) converts
  to one diagnostic anchored at the whole-document range.

### 2. Acceptance Criteria
1. An integration test drives the server over `lsp_server::Connection::
   memory()`, sends a real `initialize` request, and asserts the
   returned `ServerCapabilities.text_document_sync` is `Full`.
2. Opening a document containing plan 13's own worked example
   (`x: Int64 = +`) produces exactly one `publishDiagnostics`
   notification with one diagnostic whose `range.start` column equals
   the real byte offset of `+` (UTF-16-converted) — proof the miette
   span threads through end-to-end, not just that some diagnostic fires.
3. Opening `examples/hello.em`'s real contents produces a
   `publishDiagnostics` notification with an empty `diagnostics` array —
   the happy path doesn't spuriously flag a real, working program.

### 3. File & Module Structure
- **Create:** `crates/emerald-lsp/Cargo.toml`,
  `crates/emerald-lsp/src/main.rs`,
  `crates/emerald-lsp/tests/diagnostics.rs`
- **Modify:** root `Cargo.toml` (adds member)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-lsp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-lsp` | all 3 pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-vscode-extension

### 1. Context
- Why: leaf 2's grammar and leaf 3's `emerald-lsp` binary each work
  standalone, but nothing packages them for an actual editor. VSCode is
  the primary target (largest install base; its TextMate-grammar +
  `vscode-languageclient` model is the most direct path from what leaves
  2/3 already produce).
- Target state: `editors/vscode/package.json` (id `emerald-lang`,
  `contributes.languages` registering `.em` → `emerald`,
  `contributes.grammars` pointing at leaf 2's tmLanguage file,
  `contributes.configuration` exposing an `emerald.serverPath` setting
  defaulting to `emerald-lsp` on `PATH`) and `editors/vscode/
  extension.js` (a small client: spawns `emerald-lsp` via
  `vscode-languageclient`'s `ServerOptions.command`, starts a
  `LanguageClient` for `.em` documents).

### 2. Acceptance Criteria
1. `node --check editors/vscode/extension.js` passes.
2. `editors/vscode/check-manifest.mjs` (a small assertion script)
   verifies `package.json`'s `contributes.languages[0].id === "emerald"`,
   its `extensions` array includes `.em`, and
   `contributes.grammars[0].scopeName === "source.emerald"` matches leaf
   2's actual grammar file — real, running proof the manifest and the
   grammar agree, not just "looks right" on inspection.
3. Disclosed, not automated: installing `vscode-languageclient` via
   `npm install` and F5-launching the Extension Development Host to
   confirm live highlighting + diagnostics in a real VSCode window —
   blocked on this sandbox's missing `npm`/`vsce` (see Decision log),
   not on anything in the extension's own code.

### 3. File & Module Structure
- **Create:** `editors/vscode/package.json`, `editors/vscode/
  extension.js`, `editors/vscode/check-manifest.mjs`,
  `editors/vscode/README.md`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Syntax check | `node --check editors/vscode/extension.js` | clean | agent-claimed-locally |
| Manifest check | `node editors/vscode/check-manifest.mjs` | prints PASS | agent-claimed-locally |

---

## Leaf: leaf-mcp-server

### 1. Context
- Why: the only interface an AI coding agent has to Emerald today is
  shelling out to the `emerald-cli` binary and scraping stderr text —
  no structured tool access exists.
- Target state: `crates/emerald-mcp`, a binary on `rmcp` 3.2.0 (see
  Decision log), stdio transport, exposing three tools built on
  `emerald-driver` (leaf 1): `check_source` (source text in →
  structured diagnostics out: parse errors with real line/column from
  the miette span, sema errors with their message and the disclosed
  whole-document-range caveat), `compile_and_run` (source text in →
  writes a scratch temp file, calls `emerald_driver::compile`, runs the
  resulting binary, returns stdout + exit code), and `list_examples`
  (returns the checked-in `examples/*.em` corpus plus `examples/
  README.md`'s descriptions, so an agent can ground itself in real,
  working Emerald syntax instead of guessing at unimplemented features
  like `case` or string literals — see Decision log).

### 2. Acceptance Criteria
1. An integration test spawns the real `emerald-mcp` binary as a child
   process over stdio, performs the real MCP `initialize` handshake, and
   asserts all three tools are listed by `tools/list`.
2. Calling `check_source` with `x: Int64 = +` (plan 13's example)
   returns a result containing that diagnostic's message and
   line/column; calling it with `examples/hello.em`'s real contents
   returns an empty diagnostics list.
3. Calling `compile_and_run` with `examples/hello.em`'s real contents
   returns exit code `0` and stdout `"42\n"` — the same value `tests/
   hello_em.rs` already asserts, now reached through a real MCP tool
   call end-to-end, not a mocked response.

### 3. File & Module Structure
- **Create:** `crates/emerald-mcp/Cargo.toml`,
  `crates/emerald-mcp/src/main.rs`,
  `crates/emerald-mcp/tests/mcp_protocol.rs`
- **Modify:** root `Cargo.toml` (adds member)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-mcp` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-mcp` | all 3 pass | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-editor-docs

### 1. Context
- Why: leaves 2–5 each produce a real, independently-working artifact,
  but nothing tells a human — or an agent like Claude Code itself — how
  to wire them into an actual editor or MCP client.
- Target state: `editors/README.md` covering VSCode (leaf 4's extension,
  npm caveat repeated plainly), Neovim (an `nvim-lspconfig` custom
  server entry pointing at the built `emerald-lsp` binary plus `.em`
  filetype detection — Neovim has no per-language extension model, one
  generic LSP registration is everything it needs), Helix (the
  equivalent `languages.toml` entry), and MCP client setup (the exact
  `.mcp.json`/Claude Desktop config snippet registering `emerald-mcp` as
  a local stdio server). Zed is documented as LSP-only (diagnostics work
  via `emerald-lsp`; native highlighting doesn't, since Zed only
  consumes tree-sitter grammars — see Decision log's "no tree-sitter
  grammar" call).

### 2. Acceptance Criteria
1. Every command, path, and binary name in the doc matches something
   leaves 1–5 actually created — cross-checked against the real
   repository state, not written aspirationally ahead of it.
2. The doc states plainly, per editor, exactly what works today
   (VSCode: coloring + diagnostics; Neovim/Helix: diagnostics only, no
   coloring; Zed: diagnostics only) rather than implying uniform support.

### 3. File & Module Structure
- **Create:** `editors/README.md`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Accuracy pass | manual cross-check of every path/command in the doc against leaves 1–5's actual output | no stale/aspirational references | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
node editors/vscode/check-grammar.mjs
node editors/vscode/check-manifest.mjs
node --check editors/vscode/extension.js
```

## Out of scope / deferred
- **Tree-sitter grammar** (would unlock native Zed highlighting and
  `textDocument/semanticTokens`-free accuracy elsewhere) — see Decision
  log; a genuine second-parser investment, not attempted here.
- **LSP: semantic tokens, go-to-definition, completion, rename,
  formatting/`textDocument/formatting`** — blocked on `emerald-sema`
  exposing a real symbol table, which it doesn't today; see Decision
  log.
- **Precise sema diagnostic spans** — `emerald_sema::Diagnostic` has no
  span field; plan 13's own already-disclosed gap, inherited here
  unchanged, not re-solved.
- **Incremental compilation / incremental LSP sync** — no `salsa`
  (inception §14.5, plan 13's own deferral); full-document re-check on
  every edit is this plan's honestly-scoped behavior.
- **VSCode Marketplace publishing, `vsce package`, Extension Development
  Host live verification** — blocked on this sandbox's missing
  `npm`/`vsce`; see Decision log. The extension's source and manifest
  are authored and checked; installing and running it is a disclosed
  manual step.
- **Implementing `case`/`when` or `#` comments in the compiler itself**
  — those are `emerald-parser`/`emerald-sema` scope, not IDE-tooling
  scope; this plan's grammar/LSP/MCP tooling faithfully reflects that
  they aren't implemented rather than silently working around it.
- **Neovim/Helix/Zed native syntax highlighting** — all three either
  need a tree-sitter grammar (Zed, and Neovim's modern highlighter) or
  a separate `.vim`/Sublime-syntax port (classic Vim); only VSCode's
  TextMate-consuming highlighter is targeted directly. All three still
  get LSP diagnostics from `emerald-lsp`; see `leaf-editor-docs`.
- **Wiring this plan into `plan-of-plans.md`** — deferred until plan
  16's concurrent authoring lands; see this plan's introduction.
