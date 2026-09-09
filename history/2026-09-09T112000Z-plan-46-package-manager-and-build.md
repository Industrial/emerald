---
name: Package Manager and Build
overview: "An `emerald.toml` manifest (name/version/entry + path/git-pinned dependencies, no registry), `emerald new`/`build`/`run` subcommands added to today's flag-only `emerald-cli`, a resolver that fetches path/git dependencies and makes them `require`-able via a dot-free `deps/<name>` symlink (plan 23's containing-file-relative resolver, unchanged), and an `emerald.lock` pinning exact resolved paths/git revisions for reproducible rebuilds."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-manifest-schema
    content: "emerald.toml parsing — [package] {name, version, entry} and [dependencies] as {path=\"...\"} or {git=\"...\", rev=\"...\"}, via toml+serde in a new crates/emerald-cli/src/manifest.rs"
    status: pending
  - id: leaf-dependency-resolution
    content: "Fetch path/git dependencies (git clone/checkout into .emerald/deps/, path deps read in place), then materialize a dot-free deps/<name> symlink each dependency's require-ing file actually targets — feeding plan 23's existing containing-file-relative resolver unchanged, no transitive (dependency-of-a-dependency) resolution"
    status: pending
  - id: leaf-lockfile
    content: "emerald.lock recording exact resolved path/git-revision per dependency; read on subsequent builds to skip re-resolution unless the manifest's own dependency spec changed; emerald update explicitly deferred"
    status: pending
  - id: leaf-cli-subcommands
    content: "emerald new/build/run subcommand dispatch in emerald-cli's main.rs, [[bin]] renamed emerald-cli -> emerald, legacy bare `emerald-cli <source.em> [-o <output>]` single-file mode preserved unchanged for programs with no emerald.toml"
    status: pending
isProject: false
---

# Plan 46 — Package Manager and Build

