---
name: id_effect Integration
overview: "Orchestrate emerald-cli's parse -> check -> codegen -> link pipeline as a chain of id_effect::Effect values, replacing repeated match/exit boilerplate — inception §15's targeted use case, not a blanket rewrite."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-cli-effect-pipeline
    content: "CliError unifying the 4 stages' distinct error types; each stage as an Effect<_, CliError, ()>; sequenced via flat_map; executed via run_blocking"
    status: pending
isProject: false
---

# Plan 14 — id_effect Integration

This is `id-effect-integration`, row `14` of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §15: "the project owner has written `id_effect`...
use it where it genuinely improves compiler architecture, particularly
for: compiler pipeline orchestration [...] Do not force every compiler
function through `Effect`. Use it where its abstraction makes the
architecture clearer." `id_effect` 0.4.0 (confirmed live on crates.io/
docs.rs this session — not assumed from memory) is a genuinely large,
Effect.ts-scale crate (17 "strata": algebra, capability DI, collections,
compute fabric, concurrency, coordination, resource management,
scheduling, streaming, STM, observability, testing...). This plan uses
exactly one corner of it — the core `kernel::Effect<A, E, R>` type,
`.flat_map`, and `runtime::run_blocking` — for the one use case inception
explicitly names and `emerald-cli` genuinely has: **pipeline
orchestration**. No worked example exists to reuse; the concrete proof is
`emerald-cli`'s own existing, already-passing integration tests (in
`crates/emerald-cli/tests/hello_em.rs`) continuing to pass unmodified
after the pipeline is rewired through `Effect`/`flat_map`/`run_blocking`
— proof that the orchestration genuinely changed, not just cosmetic.

## Decision log

- **Scope: `emerald-cli`'s top-level orchestration only.** `parse`,
  `check_program`, `compile_to_object`, and the `cc` link step each stay
  exactly what they already are (plain `Result`-returning functions) —
  `id_effect` wraps *calls* to them, it doesn't reach inside
  `emerald-parser`/`emerald-sema`/`emerald-codegen` at all. This is the
  literal reading of inception's own caution: "do not force every
  compiler function through `Effect`" — sema's own multi-diagnostic
  accumulation, codegen's internal `Result` threading, etc. are already
  clear as plain Rust and gain nothing from `Effect`.
- **Environment `R = ()` — no capability/dependency injection.** `id_effect`'s
  `capability` module (`Env`, `ProviderSpec`, `CapabilityGraph`,
  `run_with`) is a full DI system for swapping dependencies (e.g. a
  mockable toolchain/filesystem capability) — genuinely useful *if*
  `emerald-cli` needed to run against injected test doubles, but its
  existing tests already exercise the real `cc`/filesystem directly and
  pass reliably (see `tests/hello_em.rs`). Reaching for capability DI
  here would be exactly the "force it through the abstraction because
  it's available" inception warns against. Deferred until a concrete
  need (e.g. wanting to unit-test the pipeline without invoking a real
  `cc`) makes it worth the machinery.
- **Synchronous only (`run_blocking`), no async, no fibers/concurrency,
  no streaming, no STM.** `emerald-cli` compiles one file, sequentially,
  once per invocation — none of `id_effect`'s async/concurrent/streaming
  strata apply. `Effect::new` (sync closure) + `.flat_map` + `run_blocking`
  is the complete, correct-sized subset.
- **A new `CliError` enum unifies the 4 stages' otherwise-distinct error
  types** (`emerald_parser::ParseError`, `Vec<emerald_sema::Diagnostic>`,
  and two flavors of `String` for codegen/link) — required because
  `Effect<A, E, R>::flat_map` needs one consistent `E` across the whole
  chain. This is genuine, necessary plumbing for the refactor, not
  incidental complexity `id_effect` introduced.

## Leaf: leaf-cli-effect-pipeline

### 1. Context
- Why: `emerald-cli`'s `main` currently threads the pipeline through four
  repeated `match { Ok(x) => x, Err(e) => { eprintln!(...); process::exit(1) } }`
  blocks — imperative, but the *sequencing* (each stage only runs if the
  previous succeeded) is implicit in the control flow rather than
  expressed as a first-class value.
- Target state: four small `fn stage(...) -> Effect<T, CliError, ()>`
  constructors (parse, check, codegen, link), composed via
  `.flat_map(...).flat_map(...).flat_map(...)` into one pipeline
  `Effect`, executed once via `id_effect::run_blocking(pipeline, ())`.
  `main` pattern-matches the single resulting `Result<(), CliError>` to
  decide the exit code/rendering per variant (parse errors still render
  via `miette`, per plan 13 — unchanged).

### 2. Acceptance Criteria
1. `crates/emerald-cli/tests/hello_em.rs`'s three existing tests —
   successful compile-and-run, sema-rejection-with-diagnostic, and
   plan 13's miette-rendered parse error — all still pass unmodified,
   proving the `Effect`-orchestrated pipeline is behaviorally identical
   to the original, not just refactored code that happens to compile.
2. The pipeline genuinely short-circuits: a program that fails to parse
   never reaches `check_program`/`compile_to_object`/`cc` (verified by
   the existing "must not produce an executable for a rejected program"
   assertions already in the test file).
3. `CliError`'s four variants each carry the *original* stage error
   value (no information lost converting into the unified type) — the
   existing stderr-content assertions (e.g. "stderr should name both
   types") continue to pass without loosening them.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs`,
  `crates/emerald-cli/Cargo.toml` (adds `id_effect`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-cli` | all pass (3/3 existing tests, unmodified) | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `id_effect::capability` (DI/`Env`/`ProviderSpec`) — see Decision log;
  no concrete need for injectable dependencies yet.
- Async/streaming/STM/concurrency/scheduling/observability strata — none
  apply to a single-shot synchronous CLI invocation.
- Using `id_effect` inside `emerald-sema`/`emerald-codegen` themselves
  (e.g. diagnostics accumulation) — those are already clear as plain
  Rust; see Decision log's literal reading of inception §15's caution.
