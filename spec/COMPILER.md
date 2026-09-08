# Emerald — COMPILER.md

**Status:** toolchain decisions only (plan `02 toolchain-prototype`). Full
pipeline architecture (how `emerald-driver` wires the stages together) is
deferred to the milestone plans that build it (`04`–`06`), per inception
§4's list of six spec documents and plan `01`'s decision log.

---

## Toolchain Decisions

Each decision below was made by building and running a real prototype
against inception §17's milestone-1 slice (`def add(a: Int64, b: Int64)
-> Int64 \n a + b \n end`), per inception §14's "do a small prototype...
before committing" instruction — not by research alone.

### Lexer: `logos`

**Chosen.** Inception §14.2 names no alternative to compare against, so
this was a feasibility check rather than a bake-off. `crates/emerald-lexer`
tokenizes the full milestone-1 source correctly, including the `->` and `:`
punctuation `spec/GRAMMAR.md` introduces, and correctly rejects both an
unterminated string and `@@` (the removed class-variable sigil,
`spec/GRAMMAR.md` §2) as lex errors rather than silently accepting them.

Evidence: `crates/emerald-lexer/src/lib.rs`, 3 passing tests.

### Parser: `LALRPOP` (chosen over `Chumsky`)

**Chosen: LALRPOP.** Both candidates inception §14.1 names were prototyped
against the identical milestone-1 grammar slice.

| Axis | LALRPOP 0.22.2 | Chumsky 1.0.0-alpha.8 |
|---|---|---|
| Maturity | Stable release line | Pre-1.0 alpha — crates.io has no stable `1.x` yet |
| Rough edges hit | None | `text::ascii::ident()` silently stops at the first digit (`"Int64"` → `"Int"`); had to hand-roll the identifier combinator to work around it |
| Grammar representation | Declarative `.lalrpop` grammar file (BNF-like, with a `match {}` block for tokenization) | Combinator chain in ordinary Rust code |
| Fit with inception §18 | Direct — a `.lalrpop` file *is* a grammar-production list, matching "import Ruby's grammar as an inventory, mark KEEP/MODIFY/REMOVE per production" almost structurally | Indirect — combinators express the grammar as code, not as an inventory-shaped artifact |
| Error message (missing `end`) | `Unrecognized EOF found at 44` (byte offset, no line/col) | `found end of input expected any` (no position at all in the plain `Display`) |
| Setup cost | `build.rs` + `lalrpop` build-dependency, generates a parser module at build time | No `build.rs`; parser is an ordinary function |

**Decision rationale:** the error-message quality was a wash — neither
library gives publication-quality diagnostics out of the box; both need the
`miette`/`ariadne`-class layer inception §14.4 already earmarks as a
separate investigation. With that axis roughly even, the deciding factors
were (1) LALRPOP's stable release line versus Chumsky's pre-1.0 alpha
status, surfaced concretely by hitting a real bug in Chumsky's convenience
`ident()` helper during this exact prototype, and (2) the structural fit
between a `.lalrpop` grammar file and inception §18's explicit "grammar as
graded inventory" strategy — a declarative grammar file is the more natural
home for a KEEP/MODIFY/REMOVE/UNDECIDED-tagged production list than a
combinator chain is.

Evidence: `crates/emerald-parser/src/grammar.lalrpop`,
`crates/emerald-parser/src/lib.rs`, 2 passing tests (happy path + missing
`end` error).

**Not decided on popularity alone**, per inception §14.1's explicit
constraint — the table above is the full comparison basis.

### Codegen: `Cranelift` (v1 only — Inkwell/LLVM deferred)

**Chosen for v1: Cranelift.** Per the plan's Deviation note, this is *not*
a completed Cranelift-vs-Inkwell comparison — this environment has no
`llvm-config` on `PATH` and no `LLVM_SYS_*_PREFIX` configured (the Nix
store holds an unbuilt `llvm-19.1.7.drv` and an `llvm-19.1.7-lib` output,
neither of which is a usable `llvm-config` binary). Standing up Inkwell
requires adding LLVM to the shared `devenv.nix`, a repo-environment change
out of scope for this plan.

Inception §14.6 explicitly sanctions this: *"Prototype Emerald code
generation with Cranelift first if rapid implementation is the priority."*
`crates/emerald-codegen` hand-builds Cranelift IR for `fn add(a: i64, b:
i64) -> i64 { a + b }`, JIT-compiles it via `cranelift-jit`, and calls the
resulting native machine code — `add(20, 22)` returns `42` as real,
executed native code, not a simulation.

**Revisit trigger (not "permanent," per inception §14.6's own wording):**
if `15 benchmarking`'s numeric benchmarks (inception §11/§21) show
Cranelift-generated code meaningfully behind hand-written/LLVM-generated
code on the hot paths inception §11 cares about (loop optimization,
vectorization), re-open this decision with LLVM properly wired into
`devenv.nix` first — not by silently reaching for Inkwell mid-milestone.

Evidence: `crates/emerald-codegen/src/lib.rs`, 2 passing tests
(`add(20, 22) == 42`, and a negative-number case).

**Ahead-of-time note:** this prototype uses `cranelift-jit`. Milestone `06
milestone1-codegen`'s "link an executable" requirement needs
`cranelift-object` (object-file emission) instead — noted as follow-up
work for that plan, not built here.

---

## Cross-references

- Grammar-file location and content: [`GRAMMAR.md`](./GRAMMAR.md),
  `crates/emerald-parser/src/grammar.lalrpop`.
- Type representations the codegen must eventually honor (unboxed
  `Array[T]`, `Int64` default, etc.): [`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md).
- Semantic decisions with a direct toolchain dependency (native-unwinding
  exceptions, closure heap-allocation strategy): [`SEMANTICS.md`](./SEMANTICS.md)
  §5, §7 — both remain compatible with a Cranelift backend.