This is plan 46 of the 36-47 follow-up batch: twelve independent
siblings, each closing one distinct gap toward the ~45% Ruby-surface-
parity ceiling the prior gap analysis set for Emerald, none of them
conceding the identity constraints (no `method_missing`/`eval`/`send`/
reflection, no mixins/open classes/monkey-patching, no dynamic dispatch
or vtables, no tracing GC). This plan is the one sibling in the batch
that is **pure ecosystem/tooling work** — it adds a project manifest, a
dependency resolver, and a lockfile around the existing compiler; it
does not add or change a single `Expr`/`Stmt`/`Item` variant, grammar
production, sema rule, or codegen path. Because of that, it has no
dynamism-vs-identity tradeoff to weigh at all (see Decision log for why
that makes this plan's growth budget structurally different from every
other sibling's). Like every other post-v1 plan in this repository, it
is **not** a row in [`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
— that table's own Completion note calls Emerald v1 done at row 15, and
`plan-of-plans.md` (and this batch's own eventual summary) will be
updated separately after all twelve of plans 36-47 are authored. This
plan does not touch `plan-of-plans.md` or any other plan file.

Concrete proof this plan targets — a real two-package workspace, one
library consumed by one consumer, resolved and compiled across a real
path dependency with no manual file copying:

```
workspace/
  mathutils/
    emerald.toml      # [package] name="mathutils" version="0.1.0" entry="lib.em"
    lib.em            # module MathUtils; def add(a: Int64, b: Int64) -> Int64
  app/
    emerald.toml       # [package] name="app" ...; [dependencies] mathutils = { path = "../mathutils" }
    main.em            # require deps/mathutils/lib ; puts MathUtils.add(3, 5)
```

```
$ cd workspace/app
$ emerald build
   Resolving mathutils (path ../mathutils)
   Compiling app v0.1.0 (main.em)
    Finished build: ./app
$ ./app
8
$ emerald run
   Finished build: ./app          # emerald.lock unchanged -> resolution skipped, only recompiled
8
```

`emerald build` creates `app/emerald.lock` (pinning `mathutils`'s
resolved absolute path) and `app/deps/mathutils` (a symlink — see
Decision log for exactly what it points at and why) on its first run;
neither file existed before. Deleting `./app` and re-running
`emerald run` recompiles and reprints `8` without touching either.

## Decision log

- **This plan touches no language semantics, so the ~45% ceiling
  doesn't bound it the way it bounds every other 36-47 sibling.** Every
  other plan in this batch closes a *language/stdlib* gap and must stop
  the instant closing it further would require dynamism this project's
  identity constraints forbid (reflection, open classes, vtables, a
  tracing GC). A package manager adds zero new `Expr`/`Stmt`/`Item`
  variants, zero new sema rules, zero new codegen paths — verified this
  session against `crates/emerald-parser/src/ast.rs` (no `Item::Require`
  variant exists yet at all — see below) and against this plan's own
  leaf list, none of which touches `emerald-parser`, `emerald-sema`, or
  `emerald-codegen`. There is no dynamism this plan could be tempted to
  reach for and no identity line it could cross, so "how far should this
  go" is purely a question of effort and real-world usefulness, not of
  distance from a static-dispatch ceiling — this is exactly why
  ecosystem/tooling work is a comparatively unbounded place to spend
  this batch's growth budget, and it's stated here explicitly rather
  than left implicit.
- **Depends on plan 23's `require` design, not on plan 23 having
  shipped.** Verified this session: `crates/emerald-parser/src/ast.rs`
  has no `Item::Require` variant, `crates/emerald-parser/src/
  grammar.lalrpop` has no `require` production, and the workspace root
  `Cargo.toml`'s `[workspace] members` still lists only the original
  five crates (`emerald-lexer`, `emerald-parser`, `emerald-codegen`,
  `emerald-sema`, `emerald-cli`) — no `emerald-driver` crate exists.
  `history/2026-09-08T222500Z-plan-23-multi-file-compilation.md` is
  authored but unexecuted, exactly like this plan. This plan builds on
  plan 23's *design* (a `require <bare/path>` token resolved relative to
  the containing file's own directory, `.em` implied, canonicalized-path
  dedup, in-progress-stack cycle detection via `emerald-driver`'s
  `resolve_program`, or — if plan 17's driver extraction hasn't landed
  either by execution time — whatever equivalent lives directly in
  `emerald-cli`, mirroring plan 23's own identical sequencing note about
  this exact ambiguity) rather than redoing or forking that mechanism.
  If plan 23 executes first, this plan's resolver hands it ordinary,
  already-resolvable relative paths and changes nothing about it; if
  this plan somehow executes first, `leaf-dependency-resolution`'s own
  acceptance criteria are unverifiable until plan 23 lands, and that
  ordering dependency is stated here rather than silently assumed.
- **Path dependencies become `require`-able through a dot-free
  `deps/<name>` symlink at the consuming package's manifest directory —
  not by teaching `require` a new lookup mechanism.** Plan 23's own
  Decision log spells its proposed `RequirePath` token as
  `r"[A-Za-z_][A-Za-z0-9_/]*"` — letters, digits, underscore, slash,
  **no dot** — so neither a parent-directory escape (`require ../foo`)
  nor a dot-prefixed segment (`require .emerald/deps/foo`) can ever be
  written in source, by design (plan 23 also states plainly there is
  "no load-path/stdlib-search-path concept here at all"). Rather than
  extending that token's grammar to admit a dot — which would mean
  editing `emerald-parser`'s lexer/grammar, a real language-semantics
  touch this plan deliberately avoids per its own stated scope — this
  plan's resolver instead arranges the *filesystem* so plan 23's
  resolver already reaches a dependency without any grammar change:
  it creates a plain, dot-free `deps/<name>` symlink directly beside the
  consuming package's manifest (a real subdirectory of the entry file's
  own directory, exactly the shape plan 23's containing-file-relative
  rule already walks), pointing at the dependency's resolved root. A
  source file writes `require deps/mathutils/lib` — an entirely ordinary
  path under plan 23's existing token and resolution rule, appended
  `.em` and all. Nothing about `resolve_program`'s algorithm changes;
  this plan only ever arranges to be one more real file plan 23 was
  always able to find.
- **Git dependencies get a real fetch cache at `.emerald/deps/<name>-
  <rev-prefix>/`; path dependencies do not, because there is nothing to
  fetch.** For a git dependency, resolution shells out to a real
  `git clone <url> .emerald/deps/<name>-<rev-prefix>/` followed by
  `git checkout <rev>` inside it (`<rev-prefix>` is the first 7 hex
  characters of the *resolved* commit SHA — see the lockfile bullet
  below — not the possibly-unresolved `rev` string from the manifest,
  so two different branches that happen to resolve to the same commit
  share one cache directory instead of two). This is a thin wrapper
  over the system `git` binary — no VCS logic of Emerald's own exists or
  is added; a missing/unreachable remote surfaces `git`'s own exit
  status and stderr, not a re-implemented error. A path dependency's
  content already lives on disk at the manifest-declared path; copying
  or symlinking it into `.emerald/deps/` first would be pure indirection
  with no benefit, so path dependencies skip `.emerald/deps/` entirely —
  their `deps/<name>` symlink (previous bullet) points straight at the
  canonicalized path-dependency root.
- **No transitive dependency resolution — only the package currently
  being built's own `[dependencies]` table is read.** A dependency's
  `emerald.toml`, if it has one, is read for exactly one thing: its
  `[package]` table's `name`/`entry`, to know what to name and where to
  point the `deps/<name>` symlink (see above) — its own `[dependencies]`
  table is never opened or followed. This is a genuine, substantial,
  deliberately deferred feature, not an oversight: real transitive
  resolution needs a diamond-conflict story (what happens when two
  direct dependencies each declare their own, possibly different,
  dependency on a same-named third package) that this plan's flat,
  per-manifest `deps/<name>` symlink namespace has no way to represent
  today. A first cut that only resolves direct dependencies is already
  enough to prove the whole manifest -> fetch -> lock -> `require` ->
  compile path works end to end, which is this plan's actual target.
- **`emerald.lock` records the exact resolved git commit SHA (even when
  the manifest's own `rev` is a loose ref like a branch or tag name) or
  the exact resolved absolute path, written after a successful resolve,
  and is read on every later build to skip re-resolution entirely as
  long as the manifest's dependency spec is unchanged.** This mirrors
  the same reproducibility guarantee Cargo's own `Cargo.lock` and
  Bundler's own `Gemfile.lock` give — a `rev = "main"` in the manifest
  is a moving target, but `emerald.lock`'s `resolved_rev` is a fixed
  commit, so a second developer running `emerald build` against the
  same manifest and lockfile gets bit-for-bit the same dependency
  source, not whatever `main` happens to point at that day. Detecting
  "the manifest's dependency spec changed" is a plain equality check
  between the manifest's `{path}`/`{git, rev}` table and the lockfile's
  recorded source spec for that name — not a content hash of the
  dependency's files, which real re-resolution (a fresh `git checkout`,
  or nothing at all for an unchanged path) already re-establishes
  correctness for on every changed-spec build.
- **`emerald update` is explicitly deferred, not built here.** The
  distinction the task calls out — "any/latest version ranges are out
  of scope, only exact path/git-rev pins are supported" — already
  covers the common case without a dedicated subcommand: a first build
  with no lockfile resolves and locks everything; editing a dependency's
  `path`/`git`/`rev` in the manifest is detected as a spec change (see
  above) and re-resolved automatically on the very next build. The one
  case a bare `emerald build` genuinely cannot do is re-resolve a
  *loose* ref (`rev = "main"`) whose remote has moved forward while the
  manifest text itself is unchanged — that needs its own "ignore the
  lockfile's recorded SHA for this one dependency, re-fetch, and
  compare" code path, which is real, separate design surface (which
  dependencies does it apply to — all of them, or one named on the
  command line? does it fail closed if the manifest pins an exact SHA
  already?) this plan doesn't need answered to prove manifest-driven
  path/git resolution and locking work. Deferred to a follow-up plan,
  not silently assumed away.
- **No hosted registry/index.** A registry is a server-side project
  (an index format, a publish API, a download CDN, account/ownership
  management) — not a compiler-repo concern, and explicitly out of
  scope for the same reason plan 27 declined a Homebrew formula or a
  Nix flake output: no existing scaffold in this repo to extend, and
  nothing about this plan's own concrete target (a real two-package
  build resolving across a path/git dependency) needs one. The task's
  own framing — `{path=...}`/`{git=..., rev=...}` dependencies only,
  explicitly no central registry — mirrors how early Cargo (pre-
  crates.io) and early Bundler (pre-rubygems.org-as-default) both
  bootstrapped from exactly these two dependency shapes before either
  ecosystem had a hosted index; this plan makes that same bootstrapping
  choice deliberately, not as a stopgap it forgot to revisit.
- **No semantic-version constraint solving.** `version` in `[package]`
  is metadata only (identical in spirit to `Cargo.toml`'s own
  `package.version` field before any dependency of *this* plan's design
  ever reads it for constraint-solving purposes) — the resolver never
  compares two dependents' version requirements for the same package,
  because there is no "same package required transitively by two paths"
  case in scope at all (see the no-transitive-resolution bullet above).
  A real `^1.2`/`~> 1.2`/`>= 1.0, < 2.0` range solver is substantial,
  separate machinery (a full SAT-style resolver, the way Cargo's or
  Bundler's own dependency resolution works) that this plan's exact-pin
  model has no need for and does not build any scaffolding toward.
- **Manifest and lockfile are both TOML, via the `toml` + `serde`
  crates** — not independently re-confirmed live on crates.io this
  session the way plan 27 checked `cargo-dist` 0.32.0, but both are
  long-stable, ubiquitous crates (current majors `toml` 0.8.x, `serde`
  1.x with the `derive` feature) and TOML is not a new format choice
  for this workspace: the workspace's own root `Cargo.toml` (verified
  this session) is already TOML, so `emerald.toml`/`emerald.lock`
  mirror `Cargo.toml`/`Cargo.lock`'s own format precisely — the same
  ecosystem-bootstrapping analogy the task calls out, extended to file
  format, not just dependency shape.
- **Manifest/lockfile/resolver logic lives as new modules inside
  `crates/emerald-cli`, not a new crate.** `crates/emerald-cli/src/
  main.rs`'s own doc comment (verified this session) states the
  project's actual precedent directly: it orchestrates parse/check/
  codegen/link "directly here rather than in a separate `emerald-driver`
  crate ... extracted when a second caller needs the same pipeline."
  No second caller needs manifest parsing, dependency resolution, or
  lockfile handling today (`emerald-lsp`/`emerald-mcp` don't exist yet
  either — plan 17 is still unexecuted, same as plan 23) — so, by the
  identical logic, this plan's three new modules (`manifest.rs`,
  `lockfile.rs`, `deps.rs`) stay inside `emerald-cli` rather than
  becoming a new workspace member. If a second caller (an `emerald-lsp`
  "go to dependency definition" feature, say) ever needs this logic,
  extracting it is the same small, disclosed follow-up plan 06 already
  described for the compiler pipeline itself — not redone here.
- **The shipped binary is renamed `emerald-cli` -> `emerald`; the crate
  stays `emerald-cli`.** Verified this session: `crates/emerald-cli/
  Cargo.toml` has no `[[bin]]` table today, so Cargo's default rule
  applies and the binary Cargo produces is literally named
  `emerald-cli` — not `emerald`, which is what every `emerald new`/
  `build`/`run` invocation in this plan's own worked scenario and in the
  task's own scope description assumes. `crates/emerald-cli/tests/` is
  verified empty (no existing test references `CARGO_BIN_EXE_emerald-
  cli` or any other binary-name-sensitive string) and no CI workflow
  exists yet (plan 27's `.github/` directory doesn't exist either), so
  adding one `[[bin]] name = "emerald"` entry is a safe, disclosed,
  zero-regression rename available to take now rather than living with
  an awkward `emerald-cli build` invocation indefinitely. The crate/
  package name (`emerald-cli`, used by `cargo build -p emerald-cli` and
  everywhere else in this repo's tooling) is unchanged.
- **The legacy bare invocation, `emerald <source.em> [-o <output>]`
  with no subcommand, keeps working unchanged for a file with no
  `emerald.toml`.** `crates/emerald-cli/src/main.rs`'s actual current
  `fn main` (verified this session) takes exactly this shape today —
  `args.get(1)` as the source path, an optional `-o <output>` pair, no
  subcommand dispatch of any kind. `leaf-cli-subcommands` adds `new`/
  `build`/`run` as dispatched-on `args[1]` values; any other `args[1]`
  (a real `.em` path, as today) falls through to the exact same parse-
  check-codegen-link pipeline that already exists, byte-for-byte
  unchanged — a manifest-driven build is strictly additive, not a
  breaking replacement of the single-file compile path every existing
  example (`examples/hello.em`, etc.) already relies on.
- **Out of scope: workspaces (one root manifest listing multiple local
  packages, Cargo-workspace-style).** Real, useful, and a natural
  extension once multiple local packages are common in one checkout —
  but this plan's own worked scenario (one library, one consumer, two
  independent manifests) already proves cross-package resolution works
  without needing a third, workspace-level manifest format invented and
  justified. Deferred as a distinct, separately-scoped follow-up.
- **Out of scope: any build-artifact caching of an already-compiled
  dependency.** Plan 23's own design compiles a `require`d dependency's
  source by splicing its parsed `Item`s into one flat `Program` ahead of
  a single `emerald_sema::check_program`/`emerald_codegen::
  compile_to_object` call over the whole merged result — there is no
  separate "compile `mathutils` once, reuse its object file across
  builds" step for this plan to hook into, because no such per-file
  compilation unit exists in this compiler at all (plan 23's Decision
  log states this precisely: real separate compilation units are future
  work, not built yet). This plan's dependency resolution is therefore
  a *source*-fetching step only; every `emerald build` recompiles the
  full merged program, dependencies included, from source — identical
  in spirit to how `rustc`, absent incremental compilation, would
  recompile everything too. A real dependency-build cache is future
  work layered on top of whatever plan eventually gives this compiler
  per-unit compilation, not this one.

## Leaf: leaf-manifest-schema

### 1. Context
- Why: nothing in this workspace parses any Emerald-specific config
  file today — verified this session against `crates/emerald-cli/
  Cargo.toml` (dependencies: `emerald-parser`, `emerald-sema`,
  `emerald-codegen`, `miette`, `id_effect` — no `toml`, no `serde`).
- Target state: `crates/emerald-cli/src/manifest.rs` defines
  `Manifest { package: PackageMeta, dependencies: BTreeMap<String,
  DependencySpec> }`, `PackageMeta { name: String, version: String,
  entry: String }` (`entry` defaults to `"lib.em"` via `#[serde(default
  = "default_entry")]` when parsing a *dependency's* manifest, and to
  `"main.em"` when scaffolding a new package via `leaf-cli-
  subcommands`'s `emerald new` — the default differs by role, not by a
  hardcoded global constant), and `DependencySpec` as a `#[serde(untagged)]`
  enum of `Path { path: String }` / `Git { git: String, rev: String }`.
  `Manifest::load(dir: &Path) -> Result<Manifest, ManifestError>` reads
  `<dir>/emerald.toml`; a missing `[package].entry` for the *top-level*
  package being built (not a dependency) is also defaulted to
  `"main.em"`.

