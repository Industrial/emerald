---
name: Tree-sitter Grammar
overview: "A real tree-sitter grammar (grammar.js, generated parser, highlight queries, corpus tests) for Emerald — the explicit follow-up to plan 17's own deferred 'no tree-sitter grammar' call, unlocking native highlighting in Zed and modern Neovim."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-grammar-js
    content: "tree-sitter-emerald/grammar.js — a from-scratch tree-sitter grammar grounded in grammar.lalrpop's real terminals, generated + built with the CLI"
    status: pending
  - id: leaf-highlight-queries
    content: "tree-sitter-emerald/queries/highlights.scm — standard-capture-name highlight query authored against the new grammar's node types"
    status: pending
  - id: leaf-corpus-tests
    content: "tree-sitter-emerald/test/corpus/*.txt — tree-sitter's native corpus-test format, one family per examples/*.em construct, run via `tree-sitter test`"
    status: pending
isProject: false
---

# Plan 24 — Tree-sitter Grammar

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
same posture as [plan 17](./2026-09-08T213000Z-plan-17-ide-integration.md):
new, post-v1 tooling scope, not an inception §§ item. It does not touch
`plan-of-plans.md` or any other plan file.

This plan is the named, deliberate reversal of one specific call plan
17's own Decision log made: *"No tree-sitter grammar. Tree-sitter needs a
from-scratch grammar authored in its own DSL and generated into a
separate C parser — a second, parallel-maintained grammar implementation
with its own drift risk against `grammar.lalrpop`... Zed and Neovim's
native (as opposed to LSP-fed) highlighting are the concrete casualties
of this call."* Plan 17 made the right call for its own scope — a
TextMate grammar was the right *first* investment for a language with no
editor support at all. This plan is what's worth doing once that
tradeoff is worth taking: real native highlighting in Zed, in modern
`nvim-treesitter`-based Neovim, and in any other tree-sitter-consuming
tool (GitHub's own code navigation, `difftastic`, etc.) — none of which
can ever consume plan 17's TextMate grammar, no matter how complete it
gets.

Concrete proof this plan targets: `tree-sitter parse` on every file in
`examples/*.em` produces a clean parse tree (zero `ERROR`/`MISSING`
nodes) from a grammar this plan writes from scratch — not derived from,
converted from, or dependent on `grammar.lalrpop` in any mechanical way,
since no such conversion tool exists (see Decision log).

## Decision log

- **Ground truth is `crates/emerald-parser/src/grammar.lalrpop`'s actual
  terminals, re-verified directly this session** (same discipline as
  plan 17, which found `plan-of-plans` and `spec/GRAMMAR.md` both already
  diverge from what's implemented): keywords `class`, `module`, `def`,
  `end`, `if`, `else`, `while`, `return`, `break`, `next`, `puts`,
  `raise`, `begin`, `rescue`, `new`; the two reserved type-position
  keywords `Array`/`Proc`; operators/punctuation `->`, `=>`, `.`, `,`,
  `:`, `=`, `+`, `>`, `<`, `>=`, `<=`, `==`, `!=`; brackets `(` `)` `[`
  `]` `{` `}`; integer (`[0-9]+`) and float (`[0-9]+\.[0-9]+`) literals;
  identifiers; instance variables (`@name`). No `#` comments (plan 17
  added them to its TextMate grammar anyway, per `spec/GRAMMAR.md` §12's
  KEEP mark, despite the compiler not accepting them — this plan does the
  same, for the same reason: purely editorial, zero-risk, and a
  TextMate/tree-sitter grammar has no dependency on what the compiler
  accepts). At the time of writing, `history/` has no `plan-18`/`19`/`20`
  files yet — the sibling plans in this same requested batch that will
  add real new operators (`-`, `*`, `/`, `%`, `&&`, `||`, `!`), string
  literals, and comments/`case`. **This grammar's terminal inventory will
  need revisiting once those land** — flagged here rather than silently
  left stale.
- **Tooling is genuinely available in this sandbox, confirmed by actually
  running it this session** — a real contrast to plan 17's npm-blocked
  VSCode-extension leaf. `tree-sitter` 0.26.11 (the Rust CLI) is already
  on `PATH` via this environment's Nix packaging; `tree-sitter`/
  `tree-sitter-cli` 0.27.0 are separately confirmed live on crates.io
  this session (one minor ahead of the installed CLI — not a blocker,
  noted for completeness). A real smoke test this session — a throwaway
  grammar, `tree-sitter generate`, `tree-sitter build`, and `tree-sitter
  test` against a one-line corpus file — completed successfully
  end-to-end with no `npm`/`node` step required (the CLI evaluates
  `grammar.js` itself and drives its own native build via `cc`, which is
  also present). This plan's quality gates are therefore real, automated
  CLI runs, not disclosed-as-manual steps.
- **`tree-sitter init` is run first, not skipped.** The smoke test above
  surfaced a real, concrete finding: without a `tree-sitter.json`
  manifest, `tree-sitter generate` silently falls back to legacy ABI 14
  with a warning, instead of the current ABI 15. `tree-sitter init`
  scaffolds a proper `tree-sitter.json` (name, repository metadata, ABI
  version) before `grammar.js` is written against it — avoids building
  the entire grammar against a deprecated ABI by omission.
- **Lives in-repo at `tree-sitter-emerald/`, not a separate repository.**
  Matches this project's everything-in-one-repo convention so far.
  Disclosed real limitation: both `nvim-treesitter`'s official parser
  list and Zed's extension registry conventionally expect a dedicated
  repository with a stable git URL/rev to point at — this plan's
  in-repo placement is a genuine, if minor, blocker for *that specific*
  future step (upstream registration), not for local or manual use,
  which works identically pointing at a subdirectory of this repo.
- **Generated output (`src/parser.c`, `src/node-types.json`,
  `src/grammar.json`, `src/tree_sitter/parser.h`) is committed, not
  gitignored.** This is the standard tree-sitter convention, not a
  one-off choice: downstream consumers (`nvim-treesitter`, Zed) compile
  the committed C source with their own toolchain — they do not run
  `tree-sitter generate` themselves, so the generated output has to be
  the checked-in artifact, the same way a compiled `.tmLanguage.json`
  (not some intermediate DSL) was plan 17's checked-in artifact.
- **`queries/highlights.scm` (tree-sitter's own S-expression query
  language) is authored fresh against this grammar's node types — no
  relationship to, or reuse of, plan 17's `emerald.tmLanguage.json`.**
  The two grammars (LALRPOP-adjacent-but-independent tree-sitter grammar
  vs. regex-based TextMate grammar) produce structurally unrelated
  outputs (a concrete syntax tree vs. a flat token stream), so there is
  no shared intermediate representation to derive one query format from
  the other. Capture names follow the de facto standard vocabulary both
  Zed's and `nvim-treesitter`'s default themes already recognize
  (`@keyword`, `@type`, `@number`, `@variable`, `@variable.parameter`,
  `@property`, `@function`, `@punctuation.bracket`, `@operator`) rather
  than inventing project-specific capture names no theme would color.
- **No `locals.scm`, no injections.** Basic highlighting only — the same
  "prove the core path, stop there" shape as plan 17's LSP scoping
  (diagnostics only, no semantic tokens/completion). Emerald doesn't
  embed another language's syntax anywhere, so injections don't apply at
  all; scope/reference queries (`locals.scm`) are a real, separate
  follow-up once something actually consumes them (most editors get
  usable highlighting from `highlights.scm` alone).
- **No Zed extension packaging or `nvim-treesitter` registry submission.**
  Both are PRs into someone else's repository/registry — a largely
  non-technical, later step, not a technical extension of this plan's
  leaves. This plan documents how a Zed extension's `extension.toml` or
  an `nvim-treesitter` parser entry would point at this grammar; it
  doesn't submit either.

## Leaf: leaf-grammar-js

### 1. Context
- Why: no tree-sitter grammar exists anywhere in this repo; plan 17
  deliberately deferred one (see Decision log/intro).
- Target state: `tree-sitter-emerald/tree-sitter.json` (via
  `tree-sitter init`) and `tree-sitter-emerald/grammar.js`, a from-scratch
  grammar covering exactly the terminal inventory re-derived above:
  `source_file` → repeated top-level items (function/class/module
  definitions or statements); statement rules for `let`
  (`ident : type = expr`), `if`/`else`, `while`, `return`, `break`,
  `next`, `puts`, `raise`, `begin`/`rescue`, instance-var/index
  assignment; expression rules with `prec.left` for `+` and comparison
  operators, call/method-call/`.new`/indexing postfix forms, array
  literals, and the `->(params) -> Type { body }` lambda literal.
  Generated via `tree-sitter generate` into `tree-sitter-emerald/src/`
  (committed — see Decision log) and built via `tree-sitter build`.

### 2. Acceptance Criteria
1. `tree-sitter generate` (run for real, in `tree-sitter-emerald/`)
   completes with no LR conflict errors.
2. `tree-sitter parse <file>` on every file in `examples/*.em`
   (`classes.em`, `closures.em`, `collections.em`, `control_flow.em`,
   `exceptions.em`, `hello.em`, `modules.em`) produces a parse tree with
   zero `ERROR`/`MISSING` nodes — real proof the grammar accepts the
   real corpus end-to-end, not just a synthetic one-line smoke test.
3. `tree-sitter build` compiles the generated parser into a loadable
   native library with no errors.

### 3. File & Module Structure
- **Create:** `tree-sitter-emerald/tree-sitter.json`,
  `tree-sitter-emerald/grammar.js`, `tree-sitter-emerald/src/parser.c`,
  `tree-sitter-emerald/src/node-types.json`,
  `tree-sitter-emerald/src/grammar.json`,
  `tree-sitter-emerald/src/tree_sitter/parser.h`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Init | `tree-sitter init` (once, before authoring `grammar.js`) | scaffolds `tree-sitter.json` | agent-claimed-locally |
| Generate | `tree-sitter generate` (in `tree-sitter-emerald/`) | clean, no conflicts | agent-claimed-locally |
| Corpus parse | `tree-sitter parse` on each file in `../examples/*.em` | zero `ERROR`/`MISSING` nodes on every file | agent-claimed-locally |
| Build | `tree-sitter build` | clean | agent-claimed-locally |

---

## Leaf: leaf-highlight-queries

### 1. Context
- Why: leaf 1's grammar produces a concrete syntax tree but colors
  nothing on its own — every tree-sitter consumer (Zed, `nvim-treesitter`)
  needs a `highlights.scm` mapping node types to capture names.
- Target state: `tree-sitter-emerald/queries/highlights.scm`, using the
  standard capture-name vocabulary (see Decision log) for keywords,
  types (`Array`/`Proc`/class names in type position), numeric literals,
  identifiers/parameters, instance variables (`@property` — the closest
  standard capture to a field/instance-var read), function/method/class
  names, and punctuation/brackets/operators.

### 2. Acceptance Criteria
1. `tree-sitter query tree-sitter-emerald/queries/highlights.scm
   examples/hello.em` (real CLI command) runs with no query-compile
   error and prints at least one capture each for a keyword, a number
   literal, an identifier, and a punctuation/bracket — proof the query
   file is syntactically valid and actually matches real source, not
   just handwritten and untested.
2. Every keyword in `grammar.js`'s terminal list gets a `@keyword`-family
   capture at least once across `examples/*.em` — the same coverage
   discipline plan 17's TextMate grammar used, checked here with the
   native `tree-sitter query` tool instead of a custom Node script.

### 3. File & Module Structure
- **Create:** `tree-sitter-emerald/queries/highlights.scm`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Query check | `tree-sitter query tree-sitter-emerald/queries/highlights.scm examples/*.em` | captures printed for every file, no errors | agent-claimed-locally |

---

## Leaf: leaf-corpus-tests

### 1. Context
- Why: leaf 1's "zero `ERROR` nodes on the real corpus" proves the
  grammar *accepts* real programs; it doesn't pin down the exact tree
  shape, so a later grammar edit could silently reshape the parse tree
  (e.g. change precedence or nesting) without anything noticing.
- Target state: `tree-sitter-emerald/test/corpus/*.txt`, tree-sitter's
  native paired source/expected-S-expression format, one file per
  construct family already proven by a real `examples/*.em` file:
  `functions.txt` (from `hello.em`), `classes.txt` (`classes.em`),
  `control_flow.txt` (`control_flow.em`), `collections.txt`
  (`collections.em`), `closures.txt` (`closures.em`), `exceptions.txt`
  (`exceptions.em`), `modules.txt` (`modules.em`) — each corpus case's
  source text drawn directly from the real example file, not invented
  from scratch, so a failure traces back to an actual working program.

### 2. Acceptance Criteria
1. `tree-sitter test` (run for real in `tree-sitter-emerald/`) reports
   100% passing — every corpus case's actual parse tree matches its
   committed expected S-expression exactly.
2. Every one of the 7 `examples/*.em` files has at least one
   corresponding corpus case whose source is that file's real content
   (a whole-file case, a representative excerpt, or both).

### 3. File & Module Structure
- **Create:** `tree-sitter-emerald/test/corpus/functions.txt`,
  `tree-sitter-emerald/test/corpus/classes.txt`,
  `tree-sitter-emerald/test/corpus/control_flow.txt`,
  `tree-sitter-emerald/test/corpus/collections.txt`,
  `tree-sitter-emerald/test/corpus/closures.txt`,
  `tree-sitter-emerald/test/corpus/exceptions.txt`,
  `tree-sitter-emerald/test/corpus/modules.txt`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Corpus tests | `tree-sitter test` (in `tree-sitter-emerald/`) | 100% passing | agent-claimed-locally |

## Total quality gate
```bash
cd tree-sitter-emerald
tree-sitter generate
tree-sitter build
tree-sitter test
for f in ../examples/*.em; do tree-sitter parse "$f" || exit 1; done
tree-sitter query queries/highlights.scm ../examples/*.em
```

## Out of scope / deferred
- **Zed extension packaging (`extension.toml` + a git URL/rev pointer)
  and `nvim-treesitter` parser-list registration** — both are PRs into
  someone else's repository/registry, not a technical extension of this
  plan's leaves; see Decision log. This plan documents how either would
  point at `tree-sitter-emerald/`, but doesn't submit to either.
- **`queries/locals.scm` (scope/variable-reference queries) and
  injections** — basic highlighting only; see Decision log.
- **Incremental re-parse performance tuning** — no evidence of a
  performance problem exists yet to tune against.
- **Revisiting this grammar's terminal inventory once plans 18
  (operators), 19 (string literals), and 20 (comments/`case`) land** —
  this grammar is grounded in `grammar.lalrpop` exactly as it stood this
  session; those sibling plans (same requested batch) will add real new
  terminals this grammar doesn't yet know about. Flagged, not solved
  here.
- **An npm `package.json`/native Node binding for this grammar** — not
  needed; the Nix-provided `tree-sitter` 0.26.11 CLI drives
  generate/build/test directly (confirmed this session), and nothing
  else in this plan has a reason to speak npm.
