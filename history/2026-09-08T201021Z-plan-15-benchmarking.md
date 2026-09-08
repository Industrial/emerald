---
name: Benchmarking
overview: "Emerald vs Rust vs C vs C++ on two loop-based benchmarks (integer summation, array traversal), reporting real compile time/runtime/binary size — inception §21's performance-comparison requirement."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-benchmark-programs
    content: "benchmarks/{sum,array_traversal}/*.{em,rs,c,cpp} — equivalent programs in all 4 languages"
    status: pending
  - id: leaf-benchmark-runner
    content: "An #[ignore]d emerald-cli integration test that compiles+runs+measures every language x benchmark combo and reports a real table"
    status: pending
isProject: false
---

# Plan 15 — Benchmarking

This is `benchmarking`, row `15` (the last) of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §21's performance-comparison requirement — "the
initial target is: Emerald's generated machine code should be capable of
approaching ordinary optimized native code for statically typed
workloads," compared against Rust/C/C++/Ruby on inception's own suggested
first-benchmark list (integer summation, sum of squares, Fibonacci,
matrix multiplication, array traversal, string processing, object
allocation, method dispatch, recursive calls), reporting runtime, binary
size, compile time, and memory usage.

## Decision log

- **Only 2 of inception's 9 suggested benchmarks are actually
  expressible in this compiler today: integer summation and array
  traversal.** Sum of squares, Fibonacci (the natural subtraction-based
  formulation), and matrix multiplication all need a `-`/`*` operator —
  **this language has neither**; the grammar only ever added `+`
  (plan 04) and comparisons (plan 07). String processing needs string
  *literals*, which were never added (`Expr` has no `StringLit` variant
  despite `Type::String` existing in the type system since plan 05).
  This is a genuine, benchmark-driven discovery about the compiler's
  actual surface, not a benchmark-design shortcut — it's recorded here
  because writing the benchmark suite is what surfaced it.
- **Object allocation, method dispatch, and recursive calls are also
  deferred**, but for scope, not a capability gap: they're expressible
  (repeated `ClassName.new`, repeated method calls, addition-based
  recursive counting instead of subtraction-based), but two solid,
  cross-checked benchmarks are enough to prove the comparison
  methodology genuinely works end-to-end; three more of the same
  *kind* of proof isn't a good use of the last plan in this session.
- **Ruby is excluded from the comparison** — not installed in this
  environment (verified: `ruby` is not on `PATH` here), a real
  environment constraint, not a design choice. Rust (`rustc`), C
  (`cc`/gcc), and C++ (`g++`) are all confirmed available and are what's
  actually compared.
- **Array traversal uses a small (20-element) array literal, traversed
  repeatedly in an outer loop, rather than one large runtime-sized
  array.** `Array[T]` can currently only be constructed via a literal
  (`[e1, e2, ...]`) — there is no `Array.new(size)`/fill-from-empty
  constructor (plan 09's scope), so a 10-million-element literal isn't
  writable. Repeating a small traversal 1,000,000× reaches a comparable
  total read count (20,000,000) with an honest, currently-expressible
  program — another real capability boundary this benchmark surfaced.
- **Memory usage is not measured.** Portably capturing a child process's
  peak RSS without shelling out to `/usr/bin/time -v` (unavailable/
  disallowed in this session's sandboxed shell) means hand-rolling a
  `wait4`/`getrusage` FFI capture — real, buildable, but more machinery
  than the last plan in a long session should add for one metric out of
  four. Runtime, binary size, and compile time are all measured for
  real; memory usage is a disclosed, honest gap, not a fabricated number.
- **The runner is an `#[ignore]`d integration test** in
  `crates/emerald-cli/tests/`, not a new standalone binary crate —
  reuses the exact `CARGO_BIN_EXE_emerald-cli` mechanism
  `tests/hello_em.rs` already relies on (a separate `emerald-bench`
  binary crate can't get emerald-cli's build path for free the same
  way, since `emerald-cli` has no library target to depend on).
  `#[ignore]` because a real, multi-language, multi-benchmark comparison
  takes real wall-clock time and isn't part of the fast correctness
  suite `cargo test --workspace` already runs on every other plan.

## Leaf: leaf-benchmark-programs

### 1. Context
- Target state: `benchmarks/sum/{sum.em,sum.rs,sum.c,sum.cpp}` (sum
  `0..9_999_999`, expected `49999995000000`) and
  `benchmarks/array_traversal/{array_traversal.em,array_traversal.rs,
  array_traversal.c,array_traversal.cpp}` (a 20-element array summed
  1,000,000 times, expected `210000000`) — four equivalent programs per
  benchmark, one per language, each printing just the final integer to
  stdout so correctness (all four agree) is checked alongside speed.

### 2. Acceptance Criteria
1. All 8 programs compile with their language's normal optimizing flags
   (`rustc -O`, `cc -O2`, `g++ -O2`; `emerald-cli` has no such flag —
   codegen has never had an optimization-level dial, itself a Decision
   log note worth carrying forward) and, when run, print the exact
   expected value for their benchmark — real executed proof of
   correctness, not just "it compiled."

### 3. File & Module Structure
- **Create:** `benchmarks/sum/*`, `benchmarks/array_traversal/*`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Manual run | each program compiled + run directly | prints the expected value | agent-claimed-locally |

---

## Leaf: leaf-benchmark-runner

### 1. Context
- Target state: `crates/emerald-cli/tests/benchmarks.rs`, an
  `#[ignore]`d test that, for each (benchmark, language) pair: compiles
  it (timing the compile), runs the resulting binary (timing the run,
  asserting its stdout matches the expected value), and measures the
  binary's file size — then prints a markdown table and writes it to
  `benchmarks/REPORT.md`.

### 2. Acceptance Criteria
1. Running the test (`cargo test --release -p emerald-cli -- --ignored
   --nocapture benchmarks`) produces real measured numbers for all 8
   (benchmark × language) combinations — not simulated, not hand-typed;
   the assistant runs it for real this session and commits the resulting
   `benchmarks/REPORT.md`.
2. Every language's output is asserted to equal the benchmark's expected
   value before its timing is recorded — a benchmark number is only
   meaningful attached to a program that's actually correct.
3. `benchmarks/REPORT.md` states the machine/session it was measured on
   and that absolute numbers will vary elsewhere — the source programs
   and the runner are the durable, reusable artifact; one run's numbers
   are a snapshot, not a permanent claim.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/tests/benchmarks.rs`,
  `benchmarks/REPORT.md`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test --release -p emerald-cli -- --ignored --nocapture benchmarks` | real report produced | agent-claimed-locally |
| Workspace (unaffected) | `cargo test --workspace` | all pass (the new test is `#[ignore]`d, doesn't run by default) | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo test --release -p emerald-cli -- --ignored --nocapture benchmarks
```

## Out of scope / deferred
- Sum of squares, Fibonacci, matrix multiplication, string processing —
  blocked on missing `-`/`*` operators and string literals; see Decision
  log. Real language gaps, not benchmark-scope cuts.
- Object allocation, method dispatch, recursive-call benchmarks — see
  Decision log (expressible, deferred for session-scope reasons).
- Ruby comparison — not installed in this environment; see Decision log.
- Memory usage measurement — see Decision log.
- Any Emerald-side codegen optimization work motivated by these
  numbers — inception §21/§22 are explicit that benchmarking comes
  *before* optimizing ("do not optimize the compiler before there is a
  working compiler" / "avoid premature optimization in the compiler
  itself"); this plan's job is measurement, not response to what it
  measures.