### 2. Acceptance Criteria
1. A real `emerald.toml` with `[package] name = "app"` `version =
   "0.1.0"` and `[dependencies] mathutils = { path = "../mathutils" }`
   parses to a `Manifest` whose `dependencies["mathutils"]` is
   `DependencySpec::Path { path: "../mathutils".into() }`.
2. A `[dependencies] mathutils = { git = "...", rev = "v1.0.0" }` entry
   parses to `DependencySpec::Git { git: "...", rev: "v1.0.0".into() }`.
3. A manifest missing `[package].entry` defaults to `"main.em"` when
   loaded as the top-level package and to `"lib.em"` when loaded as a
   dependency's own manifest (two distinct call sites, asserted
   separately).
4. A manifest with a syntactically invalid TOML body, or a
   `[dependencies]` entry that is neither `{path=...}` nor `{git=...,
   rev=...}` (e.g. both keys present, or neither), is rejected with a
   descriptive `ManifestError`, not a panic.
5. Regression: this leaf adds a new module and a new `Cargo.toml`
   dependency only — `cargo build --workspace` and every existing
   `emerald-cli` behavior (the legacy bare-file invocation) are
   unaffected, since nothing yet calls `Manifest::load` from `main`.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/src/manifest.rs`,
  `crates/emerald-cli/tests/manifest.rs`
- **Modify:** `crates/emerald-cli/Cargo.toml` (adds `toml`, `serde`
  with the `derive` feature), `crates/emerald-cli/src/main.rs` (adds
  `mod manifest;`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-cli manifest` | all pass, incl. default-entry and malformed-dependency-table negative cases | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-dependency-resolution

