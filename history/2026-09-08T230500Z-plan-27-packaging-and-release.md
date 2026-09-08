---
name: Packaging & Release
overview: "GitHub Releases (via cargo-dist) for emerald-cli/emerald-lsp/emerald-mcp, a real fix for the runtime's dev-time-only path lookup, and a reviewed-not-run CI job for plan 17's disclosed npm/vsce gap — the actual 'get a binary onto your machine' half of adoption, not just buildable source."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ci-release-workflow
    content: "cargo-dist-driven .github/workflows/release.yml building emerald-cli (+ emerald-lsp/emerald-mcp once plan 17 lands) for x86_64-unknown-linux-gnu, plus a build.rs that precompiles runtime/emerald_runtime.c so a shipped binary no longer needs this repo's checkout to link a user's program"
    status: pending
  - id: leaf-vscode-publish-workflow
    content: ".github/workflows/vscode-publish.yml running plan 17's npm/vsce steps for real on a runner that actually has npm — authored and manually reviewed, not executed in this sandbox"
    status: pending
  - id: leaf-packaging-docs
    content: "RELEASING.md — real install instructions cross-checked against what the other two leaves actually produce"
    status: pending
isProject: false
---

# Plan 27 — Packaging & Release

Like [plan 17](./2026-09-08T213000Z-plan-17-ide-integration.md), this is
**not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
post-v1 tooling scope, not an inception §§ item. It also has a real
forward dependency plan-of-plans' own "Depends on" column has no place
for: `emerald-lsp` and `emerald-mcp` don't exist yet (verified this
session — the root `Cargo.toml`'s `[workspace] members` still lists only
the original five crates: `emerald-lexer`, `emerald-parser`,
`emerald-codegen`, `emerald-sema`, `emerald-cli`; plan 16's separate
Cranelift-vs-LLVM codegen-backend bake-off and plan 17's `emerald-lsp`/
`emerald-mcp` are both authored but not executed). This plan's
release workflow is written to build whichever of the three plan-17
binaries currently exist, so it doesn't have to block on plan 17
landing first — see the Decision log and `leaf-ci-release-workflow`'s
own scoping note.

Concrete proof this plan targets: a tagged release produces GitHub
Release assets containing a real `emerald-cli` binary that, copied
*alone* to an empty scratch directory with no access to this repo's
`runtime/emerald_runtime.c`, still compiles and links
`examples/hello.em` and prints `42` — proof that packaging actually
removed the dev-time-only path dependency, not just that the binary
still happens to run inside the source checkout it was built in.

## Decision log

- **No `.github/` directory exists at all today** (verified this
  session). This plan is establishing release CI from zero, not
  modifying an existing pipeline — there is nothing to preserve
  compatibility with.
