# Emerald Benchmark Report

Plan 15 (`benchmarking`) established this harness; plan 16 (`codegen-backend-bakeoff`), measured 2026-09-08, added a second, tuned Emerald backend (LLVM via `inkwell`) alongside a real `opt_level=speed` fix to the original Cranelift backend, and re-measured everything for real by compiling and running each program in `benchmarks/` during this test. Ruby is excluded (not installed in this environment); memory usage is not measured. See `.cursor/plans/benchmarking.plan.md` and `.cursor/plans/codegen-backend-bakeoff.plan.md` for the full Decision logs, including why only 2 of inception §21's 9 suggested benchmarks are expressible in this compiler today, and why the LLVM backend is scoped to exactly the AST shape those 2 programs use.

**These specific numbers are a snapshot from one run on one machine** — they will vary on different hardware/load; the durable artifacts are the benchmark source programs and this runner, not these exact figures.

Every row's binary is asserted to produce the exact expected output before its timing is recorded, so a wrong-but-fast program can't appear here.

## sum

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald (Cranelift) | 629.758 ms | 4.260 ms | 16528 bytes |
| Emerald (LLVM) | 634.875 ms | 0.762 ms | 16488 bytes |
| Rust | 601.664 ms | 0.824 ms | 4354832 bytes |
| C | 631.942 ms | 0.620 ms | 15864 bytes |
| C++ | 683.693 ms | 1.279 ms | 15864 bytes |

## array_traversal

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald (Cranelift) | 615.733 ms | 12.729 ms | 16528 bytes |
| Emerald (LLVM) | 622.159 ms | 0.716 ms | 16488 bytes |
| Rust | 574.101 ms | 0.906 ms | 4354896 bytes |
| C | 597.310 ms | 2.360 ms | 15928 bytes |
| C++ | 641.266 ms | 3.206 ms | 15928 bytes |