### 1. Context
- Why: a parsed `Manifest` (leaf 1) names dependencies but nothing
  fetches, canonicalizes, or makes any of them `require`-able yet.
- Target state: `crates/emerald-cli/src/deps.rs` exposes
  `resolve_dependencies(manifest_dir: &Path, manifest: &Manifest) ->
  Result<Vec<ResolvedDependency>, DepsError>`. For each `(name, spec)`:
  a `Path` spec is canonicalized (`std::fs::canonicalize`) relative to
  `manifest_dir`, its own `emerald.toml` is read (if present) purely for
  its `[package]` `name`/`entry` (see Decision log — no `[dependencies]`
  table of a dependency is ever read); a `Git` spec is fetched via `git
  clone <url> <manifest_dir>/.emerald/deps/<name>-<sha-prefix>/` +
  `git checkout <rev>` (shelling out via `std::process::Command`, no
  VCS logic of Emerald's own), then the same dependency-manifest read.
  Either way, `<manifest_dir>/deps/<name>` is created (or replaced) as
  a symlink to the resolved root directory. A dependency whose path
  doesn't exist, or whose `git clone`/`checkout` exits non-zero,
  produces a descriptive `DepsError`, never a panic.

### 2. Acceptance Criteria
1. This plan's own worked scenario — `app/emerald.toml` declaring
   `mathutils = { path = "../mathutils" }` — resolved via
   `resolve_dependencies`, produces `app/deps/mathutils` as a real
   symlink whose target, followed, contains `lib.em`, and returns a
   `ResolvedDependency` naming `mathutils`'s canonicalized absolute
   path.
2. A git dependency resolved against a **local filesystem git
   repository fixture** (`git init`-ed and committed to under a test's
   own temp directory — no outbound network access exercised or
   required) is cloned into `.emerald/deps/<name>-<sha-prefix>/`,
   checked out at the requested `rev`, and gets the same `deps/<name>`
   symlink treatment as a path dependency — real proof git resolution
   and path resolution converge on one identical require-facing shape.
3. A path dependency naming a directory that doesn't exist returns
   `Err(DepsError::MissingPath(_))` naming the missing path, not a
   panic and not a silently-empty `deps/<name>`.
4. A git dependency whose clone fails (an invalid/unreachable local
   fixture path used as the "remote") returns `Err(DepsError::Git(_))`
   carrying `git`'s own exit status, not a re-interpreted message that
   hides what `git` actually reported.
5. Regression: a package with an empty `[dependencies]` table resolves
   to an empty `Vec` and creates no `deps/` directory at all — a
   dependency-free build's on-disk footprint is unchanged from today.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/src/deps.rs`,
  `crates/emerald-cli/tests/deps.rs`,
  `examples/packages/mathutils/emerald.toml`,
  `examples/packages/mathutils/lib.em`,
  `examples/packages/app/emerald.toml`,
  `examples/packages/app/main.em`
- **Modify:** `crates/emerald-cli/src/main.rs` (adds `mod deps;`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-cli deps` | all pass, incl. real local-fixture git clone+checkout, missing-path and failed-clone negative cases | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-lockfile

### 1. Context
- Why: `resolve_dependencies` (leaf 2) re-resolves — re-canonicalizing
  paths, re-cloning git repos — on every single build with nothing
  recorded, and a loose `rev` (a branch/tag) has no pinned meaning
  across two different checkouts of the same manifest.
- Target state: `crates/emerald-cli/src/lockfile.rs` defines
  `Lockfile { version: u32, packages: Vec<LockedPackage> }`,
  `LockedPackage { name: String, source: LockedSource }` where
  `LockedSource` is `Path { path: String }` or `Git { git: String, rev:
  String, resolved_rev: String }` (`resolved_rev` is always the real
  resolved commit SHA — see Decision log). `Lockfile::load`/`::save`
  read/write `<manifest_dir>/emerald.lock` as TOML. A new
  `resolve_or_reuse(manifest_dir, manifest, existing_lock)` wraps leaf
  2's `resolve_dependencies`: for each dependency whose manifest spec
  exactly matches the lockfile's recorded source, its `deps/<name>`
  symlink is refreshed from the *lockfile's* recorded path/`resolved_rev`
  without re-invoking `git`/re-canonicalizing; a changed or newly-added
  spec is fully re-resolved and the lockfile entry is overwritten.

### 2. Acceptance Criteria
1. A first `emerald build` with no `emerald.lock` present creates one,
   recording `mathutils`'s exact canonicalized path (path dependency)
   or exact resolved commit SHA (git dependency, even when the
   manifest's `rev` was a branch name).
2. A second `emerald build` with an unchanged manifest and an existing,
   matching `emerald.lock` does not invoke `git` again for a git
   dependency (asserted via a fixture whose remote is deliberately made
   unreachable *after* the first successful lock — a second build must
   still succeed, proving it truly skipped re-resolution rather than
   silently re-fetching and getting lucky).
3. Editing a dependency's `path`/`git`/`rev` in `emerald.toml` while an
   old `emerald.lock` entry for that name still exists triggers
   re-resolution for that dependency (and only that one) on the next
   build, and the lockfile is rewritten with the new resolution.
4. `emerald update` is not implemented by this leaf — a test asserting
   `emerald update` exits with a clear "not yet supported" message (not
   a panic, not silently doing nothing) documents the deferral from the
   Decision log as a real, checked behavior rather than an unstated gap.
5. Regression: `leaf-dependency-resolution`'s own tests, run again with
   this leaf's `resolve_or_reuse` substituted for a direct
   `resolve_dependencies` call, still pass unchanged.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/src/lockfile.rs`,
  `crates/emerald-cli/tests/lockfile.rs`
- **Modify:** `crates/emerald-cli/src/deps.rs` (adds
  `resolve_or_reuse`), `crates/emerald-cli/src/main.rs` (adds `mod
  lockfile;`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-cli lockfile` | all pass, incl. skip-on-unreachable-remote and spec-change re-resolution cases | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-cli-subcommands

### 1. Context
- Why: `crates/emerald-cli/src/main.rs`'s actual `fn main` (verified
  this session) recognizes no subcommands at all — `args.get(1)` is
  read only as a source-file path, with an optional trailing `-o
  <output>` pair; there is no `emerald new`/`build`/`run` today.
- Target state: `main` dispatches on `args.get(1).map(String::as_str)`:
  `Some("new")` scaffolds `args[2]/{emerald.toml, main.em}` (a real
  `puts "Hello from <name>!"` starter, package `entry = "main.em"`);
  `Some("build")` loads `./emerald.toml` (erroring clearly if absent —
  `build`/`run` are manifest-only, never a fallback to legacy mode),
  runs `resolve_or_reuse` (leaf 3), compiles the package's `entry` file
  (with `require`s now resolving through `deps/<name>` per leaf 2)
  through the existing parse/check/codegen/link pipeline unchanged, and
  writes the linked binary named after `[package].name` into the
  manifest directory; `Some("run")` does exactly what `build` does, then
  executes the resulting binary and streams its stdout/stderr/exit code
  through unchanged. Any other `args[1]` (or none of the three
  keywords) falls through to today's exact legacy single-file path —
  see Decision log.

### 2. Acceptance Criteria
1. This plan's own worked scenario, run for real: `cd app && emerald
   build` prints resolution and compile progress lines (see this plan's
   worked-scenario transcript) and produces a real, executable `./app`
   binary; running `./app` directly prints `8`.
2. `emerald run` in the same directory, with `emerald.lock` and
   `deps/mathutils` already present and unchanged, recompiles (per
   leaf 4's Decision log: no per-file artifact cache exists) but does
   **not** re-invoke `git`/re-canonicalize (per `leaf-lockfile`), and
   prints `8` directly to the terminal.
3. `emerald build` run in a directory with no `emerald.toml` at all
   fails with a clear, descriptive error (not a panic, not silently
   falling back to legacy single-file mode) — proves `build`/`run` are
   genuinely manifest-only, not an ambiguous overload of the legacy
   path.
4. `emerald new mathutils2` (bare, no existing directory) creates
   `mathutils2/emerald.toml` and `mathutils2/main.em`; a subsequent `cd
   mathutils2 && emerald run` compiles and runs the generated starter
   program with no manual edits required.
5. Regression: `emerald examples/hello.em` (the legacy bare-file
   invocation, unrelated to any manifest) still compiles, links, and
   runs exactly as it does before this leaf, printing `42` — proves the
   subcommand dispatch is additive, not a breaking change to the
   existing single-file entry point every prior plan's example relies
   on.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs` (subcommand dispatch),
  `crates/emerald-cli/Cargo.toml` (adds `[[bin]] name = "emerald"`)
- **Create:** `crates/emerald-cli/tests/subcommands.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean, produces a binary named `emerald` | agent-claimed-locally |
| Test (real end-to-end run) | `cargo test -p emerald-cli subcommands` | all pass, incl. real linked-and-run worked scenario printing `8`, manifest-missing error, and `emerald new` round-trip | agent-claimed-locally |
| Regression (legacy path) | `cargo test --workspace` | all pass, incl. existing `hello.em`-style single-file invocation still printing `42` | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
