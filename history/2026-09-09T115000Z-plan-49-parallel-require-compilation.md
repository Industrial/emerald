---
name: Parallel Require-Graph Compilation
overview: "Once plan 23's `require` dependency graph exists, compile files with no path between them concurrently across CPU cores — a leveled (rank-order) scheduler over a fixed worker pool, not a general work-stealing engine, built on plan 48's per-file query cache as the unit of parallel work."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-require-graph-leveling
    content: "Turn plan 23's DFS require-splice into a queryable graph (canonical-path nodes, require edges), confirm the whole graph acyclic up front, then rank it into topological levels via Kahn's algorithm — no speculative parallelism before the whole-graph cycle check completes"
    status: pending
  - id: leaf-parallel-parse-and-typecheck
    content: "Parse every file in the graph fully in parallel regardless of level (verified: the parser never needs another file's symbols); type-check within a level in parallel via plan 48's per-file query cache, with a hard barrier between levels since a file may reference symbols a dependency exports"
    status: pending
  - id: leaf-parallel-codegen-and-jobs-flag
    content: "Restructure codegen from today's single combined LLVM module/Context to one Context+Module per file (verified necessary against real source), a std::thread::scope fixed worker pool bounded by a new --jobs N flag draining each level's task queue, and a multi-object-file link step; worked 3-file proof with timing/thread-id instrumentation showing genuine concurrency"
    status: pending
isProject: false
---

# Plan 49 — Parallel Require-Graph Compilation

This is plan 49 of the 48-57 batch implementing "Beyond the Ceiling" in
full — ten independent sibling plans covering compiler-implementation
improvements plus the actor-model concurrency pillar and ADTs/`Result`,
none of them conceding this project's identity constraints (no
`method_missing`/`eval`/`send`/reflection, no mixins/open classes/
monkey-patching, no dynamic/virtual dispatch or vtables, no tracing GC,
no runtime reflection). Like every batch before it, this is post-v1
scope (`plan-of-plans.md`'s own Completion note calls Emerald v1 done at
row 15); this plan does not touch `plan-of-plans.md` or any other plan
file.

This plan depends on two siblings, neither of which has been executed
in this checkout yet (verified this session: `crates/emerald-driver`
does not exist; the root `Cargo.toml`'s `[workspace] members` still
lists only `emerald-lexer`/`emerald-parser`/`emerald-codegen`/
`emerald-sema`/`emerald-cli`; `crates/emerald-parser/src/ast.rs` has no
`Item::Require` variant — the same "still a plan document, not landed
code" state plan 46 already disclosed about the same dependency). This
plan builds on their *designs*, not on shipped code:

- **Plan 23** (`2026-09-08T222500Z-plan-23-multi-file-compilation.md`,
  read in full this session) — its `require` grammar, its
  `resolve_program`'s canonical-path `visited`/in-progress `stack`
  cycle-detection and diamond-dedup semantics, and its choice to splice
  every required file's items into one flat `Program` before sema or
  codegen ever runs. This plan reuses plan 23's cycle/dedup *semantics*
  exactly but changes *how* the graph is walked (see Decision log) and
  explicitly stops reusing its "splice into one flat `Program`" step,
  since that step is precisely what makes per-file parallel work
  impossible today.
- **Plan 48** (incremental-query-compilation) — authored in parallel by
  a sibling agent; no `2026-09-...-plan-48-*.md` file exists in
  `history/` as of this session, so this plan relies on the contract as
  given rather than on plan 48's own source text, and says so rather
  than silently assuming it: each pipeline stage (parse/typecheck/
  codegen) sits behind a memoized query cache keyed by content hash, at
  whole-file granularity, with zero change to observable compiler
  behavior. That contract is exactly the restructuring this plan's own
  codegen leaf would otherwise have to do from scratch — a per-file
  cache entry for "typecheck" or "codegen" cannot exist unless those
  stages are already re-expressed as per-file operations rather than
  the single whole-`Program` passes verified below. Plan 49 takes that
  restructuring as given and adds only what plan 48's contract doesn't
  cover: *scheduling* those already-independent per-file query
  invocations onto worker threads, and one real LLVM-specific
  correctness gap plan 48's contract doesn't mention at all (per-thread
  `Context` ownership — see Decision log).

Concrete proof this plan targets — a real three-file program where two
files share no dependency and a third depends on both:

```
examples/parallel/b.em      # def b_value -> Int64 \n 10 \n end
examples/parallel/c.em      # def c_value -> Int64 \n 20 \n end
examples/parallel/main.em   # require b \n require c \n puts b_value + c_value
```

```
$ emerald-cli --jobs 2 examples/parallel/main.em -o main
$ ./main
30
```

That output alone proves correctness, not concurrency — a `--jobs 2`
run and a `--jobs 1` run produce the identical `30` either way. The
concurrency claim is proven separately, by instrumentation (see
`leaf-parallel-codegen-and-jobs-flag`): with `EMERALD_TEST_COMPILE_DELAY_MS=200`
injecting a fixed artificial delay into each file's compilation (a
test-only seam — real `.em` toy programs compile in far under a
millisecond, too fast for wall-clock timing to prove anything), `--jobs
2` compiles `b.em` and `c.em`'s level (level 0) in ~200-250ms wall time
with their recorded `[start, end]` windows overlapping and distinct OS
thread IDs, while `--jobs 1` takes ~400ms with non-overlapping windows
on a single thread ID — a real, timed, non-flaky proof that the two
independent files are actually processed concurrently, not merely
dispatched to different threads that happen to run one after another.

