# Plan 16 — codegen backend bake-off (Cranelift vs LLVM)

## Why

`spec/COMPILER.md`'s plan-02 decision record chose Cranelift for v1 with an
explicit, non-permanent revisit trigger: if `15 benchmarking`'s numbers show
Cranelift meaningfully behind hand-written/LLVM-generated code on loop
optimization/vectorization, re-open the decision with LLVM properly wired
into `devenv.nix` first.

`benchmarks/REPORT.md` (plan 15) shows exactly that: Emerald is 4.9x slower
than Rust and 9.3x slower than C on `sum`, and 18x slower than Rust and 5.3x
slower than C on `array_traversal`. Investigation (this session, no code
changed at the time) found the root cause: `host_isa()` in
`crates/emerald-codegen/src/lib.rs` never sets Cranelift's `opt_level`, which
defaults to `OptLevel::None` — no CSE, no redundant load/store elimination,
no loop-invariant code motion, weak regalloc. The revisit trigger is
satisfied. This plan (a) fixes the Cranelift tuning gap for real, (b) stands
up a genuine second backend on LLVM via `inkwell` (not a hand-written `.ll`
comparison file — user's explicit choice), (c) benchmarks both, tuned, and
(d) records the decision.

## Scope

**In scope**: a real, executed, apples-to-apples comparison on the existing
`sum`/`array_traversal` benchmarks, both backends tuned for speed, plus a
dated decision addendum.

**Out of scope, explicitly**: LLVM feature parity with the Cranelift
backend. `emerald-codegen` (Cranelift) supports the full language — classes,
lambdas, exceptions, modules, user functions. Rebuilding all of that on
LLVM in one plan is disproportionate to what the bake-off needs. The new
`emerald-codegen-llvm` crate supports exactly the AST subset the two
existing benchmark programs use:

- Top level only: `Item::Stmt` (no `Item::Function`/`Class`/`Module` — the
  benchmarks define none).
- `Stmt::{Let, While, If, Expr}`.
- `Expr::{Ident, Int, Float, Add, Compare, Call("puts", [x]), ArrayLit,
  Index}`.
- Types `Int64`, `Float64`, `Array[Int64]`, `Array[Float64]`.

Anything outside this subset returns `Err(...)`, never panics (matches
`emerald-codegen`'s existing `unsupported_top_level_shape_errors_not_panics`
pattern). If a later plan wants LLVM to cover functions/classes/etc., that's
its own plan, scoped and justified on its own merits — not silently expanded
here.

## Decision log

- **Wiring approach**: add LLVM 21 (`pkgs.llvmPackages_21.llvm`,
  `pkgs.libffi`, `pkgs.libxml2`) and `LLVM_SYS_211_PREFIX` to `devenv.nix`,
  and `inkwell` with feature `llvm21-1` as a real dependency compiling a real
  second backend — the user's explicit choice over a lighter-weight
  hand-written-`.ll`-plus-`clang` comparison. Verified via a throwaway smoke
  test (`.tmp/inkwell-smoke`, deleted after use, not committed): a JIT'd
  LLVM IR function returning 42 actually ran under `devenv shell`.
- **Why fix Cranelift's `opt_level` in the same plan, not a separate one**:
  benchmarking untuned Cranelift against tuned LLVM would be a rigged
  comparison, not a real bake-off. Both backends get to be fast before the
  numbers are compared.
- **Object emission, not JIT**: both backends must produce a real linked
  executable via the existing `cc -no-pie <obj> runtime/emerald_runtime.c -o
  <out>` path in `emerald-cli`'s `link_stage` — consistent with how
  `emerald-codegen`'s AOT path already works, and how the benchmark harness
  measures things (compiled binary run time, not JIT warmup time).
- **`emerald-codegen-llvm` mirrors `emerald-codegen`'s public entry point**
  exactly: `pub fn compile_to_object(program: &Program, out_path: &Path) ->
  Result<(), String>`. This lets `emerald-cli` pick a backend with a single
  `match`, no trait object, no dyn dispatch overhead in a compiler that
  isn't the hot path anyway.
- **CLI surface**: `emerald-cli` gains `--backend=cranelift|llvm` (default
  `cranelift` — changing the default is a decision for the addendum step
  below, made from the real numbers, not assumed up front).
- **LLVM optimization**: run LLVM's pass pipeline at `OptimizationLevel::
  Aggressive` (inkwell's `PassManager`/`PassBuilder`) before object emission
  — matching Cranelift's `opt_level=speed` fix, so neither backend is left
  at its unoptimized default.

## Leaves

1. `devenv.nix` — LLVM 21 + libffi + libxml2 + `LLVM_SYS_211_PREFIX`.
   **Done and verified** (this session): `devenv shell` resolves the env
   var and `llvm-config`; a throwaway `inkwell` smoke test built, linked,
   JIT-ran, and returned 42.
2. Fix `crates/emerald-codegen/src/lib.rs`'s `host_isa()`: set
   `opt_level=speed`. Re-run `cargo test --workspace` (particular attention
   to the setjmp/longjmp exception tests from plan 11 — opt_level shouldn't
   affect their correctness, but this needs real confirmation, not
   assumption).
3. New crate `crates/emerald-codegen-llvm`: `inkwell` (`llvm21-1`) backend
   covering the scoped AST subset above, producing a native object file.
4. `emerald-cli`: add `--backend` flag; `codegen_stage` dispatches to
   whichever crate's `compile_to_object`.
5. Extend `crates/emerald-cli/tests/benchmarks.rs` to compile+run+time each
   benchmark through both backends (`Emerald (Cranelift)` / `Emerald
   (LLVM)`), alongside the existing Rust/C/C++ rows. Regenerate
   `benchmarks/REPORT.md` for real.
6. `spec/COMPILER.md`: append a dated addendum under the plan-02 record —
   revisit-trigger resolution, the new numbers, and the final backend
   decision (which backend is the CLI default going forward, and why).
7. Mark this plan done in `history/`, commit.

## Acceptance

- `cargo test --workspace` green, including exception tests, after the
  `opt_level` change.
- `emerald-codegen-llvm` actually compiles+links+runs `sum.em` and
  `array_traversal.em` to the correct expected outputs
  (`49999995000000` / `210000000`) — same correctness bar plan 15 set.
- `benchmarks/REPORT.md` has real, freshly-measured rows for both backends,
  tuned, next to Rust/C/C++.
- `spec/COMPILER.md` addendum is dated and references real evidence (file
  paths, test names, the regenerated report).
