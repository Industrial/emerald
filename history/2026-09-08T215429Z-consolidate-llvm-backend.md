# Consolidate on the LLVM backend

## Why

Plan 16's bake-off measured LLVM 5-18x faster than tuned Cranelift on
`sum`/`array_traversal`, matching or beating hand-written Rust/C. User
directive: "Remove everything but the best option and implement the best
option 100% now." Best option = LLVM. This plan removes Cranelift entirely
and brings the LLVM backend (previously scoped to the benchmark AST subset)
up to full parity with what Cranelift supported: functions, classes
(fields/methods/`new`/`@field`), lambdas/closures (top-level `Proc` `Let`s,
by-value capture, static `.call`), exceptions (`raise`/`begin`/`rescue` via
setjmp/longjmp), modules (namespace static dispatch), arrays (literal,
index read/write), control flow (`if`/`while`/`break`/`next`/`return`).

## Decision log

- **Delete `crates/emerald-codegen` (Cranelift), rename
  `crates/emerald-codegen-llvm` -> `crates/emerald-codegen`.** One
  canonical codegen crate, matching `AGENTS.md`'s documented layout.
  `emerald-cli` loses its `--backend` flag — there is only one backend
  again, so the flag is dead surface, not a feature to keep for its own
  sake.
- **Faithful port, not a redesign.** Every restriction Cranelift's backend
  had (lambdas only as top-level `Let`s, method/index receivers must be a
  plain local-variable `Ident`, single `rescue` clause, no inheritance) is
  preserved exactly — this plan replaces the backend, not the language.
  Proven by porting Cranelift's own test suite verbatim (same source
  strings, same expected outputs) into the new crate.
- **Real LLVM pointers, not Cranelift's "everything is i64" blur.**
  Class instances / array bases / lambda envs / exception instances are
  LLVM `ptr`; `Int64` is `i64`; `Float64` is `f64`. Byte layout is
  unchanged (still 8 bytes/slot, matching `runtime/emerald_runtime.c`'s
  `emerald_alloc`) — this is a type-system correctness improvement with
  zero behavior change for well-typed Emerald programs, not scope creep.
- **`opt_level`/`OptimizationLevel::Aggressive` (`default<O3>`) stays** —
  this crate inherits the tuning plan 16 already proved out.

## Acceptance

- `crates/emerald-codegen`'s test suite includes every Cranelift test
  (functions, classes/Point example, lambdas, exceptions, modules,
  arrays, break/if/while) ported verbatim, all passing.
- `cargo test --workspace` green.
- `benchmarks/REPORT.md` re-measured for real with the single backend.
- `spec/COMPILER.md` gets a final dated addendum recording the
  consolidation.
- No `--backend` flag, no Cranelift dependency, anywhere in the repo.