## Decision log

- **Verified: today's (and plan 23's own designed) codegen is a single
  combined LLVM module, not one module per file — this plan cannot
  "trivially" parallelize codegen; it must restructure it first.**
  `emerald_codegen::compile_to_object(program: &Program, out_path:
  &Path)` (`crates/emerald-codegen/src/lib.rs:4094`) calls
  `Context::create()` and `context.create_module("emerald_module")`
  exactly once per call (L4109-4111), then declares every runtime
  extern function, every user function (`declare_user_functions`,
  L3993, iterating `program.items` once), and every lambda
  (`declare_lambda_functions`, L4055) into that one `Module` before
  building any function body. `emerald_sema::check_program(program:
  &Program)` (`crates/emerald-sema/src/lib.rs:1764`) is the same shape:
  one shared `classes: HashMap<String, ClassInfo>` and one shared `sigs:
  HashMap<String, FunctionSig>`, both built by iterating `program.items`
  once (L1771-1830) before any function body is checked. Plan 23's own
  Decision log states this is deliberate: `resolve_program` "splice[s]
  the resolved dependency graph's items into one flat `Program` *before*
  sema or codegen ever runs" so that "to that code, a merged multi-file
  `Program` looks exactly like one big file." That design is exactly
  right for making `require` work with zero sema/codegen changes (plan
  23's stated goal) and exactly wrong for this plan's goal — there is no
  per-file compilation unit anywhere in the pipeline today, at any
  stage past parsing, to hand to a separate thread. Per plan 48's
  contract, per-file typecheck/codegen query units are exactly what
  gets introduced (see above); this plan's own remaining codegen change
  is narrower — see the `Context`-ownership bullet below — and is
  detailed in `leaf-parallel-codegen-and-jobs-flag`.
- **Parsing needs no other file's symbols — verified structurally, not
  assumed.** `crates/emerald-parser/Cargo.toml` depends on
  `lalrpop-util`, `miette`, and `thiserror` only; it has no dependency
  on `emerald-sema` (the reverse is true — `emerald-sema/Cargo.toml`
  depends on `emerald-parser`). A crate that cannot even name a
  `ClassInfo`/`FunctionSig` type cannot consult one during parsing.
  Structurally, `Item::Require(String)` (plan 23) carries an opaque
  path string the LALR grammar never resolves — parsing a file only
  ever needs that file's own byte stream. This is what makes full,
  level-independent parallel parsing safe: every file in the require
  graph can be parsed the moment its own bytes are read, in any order,
  on any thread, before the graph's levels are even known. Type-
  checking is the opposite: a file referencing a symbol a dependency
  exports needs that dependency's already-computed `ClassInfo`/
  `FunctionSig` (or plan 48's equivalent per-file signature query)
  available first — hence level-ordered, not fully free, parallelism
  for that stage.
- **Scheduling is topological leveling (Kahn's algorithm) within plan
  23's DAG, not a general work-stealing scheduler.** All files with no
  unresolved dependency form level 0; a file enters level N once every
  file it `require`s has been assigned a level less than N. Work is
  parallelized *within* a level only; a hard barrier separates levels
  (level N+1 does not start until every file in level N has finished
  type-checking — codegen has no such constraint once each file's own
  typecheck query is done, but this plan keeps codegen leveled too, for
  one concrete reason: keeping one scheduling shape for both stages
  means one worker-pool implementation, not two, and the two stages'
  per-level wall-clock cost is dominated by codegen anyway for anything
  past a toy example). This is a real, disclosed simplification over a
  fully general dependency-aware scheduler that could, in principle,
  start file X's codegen the instant X's own dependencies finish
  regardless of what else is mid-flight in X's level — that finer-
  grained scheduler is real future work (see Out of scope), not
  something this plan quietly approximates and calls done.
- **Explicitly declined: speculative/optimistic parallelism — starting
  a file's compilation before its dependency subgraph is confirmed
  cycle-free.** The alternative this plan does not take: begin
  type-checking/codegen-ing files as soon as they're discovered, and
  abort/roll back whatever's in flight if a later-discovered edge
  closes a cycle. That needs cancellation-safe partial work, a rollback
  path for whatever a half-finished sema/codegen query already wrote
  into plan 48's cache, and reasoning about a codegen query racing a
  cycle-detection failure. This plan instead computes the *entire*
  require graph first — a fast, cheap, single-threaded DFS over each
  file's own `Item::Require` list only (no body type-checking, no
  codegen; exactly plan 23's existing `resolve_program` traversal cost)
  — and only begins any parallel typecheck/codegen work once that whole
  graph is confirmed acyclic via plan 23's existing `visited`/`stack`
  check. Simpler, always correct, and consistent with this project's
  demonstrated posture elsewhere (plan 13 and plan 17 both declined
  `salsa` for the same "real, disclosed, simpler mechanism over general
  machinery" reason plan 23 itself repeats).
- **Threading mechanism: `std::thread::scope`, a hand-rolled
  fixed-size pool draining a per-level task list — not `rayon`.**
  Verified this session across every `Cargo.toml` in the workspace
  (root, `emerald-lexer`, `emerald-parser`, `emerald-sema`,
  `emerald-codegen`, `emerald-cli`): none depends on `rayon` or any
  other thread-pool crate today; adopting `rayon` would be this
  workspace's first parallelism dependency, full stop. The scheduling
  shape this plan actually needs — split one level's file list into
  `--jobs` chunks, run each chunk on its own thread, join the whole
  level before starting the next — is a bounded fan-out/fan-in barrier,
  not a fine-grained work-stealing task graph with many small
  heterogeneous units; a `Mutex<VecDeque<FileId>>` drained by a fixed
  number of `std::thread::scope`-spawned workers is fewer moving parts
  than standing up `rayon`'s global pool and its own heuristics for a
  shape this simple. `std::thread::scope` (stable since Rust 1.63)
  specifically lets worker threads borrow the shared, already-computed
  require graph, plan 48's cache, and a shared per-file result map by
  reference for the scope's duration, with no `Arc`/`'static` lifetime
  workaround needed — the existing pipeline
  (`crates/emerald-cli/src/main.rs`'s `id_effect::Effect` chain, driven
  through `run_blocking`) is itself synchronous, not async, so a
  synchronous scoped-thread pool is the option that composes with the
  existing pipeline shape rather than introducing a second concurrency
  model (async) alongside it. `rayon` remains the right tool if this
  scheduler ever needs true work-stealing across many small,
  heterogeneous, non-leveled tasks; that is not what a per-level,
  bounded-fan-out compile phase is, so it is declined here rather than
  reached for by default.
- **One real codegen-structure change this plan must make, beyond
  restructuring per-file units: each worker thread creates and owns its
  own `inkwell::context::Context` — a `Context` is never shared or
  passed between threads.** LLVM's own C API documents a `Context` as
  unsafe for concurrent use from more than one thread; `inkwell`'s
  `Context` wraps it with no `Send`/`Sync` implementation, matching that
  contract. `compile_to_object` today creates exactly one `Context` for
  the entire (already-merged) program (L4109); once codegen is per-file
  and per-thread, each thread's per-file codegen query must call
  `Context::create()` itself and build that file's own `Module` inside
  it — never reuse a `Context` created on another thread, and never
  hand a `Context`/`Module`/`Builder` across a thread boundary at all.
  A consequence, not a cost: each file's own `Module` now redeclares the
  runtime extern functions (`emerald_print_i64`, `emerald_alloc`, etc.)
  that `compile_to_object` declares once today — harmless, since these
  are `Linkage::External` declarations, not definitions, exactly the
  same shape multi-translation-unit C compilation already produces when
  two `.c` files both `#include` the same header.
- **Linking gains N object-file arguments, stays otherwise unchanged
  and stays single-threaded.** `crates/emerald-cli/src/main.rs`'s
  `link_stage` invokes `Command::new("cc").arg(&obj_path)...` with
  exactly one object file today; this plan changes that one call site
  to `.args(&obj_paths)` (one path per file in the require graph) ahead
  of the embedded runtime archive argument — a small, mechanical change
  to an existing call, not a new linking model. The system linker
  itself (`cc`/`ld`, one external process) is not parallelized by this
  plan; that is a real, disclosed non-goal, not an oversight — see Out
  of scope.
- **Cross-file citation: plan 26's multi-error-per-file reporting
  becomes multi-error-per-*graph* reporting.** `emerald_parser::
  parse_named` (plan 26, `2026-09-08T225500Z-plan-26-parser-error-
  recovery.md`) already reports every top-level `Item` boundary's
  syntax error within *one* file in a single pass (confirmed via
  `crates/emerald-cli/src/main.rs`'s own doc comment on `CliError::
  Parse(Vec<ParseError>)`). Because this plan parses every file in the
  graph concurrently, two or more files can each independently produce
  a `Vec<ParseError>` in the same wall-clock window; the driver must
  collect every file's `Vec<ParseError>` before reporting anything
  (never "first thread to finish wins") and print them in a
  deterministic order (by canonical file path, not thread-arrival
  order) so a given input always produces the same diagnostic ordering
  regardless of how the scheduler happened to interleave threads that
  run.
- **Cross-file citation: plan 35 (debug info) benefits from, and should
  target, the per-file module split this plan introduces — not the
  other way around.** Plan 35's own Decision log designs one DWARF
  compile-unit resolving "`sum.em` as the compile unit's source file,"
  implicitly one CU per compiled module; today's single combined
  `"emerald_module"` (see above) would force a multi-file program's
  debug info to misattribute every required file's code to whichever
  filename the combined module happens to carry. This plan's per-file
  `Context`/`Module` split gives plan 35 exactly the per-file DWARF
  compile-unit boundary it already wants, at no extra cost to either
  plan — if plan 35 lands first against today's single-module design,
  whoever executes this plan afterward must re-verify plan 35's debug
  info still resolves correctly per file once modules split, rather
  than assuming it does.
- **Cross-file citation: plan 17's `emerald-driver` is where this
  plan's scheduler actually lives, so plan 21 (LSP)/plan 47 (REPL/test
  framework) need no code changes of their own to benefit.** Both call
  through `emerald_driver::check`/`compile` (plan 17's extracted API,
  reused unchanged by plan 23's own design) rather than reimplementing
  the pipeline; putting the leveled scheduler inside
  `resolve_program`/`compile` itself, not in `emerald-cli`, means an
  LSP recheck-on-save or an `emerald test` run transparently gets
  leveled parallel compilation too. The one caller-visible knob this
  plan adds, `jobs: usize` (default `std::thread::available_parallelism()`),
  is worth a different default for an interactive single-document LSP
  recheck than for a batch CLI build — spinning up a multi-thread pool
  for a two- or three-file recheck-on-keystroke has overhead an
  editor's responsiveness budget may not want — so `emerald-lsp`
  passing `jobs: 1` explicitly (rather than inheriting the CLI's
  default) is this plan's one recommended, disclosed LSP-specific
  choice, not a requirement this plan enforces on plan 21 or 47's own
  future implementation.
- **Out of scope: fine-grained (non-leveled) dependency-aware
  scheduling**, where a file starts codegen the instant its own direct
  dependencies finish rather than waiting for its whole level — a real
  possible future refinement once this plan's simpler leveled scheduler
  is proven, not required to prove files with no path between them
  compile concurrently.
- **Out of scope: parallelizing the system linker invocation itself,
  cross-package/multi-crate parallel builds (plan 46's package-manager
  scope, building multiple independent packages concurrently rather
  than multiple files within one package), and distributed/multi-
  machine compilation.** All three are real, larger extensions of "more
  than one thing compiles at once" that this plan's single-package,
  single-process, per-file worker pool does not attempt.

## Leaf: leaf-require-graph-leveling

### 1. Context
- Why: plan 23's `resolve_program` is designed as a single recursive
  DFS that resolves and splices in one pass — it produces a merged
  `Program`, not a data structure this plan can query for "which files
  have no unresolved dependency." Nothing in this workspace computes
  topological levels today (verified: no `Item::Require`/driver/graph
  code exists anywhere yet).
- Target state: `crates/emerald-driver` (plan 17/23) gains
  `build_require_graph(entry: &Path) -> Result<RequireGraph,
  DriverError>` — a canonical-path-keyed adjacency map built by exactly
  plan 23's own traversal (parse each file, read its `Item::Require`
  list, canonicalize each resolved path, recurse) but *returning* the
  graph instead of splicing items, reusing plan 23's `visited`/`stack`
  cycle-detection and diamond-dedup verbatim — a cycle or a missing
  file still produces the same `DriverError::Require` plan 23 already
  defines. `compute_levels(graph: &RequireGraph) -> Vec<Vec<PathBuf>>`
  runs Kahn's algorithm over that graph only once it's confirmed
  acyclic (the cycle check itself already ran during graph
  construction — level computation never has to re-discover a cycle).

### 2. Acceptance Criteria
1. This plan's worked three-file example (`main.em` requiring `b.em`
   and `c.em`, which require nothing) produces `compute_levels` output
   `[[b.em, c.em], [main.em]]` (order within a level unspecified, level
   order fixed) — a real, structural proof the DAG is ranked correctly,
   not just that resolution still works.
2. Plan 23's own three acceptance tests still pass unmodified against
   the new `build_require_graph`-based implementation: the two-file
   real-run case, the diamond-dependency dedup case, and the two-file
   cycle case (still rejected, still naming both files, still bounded
   time) — proof this leaf preserves plan 23's contract rather than
   quietly changing its behavior while changing its internals.
3. A graph with a cycle never reaches `compute_levels` at all (verified
   by a test asserting `build_require_graph` itself returns
   `Err(DriverError::Require(_))` before any level-computation code
   path runs) — the concrete proof behind "confirm the whole graph
   acyclic up front," not just an assertion in prose.

### 3. File & Module Structure
- **Modify:** `crates/emerald-driver/src/lib.rs`
- **Create:** `crates/emerald-driver/tests/leveling.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass: leveling, plan 23 regression (2-file, diamond, cycle) | agent-claimed-locally |

---

## Leaf: leaf-parallel-parse-and-typecheck

### 1. Context
- Why: parsing every file the moment its bytes are available is safe
  today with zero new machinery (see Decision log's structural proof);
  type-checking is not, since a file's own typecheck needs its
  dependencies' already-computed signatures first.
- Target state: `resolve_program`'s new graph-based shape (leaf 1)
  parses every node in `RequireGraph` via `std::thread::scope`-spawned
  workers with no level ordering at all — one chunk of the *entire*
  file list per worker, bounded by `--jobs`. Type-checking then runs
  level by level: for level N, spawn up to `--jobs` workers draining a
  `Mutex<VecDeque<PathBuf>>` of that level's files, each worker calling
  plan 48's per-file typecheck query (which itself reads the already-
  cached signature results of that file's dependencies — all in prior,
  already-finished levels) and writing its own result into a shared,
  `Mutex`-guarded per-file result map; the whole pool is joined (a hard
  barrier) before level N+1's workers are spawned.

### 2. Acceptance Criteria
1. This plan's worked three-file example, run with `--jobs 2`: all
   three files parse successfully before any type-checking begins (an
   assertable ordering property, not just an end-to-end pass), `b.em`
   and `c.em`'s typecheck queries both run before `main.em`'s begins
   (asserted via plan 48's cache population order, or via the same
   trace instrumentation leaf 3 introduces), and the whole program
   still type-checks `Ok(())`.
2. A version of the three-file example where `main.em` references a
   function `b.em` does *not* actually export is rejected with the same
   undefined-symbol diagnostic single-file type-checking already
   produces — proof that level-ordering isn't merely fast, it's
   necessary, and this leaf didn't accidentally make cross-level
   visibility unconditionally permissive to get parallelism.
3. Regression: every existing multi-file and single-file example in
   this repository still type-checks identically to before this leaf
   (a single-file program is one level containing one file — no
   behavior change for the common case).

### 3. File & Module Structure
- **Modify:** `crates/emerald-driver/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass: full-graph parallel parse, leveled typecheck ordering, cross-level visibility rejection, regression | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-parallel-codegen-and-jobs-flag

### 1. Context
- Why: codegen is still a single combined `Context`/`Module` over one
  merged `Program` today (see Decision log) — the one stage this plan
  cannot parallelize without an explicit structural change, and the one
  stage where an LLVM-specific thread-ownership rule (a `Context` is
  never shared across threads) must be honored on top of whatever
  per-file query shape plan 48 provides.
- Target state: each file's codegen query creates its own
  `inkwell::context::Context`, builds that file's own `Module`
  (declaring the runtime externs and only that file's own
  functions/classes/modules — referencing a dependency's function by
  name and an `External` declaration, the same cross-TU pattern C
  already uses), and emits its own `.o` object file via the same
  `TargetMachine`-based emission `compile_to_object` already performs
  (L4094-4112) — but once per file, on whatever thread that file's
  codegen query runs on. Codegen is scheduled with the same leveled,
  `std::thread::scope`-based worker pool as leaf 2's type-checking
  (kept as one scheduling shape — see Decision log), bounded by a new
  `--jobs N` flag (`emerald-cli` argument; `emerald_driver::compile`
  gains a `jobs: usize` parameter, defaulting to
  `std::thread::available_parallelism()`). `link_stage`
  (`crates/emerald-cli/src/main.rs`) is updated to pass every file's
  `.o` path to `cc`, not just one.

### 2. Acceptance Criteria
1. This plan's worked three-file example, compiled and linked via
   `emerald-cli --jobs 2 examples/parallel/main.em -o main`, runs and
   prints `30` — real, executed, end-to-end proof spanning three
   separately-emitted object files linked into one binary.
2. With `EMERALD_TEST_COMPILE_DELAY_MS=200` set (a test-only seam: each
   file's codegen query sleeps that many milliseconds before compiling,
   purely so wall-clock timing can carry a real signal over toy-program
   compile times), and `EMERALD_TRACE_COMPILE=1` set (each worker
   records `(file, thread::current().id(), start: Instant, end:
   Instant)` into a shared, `Mutex`-guarded `Vec`, dumped to a file the
   test reads): running the worked example with `--jobs 2` produces
   `b.em` and `c.em` records with genuinely overlapping `[start, end]`
   windows and two distinct thread IDs, and total level-0 wall time
   under ~300ms; running it with `--jobs 1` produces non-overlapping
   windows, an identical thread ID for both records, and total level-0
   wall time over ~350ms — the concrete, timed, non-flaky proof that
   `--jobs` genuinely controls concurrency rather than being a
   no-op flag.
3. Codegen for a file never constructs more than one `inkwell::
   context::Context` and never moves one across a thread boundary
   (verified structurally: each codegen-query call site creates its own
   `Context` locally and neither returns it nor stores it in any
   shared/`static` location).
4. Regression: every existing single-file example still compiles,
   links, and runs identically (`--jobs` defaulting to
   `available_parallelism()` for a one-file/one-level program still
   produces exactly one object file, unchanged from today's
   `compile_to_object` output modulo the module name).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`,
  `crates/emerald-driver/src/lib.rs`, `crates/emerald-cli/src/main.rs`
- **Create:** `examples/parallel/main.em`, `examples/parallel/b.em`,
  `examples/parallel/c.em`, `crates/emerald-driver/tests/parallel_jobs.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test (timed concurrency proof) | `cargo test -p emerald-driver --test parallel_jobs` | overlapping windows + distinct thread IDs under `--jobs 2`; serialized + single thread ID under `--jobs 1` | agent-claimed-locally |
| Real end-to-end run | `emerald-cli --jobs 2 examples/parallel/main.em -o /tmp/main && /tmp/main` | prints `30` | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass, no regression | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