- **`cargo-dist` 0.32.0** (confirmed live on crates.io this session,
  same discipline as plans 14/17's crate-existence checks) is used
  instead of a hand-rolled build-per-target-triple-and-upload workflow.
  AGENTS.md's own convention — "match existing code style; use
  established libraries before adding new ones" — applies directly:
  `cargo-dist` generates its own GitHub Actions workflow (`dist init`/
  `dist generate`) from a small `[workspace.metadata.dist]` config
  block, leaving almost no hand-authored release YAML for this repo to
  maintain, versus writing and debugging a matrix-build/checksum/
  upload pipeline from scratch for a project that has never had any CI
  at all.
- **Only `x86_64-unknown-linux-gnu` is a real, committed release
  target.** Confirmed via `rustc -vV` this session — this sandbox's own
  host triple (`rustc` 1.97.1, `x86_64-unknown-linux-gnu`; its LLVM
  22.1.6 backend is `rustc`'s own, unrelated to plan 16's separate
  Cranelift-vs-LLVM *codegen*-backend bake-off). macOS and Windows
  cross-compilation are explicitly deferred: nothing in this sandbox
  can verify a macOS or Windows cross-toolchain actually produces a
  working binary, and `cargo-dist`'s cross-built targets are only
  meaningfully exercised on GitHub's own hosted runners once this
  workflow actually runs there. A real, disclosed scope cut — the same
  shape as plan 15's "`ruby` not installed" — not an oversight.
- **Runtime bundling is a real, load-bearing fix, not packaging
  boilerplate.** `emerald-cli`'s own `link_stage` already documents the
  gap it's shipping with today: it locates `runtime/emerald_runtime.c`
  via `env!("CARGO_MANIFEST_DIR")`-relative lookup and its own doc
  comment says plainly, "A real install would bundle a compiled runtime
  object/archive instead — noted as follow-up packaging work, not
  solved here." This plan is that follow-up: a `build.rs` compiles
  `runtime/emerald_runtime.c` once, at `emerald-cli`'s own build time
  (via the `cc` crate — the standard, already-idiomatic way to invoke a
  C compiler from a Rust build script), into a static archive that gets
  linked into the shipped binary itself, so the *runtime's own code* no
  longer needs the source `.c` file to be present at all. This does
  **not** remove `emerald-cli`'s dependency on a `cc`-compatible linker
  being present on the end user's machine — linking a *user's* compiled
  `.em` program is a normal part of every `emerald-cli` invocation, not
  a one-time install step, so that dependency is permanent and is
  disclosed as such, not solved by this plan.
- **VSCode publish still can't run for real in this sandbox** — the
  identical constraint plan 17 already disclosed (`npm`, `npx`, `yarn`,
  `pnpm` all absent from `PATH`; `node` and npm-registry network access
  both present, confirmed again this session). This plan authors
  `.github/workflows/vscode-publish.yml` to run those exact steps for
  real on a standard `ubuntu-latest` GitHub Actions runner (which ships
  npm), and verifies it by careful manual review of the YAML rather
  than an actual run — the same "author + disclose can't run it here"
  pattern plan 17 used for the Extension Development Host step.
- **No Homebrew formula, no Nix package/flake output.** This repo's Nix
  footprint (`devenv.nix`, `devenv.yaml`, `.envrc`) is a `devenv`
  *development*-shell setup, not a `flake.nix` package output —
  confirmed no `flake.nix` exists. Building a real Nix package or
  Homebrew formula is genuine additional packaging surface with no
  existing scaffold to extend here, and `cargo-dist`'s GitHub Releases
  already satisfy this plan's concrete "get a real binary" target.
  Deferred, not attempted, and not silently implied by "packaging and
  release" being the plan's title.
- **No version bump.** AGENTS.md's convention — "bump the relevant
  version when behavior changes" — is applied literally: this plan adds
  release *infrastructure*, it doesn't change `emerald-cli`/`emerald-
  lsp`/`emerald-mcp`'s observable behavior. `workspace.package.version`
  stays `0.1.0`; the first tag this workflow ever runs against is
  `v0.1.0`, matching the version already in the tree rather than
  inventing a bump this plan has no behavioral justification for.
- **`leaf-vscode-publish-workflow`'s YAML is verified by manual review,
  not an automated parse.** This sandbox's `python3` has its `-c`
  inline-eval flag blocked (a permanent restriction, confirmed this
  session) and no YAML-parsing CLI is confirmed installed; rather than
  build a shaky ad hoc verification path for one config file, this leaf
  states plainly that its acceptance is a careful structural review,
  the same disclosed-manual-step posture plan 17 already used instead
  of fabricating automation that isn't really there.

## Leaf: leaf-ci-release-workflow

### 1. Context
- Why: today the only way to get a working `emerald-cli` binary is
  `cargo build --workspace` inside a full source checkout with a
  working `devenv` shell — no release artifact exists anywhere.
- Target state: `[workspace.metadata.dist]` in the root `Cargo.toml`
  configured for `cargo-dist` 0.32.0, naming `emerald-cli` as a release
  binary for `x86_64-unknown-linux-gnu` (see Decision log); `dist
  generate` produces `.github/workflows/release.yml`, triggered on
  `v*` tag pushes. `crates/emerald-cli/build.rs` (new) compiles
  `runtime/emerald_runtime.c` via the `cc` crate into a static archive
  under `OUT_DIR` and links it into the `emerald-cli` binary itself;
  `link_stage` in `main.rs` (or, if plan 17 has landed by the time this
  executes, `emerald-driver`'s equivalent) stops resolving the `.c`
  file via `CARGO_MANIFEST_DIR` and instead links the user's compiled
  object against the archive already inside the binary's own build
  output, via a path `build.rs` exposes through `OUT_DIR`/a build-time
  environment variable rather than a runtime source-tree lookup.
- Sequencing note: if this leaf executes before plan 17 lands, the
  `dist` config and this fix cover `emerald-cli` only; adding
  `emerald-lsp`/`emerald-mcp` to `[workspace.metadata.dist]` and giving
  them the same `build.rs` treatment once plan 17's crates exist is a
  small, additive config/build-script change, not a re-architecture —
  recorded here so it isn't lost, not treated as blocking this leaf.

### 2. Acceptance Criteria
1. `cargo dist plan` runs cleanly against the root workspace and lists
   `emerald-cli` as a release artifact for `x86_64-unknown-linux-gnu`
   (plus `emerald-lsp`/`emerald-mcp` if plan 17 has landed by the time
   this executes).
2. A real local release-mode build's `emerald-cli` binary, **copied
   alone** to an empty scratch directory with no access to this repo's
   `runtime/emerald_runtime.c`, still successfully compiles and links
   `examples/hello.em`'s real contents and prints `42` — proof the
   dev-time-only path dependency is actually gone, not just unexercised
   by an in-repo test run.
3. `.github/workflows/release.yml` (cargo-dist's own generated output)
   exists and its trigger is a `v*` tag push — verified by reading the
   generated YAML, not by executing it (no GitHub Actions runner
   available in this sandbox).
4. Regression: `crates/emerald-cli/tests/hello_em.rs`'s existing tests
   (or `emerald-driver`'s, if plan 17 has landed) still pass unmodified
   — the runtime-linking change alters *where* the archive comes from,
   not the pipeline's observable behavior.

### 3. File & Module Structure
- **Create:** `.github/workflows/release.yml` (cargo-dist generated),
  `crates/emerald-cli/build.rs`
- **Modify:** root `Cargo.toml` (`[workspace.metadata.dist]`),
  `crates/emerald-cli/Cargo.toml` (adds `cc` as a build-dependency),
  `crates/emerald-cli/src/main.rs` (`link_stage`'s runtime-archive
  lookup)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Dist plan | `cargo dist plan` | lists `emerald-cli` for `x86_64-unknown-linux-gnu` | agent-claimed-locally |
| Build | `cargo build --workspace --release` | clean | agent-claimed-locally |
| Portability smoke test | copy `emerald-cli` to an empty `/tmp`-style scratch dir; run it there against `examples/hello.em` | prints `42` with no access to `runtime/emerald_runtime.c` | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pass | agent-claimed-locally |
| Workflow review | manual read of generated `.github/workflows/release.yml` | triggers on `v*` tags, builds the right binaries | agent-claimed-locally |

---

## Leaf: leaf-vscode-publish-workflow

### 1. Context
- Why: plan 17's `editors/vscode/` extension source exists (once that
  plan lands) but its `npm install`/`vsce package`/Marketplace publish
  steps were explicitly disclosed as not runnable in this sandbox — see
  Decision log.
- Target state: `.github/workflows/vscode-publish.yml`, triggered on
  the same `v*` tag push (or a dedicated `vscode-v*` tag, to allow
  publishing the extension independently of a compiler release): runs
  on `ubuntu-latest` (ships `npm`), `cd editors/vscode && npm install`,
  runs plan 17's `check-grammar.mjs` and `check-manifest.mjs` as a CI
  gate (the same two checks that sandbox could run directly), then
  `npx vsce package`, and `npx vsce publish` gated behind a `VSCE_PAT`
  repository secret being present — so a fork or a PR run never
  attempts a real Marketplace publish.

### 2. Acceptance Criteria
1. `.github/workflows/vscode-publish.yml` exists, is well-formed YAML
   (2-space indentation, consistent with `release.yml`'s own generated
   style), and its `publish` step is conditioned on
   `secrets.VSCE_PAT != ''` (or equivalent), verified by manual review
   — see Decision log for why this is a reviewed, not automated, gate
   in this sandbox.
2. The workflow re-runs plan 17's own `check-grammar.mjs`/
   `check-manifest.mjs` before packaging — a real regression gate, not
   just a rebuild, so a future grammar/manifest drift fails CI instead
   of silently shipping.
3. Not claimed as verified here: an actual `npm install`/`vsce package`
   run. Disclosed explicitly in `RELEASING.md` (`leaf-packaging-docs`)
   as "authored, not yet executed" until this workflow runs for real on
   GitHub Actions.

### 3. File & Module Structure
- **Create:** `.github/workflows/vscode-publish.yml`
- **Modify:** `editors/vscode/README.md` (plan 17's — notes that a real
  CI publish path now exists, still not yet run for real)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| YAML review | manual read of `.github/workflows/vscode-publish.yml` | well-formed, publish step gated on `VSCE_PAT` presence | agent-claimed-locally |
| Doc cross-check | manual read of `editors/vscode/README.md`'s updated claim | states plainly the publish step is authored but unexecuted here | agent-claimed-locally |

---

## Leaf: leaf-packaging-docs

### 1. Context
- Why: `leaf-ci-release-workflow` and `leaf-vscode-publish-workflow`
  each produce a real pipeline, but nothing tells a real user how to
  actually install the result once a release exists.
- Target state: `RELEASING.md` at the repo root, covering: how to cut a
  release (push a `v*` tag; what `cargo-dist` does from there); where
  the resulting binaries land (GitHub Releases, `x86_64-unknown-linux-
  gnu` only — macOS/Windows explicitly marked not yet supported, per
  Decision log); the permanent `cc`-on-`PATH` requirement for
  `emerald-cli` users (linking a compiled `.em` program always shells
  out to `cc` — packaging removes the *runtime bundle* dependency, not
  this one); and the VSCode extension's current real status (source and
  a reviewed-but-unexecuted publish workflow exist; installing today
  means building the `.vsix` locally with a real `npm`, not yet "search
  the Marketplace").

### 2. Acceptance Criteria
1. Every claim in the doc is cross-checked against what
   `leaf-ci-release-workflow`/`leaf-vscode-publish-workflow` actually
   produced — no aspirational "just `brew install emerald`"-style
   instructions for anything this plan didn't build.
2. The doc states plainly, in one place, exactly which OS/architecture
   combinations have a real release binary today (one:
   `x86_64-unknown-linux-gnu`) versus which don't (macOS, Windows) —
   matching Decision log's disclosed scope cut precisely.

### 3. File & Module Structure
- **Create:** `RELEASING.md`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Accuracy pass | manual cross-check of every claim against leaves 1–2's real output | no stale/aspirational instructions | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace --release
cargo test --workspace
cargo dist plan
```
(`cargo dist plan` requires `cargo-dist` installed locally first —
`cargo install cargo-dist@0.32.0` — a one-time local tool install, not
a new runtime dependency of the workspace itself.)

## Out of scope / deferred
- **Actually cutting a real `v0.1.0` GitHub Release** — this plan
  builds the pipeline; pushing the first real tag and watching it run
  on GitHub's own infrastructure is a follow-up action, not authored
  here (no GitHub Actions runner exists in this sandbox to prove it on).
- **macOS and Windows binaries**, and any code-signing/notarization
  those platforms would additionally require — see Decision log.
- **Homebrew formula, Nix package/flake output** — see Decision log.
- **A real, executed `npm install`/`vsce package`/Marketplace publish**
  — authored and reviewed in `leaf-vscode-publish-workflow`, not run;
  see Decision log.
- **An Emerald-language package manager or code registry** (a
  crates.io/npm equivalent for distributing *Emerald programs/
  libraries*, as opposed to distributing the compiler's own binaries) —
  an entirely separate, much larger future initiative; not conflated
  with this plan's "install the compiler" scope.
- **Removing `emerald-cli`'s dependency on a system `cc`** — permanent,
  disclosed in the Decision log; this plan bundles the *runtime*, not a
  linker.
