---
name: WebAssembly (WASI) Codegen Target
overview: "`emerald build --target wasm32-wasi` as a second selectable LLVM target triple alongside today's implicit native one — reusing the same AST-to-LLVM-IR codegen path wholesale and touching only the genuinely target-specific seams: target/triple selection (today hardcoded to the host), the C runtime's pthread-based worker pool (which has nothing to spawn onto under WASI's single-threaded model, so actors degrade to sequential mailbox draining), the link step's compiler/archive choice, and a disclosed no-arbitrary-FFI restriction WASI's sandboxing model forces on this target alone."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-target-triple-and-selection
    content: "Replace compile_to_object_impl's hardcoded Target::initialize_native/get_default_triple/get_host_cpu_name/get_host_cpu_features with a CodegenTarget{Native,Wasm32Wasi} parameter threaded from the CLI down; verify LLVM 21 + inkwell 0.10's actual WebAssembly target support and devenv.nix's llvmPackages_21.llvm build"
    status: pending
  - id: leaf-wasi-runtime-and-sequential-actors
    content: "runtime/emerald_runtime.c gains a __wasi__-guarded sequential fallback for the pthread worker pool (plan 55's mailbox/enqueue logic kept, spawning nothing); build.rs cross-compiles a second archive for wasm32-wasip1 via a WASI sysroot; malloc/calloc-backed allocation and stdio verified to need no change"
    status: pending
  - id: leaf-cli-target-flag-and-linking
    content: "emerald build gains --target wasm32-wasi (plan 46's cmd_build/flag-scanning convention, default unchanged); emerald-driver's link() picks the wasm32-wasi archive and a WASI-capable compiler/flags instead of cc -no-pie"
    status: pending
  - id: leaf-spec-and-restrictions-doc
    content: "spec/COMPILER.md addendum: per-target restrictions table — sequential actors, no arbitrary C FFI, wasmtime as the disclosed test-time tool dependency — and the declined-work list (WASM threads proposal, SharedArrayBuffer)"
    status: pending
  - id: leaf-wasm-worked-proof
    content: "benchmarks/sum/sum.em compiled and linked under both targets; native binary run directly, wasm32-wasi module run under wasmtime; byte-for-byte stdout match asserted in a real crates/emerald-driver integration test"
    status: pending
isProject: false
---

# Plan 64 — WebAssembly (WASI) Codegen Target

This is plan 64 of the 58-64 theoretical-maximum batch — the follow-up
that asked, ignoring maturity/ecosystem/stability, what Emerald's
theoretical technical ceiling looks like against C/C++/Rust/Python/
Ruby/TypeScript/Go/Haskell/Elixir combined, holding the project's
identity constraints fixed (no `method_missing`/`eval`/`send`/
reflection, no mixins/open classes/monkey-patching, no dynamic/virtual
dispatch or vtables, no tracing garbage collector). That debate named
TypeScript's one genuine structural advantage over a native-compiled
language as "runs in the browser" — not a language-level feature at
all, a *codegen-target* one — and identified a second LLVM target
triple, `wasm32-wasi`, as the way to close it without touching the
language, the grammar, sema, or a single AST node. Like every other
post-v1 plan in this repository, it is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
and does not touch that file or any other plan file. It depends on
plan 16 (`codegen-backend-bakeoff`, `2026-09-08T212216Z`) and
`consolidate-llvm-backend` (`2026-09-08T215429Z`) for the LLVM/inkwell
wiring this plan reuses rather than re-litigates, on plan 06
(`milestone1-codegen`) for the AST-to-object-file pipeline shape, and
on plan 55 (`scheduler-and-message-passing`, `2026-09-09T125000Z`) for
the concrete threading mechanism this plan must degrade under WASI —
all three read in full this session, not assumed from memory.

This plan's core claim, stated up front because every leaf below exists
only to defend or narrow it: **the overwhelming majority of
`crates/emerald-codegen/src/lib.rs` (13,964 lines, verified this
session) requires zero change to target `wasm32-wasi`.** Every
`build_expr`/`build_stmt`/`build_method_call`/class-layout/enum-layout
function emits ordinary LLVM IR — `iadd`, `load`, `store`, `call`,
`br` — through `inkwell`'s target-agnostic `Builder` API, and LLVM's
own instruction selection lowers that same IR differently per target
machine without codegen ever naming a target itself, anywhere, except
in exactly one function (see `leaf-target-triple-and-selection`). This
plan's entire job is finding and closing that one seam, plus the seams
one layer below codegen (the C runtime, the linker invocation) that
were never asked to be portable because nothing but `x86_64-unknown-
linux-gnu` (the only target `workspace.metadata.dist` — plan 27 —
ships today) has ever compiled through them.

