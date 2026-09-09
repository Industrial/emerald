# Releasing Emerald

This documents the actual, currently-real state of Emerald's release
tooling (plan 27) — every claim here is cross-checked against what
exists in this checkout right now, not what's aspirational.

## What exists today

- **`emerald-cli` is a real, portable binary.** Its build script
  (`crates/emerald-cli/build.rs`) compiles `runtime/emerald_runtime.c`
  into a static archive at build time and embeds the archive's bytes
  directly into the compiled binary (`include_bytes!`). A copy of
  `emerald-cli`, moved to a machine or directory with no access to this
  repository at all, can still compile, link, and run a `.em` program —
  proven by a real, executed test:
  `cargo test -p emerald-cli --test portability`.
- **`[workspace.metadata.dist]`** in the root `Cargo.toml` configures
  `cargo-dist` 0.32.0 to build `emerald-cli` for
  `x86_64-unknown-linux-gnu`.

## What does not exist yet

- **`.github/workflows/release.yml` has not been generated.**
  `cargo-dist` is not installed in the environment this plan was
  executed in (`cargo dist --version` fails: no such subcommand). The
  config above is real and ready, but nobody has actually run
  `cargo dist init` / `cargo dist generate` against it yet. Do that
  first, on a machine with `cargo-dist` installed:

  ```sh
  cargo install cargo-dist@0.32.0
  cargo dist init
  cargo dist generate
  ```

  Review the generated `.github/workflows/release.yml` before
  committing it — this document does not claim to have seen its actual
  contents.
- **No tagged release has ever been cut.** There is no `v0.1.0` GitHub
  Release, because there is no release workflow yet to produce one.
- **`emerald-lsp` and `emerald-mcp` don't exist.** Plan 17 (IDE
  integration) is what introduces those crates. Once it lands, add them
  to `[workspace.metadata.dist]` alongside `emerald-cli` — the config
  shape doesn't need to change, just the binary list.
- **No VSCode extension publish workflow.** That also depends on plan
  17's `editors/vscode/` extension source existing first, and on `npm`
  being available (it is not, in this sandbox) to actually exercise
  `npm install`/`vsce package`. Not attempted here.
- **No macOS or Windows binaries**, and no code-signing/notarization
  for either. Nothing in this sandbox can build or verify a macOS or
  Windows toolchain's output, so only `x86_64-unknown-linux-gnu` is a
  real, committed release target. Adding those platforms is real,
  disclosed future work — not silently implied by this document's
  title.
- **No Homebrew formula, no Nix flake package output.** This repo's
  Nix files (`devenv.nix`, `devenv.yaml`, `.envrc`) are a development
  shell, not a package definition.

## The permanent `cc` dependency

Bundling the runtime archive removes the *dev-time-only* dependency on
this repository's source tree. It does **not** remove `emerald-cli`'s
dependency on a `cc`-compatible C compiler/linker being present on the
machine that runs it: linking a user's compiled `.em` program into a
final executable is a normal part of every `emerald-cli` invocation,
not a one-time install step. Anyone running `emerald-cli` needs `cc` on
their `PATH`, the same way `rustc` needs a linker.

## Installing today

Until a real tagged release exists, the only way to get `emerald-cli`
is building it from source inside this repository:

```sh
cargo build --release -p emerald-cli
# binary at target/release/emerald-cli
```

The resulting binary is portable (see above) — it can be copied
anywhere a `cc`-compatible compiler is available.
