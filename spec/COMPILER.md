# Emerald — COMPILER.md

**Status:** toolchain decisions only (plan `02 toolchain-prototype`). Full
pipeline architecture (how `emerald-driver` wires the stages together) is
deferred to the milestone plans that build it (`04`–`06`), per inception
§4's list of six spec documents and plan `01`'s decision log.

---

## Toolchain Decisions

Each decision below was made by building and running a real prototype
against inception §17's milestone-1 slice (`def add(a: Int64, b: Int64)
-> Int64 \n a + b \n end`), per inception §14's "do a small prototype...
before committing" instruction — not by research alone.

### Lexer: `logos`

**Chosen.** Inception §14.2 names no alternative to compare against, so
this was a feasibility check rather than a bake-off. `crates/emerald-lexer`
tokenizes the full milestone-1 source correctly, including the `->` and `:`
punctuation `spec/GRAMMAR.md` introduces, and correctly rejects both an
unterminated string and `@@` (the removed class-variable sigil,
`spec/GRAMMAR.md` §2) as lex errors rather than silently accepting them.

Evidence: `crates/emerald-lexer/src/lib.rs`, 3 passing tests.

### Parser: `LALRPOP` (chosen over `Chumsky`)

**Chosen: LALRPOP.** Both candidates inception §14.1 names were prototyped
against the identical milestone-1 grammar slice.

| Axis | LALRPOP 0.22.2 | Chumsky 1.0.0-alpha.8 |
|---|---|---|
| Maturity | Stable release line | Pre-1.0 alpha — crates.io has no stable `1.x` yet |
| Rough edges hit | None | `text::ascii::ident()` silently stops at the first digit (`"Int64"` → `"Int"`); had to hand-roll the identifier combinator to work around it |
| Grammar representation | Declarative `.lalrpop` grammar file (BNF-like, with a `match {}` block for tokenization) | Combinator chain in ordinary Rust code |
| Fit with inception §18 | Direct — a `.lalrpop` file *is* a grammar-production list, matching "import Ruby's grammar as an inventory, mark KEEP/MODIFY/REMOVE per production" almost structurally | Indirect — combinators express the grammar as code, not as an inventory-shaped artifact |
| Error message (missing `end`) | `Unrecognized EOF found at 44` (byte offset, no line/col) | `found end of input expected any` (no position at all in the plain `Display`) |
| Setup cost | `build.rs` + `lalrpop` build-dependency, generates a parser module at build time | No `build.rs`; parser is an ordinary function |

**Decision rationale:** the error-message quality was a wash — neither
library gives publication-quality diagnostics out of the box; both need the
`miette`/`ariadne`-class layer inception §14.4 already earmarks as a
separate investigation. With that axis roughly even, the deciding factors
were (1) LALRPOP's stable release line versus Chumsky's pre-1.0 alpha
status, surfaced concretely by hitting a real bug in Chumsky's convenience
`ident()` helper during this exact prototype, and (2) the structural fit
between a `.lalrpop` grammar file and inception §18's explicit "grammar as
graded inventory" strategy — a declarative grammar file is the more natural
home for a KEEP/MODIFY/REMOVE/UNDECIDED-tagged production list than a
combinator chain is.

Evidence: `crates/emerald-parser/src/grammar.lalrpop`,
`crates/emerald-parser/src/lib.rs`, 2 passing tests (happy path + missing
`end` error).

**Not decided on popularity alone**, per inception §14.1's explicit
constraint — the table above is the full comparison basis.

### Codegen: `Cranelift` (v1 only — Inkwell/LLVM deferred)

**Chosen for v1: Cranelift.** Per the plan's Deviation note, this is *not*
a completed Cranelift-vs-Inkwell comparison — this environment has no
`llvm-config` on `PATH` and no `LLVM_SYS_*_PREFIX` configured (the Nix
store holds an unbuilt `llvm-19.1.7.drv` and an `llvm-19.1.7-lib` output,
neither of which is a usable `llvm-config` binary). Standing up Inkwell
requires adding LLVM to the shared `devenv.nix`, a repo-environment change
out of scope for this plan.

Inception §14.6 explicitly sanctions this: *"Prototype Emerald code
generation with Cranelift first if rapid implementation is the priority."*
`crates/emerald-codegen` hand-builds Cranelift IR for `fn add(a: i64, b:
i64) -> i64 { a + b }`, JIT-compiles it via `cranelift-jit`, and calls the
resulting native machine code — `add(20, 22)` returns `42` as real,
executed native code, not a simulation.

**Revisit trigger (not "permanent," per inception §14.6's own wording):**
if `15 benchmarking`'s numeric benchmarks (inception §11/§21) show
Cranelift-generated code meaningfully behind hand-written/LLVM-generated
code on the hot paths inception §11 cares about (loop optimization,
vectorization), re-open this decision with LLVM properly wired into
`devenv.nix` first — not by silently reaching for Inkwell mid-milestone.

Evidence: `crates/emerald-codegen/src/lib.rs`, 2 passing tests
(`add(20, 22) == 42`, and a negative-number case).

**Ahead-of-time note:** this prototype uses `cranelift-jit`. Milestone `06
milestone1-codegen`'s "link an executable" requirement needs
`cranelift-object` (object-file emission) instead — noted as follow-up
work for that plan, not built here.

### 2026-09-08 addendum (plan `16 codegen-backend-bakeoff`): revisit trigger resolved

The revisit trigger above fired for real: `15 benchmarking`'s numbers
(`benchmarks/REPORT.md`, measured 2026-09-08) showed Emerald 4.9x slower
than Rust and 9.3x slower than C on `sum`, and 18x slower than Rust and
5.3x slower than C on `array_traversal`. Root cause: `host_isa()` in
`crates/emerald-codegen/src/lib.rs` never configured Cranelift's
`opt_level`, defaulting to `OptLevel::None`.

Two things happened, both real and measured, not simulated:

1. **Cranelift got tuned first**, so the bake-off wouldn't be rigged:
   `host_isa()` now sets `opt_level=speed`. Effect on these two
   benchmarks was small — `sum` run time went from 4.279ms to 4.260ms,
   `array_traversal` from 12.633ms to 12.729ms (within noise). Cranelift's
   `speed` level does not include the loop-invariant code motion /
   auto-vectorization inception §11's hot-path concern is actually about.
2. **LLVM was wired into `devenv.nix` for real** (`pkgs.llvmPackages_21.llvm`,
   `libffi`, `libxml2`, `LLVM_SYS_211_PREFIX`) and a genuine second backend,
   `crates/emerald-codegen-llvm`, was built on `inkwell` (`llvm21-1`),
   running LLVM's `default<O3>` pass pipeline before object emission.
   Deliberately scoped to the AST subset `sum.em`/`array_traversal.em`
   use (top-level statements, `Int64`/`Float64`, arrays — no
   functions/classes/modules/lambdas/exceptions yet; see
   `.cursor/plans/codegen-backend-bakeoff.plan.md`'s Decision log for why
   that's the right amount of scope for a bake-off).

**Result** (`benchmarks/REPORT.md`, re-measured 2026-09-08 with both
backends tuned):

| Benchmark | Emerald (Cranelift) | Emerald (LLVM) | Rust | C |
|---|---|---|---|---|
| `sum` run time | 4.260 ms | **0.762 ms** | 0.824 ms | 0.620 ms |
| `array_traversal` run time | 12.729 ms | **0.716 ms** | 0.906 ms | 2.360 ms |

LLVM at O3 doesn't just close the gap — on both loop-heavy benchmarks it
matches or beats hand-written Rust, and beats hand-written C on
`array_traversal` (auto-vectorizing the inner 20-element loop in a way
neither Cranelift nor the hand-written C did). This is exactly the "loop
optimization, vectorization" gap inception §11 flagged as worth revisiting
Cranelift over.

**Decision: dual-backend, Cranelift stays the CLI default, LLVM is
opt-in.** `emerald-cli` gains `--backend=cranelift|llvm`. Cranelift
remains the default because it is the only backend covering the full
language (classes, lambdas, exceptions, modules) — `emerald-codegen-llvm`
does not, by explicit scope decision (see above), and building that parity
is real, separate future work, not assumed here. For numeric,
loop-dominated code that fits the LLVM backend's current scope, `--backend=llvm`
is measurably faster than either Cranelift or hand-written C/Rust on this
benchmark pair. This is not a "permanent" decision either
(inception §14.6) — if/when `emerald-codegen-llvm` reaches feature parity,
revisit which backend the CLI defaults to.

Evidence: `crates/emerald-codegen-llvm/src/lib.rs` (6 passing tests,
including both benchmark programs run end-to-end through the LLVM
backend); `crates/emerald-cli/tests/benchmarks.rs` (real, executed,
`--backend`-parameterized measurement); `benchmarks/REPORT.md`;
`.cursor/plans/codegen-backend-bakeoff.plan.md`.

### 2026-09-09 addendum (`consolidate-llvm-backend`): Cranelift removed, LLVM is now the only backend

Explicit user directive: "Remove everything but the best option and
implement the best option 100% now." The dual-backend state above was
scoped to plan 16's bake-off measurement, not meant to be permanent —
this addendum is that follow-through. `crates/emerald-codegen` (Cranelift)
is deleted; `crates/emerald-codegen-llvm` was promoted to
`crates/emerald-codegen`, the sole codegen crate, and built out to full
feature parity with what Cranelift supported: functions, classes
(fields/methods/`new`/`@field`), lambdas/closures (top-level `Proc`
`Let`s, by-value capture, static `.call`), exceptions (`raise`/`begin`/
`rescue` via the same setjmp/longjmp runtime), modules, arrays. Every
restriction the old backend had (lambdas only as a top-level `Let`,
method/index receivers must be a plain local variable, one `rescue`
clause, no inheritance) carries over unchanged — this is a backend swap,
not a language change. `emerald-cli` loses `--backend`; there is only one
backend again.

Proof of parity: the old Cranelift backend's entire test suite (17
tests — functions, the `Point` class example, `break`/`if`/`while`,
lambda capture, both exception tests, the module example, both array
tests, both benchmark programs) was ported verbatim (same source
strings, same expected outputs) into `crates/emerald-codegen`'s own test
module and all 17 pass. `cargo test --workspace` is green.

Re-measured `benchmarks/REPORT.md` with the single consolidated backend,
still at `OptimizationLevel::Aggressive` (`default<O3>`): Emerald now
**beats hand-written Rust on both benchmarks** (`sum`: 0.533ms vs Rust's
0.718ms; `array_traversal`: 0.559ms vs Rust's 0.738ms) and beats C on
`array_traversal` (0.559ms vs C's 2.488ms) — the auto-vectorized loop
that motivated this whole line of work in the first place.

Evidence: `crates/emerald-codegen/src/lib.rs` (17 passing tests);
`benchmarks/REPORT.md`; `.cursor/plans/consolidate-llvm-backend.plan.md`.

---

## Codegen targets

### 2026-09-09 addendum (`64 wasm-codegen-target`): `wasm32-wasi` as a second, selectable target

`emerald build --target wasm32-wasi` reuses the same AST-to-LLVM-IR
codegen path `crates/emerald-codegen/src/lib.rs` already builds for
the native target — `Target::initialize_webassembly`/`TargetTriple::
create("wasm32-wasi")` replace the four native-only calls
(`compile_to_object_impl`) that were the one real hardcoded-target seam
in the whole file (verified this session: LLVM 21's own build here has
`WebAssembly` in `llvm-config --targets-built`, and `inkwell` 0.10
exposes `Target::initialize_webassembly`). No AST node, grammar
production, or sema check changed — this is purely a second selectable
target machine plus the runtime/link seams below it.

| | `x86_64-unknown-linux-gnu` (default) | `wasm32-wasi` (`--target wasm32-wasi`) |
|---|---|---|
| Concurrency | Real OS threads — a fixed pool of `EMERALD_WORKERS` (or `nproc`) `pthread`-spawned workers pulling actor messages off a shared runnable queue (plan 55). | Sequential only. WASI preview 1 has no `pthread_create` — `M` is 1 and cannot be otherwise (harder than plan 55's own `EMERALD_WORKERS=1`, which still spawns one real thread). `emerald_worker_pool_drain_and_join` becomes the sequential mailbox-drain loop itself; per-actor FIFO ordering is preserved, "multiple actors run literally simultaneously" is not. `EMERALD_WORKERS` is accepted and silently ignored — a real, documented no-op. |
| FFI | Arbitrary `extern "C"` linking against a named library (`emerald.toml`'s `[ffi] link = [...]`, plan 59). | None. WASI's sandboxing model exposes only the fixed `wasi_snapshot_preview1` import set — no `dlopen`, no linking against a native `.so`/`.a` that assumes syscalls beyond that set. Remote/distributed actors (plan 60's `.register`/`.remote`, real sockets) are in the same unsupported category — a real, disclosed gap this plan's own text did not originally account for (plan 60 landed after this plan's initial Decision log was written; corrected here). |
| Required external tooling | `cc` (already required for every native build). | A WASI-capable cross-compiler (`CC_wasm32_wasip1`, resolved the same way at both `crates/emerald-driver/build.rs`'s runtime-archive cross-compile time and `link`'s own link-time invocation) plus [`wasmtime`](https://wasmtime.dev/) to actually execute a compiled `.wasm` module — `emerald run --target wasm32-wasi` is explicitly rejected (a `.wasm` module is not a directly-launchable native binary) rather than failing with a confusing native-launcher error. |
| Output | Bare executable (`./<name>`). | `<name>.wasm`. |

**Declined, explicitly** (plan 64's own Decision log, restated here per
its own `leaf-spec-and-restrictions-doc`):
- **The WASM threads proposal / `SharedArrayBuffer`-style
  multi-threading** (`wasm32-wasip1-threads` or preview 2's
  equivalent) — a real fix to the "M=1" ceiling above, but a
  substantial, separate project (a threads-enabled wasi-libc,
  atomics-enabled codegen, a full runtime concurrency re-audit for
  shared-linear-memory aliasing hazards), not a small extension of the
  sequential fallback this plan built.
- **Arbitrary C-library FFI under `wasm32-wasi`** — a preemptive scope
  boundary for whichever future plan adds general C-library linking:
  it must not extend to this target, for the sandboxing reason stated
  in the table above, not an arbitrary restriction.

**A real, disclosed verification gap, stated plainly rather than
silently assumed away:** this workspace's own `devenv shell` (verified
this session) has no WASI toolchain (`wasi-sdk`/`WASI_SDK_PATH`) and no
`wasmtime` installed, and `devenv.nix` was deliberately **not** modified
to add either — an untested new Nix package fetch inside this
already-long session risked breaking the shared dev shell this whole
session's own tooling (`lean-ctx`, `roam-code`, `maestro`, every prior
plan's own verification) depends on, a blast radius judged not worth
the reward. What IS real and verified this session: LLVM genuinely
emits a valid `wasm32-wasi` object file for `benchmarks/sum/sum.em`
with zero codegen changes beyond target selection (`crates/
emerald-codegen/src/lib.rs`'s `sum_benchmark_compiled_for_wasm32_wasi_
produces_a_real_webassembly_object` test, confirmed via `file(1)`
reporting a genuine WebAssembly object); the CLI/driver plumbing
(`--target` flag, `.wasm` output extension, `emerald run` rejection,
unrecognized-target rejection) is real and tested end-to-end
(`crates/emerald-cli/tests/target_flag.rs`); the runtime C port
(`runtime/emerald_runtime.c`'s `__wasi__`-guarded sequential worker
pool) is written and the **native** branch is confirmed byte-for-byte
regression-free (`cargo test --workspace`), but the `wasm32-wasip1`
branch has never actually been compiled against a real WASI toolchain
in this session — a real, open item for whichever future session has
one available, not a claim of proven correctness. `crates/
emerald-driver/tests/wasm_target.rs`'s own full compile-and-run proof
is written and gated correctly (verified skip-not-fail, confirmed this
session — `wasmtime` is genuinely absent here) but has never actually
executed its `wasmtime run` branch for the same reason.

Evidence: `crates/emerald-codegen/src/lib.rs` (`CodegenTarget`,
`compile_to_object_with_target`); `crates/emerald-driver/src/lib.rs`
(`compile_with_target`, `compile_program_with_libs_and_target`,
`link_with_libs_and_target`); `crates/emerald-driver/build.rs` (gated
`wasm32-wasip1` cross-compile); `runtime/emerald_runtime.c`
(`__wasi__`-guarded sections); `crates/emerald-cli/tests/target_flag.rs`;
`crates/emerald-driver/tests/wasm_target.rs`;
`history/2026-09-09T142000Z-plan-64-wasm-codegen-target.md`.

---

## Cross-references

- Grammar-file location and content: [`GRAMMAR.md`](./GRAMMAR.md),
  `crates/emerald-parser/src/grammar.lalrpop`.
- Type representations the codegen must eventually honor (unboxed
  `Array[T]`, `Int64` default, etc.): [`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md).
- Semantic decisions with a direct toolchain dependency (native-unwinding
  exceptions, closure heap-allocation strategy): [`SEMANTICS.md`](./SEMANTICS.md)
  §5, §7 — both remain compatible with a Cranelift backend.