Concrete proof this plan targets — the existing benchmark program
`benchmarks/sum/sum.em` (plan 15/16's own accumulator loop, read in
full this session: a `while`-loop summing `0..10000000` into `total`,
then `puts total`; no class, no actor, no FFI, no exception — the
plainest possible "does the target actually work" proof), compiled and
run under both targets from the *same* source file with no `.em`-level
change:

```
$ emerald build
   Compiling sum v0.1.0 (sum.em)
    Finished build: ./sum
$ ./sum
49999995000000

$ emerald build --target wasm32-wasi
   Compiling sum v0.1.0 (sum.em)
    Finished build: ./sum.wasm
$ wasmtime run ./sum.wasm
49999995000000
```

Expected: byte-for-byte identical stdout, `49999995000000\n`, from two
genuinely different object files (native ELF, WASM module) produced by
the same AST walked through the same codegen functions, linked by two
different toolchains, run by two different execution environments. Any
divergence — a different digit, a crash, a hang — is this plan's own
failure to prove its core claim, not an acceptable "close enough."

## Decision log

- **The one real hardcoded-native-target seam, found by reading the
  actual target-machine setup, not assumed.** `crates/emerald-codegen/
  src/lib.rs`'s `compile_to_object_impl` (verified this session,
  `L10558`-`L10570`) does exactly this and nothing else target-related
  anywhere in the file:
  ```rust
  Target::initialize_native(&InitializationConfig::default())...;
  let triple = TargetMachine::get_default_triple();
  let target = Target::from_triple(&triple)...;
  let target_machine = target.create_target_machine(
    &triple,
    &TargetMachine::get_host_cpu_name().to_string(),
    &TargetMachine::get_host_cpu_features().to_string(),
    OptimizationLevel::Aggressive, RelocMode::Default, CodeModel::Default,
  )...;
  ```
  Every one of these four calls is native-target-specific by name:
  `initialize_native` only registers the host's own backend with LLVM
  (WebAssembly's backend is a separate, independently-registered
  target component); `get_default_triple` returns the *host's* triple
  unconditionally, ignoring any target the caller might want; `get_
  host_cpu_name`/`get_host_cpu_features` query the running machine's
  own CPU — meaningless, and actively wrong, for a cross-compiled
  target that has no "host CPU" at all. This is the entire fix's
  surface area: `leaf-target-triple-and-selection` replaces these four
  lines with a target-dispatched equivalent; nothing else in the file's
  other ~13,900 lines names a target, an architecture, or a pointer
  width anywhere (verified — see the next bullet for the one place that
  looked like it might, and wasn't).
- **`build_class_layout`/`build_enum_layout`'s field offsets are
  already target-portable, verified rather than assumed — a fixed
  8-byte-per-field convention, not a queried pointer size.**
  `build_class_layout` (`L361`-`L388`) increments every field's offset
  by a literal `8`, regardless of that field's `ValKind` (`Int64`,
  `Float64`, or a class/actor pointer all get one 8-byte slot); `build_
  enum_layout` (`L271`-`L286`) sizes a variant payload as `8 + 8 *
  max_fields`. Neither ever calls `target_data.get_pointer_byte_size()`
  or any other query of the *actual* target's pointer width — the `8`
  is this language's own fixed word-size convention (the same one
  plan 55's `EmeraldMessage.argv[16]`/`int64_t` mailbox slots already
  use), asserted once in source, not derived from `std::mem::size_of::
  <*const u8>()` on the compiling host or from LLVM's target machine at
  all. `wasm32-wasi`'s pointers are physically 32 bits, not 64 — but
  because this codebase's struct layout never asked the host or the
  target what its pointer size *is*, that fact is simply never
  consulted, and every field keeps its portable 8-byte slot regardless
  of target (a pointer-kind field wastes 4 bytes of padding under
  `wasm32-wasi` it wouldn't waste under `x86_64`; a real, minor,
  disclosed inefficiency, not a correctness bug — `field_ptr`'s own
  byte-offset GEP arithmetic (`L4428`-`L4440`) is unaffected either
  way). **This plan does not touch `build_class_layout`/`build_enum_
  layout` at all** — stated here because the task's own framing
  specifically asked this question, and the honest answer, found by
  reading the actual code rather than assuming layout code is always
  pointer-size-fragile, is "no fix needed."
