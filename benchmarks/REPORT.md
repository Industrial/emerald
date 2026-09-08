# Emerald Benchmark Report

Plan 15 (`benchmarking`) established this harness; plan 16 (`codegen-backend-bakeoff`) benchmarked a tuned Cranelift backend against a new LLVM backend and found LLVM 5-18x faster on these two programs; the `consolidate-llvm-backend` plan then replaced Cranelift outright — `emerald-codegen` is now the single, full-language LLVM backend, measured for real by compiling and running each program in `benchmarks/` during this test. Ruby is excluded (not installed in this environment); memory usage is not measured. See `.cursor/plans/benchmarking.plan.md`, `.cursor/plans/codegen-backend-bakeoff.plan.md`, and `.cursor/plans/consolidate-llvm-backend.plan.md` for the full Decision logs, including why only 2 of inception §21's 9 suggested benchmarks are expressible in this compiler today.

**These specific numbers are a snapshot from one run on one machine** — they will vary on different hardware/load; the durable artifacts are the benchmark source programs and this runner, not these exact figures.

Every row's binary is asserted to produce the exact expected output before its timing is recorded, so a wrong-but-fast program can't appear here.

## sum

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald | 609.492 ms | 0.533 ms | 16488 bytes |
| Rust | 594.920 ms | 0.718 ms | 4354832 bytes |
| C | 583.142 ms | 0.611 ms | 15864 bytes |
| C++ | 639.304 ms | 1.415 ms | 15864 bytes |

## array_traversal

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald | 602.083 ms | 0.559 ms | 16488 bytes |
| Rust | 558.675 ms | 0.738 ms | 4354896 bytes |
| C | 585.803 ms | 2.488 ms | 15928 bytes |
| C++ | 622.210 ms | 3.111 ms | 15928 bytes |

