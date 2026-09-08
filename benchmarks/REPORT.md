# Emerald Benchmark Report

Plan 15 (`benchmarking`), measured 2026-09-08 — Emerald vs Rust vs C vs C++, measured for real by compiling and running each program in `benchmarks/` during this test. Ruby is excluded (not installed in this environment); memory usage is not measured. See `.cursor/plans/benchmarking.plan.md` for the full Decision log, including why only 2 of inception §21's 9 suggested benchmarks are expressible in this compiler today.

**These specific numbers are a snapshot from one run on one machine** — they will vary on different hardware/load; the durable artifacts are the benchmark source programs and this runner, not these exact figures.

Every row's binary is asserted to produce the exact expected output before its timing is recorded, so a wrong-but-fast program can't appear here.

## sum

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald | 478.398 ms | 4.279 ms | 16528 bytes |
| Rust | 449.512 ms | 0.871 ms | 4354832 bytes |
| C | 465.296 ms | 0.462 ms | 15864 bytes |
| C++ | 485.616 ms | 1.163 ms | 15864 bytes |

## array_traversal

| Language | Compile time | Run time | Binary size |
|---|---|---|---|
| Emerald | 483.690 ms | 12.633 ms | 16528 bytes |
| Rust | 448.425 ms | 0.702 ms | 4354896 bytes |
| C | 465.203 ms | 2.390 ms | 15928 bytes |
| C++ | 494.789 ms | 3.465 ms | 15928 bytes |