- **LLVM 21 / inkwell 0.10 (`llvm21-1` feature) — verified against
  `crates/emerald-codegen/Cargo.toml` and `devenv.nix`, not assumed
  from plan 16's own record.** `Cargo.toml`: `inkwell = { version =
  "0.10", features = ["llvm21-1"] }`. `devenv.nix`: `LLVM_SYS_211_
  PREFIX = "${pkgs.llvmPackages_21.llvm.dev}"` and `pkgs.llvmPackages_
  21.llvm` in `packages` — unchanged since plan 16 landed it, `consoli-
  date-llvm-backend` having removed only Cranelift and
  `emerald-codegen-llvm`'s separate-crate split, not this wiring.
  WebAssembly is one of LLVM's upstream in-tree target backends
  (`WebAssembly`, alongside `X86`, `ARM`, etc., in LLVM's own
  `lib/Target/` layout) and nixpkgs' `llvmPackages_21.llvm` builds with
  its default `LLVM_TARGETS_TO_BUILD` list — every upstream target,
  `WebAssembly` included — unless a downstream override trims it, and
  `devenv.nix` sets no such override (verified — no `targetsToBuild`/
  `withAllTargets`/similar attribute appears anywhere in this file).
  `inkwell`'s own target-init surface is generated per LLVM target
  family from LLVM-C's `LLVMInitializeWebAssemblyTarget*` macros the
  same way `initialize_native`/`initialize_x86` are, so `Target::
  initialize_webassembly` is expected to exist on `inkwell` 0.10 —
  **`leaf-target-triple-and-selection`'s own first acceptance criterion
  is to confirm this directly against `inkwell` 0.10's real `targets.rs`
  and, independently, `llvm-config --targets-built | grep -i
  webassembly` inside `devenv shell`, before writing any dependent
  code** — the same "verify against real source, not a stale summary"
  discipline plan 55 applied to plan 54's contract and plan 16 applied
  to its own inkwell smoke test, not a second assumption stacked on the
  first.
- **The exact WASI triple string is this plan's own remaining open
  question, and is treated as one.** LLVM's `Triple` parser accepts an
  OS component of `wasi` (e.g. `wasm32-unknown-wasi`) as well as the
  two-component `wasm32-wasi` short form; upstream tooling has also
  shifted the canonical spelling toward `wasm32-wasip1` (WASI preview
  1) as newer preview-2 tooling emerges. This plan writes `wasm32-wasi`
  throughout as its working target string (matching the `--target`
  flag's own user-facing spelling and WASI preview 1's actual runtime
  contract, the only one this plan targets — no preview 2/component-
  model support is in scope), and `leaf-target-triple-and-selection`'s
  acceptance criteria require confirming `TargetTriple::create("wasm32-
  wasi")` round-trips through `Target::from_triple` under this
  workspace's real LLVM 21 build before that string is relied on
  anywhere else — a live check, not an assumption carried from this
  document into code.
- **Concurrency degrades to sequential mailbox processing, stated
  plainly, because plan 55's actual mechanism has nothing to run on.**
  Plan 55 (read in full this session) is explicit about what it built:
  "a fixed pool of M OS worker threads, each pulling actor messages off
  thread-safe queues" — real `pthread_create`-spawned OS threads
  (`emerald_worker_pool_start`, `runtime/emerald_runtime.c` `L849`-
  `L872`, sized from `sysconf(_SC_NPROCESSORS_ONLN)` or `EMERALD_
  WORKERS`), a shared `pthread_mutex_t`/`pthread_cond_t`-guarded
  runnable queue, and a per-actor mailbox mutex — plan 55's own
  Decision log already calls this "N actors over M OS threads," a
  disclosed scope-down from "M:N green-thread scheduler" for the honest
  reason that this codebase has no coroutine/stack-switching primitive.
  `wasm32-wasi` (WASI preview 1, the only WASI this plan targets) has
  no `pthread_create` at all — its libc ships no working thread-
  spawning implementation in the single-threaded configuration this
  plan uses (a *separate* target, `wasm32-wasip1-threads`, pairs a
  threads-enabled wasi-libc with the WASM threads proposal's shared-
  linear-memory + atomics primitives and needs a host runtime built
  with that proposal enabled — explicitly not this plan's target, see
  below). So plan 55's own mechanism — M real OS threads racing to pop
  a shared runnable queue — has exactly nothing to spawn onto under
  this target: M is not "small," it is **1, and cannot be otherwise**,
  a harder ceiling than plan 55's own `EMERALD_WORKERS=1` escape hatch
  (which still spawns one real pthread; wasm32-wasi has none to spawn
  at all). `leaf-wasi-runtime-and-sequential-actors` compiles the
  `__wasi__`-guarded branch of `emerald_actor_enqueue`/`emerald_worker_
  pool_drain_and_join` to run every enqueued message's trampoline
  **synchronously, to completion, on the single available execution
  context**, in strict per-actor FIFO order (identical ordering
  guarantee to plan 55's own mailbox — only the "multiple actors run
  literally simultaneously" half of plan 55's own two-part worked proof
  stops holding, not the "per-actor ordering is correct" half). This is
  the same honesty plan 55 already applied to "M:N scheduler" →
  "N actors over M OS threads" — this plan states the next, sharper
  version of that same fact for this one target: **M=1 under
  `wasm32-wasi`, correctness preserved, concurrency gone.** Not
  discovered as a shortfall after the fact — decided and disclosed
  here.
- **Declined, explicitly: the WASM threads proposal + `SharedArray-
  Buffer`-style multi-threading under WASI.** A real fix to the
  M=1 ceiling above exists on paper — `wasm32-wasip1-threads` (or
  preview-2's equivalent), a wasi-libc built with real
  `pthread_create`-over-shared-linear-memory support, atomics-enabled
  LLVM codegen (`-matomics -mbulk-memory`, plus every runtime allocation
  in `runtime/emerald_runtime.c` re-audited for the shared-memory
  aliasing hazards that a real multi-"thread" WASM module introduces),
  and a host runtime built with the threads proposal turned on. This is
  a substantial, separate, well-scoped project — a new WASI sub-target,
  a new wasi-libc variant, and a genuine C-runtime concurrency re-audit
  — not a small extension of this plan's own sequential fallback. This
  plan declines it explicitly, real future work named and disclosed,
  the same category of decision as plan 55's own declined work-stealing
  and declined mailbox back-pressure.
- **Declined, explicitly: arbitrary C-library FFI under `wasm32-wasi`.**
  Verified this session (directory listing through plan 57, the newest
  plan on disk at authoring time): no plan numbered 58 or below adds a
  user-facing `extern`/native-library-linking construct to Emerald's
  grammar — the only `extern "C"` declarations anywhere in this
  compiler are codegen-internal calls it emits itself into its own
  fixed, compiler-authored `runtime/emerald_runtime.c` function set
  (`declare_exception_runtime_funcs`, `declare_actor_runtime_funcs`,
  etc. — never a source-level declaration a user's own `.em` file can
  write). So there is no existing capability for this plan to actively
  restrict; its contribution is a **preemptive scope boundary** for
  whichever future plan in this batch (the task's own framing calls it
  "plan 59-style") eventually adds general C-library FFI: whatever that
  plan builds for native targets, it must not extend to `wasm32-wasi`.
  The reason is WASI's own sandboxing model, not an arbitrary
  restriction of this plan's choosing: a WASI module's only interface
  to the outside world is the fixed `wasi_snapshot_preview1` import set
  (fd read/write, clock, random, and a handful of others) — there is no
  `dlopen`, no linking against an arbitrary native `.so`/`.a` that
  assumes syscalls or an ABI beyond what those WASI imports expose.
  Even a *statically*-linked arbitrary C library would need every one
  of its own syscalls (file descriptors beyond WASI's capability-
  scoped set, raw `mmap`, threads, sockets outside WASI's own narrow
  socket extensions) ported to WASI's import set or simply not called
  — real porting work belonging to whatever C library a user wants, not
  something this compiler can paper over generically. `leaf-spec-and-
  restrictions-doc` records this as a real, disclosed per-target
  restriction in `spec/COMPILER.md`, not a silent gap a user discovers
  by a failed link.
- **`wasmtime` as a disclosed test-time (and end-user run-time) tool
  dependency — reasonable, and consistent with this project's existing
  precedent.** This compiler already requires `cc` on the host to link
  every native build (`crates/emerald-driver/src/lib.rs`'s `link`,
  `L111`-`L134`, shells directly to `Command::new("cc")`) and `git` for
  plan 46's dependency resolution — neither is vendored, both are
  assumed present exactly like `cargo` itself. A compiled `wasm32-wasi`
  module needs a real WASI host to execute at all (a `.wasm` file is
  not directly runnable by the OS the way a native ELF/Mach-O binary
  is) — `wasmtime` is a real, actively maintained, Bytecode-Alliance-
  governed WASI runtime, the same one the WASI ecosystem itself treats
  as its reference implementation. `leaf-wasm-worked-proof`'s own
  integration test shells to `wasmtime run` the same way `link`'s own
  tests already shell to the linked binary directly — one more disclosed
  external tool, not a new category of dependency this project hasn't
  already accepted.
- **Runtime C code: allocation and string/stdio paths need no
  reimplementation; only the pthread-based worker pool does — verified
  against the real file, not assumed uniformly "probably fine."**
  `runtime/emerald_runtime.c` (1,109 lines, read this session):
  `emerald_alloc`/`emerald_alloc_zeroed`/`emerald_region_create`/
  `emerald_region_alloc` (`L62`-`L74`, `L117`-`L136`) call straight
  through to `malloc`/`calloc` plus a plain `__atomic_fetch_add`
  byte-counter — both are ordinary C99/C11 constructs a WASI-targeting
  `clang` and wasi-libc's own allocator support natively, no `#ifdef`
  needed. Every string/stdio helper (`emerald_string_*`, `emerald_
  file_write`, `fopen`/`fread`/`fclose` at `L508`-`L511`) is plain ISO C
  stdio/string/ctype calls — `wasi-libc` implements the full C stdio
  surface over WASI's own `fd_read`/`fd_write`/`path_open` imports, so
  these "just work" given a WASI-targeting compiler and sysroot, no
  source change. The one real reimplementation is `#include <pthread.h>
  ` / `<unistd.h>` (`L20`-`L21`) and everything built on them: `sysconf
  (_SC_NPROCESSORS_ONLN)`, `pthread_mutex_t`/`pthread_cond_t`, `pthread_
  create`/`pthread_join` inside `emerald_worker_pool_start`/`emerald_
  worker_pool_drain_and_join` (`L849`-`L900`) — none of it exists in
  WASI preview 1's libc. `_Thread_local` (plan 55's own fix to `emerald_
  handler_stack`, the exception-handler stack) needs **no** change
  either: thread-local storage is a per-instance linear-memory
  convention wasi-libc supports even in a single-"thread" build (there
  is exactly one TLS block, for the one execution context that exists)
  — it behaves identically to a plain global under `wasm32-wasi`, the
  same "no observable difference from a single thread's point of view"
  property plan 55's own Decision log already noted for the native
  single-threaded case.
- **`setjmp`/`longjmp` (plan 38's exception mechanism) is a real, open
  verification point this plan does not need to close.** `runtime/
  emerald_runtime.c` includes `<setjmp.h>` and plan 38's `raise`/
  `rescue` lowers to `setjmp`/`longjmp` frames. WASI-targeting `clang`
  toolchains support `setjmp`/`longjmp` (wasi-sdk ships a working
  implementation), but the exact mechanism — a native SjLj lowering
  pass vs. an Emscripten-style outlining transform — is toolchain-
  version-dependent and this plan's own worked proof (`sum.em`, no
  `raise`/`rescue` anywhere) never exercises it. Stated here as a real,
  disclosed open question for whichever future plan first compiles an
  exception-using program under `--target wasm32-wasi` — not silently
  assumed to work, and not blocking this plan, whose own acceptance
  criteria never require it.

## Leaf: leaf-target-triple-and-selection

### 1. Context
- Why: the one genuine hardcoded-native-target seam in codegen (see
  Decision log) — everything downstream of it (runtime, linking,
  CLI) is meaningless to build without a real second target machine to
  compile against.
- Target state: `crates/emerald-codegen/src/lib.rs` gains `pub enum
  CodegenTarget { Native, Wasm32Wasi }`; `compile_to_object_impl` (and
  its public callers — `compile_to_object`, `compile_to_object_with_
  debug_info`, `compile_program`-adjacent entry points in `emerald-
  driver`) takes a `target: CodegenTarget` parameter (default `Native`
  for every existing caller — no behavior change for a plain `emerald
  build`). The `Native` branch keeps today's four calls verbatim.
  The `Wasm32Wasi` branch calls `Target::initialize_webassembly(&
  InitializationConfig::default())`, builds `TargetTriple::create(
  "wasm32-wasi")`, and passes empty CPU name/features strings (`""`,
  `""` — there is no "host CPU" for a cross target; WASM has no
  `-mcpu`-equivalent concept LLVM's WebAssembly backend consults the
  way x86 consults `-march`) to `create_target_machine`. `devenv.nix`
  needs no new packages for this leaf specifically (LLVM 21 already
  present) — only the verification below.

### 2. Acceptance Criteria
1. `llvm-config --targets-built` (inside `devenv shell`) lists
   `WebAssembly` — a real, executed check, not assumed from nixpkgs'
   general reputation for building all targets.
2. `inkwell` 0.10's own `targets.rs` (read directly, not summarized)
   confirms `Target::initialize_webassembly` exists with the same
   signature shape as `initialize_native`.
3. `TargetTriple::create("wasm32-wasi")` round-trips through `Target::
   from_triple` without error under this workspace's real LLVM 21
   build (a throwaway smoke test, the same discipline plan 16's own
   inkwell smoke test used, deleted after use, not committed) — if the
   accepted spelling turns out to differ (e.g. `wasm32-unknown-wasi`),
   this criterion is where that's discovered and corrected, not
   assumed away.
4. A minimal program (`sum.em`) compiled with `target: Wasm32Wasi`
   produces a real `.o`/`.wasm` object file inkwell reports success
   for, verified via `file <output>` reporting a WebAssembly object,
   not merely "no error returned."
5. Regression: every existing native-target test in `crates/emerald-
   codegen`/`crates/emerald-driver` still passes unchanged — `target:
   Native`'s branch is byte-for-byte the same four calls as today.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`compile_to_object_
  impl` and its public entry points)
- **Modify:** `crates/emerald-driver/src/lib.rs` (`codegen_stage`/
  `compile_program` signatures gain the target parameter, threaded
  through, default `Native`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen -p emerald-driver` | clean | agent-claimed-locally |
| Test (native regression) | `cargo test --workspace` | all pass, unchanged | agent-claimed-locally |
| Target verification | `llvm-config --targets-built \| grep WebAssembly` | present | agent-claimed-locally |

---

## Leaf: leaf-wasi-runtime-and-sequential-actors

### 1. Context
- Why: a compiled `wasm32-wasi` object is useless without a `wasm32-
  wasi`-compiled runtime archive to link against, and today's runtime
  hard-depends on `pthread`/`unistd` for exactly the worker pool that
  cannot exist under this target (see Decision log).
- Target state: `runtime/emerald_runtime.c` wraps `#include <pthread.h
  >`/`<unistd.h>` and every `pthread_*`/`sysconf` call inside `#ifndef
  __wasi__` (clang predefines `__wasi__` when targeting `wasm32-wasi`);
  an `#else` branch gives `emerald_worker_pool_start` a no-op body,
  `emerald_actor_enqueue` appends to the target actor's mailbox with no
  locking (nothing else can run concurrently — no mutex needed) and,
  when the actor transitions idle→runnable, appends it to a plain
  singly-linked runnable list (no shared-queue mutex needed either),
  and `emerald_worker_pool_drain_and_join` becomes a plain loop: pop
  the next runnable actor, run its next mailbox message's trampoline to
  completion, re-append the actor if its mailbox still has messages,
  repeat until the runnable list is empty. `crates/emerald-driver/
  build.rs` gains a second `cc::Build::new().target("wasm32-wasip1")...
  .compile("emerald_runtime_wasm32_wasi")` invocation (the `cc` crate's
  own documented cross-compilation support, honoring `CC_wasm32_wasip1`
  /`AR_wasm32_wasip1` env vars pointing at a WASI-capable `clang`/`llvm-
  ar` and a WASI sysroot) — gated behind detecting that toolchain is
  actually present (a missing WASI toolchain skips embedding the wasm
  archive and `--target wasm32-wasi` fails with a clear "no WASI
  toolchain configured" error at build time, not a silent empty
  archive); `devenv.nix` gains the WASI-targeting toolchain package
  (verify the exact nixpkgs attribute — e.g. `pkgs.wasi-sdk` if present
  in this nixpkgs pin, or `pkgs.llvmPackages_21.clang` invoked with
  `--target=wasm32-wasi --sysroot=<wasi-libc path>` if not — a real
  verification point, not assumed) and `pkgs.wasmtime`.

### 2. Acceptance Criteria
1. `runtime/emerald_runtime.c` compiles cleanly for both `x86_64-
   unknown-linux-gnu` (unchanged archive, unchanged tests) and
   `wasm32-wasip1` (new archive) from the same source file.
2. A two-actor ping-pong program (plan 55's own worked example,
   adapted) compiled under `--target wasm32-wasi` and run under
   `wasmtime` produces the exact same 9-line output plan 55's native
   build produces — proving the sequential fallback preserves per-
   actor FIFO ordering, the correctness half of plan 55's own proof.
3. `EMERALD_WORKERS` is a no-op (accepted, ignored) under this target
   — documented, not silently misleading a user into thinking it
   controls concurrency that cannot exist here.
4. Regression: every existing native-target runtime test (string ops,
   `raise`/`rescue`, actor/supervisor tests) still passes unchanged —
   the `#ifndef __wasi__` branch is untouched code, byte-for-byte.
5. `emerald_alloc`/`emerald_region_*`/string/stdio helpers require zero
   source changes — verified by their `wasm32-wasip1` compile succeeding
   with no `#ifdef` touching any of `L62`-`L569`.

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c` (`__wasi__`-guarded worker-
  pool branch, `L20`-`L21`, `L705`-`L761`, `L849`-`L907`)
- **Modify:** `crates/emerald-driver/build.rs` (second `cc::Build`
  cross-compile invocation, new `EMERALD_RUNTIME_ARCHIVE_WASM32_WASI`
  env var mirroring the existing one)
- **Modify:** `devenv.nix` (WASI toolchain package, `pkgs.wasmtime`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Native build unaffected | `cargo build --workspace` | clean | agent-claimed-locally |
| WASI archive builds | `cargo build -p emerald-driver` (WASI toolchain present) | `libemerald_runtime_wasm32_wasi.a` produced | agent-claimed-locally |
| Runtime regression | `cargo test --workspace` | all pass, incl. sequential ping-pong | agent-claimed-locally |

---

## Leaf: leaf-cli-target-flag-and-linking

### 1. Context
- Why: plan 46's `emerald build` (`crates/emerald-cli/src/main.rs`
  `cmd_build`, `L344`-`L406`) has no target selection at all today —
  every build is implicitly native — and `emerald-driver`'s `link`
  (`L111`-`L134`) hardcodes `cc -no-pie <obj> <native archive> -o
  <out>`, both real gaps this leaf closes.
- Target state: `cmd_build` gains a `target_requested(args)` helper
  mirroring `verbose_cache_requested`/`jobs_requested`'s own existing
  flag-scanning convention (`L154`-`L178`) — scans `args` for `--target
  wasm32-wasi` (or `--target=wasm32-wasi`), returning `CodegenTarget::
  Wasm32Wasi`, defaulting to `CodegenTarget::Native` when absent (every
  existing invocation, including plan 46's own bare-`emerald-cli` legacy
  single-file mode, is unaffected — the flag is purely additive).
  `cmd_build`'s output path gains a `.wasm` extension under this
  target (`app.wasm` instead of bare `app`, matching a WASM module's
  own conventional extension — `wasmtime run app.wasm` vs. `./app`
  matches this plan's own worked-proof invocation). `emerald-driver`'s
  `link` gains a `target: CodegenTarget` parameter: the `Native` branch
  is today's unchanged `cc -no-pie <obj> <RUNTIME_ARCHIVE> -o <out>`;
  the `Wasm32Wasi` branch invokes the WASI-capable compiler (the same
  one `build.rs` used to cross-compile the runtime, resolved the same
  way) against `<obj>` plus `RUNTIME_ARCHIVE_WASM32_WASI`, **without**
  `-no-pie` (an ELF/PIC-specific flag with no WASM equivalent — WASM
  has no position-independent-executable concept to disable).

### 2. Acceptance Criteria
1. `emerald build` (no flag) produces byte-identical output to today,
   pre-this-plan — the additive-only guarantee stated above, verified
   directly, not assumed from "the branch is separate."
2. `emerald build --target wasm32-wasi` on `sum.em` produces a real
   `.wasm` file `file` identifies as a WebAssembly module.
3. An unrecognized `--target` value (anything but the two supported
   strings) is a real, reported CLI error, not a silent fallback to
   native.
4. `emerald run --target wasm32-wasi` is explicitly rejected with a
   clear error (`cmd_run`, `L408`-`L415`, directly invokes the built
   binary via `Command::new` — a `.wasm` module is not a directly-
   executable native binary; running it needs `wasmtime`, out of scope
   for `emerald run`'s own existing "invoke the linked output" contract
   this plan does not redesign) rather than a confusing native-launcher
   failure.

### 3. File & Module Structure
- **Modify:** `crates/emerald-cli/src/main.rs` (`cmd_build`, `cmd_run`,
  new `target_requested` helper)
- **Modify:** `crates/emerald-driver/src/lib.rs` (`link`/`link_stage`,
  `codegen_stage`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli -p emerald-driver` | clean | agent-claimed-locally |
| Native regression | `emerald build` on every `examples/*.em` | unchanged output | agent-claimed-locally |
| Wasm build | `emerald build --target wasm32-wasi` on `sum.em` | produces valid `.wasm` | agent-claimed-locally |

---

## Leaf: leaf-spec-and-restrictions-doc

### 1. Context
- Why: this plan's two "declined, explicitly" decisions (WASM threads/
  `SharedArrayBuffer` multi-threading, arbitrary C-library FFI) and its
  one disclosed degradation (sequential actors) need a real, findable
  artifact — matching plan 16's own precedent of a dated `spec/
  COMPILER.md` addendum recording a backend decision, not just this
  plan document's own prose.
- Target state: `spec/COMPILER.md` gains a dated addendum (mirroring
  plan 16's own addendum format) under a new "Codegen targets" section:
  a two-row table (`x86_64-unknown-linux-gnu` / `wasm32-wasi`) listing,
  per target, concurrency model (real OS threads / sequential-only),
  FFI (arbitrary C linking / none), and required external tooling (`cc`
  / a WASI toolchain + `wasmtime`) — plus a short "declined" list citing
  this plan's own Decision log bullets by name.

### 2. Acceptance Criteria
1. The addendum is dated and cites this plan's own file path, the same
   citation discipline plan 16's addendum used for its own benchmark
   evidence.
2. The restrictions table is reachable from `spec/COMPILER.md`'s own
   table of contents/section list (not buried in prose no later reader
   would find).
3. No claim in the table is unverified by this plan's own other
   leaves — the concurrency row cites `leaf-wasi-runtime-and-sequential-
   actors`'s real test, the FFI row cites the Decision log's own
   directory-listing verification.

### 3. File & Module Structure
- **Modify:** `spec/COMPILER.md`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Doc review | manual read against every other leaf's real acceptance criteria | consistent, no unverified claim | agent-claimed-locally |

---

## Leaf: leaf-wasm-worked-proof

### 1. Context
- Why: every other leaf in this plan is infrastructure; this leaf is
  the actual proof the infrastructure works — a real byte-for-byte
  stdout match between a native build and a `wasm32-wasi` build of the
  same, unmodified source file, the standard this plan's own worked
  example at the top of this document sets.
- Target state: a new integration test in `crates/emerald-driver/tests/
  wasm_target.rs` (or alongside the existing `benchmarks.rs`-adjacent
  test harness, whichever this workspace's existing test layout
  convention prefers — verify against `crates/emerald-cli/tests/` at
  execution time): compiles `benchmarks/sum/sum.em` twice, once per
  target, links both, runs the native binary directly and the `.wasm`
  module via `Command::new("wasmtime").arg("run").arg(...)`, and asserts
  `stdout_native == stdout_wasm == "49999995000000\n"` — a real `assert_
  eq!` on two real captured `Output`s, not a visual/manual comparison.
  Gated behind detecting `wasmtime` on `PATH` (skip with a clear message
  if absent, the same "tool not present, skip, don't fail the whole
  suite" pattern this project already uses for any test that shells to
  an optional external tool) so `cargo test --workspace` stays green on
  a machine without `wasmtime` installed, while still running for real
  wherever this plan's own `devenv.nix` addition (`pkgs.wasmtime`) makes
  it available.

### 2. Acceptance Criteria
1. Real, executed proof: `sum.em` compiled+linked+run natively prints
   `49999995000000`.
2. Real, executed proof: `sum.em` compiled+linked with `--target wasm32-
   wasi`, run under `wasmtime run`, prints `49999995000000`.
3. The two outputs are asserted byte-for-byte equal in one test, not
   two independently-passing tests a reader has to manually cross-
   reference.
4. The test is skip-not-fail when `wasmtime` is absent from `PATH`,
   verified by actually removing it from `PATH` in one CI-shaped run
   and confirming the suite still passes (skipped, not failed).

### 3. File & Module Structure
- **Create:** `crates/emerald-driver/tests/wasm_target.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test (wasmtime present) | `cargo test --workspace` | all pass, incl. byte-for-byte stdout match | agent-claimed-locally |
| Test (wasmtime absent) | `PATH=<stripped> cargo test -p emerald-driver` | wasm test skipped, suite still green | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
llvm-config --targets-built | grep WebAssembly
emerald build --target wasm32-wasi   # against benchmarks/sum/sum.em, from crates/emerald-cli
wasmtime run ./sum.wasm              # must print 49999995000000, matching ./sum's own native output
```
