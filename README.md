# Project template

Rust + Bun/TypeScript workspace template with Nix (devenv), moon tasks, treefmt, cargo-deny, prek git hooks, and optional Cachix.

---

## Clone (with submodules)

This repo uses git submodules under `.cursor/` (`agency-agents`, `microsoft-rust-training`). Clone with submodules in one go:

```bash
git clone --recurse-submodules <repo-url>
cd <repo>
```

If you already cloned without submodules, init and update them:

```bash
git submodule update --init --recursive
```

---

## Overview

- **Runtimes:** Rust (stable), JavaScript/TypeScript (Bun).
- **Environment:** Nix via [devenv](https://devenv.sh); reproducible toolchain and scripts.
- **Tasks:** [moon](https://moonrepo.dev) for format, check, lint, build, test, docs, audit, coverage, etc.
- **Git hooks:** [prek](https://github.com/j178/prek) (pre-commit–compatible): pre-push runs the full moon pipeline; commit-msg enforces conventional commits (commitizen).
- **Formatting:** [treefmt](https://github.com/numtide/treefmt) over Rust, Nix, shell, JS/TS, YAML, TOML.
- **Rust quality:** cargo-deny (advisories/licenses), cargo-audit, clippy, nextest, optional llvm-cov.

---

## Tooling

| Tool | Purpose | Config |
|------|--------|--------|
| **devenv** | Nix dev shell: languages (Rust, Bun, TS), packages, env vars, scripts. Enter with `devenv shell`. | `devenv.nix`, `devenv.yaml` |
| **moon** | Task runner: format, check, lint, build, test, docs, fix, bench, audit, coverage, ci-format. | `moon.yml` |
| **prek** | Git hooks (no Python). Installs from `.pre-commit-config.yaml` on `devenv shell`. | `.pre-commit-config.yaml` |
| **treefmt** | Single CLI to format all supported files (only tracked files by default). | `treefmt.toml`, `treefmt.ci.toml` |
| **rustfmt** | Rust formatter (invoked by treefmt). | `rustfmt.toml` |
| **cargo-deny** | Rust: advisories, licenses, duplicate deps. Run manually or in CI. | `deny.toml` |
| **cargo-audit** | Rust security advisories. Used in moon `:audit` and pre-push. | — |
| **cargo-nextest** | Fast Rust test runner. Used in moon `:test` and `:coverage`. | `nextest.toml` |
| **sccache** | Shared compilation cache for Rust (and C/C++) across builds. | env `RUSTC_WRAPPER=sccache` |
| **mold** | Fast linker for Rust on Linux. | — |
| **commitizen** | Validates commit messages (conventional commits). Prek hook on commit-msg. | — |
| **Cachix** | Optional Nix binary cache; pull/push configured in `devenv.nix`. | `cachix.pull` / `cachix.push` |

### Treefmt formatters (by file type)

- **Nix:** deadnix, alejandra  
- **GitHub Actions:** actionlint  
- **Bash:** beautysh  
- **JS/TS/JSON:** biome  
- **YAML:** yamlfmt  
- **TOML:** taplo  
- **Rust:** rustfmt (edition 2024)

---

## Cachix (with devenv and CI)

[Cachix](https://cachix.org) is a Nix binary cache. It stores build results (e.g. devenv shell closure, Nix packages) so you and CI can **pull** instead of rebuilding. Optionally, CI can **push** new build results to the cache so the next run is fast.

### In devenv.nix

In `devenv.nix` the cache is configured as:

```nix
cachix = {
  pull = ["project-template"];
  push = "project-template";
};
```

- **pull** — When you run `devenv shell` or any Nix command, Nix will try to fetch store paths from the `project-template` cache (if you’ve trusted it; see below). No account needed for pull.
- **push** — When a build runs with push enabled and Cachix auth, new store paths can be uploaded to the `project-template` cache. Pushing is typically done only in CI (e.g. on the main branch).

Replace `project-template` with your own cache name if you create one at [cachix.org](https://cachix.org).

### Local use (pull only)

1. **Trust the cache** so Nix uses it when building the dev shell:

   ```bash
   cachix use project-template
   ```

   (Use your cache name if different.) This adds the cache to your Nix config and lets `devenv shell` pull binaries instead of building everything.

2. **Enter the shell as usual:** `devenv shell`. If the closure is already on the cache, it will download instead of build.

You do **not** need a Cachix account or auth for pulling. For pushing (e.g. from CI) you need a Cachix auth token.

### CI pipeline

In GitHub Actions (or similar) you want to:

1. **Use the cache on every run** (pull) so CI benefits from previous builds.
2. **Push to the cache only on selected events** (e.g. push to `main`), so the cache stays up to date and you avoid push failures on PRs.

Example pattern:

1. **Install Nix** (e.g. `DeterminateSystems/nix-installer-action`).
2. **Configure Cachix** with [cachix/cachix-action](https://github.com/cachix/cachix-action):
   - `name`: your cache name (e.g. `project-template`).
   - **Pull:** always enabled when `name` is set.
   - **Push:** only when you want to update the cache. Pass the auth token only then and set `skipPush: true` otherwise, so the action doesn’t try to start the push daemon on PRs.

   Example (conceptual):

   ```yaml
   env:
     CACHIX_AUTH_TOKEN: ${{ secrets.CACHIX_AUTH_TOKEN }}
   steps:
     - uses: cachix/cachix-action@v16
       with:
         name: project-template
         authToken: ${{ github.ref == 'refs/heads/main' && secrets.CACHIX_AUTH_TOKEN }}
         skipPush: ${{ github.ref != 'refs/heads/main' }}
   ```

   So: on `main`, push if the token is set; on PRs or other branches, only pull.

3. **Install devenv** and run your pipeline (e.g. `devenv shell -- moon run :ci-format` and the rest of your checks).

Add the Cachix auth token as a repository secret (`CACHIX_AUTH_TOKEN`) if you want CI to push to your cache. If you only want to pull (e.g. from a public cache), you can omit the token and always use `skipPush: true`.

---

## Lifecycle: what happens when

### 1. Clone the repo

```bash
git clone --recurse-submodules <repo-url>
cd <repo>
```

- `.cursor/agency-agents` and `.cursor/microsoft-rust-training` are **git submodules**; `--recurse-submodules` pulls them. Without it, run `git submodule update --init --recursive` later.

### 2. Enter the dev shell (first time and daily)

```bash
devenv shell
```

**On enter, devenv runs:**

1. **prek-install** — Installs git hooks from `.pre-commit-config.yaml` (pre-push, commit-msg) into `.git/hooks`. Overwrites existing hook files so the repo’s config is always in use.
2. **moon-sync** — Runs `moon sync` so moon’s toolchain and project graph are up to date.
3. **sccache** — Ensures `$HOME/.cache/sccache` exists so the Rust compiler cache can be used.

**You get:**

- Rust (stable), clippy, rustfmt, rust-analyzer, cargo-nextest, cargo-llvm-cov, cargo-audit  
- Bun + TypeScript  
- moon, treefmt, and all formatters (alejandra, beautysh, biome, taplo, yamlfmt, etc.)  
- prek, git, gh, direnv  
- Env: `RUST_BACKTRACE=1`, `CARGO_TERM_COLOR=always`, `RUSTC_WRAPPER=sccache`, `MOON_TOOLCHAIN_FORCE_GLOBALS=rust`  
- Scripts: `prek-install`, `moon-sync`, `pre-push` (used by the pre-push hook)

Optional: if you use [direnv](https://direnv.net), `direnv allow` in the repo will enter the devenv shell automatically when you `cd` in.

### 3. Develop

- **Format:** `moon run :format` (writes changes) or `moon run :ci-format` (CI-style; fails if anything would change).
- **Check / lint / build / test:**  
  `moon run :check`, `:lint`, `:build`, `:test`  
  Or run the full gate:  
  `devenv shell -- pre-push` (same as the pre-push hook).
- **Fix auto-fixable issues:** `moon run :fix`
- **Docs:** `moon run :docs` or `:check-docs`
- **Security:** `moon run :audit`; run `cargo deny check` manually for full deny checks.
- **Coverage:** `moon run :coverage` (nextest under llvm-cov).

All of these use the tools and configs from the dev shell (rustfmt, nextest.toml, etc.).

### 4. Commit

- When you run `git commit`, the **commit-msg** hook (prek + commitizen) runs.
- It validates that the commit message follows conventional commits (e.g. `feat: add X`, `fix: Y`). If not, the commit is rejected.

### 5. Push

- When you run `git push`, the **pre-push** hook runs.
- The hook runs: `devenv shell -- pre-push`
- **pre-push** (defined in `devenv.nix` scripts) runs:  
  `moon run :format :check :lint :build :test :audit :check-docs`
- If any of these fail, the push is aborted. So the branch you push has already been formatted, checked, linted, built, tested, audited, and doc-checked in the same way as in CI.

### 6. CI (when you add it)

- In GitHub Actions (or similar), use the same commands inside a Nix/devenv setup so CI matches local and pre-push.
- **Format check:** `devenv shell -- moon run :ci-format`  
  Uses `treefmt.ci.toml` (same as treefmt but `fail-on-change = true`).
- **Full pipeline:** `devenv shell -- moon run :format :check :lint :build :test :audit :check-docs`  
  Or use `devenv shell -- pre-push` to mirror the pre-push hook exactly.

---

## Moon tasks quick reference

| Task | What it does |
|------|----------------|
| `:format` | treefmt (format tracked files) |
| `:ci-format` | treefmt with `treefmt.ci.toml` (fail if not formatted) |
| `:check` | cargo check --workspace --all-features |
| `:lint` | cargo fmt --check + cargo clippy |
| `:build` | cargo build |
| `:test` | cargo nextest run |
| `:docs` | cargo doc --no-deps --all-features |
| `:fix` | cargo fix + clippy --fix |
| `:bench` | cargo bench (no-op if no `benches/`) |
| `:audit` | cargo audit |
| `:coverage` | cargo llvm-cov nextest |
| `:check-docs` | cargo doc + clippy with missing_docs lint |

Run with: `moon run :<task>` or `devenv shell -- moon run :<task>` from outside the shell.

---

## Config files

- **devenv.nix** — Dev shell: packages, env, scripts (prek-install, moon-sync, pre-push).
- **devenv.yaml** — Nix flake inputs (fenix, nixpkgs, rust-overlay, etc.).
- **moon.yml** — Moon project and task definitions.
- **treefmt.toml** — Formatters and globs (local format; `walk = "git"`).
- **treefmt.ci.toml** — Same as above with `fail-on-change = true` for CI.
- **rustfmt.toml** — Rust format options (edition 2024, 2 spaces).
- **deny.toml** — cargo-deny: advisories, licenses, bans.
- **nextest.toml** — nextest profiles (default + ci), timeouts, cache.
- **.pre-commit-config.yaml** — prek: pre-push (devenv pre-push script), commit-msg (commitizen).

---

## Summary flow

1. **Clone** (with submodules) → **devenv shell** → prek installs hooks, moon syncs, sccache ready.  
2. **Develop** with moon tasks (`:format`, `:check`, `:lint`, `:build`, `:test`, etc.).  
3. **Commit** → commitizen checks message.  
4. **Push** → pre-push runs full moon pipeline; push only if it passes.  
5. **CI** (when added) runs the same commands (e.g. `:ci-format` and the same full pipeline) inside devenv so results match local and pre-push.
