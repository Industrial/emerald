2026-09-08T18:12:04Z

Snapshot of `.cursor/plans/workspace-bootstrap.plan.md` — plan `03
workspace-bootstrap` from
[`plan-of-plans`](./2026-09-08T174011Z-plan-of-plans.md) — captured before
execution began. Rewritten from an earlier draft that assumed this plan
would run before `02`; the commit history was reordered so plan numbering
matches commit order, and this snapshot reflects the corrected leaf content
(see the plan body's "Ordering note").

---
name: Workspace Bootstrap
overview: Scaffold the Cargo workspace's directory structure (crates/, grammar/, examples/, benchmarks/, tests/) so plan 02's toolchain prototypes and later milestones have a real, permanent home instead of throwaway scratch code.
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-workspace-dirs
    content: Create examples/, benchmarks/ directories, a grammar/ pointer doc; reshape tests/ to parser/typecheck/codegen/integration
    status: pending
  - id: leaf-retire-app-placeholder
    content: Remove the crates/app placeholder now that spec/ + real crates from plan 02 supersede it
    status: pending
isProject: false
---

# Plan 03 — Workspace Bootstrap

This is `workspace-bootstrap`, row `03` of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §16.

## Deviation note

One deviation from plan-of-plans row 03's literal scope, disclosed:

**Does not pre-create all ten crates from inception §16's proposed
layout.** Inception §16 itself says the layout "is a proposal, not a
mandate... simplify it" if productive. Creating `emerald-syntax`,
`emerald-sema`, `emerald-ir`, etc. as empty shells now — before any
milestone plan has decided whether a `rowan` lossless tree or a `salsa`
incremental layer is even adopted — is exactly the premature-structure
anti-pattern inception §22 rule 9 warns against. This leaf scaffolds the
remaining **directories** `examples/`, `benchmarks/` (the parts with no
toolchain dependency) and retires the placeholder `crates/app`; `crates/`
and `grammar/`-equivalent content already exist as of plan `02`
(`crates/emerald-lexer`, `crates/emerald-parser` with its
`src/grammar.lalrpop`, `crates/emerald-codegen`). Remaining crates
(`emerald-ast`, `emerald-sema`, `emerald-types`, `emerald-ir`,
`emerald-driver`, `emerald-cli`) are created by whichever milestone plan
(`04`–`06`) first needs real content in them.

**Ordering note (superseded from an earlier draft):** this plan originally
ran before `02` so `02`'s prototypes would have a workspace home, and its
first draft's `leaf-workspace-dirs`/`leaf-retire-app-placeholder` leaves
were written against that ordering (an empty root `grammar/` dir, a fully
empty `Cargo.toml` `members = []`). The commit history was reordered so
plan numbering matches commit order (`01` → `02` → `03`); this leaf is
rewritten below to match what `02` actually produced, rather than what an
earlier draft assumed it would.

## Executive summary

This plan produces the remaining workspace-level scaffolding inception §16
proposes, scoped down to what has no toolchain dependency: `examples/`
(seeded with `hello.em` from inception §17's first milestone source),
`benchmarks/` (empty, ready for inception §21's benchmark suite), and a
reshaped `tests/` tree matching inception §16's
`parser/typecheck/codegen/integration` split. `crates/` already holds real
content from plan `02`; this plan only retires the `crates/app` placeholder
left over from before Emerald's own crates existed. Its `add(left, right)`
function was already a stand-in for inception §17's milestone-1 `add`
example; that example now lives as prose in `spec/GRAMMAR.md` §6, as a real
fixture in `crates/emerald-lexer`/`crates/emerald-parser`'s tests, and will
gain a real codegen path once plan `04`–`06` finish the milestone.

## Leaf: leaf-workspace-dirs

### 1. Context
- Why: `examples/`, `benchmarks/` still need to exist for inception §17/§21;
  `tests/` still needs reshaping now that `crates/emerald-lexer` and
  `crates/emerald-parser` (plan 02) have real tests of their own that
  `tests/parser/` should eventually cross-reference.
- Current state: `crates/` already holds `app/`, `emerald-lexer/`,
  `emerald-parser/`, `emerald-codegen/` (verified). No `examples/`,
  `grammar/`, `benchmarks/` yet. `tests/` exists with `e2e/`,
  `integration/`, both empty — `e2e/` is inherited template cruft (browser
  E2E has no meaning for a compiler with no UI).
- Target state: `examples/hello.em` containing inception §17's first
  milestone source; `benchmarks/.gitkeep`; a `grammar/README.md` pointer
  doc (since the real grammar file now lives inside
  `crates/emerald-parser/src/grammar.lalrpop`, this doc explains that
  rather than duplicating or relocating it); `tests/` reshaped to
  `parser/`, `typecheck/`, `codegen/`, `integration/`.
- Dependencies: none.
- Maestro: intended slug `leaf-workspace-dirs`, wave 0.

### 2. Acceptance Criteria
1. `examples/`, `benchmarks/`, `grammar/` all exist.
2. `examples/hello.em` contains exactly inception §17's first milestone
   source (`add` function + `puts add(20, 22)`) — identical to the fixture
   already embedded in `crates/emerald-lexer`/`crates/emerald-parser`'s
   tests, so there is one canonical source text, not two drifting copies.
3. `tests/e2e/` is removed; `tests/parser/`, `tests/typecheck/`,
   `tests/codegen/`, `tests/integration/` all exist.
4. `grammar/README.md` points at the real `crates/emerald-parser/src/grammar.lalrpop`
   rather than describing an empty, still-waiting directory (accurate as
   of plan 02, unlike this leaf's first draft).

### 3. File & Module Structure
- **Create:** `examples/hello.em`, `grammar/README.md`,
  `benchmarks/.gitkeep`, `tests/parser/.gitkeep`, `tests/typecheck/.gitkeep`,
  `tests/codegen/.gitkeep`
- **Delete:** `tests/e2e/.gitkeep` (and the directory)

### 4. Diagrams
Not applicable — directory scaffolding only.

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Structure | `test -d crates && test -d grammar && test -d examples && test -d benchmarks` | exit 0 | agent-claimed-locally |
| Example matches inception | diff against inception §17's literal snippet | identical | agent-claimed-locally |

### 6. Implementation Notes
- `examples/hello.em` is not yet parseable by anything (no lexer/parser
  exist) — it is the target artifact plan `06 milestone1-codegen`
  eventually compiles and runs, seeded now so it has one canonical home.

### 7. Risks & Rollback
- None — pure directory scaffolding, trivially revertible.

## Leaf: leaf-retire-app-placeholder

### 1. Context
- Why: `crates/app` was a generic placeholder inherited from an earlier
  session (`fix(workspace): add placeholder app crate for cargo
  workspace`), not an Emerald-specific crate. It has served its purpose
  (proving the workspace builds) and plan 02 now provides real crates in
  its place.
- Current state: `crates/app` exists with a trivial `add(left, right)`
  function and one test; listed in the root `Cargo.toml` `members`
  alongside `crates/emerald-lexer`, `crates/emerald-parser`,
  `crates/emerald-codegen`.
- Target state: `crates/app` removed; `Cargo.toml` `members` lists only
  the three real crates plan 02 created.
- Dependencies: none.
- Maestro: intended slug `leaf-retire-app-placeholder`, wave 0, parallel
  with `leaf-workspace-dirs`.

### 2. Acceptance Criteria
1. `crates/app/` no longer exists.
2. `Cargo.toml`'s `members` no longer lists `crates/app`, and still lists
   `crates/emerald-lexer`, `crates/emerald-parser`, `crates/emerald-codegen`.
3. `cargo build --workspace` and `cargo test --workspace` both succeed.

### 3. File & Module Structure
- **Delete:** `crates/app/Cargo.toml`, `crates/app/src/lib.rs`
- **Modify:** `Cargo.toml` (`members` drops `crates/app`, keeps the rest)

### 4. Diagrams
Not applicable.

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | exit 0 | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass | agent-claimed-locally |

### 6. Implementation Notes
- No placeholder crate is needed once real crates exist to keep Cargo
  happy; `crates/app` was only ever a stand-in.

### 7. Risks & Rollback
- Risk: none — the placeholder had no real callers.

## Total quality gate
```bash
test -d crates && test -d grammar && test -d examples && test -d benchmarks \
  && test -f examples/hello.em && ! test -d tests/e2e && ! test -d crates/app \
  && cargo build --workspace && cargo test --workspace
```

## Out of scope / deferred
- Creating any of the remaining six crates from inception §16's proposed
  layout (`emerald-ast`, `emerald-sema`, `emerald-types`, `emerald-ir`,
  `emerald-driver`, `emerald-cli`) — deferred to the plan that first needs
  each one (see Deviation note).
