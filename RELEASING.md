# Releasing Emerald

This documents the actual, currently-real state of Emerald's release
tooling (plan 27) — every claim here is cross-checked against what
exists in this checkout right now, not what's aspirational.

## What exists today

- **`emerald-cli` is a real, portable binary.** Its build script,
  `crates/emerald-driver/build.rs` (moved here from `emerald-cli` in a
  later refactor — if you see a reference to `emerald-cli/build.rs`
  elsewhere, it's stale), compiles `runtime/emerald_runtime.c` into a
  static archive at build time and embeds the archive's bytes directly
  into the compiled binary (`include_bytes!`). A copy of `emerald-cli`,
  moved to a machine or directory with no access to this repository at
  all, can still compile, link, and run a `.em` program — proven by a
  real, executed test: `cargo test -p emerald-cli --test portability`.
- **`emerald-lsp`** (the language server, binary name `emerald-lsp`)
  and **`emerald-mcp`** (an MCP server exposing the compiler, binary
  name `emerald-mcp`) both exist as real, working crates today —
  `crates/emerald-lsp/` and `crates/emerald-mcp/`.
- **`editors/vscode/`** is a real, working VS Code extension
  (`emerald-lang`, currently version `0.2.0`) built with `esbuild` and
  `vscode-languageclient`, driving `emerald-lsp` over stdio. See
  `editors/vscode/README.md` for its own manual build/dev-loop
  instructions.
- **`[workspace.metadata.dist]`** in the root `Cargo.toml` configures
  `cargo-dist` 0.32.0 to build all three binaries — `emerald`
  (`emerald-cli`'s `[[bin]]` name), `emerald-lsp`, and `emerald-mcp` —
  for `x86_64-unknown-linux-gnu` only. cargo-dist auto-discovers every
  workspace member that defines a `[[bin]]`; none of these three crates
  needed an explicit `[package.metadata.dist] dist = true` opt-in (that
  key exists in cargo-dist for *excluding* a binary crate, via
  `dist = false`, not for opting one in), and the pure-library crates
  (`emerald-lexer`, `emerald-parser`, `emerald-codegen`, `emerald-sema`,
  `emerald-driver`, `emerald-fmt`, `emerald-rt`) were correctly left out
  automatically since none of them ships a binary.
- **`.github/workflows/release.yml` has been generated for real.**
  `cargo-dist` 0.32.0 (matching the pinned `cargo-dist-version`) was
  installed in the sandbox that authored this document via the nixpkgs
  `cargo-dist` derivation (`nix shell nixpkgs#cargo-dist`, which
  resolves to the exact 0.32.0 build), and its actual `dist init -y
  --ci=github --installer=shell -t x86_64-unknown-linux-gnu` /
  `dist generate` commands were run against this repository. The only
  hand-made changes were adding a `repository` URL (cargo-dist's GitHub
  CI support requires one) to `[workspace.package]` and inheriting it
  into each of the three dist-packaged crates via
  `repository.workspace = true` — `dist generate` refused to proceed
  without it and named the exact three `Cargo.toml` files that needed
  it. Everything else in `release.yml`, and the `[profile.dist]` /
  `[workspace.metadata.dist]` blocks in the root `Cargo.toml`, is
  verbatim tool output. Re-run `dist generate` (same version) after any
  future change to `[workspace.metadata.dist]` rather than hand-editing
  `release.yml` — the file's own header says as much.
- **`.github/workflows/vscode-publish.yml`** packages and publishes
  `editors/vscode/` to the VS Code Marketplace (and optionally Open
  VSX) on the same tag-push trigger `release.yml` uses. Unlike
  `release.yml`, this file is **hand-authored**, not `dist`-generated —
  cargo-dist has no concept of a VS Code extension. It has been
  verified locally: `npm ci`, `npm run build`, and `npm run package`
  (a real `vsce package` run producing a working, uninstalled `.vsix`)
  all succeed as of this commit. The actual `vsce publish` /
  `ovsx publish` steps have never been run — see "What a maintainer
  still needs to do" below.
- **`editors/vscode/package.json`** gained a `package` script
  (`vsce package`) and a `publish` script (`vsce publish`), plus
  `@vscode/vsce` as a devDependency, alongside the pre-existing `build`
  / `vscode:prepublish` scripts.

## What does not exist yet / what a maintainer still needs to do

- **No tagged release has ever been cut.** There is no `v0.1.0` (or
  any other) GitHub Release. Nothing in this plan's work pushes a tag,
  creates a release, or publishes anything — that is deliberately left
  for a human maintainer to trigger.
- **`VSCE_PAT` does not exist as a repository secret.** Add a VS Code
  Marketplace Personal Access Token (Azure DevOps org backing the
  `emerald-lang` publisher, "Marketplace > Manage" scope — see
  https://code.visualstudio.com/api/working-with-extensions/publishing-extension#get-a-personal-access-token)
  under repo Settings > Secrets and variables > Actions before
  `vscode-publish.yml`'s Marketplace step can do anything but fail
  fast with an explicit `::error::`.
- **`OVSX_PAT` does not exist either**, and is optional — its absence
  causes `vscode-publish.yml` to skip the Open VSX step with a
  `::notice::` rather than failing the workflow.
- **No macOS or Windows binaries**, and no code-signing/notarization
  for either. Nothing in this sandbox can build or verify a macOS or
  Windows toolchain's output (this project's codegen depends on LLVM,
  and `emerald-driver`'s build script compiles and embeds a C runtime
  archive — both are meaningfully more work to get cross-compiling
  correctly than to declare untested), so only `x86_64-unknown-linux-
  gnu` is a real, committed release target in
  `[workspace.metadata.dist]`. Adding those platforms is real,
  disclosed future work — verify each one actually builds before
  adding its target triple, don't add it speculatively.
- **The Cargo workspace version and the VS Code extension version are
  independent.** `[workspace.package] version` is `0.1.0`;
  `editors/vscode/package.json`'s `version` is `0.2.0`. Pushing a tag
  like `v0.1.1` fires both `release.yml` and `vscode-publish.yml`, but
  `vscode-publish.yml` always publishes whatever `package.json`
  currently says, regardless of the tag's own numbers. Keep that in
  mind before tagging, or bump `package.json` deliberately first if you
  want the two aligned for a given release.
- **No Homebrew formula, no Nix flake package output.** This repo's
  Nix files (`devenv.nix`, `devenv.yaml`, `.envrc`) are a development
  shell, not a package definition. (The README's Nix install
  instructions — `nix run` / `nix build` via `flake.nix` — package
  `emerald-cli`/`emerald-lsp` for local Nix users directly from source;
  that is a separate, already-working path, complementary to a
  GitHub-Releases pipeline for non-Nix users, not a substitute for it.)

## Cutting an actual release (once the secrets above exist)

```sh
# Bump crate versions / editors/vscode/package.json's version as needed first.
git tag v0.1.1
git push origin v0.1.1
```

Pushing that tag triggers both `.github/workflows/release.yml`
(builds `emerald`, `emerald-lsp`, `emerald-mcp` for
`x86_64-unknown-linux-gnu`, uploads a GitHub Release with shell
installers) and `.github/workflows/vscode-publish.yml` (packages and
publishes the current `editors/vscode/package.json` version to the
Marketplace, and to Open VSX if `OVSX_PAT` is set).

## The permanent `cc` dependency

Bundling the runtime archive removes the *dev-time-only* dependency on
this repository's source tree. It does **not** remove `emerald-cli`'s
dependency on a `cc`-compatible C compiler/linker being present on the
machine that runs it: linking a user's compiled `.em` program into a
final executable is a normal part of every `emerald-cli` invocation,
not a one-time install step. Anyone running `emerald-cli` needs `cc` on
their `PATH`, the same way `rustc` needs a linker.

## Installing today

Until a real tagged release exists, the only way to get `emerald-cli`,
`emerald-lsp`, or `emerald-mcp` from this repo directly is building
from source:

```sh
cargo build --release -p emerald-cli -p emerald-lsp -p emerald-mcp
# binaries at target/release/{emerald,emerald-lsp,emerald-mcp}
```

`emerald-cli`'s resulting binary is portable (see above) — it can be
copied anywhere a `cc`-compatible compiler is available. Nix users can
also use `nix run`/`nix build` per the README's Install section.
