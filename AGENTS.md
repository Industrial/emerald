# Project Conventions

Repo-level conventions for agents working in this codebase. The harness pointer surface
lives in `.maestro/AGENTS.md`; this file holds code conventions, build commands, and
feature boundaries.

## Build / test / verify

```bash
# build:   cargo build --workspace
# test:    cargo nextest run --workspace   (falls back to `cargo test --workspace`)
# lint:    cargo clippy --workspace --all-targets
# format:  treefmt
```

## Layout

- `crates/` — Rust workspace crates (compiler pipeline: lexer, parser, sema, types, ir, codegen, driver, cli)
- `spec/` — language specification (`GRAMMAR.md`, `SEMANTICS.md`, `TYPE_SYSTEM.md`, `RUNTIME.md`, `COMPILER.md`)
- `history/` — timestamped project-brief and plan-of-plans records
- `tests/` — automated tests
- `.maestro/` — harness state (read `.maestro/AGENTS.md` first)

## Conventions

- Match existing code style; use established libraries before adding new ones.
- Surgical edits only — touch what the task requires.
- Bump the relevant version when behavior changes.

## See also

- `.maestro/MAESTRO.md` — read order, lane policy, daily commands
- `.maestro/docs/HARNESS.md` — product-delta vs harness-delta model
- `.maestro/docs/FEATURE_INTAKE.md` — work-type classification decision tree
- `.maestro/docs/VALIDATION_LADDER.md` — 7-step verification protocol

<!-- maestro-setup:start -->
## Maestro

This project is wired into the Maestro harness. State and config live
under `.maestro/`. Run `./init.sh` to bring a fresh checkout up; run
`maestro doctor` and `maestro status` to see what Maestro knows.

Preserve content outside this managed block; the block is rewritten by
`maestro setup` and the `maestro-setup` skill, but everything else in
this file is yours.
<!-- maestro-setup:end -->
