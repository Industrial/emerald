---
name: Scope-Based Arena Allocation
overview: "A bump-allocated region scoped to a function's call frame, freed in one bulk operation on that function's return, for any allocation plan 50's escape analysis proves does not outlive its allocating function — sitting alongside (never replacing) the existing never-free `emerald_alloc`; the region's own data structure is deliberately general enough that plan 54 can later reuse it unmodified as the arena backing one actor's isolated heap."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-runtime-region-arena
    content: "New runtime data structure and C API in runtime/emerald_runtime.c: emerald_region_create, emerald_region_alloc, emerald_region_destroy, plus process-wide bytes-outstanding accounting instrumentation shared with emerald_alloc for the contrast proof"
    status: pending
  - id: leaf-codegen-function-frame-regions
    content: "Thread a per-function region pointer through emerald-codegen's Ctx/function-entry codegen: create on entry, destroy on every exit path (including early return), skipped entirely for functions plan 50 finds nothing to place in it"
    status: pending
  - id: leaf-codegen-arena-allocation-sites
    content: "Consume plan 50's per-allocation-site escape verdict at every New/object-allocating codegen site: route non-escaping sites through emerald_region_alloc against the current function's region instead of emerald_alloc"
    status: pending
  - id: leaf-bounded-memory-proof
    content: "Instrumented proof: a function called N times from a loop in its caller, each call allocating several non-escaping class instances, shows constant peak bytes-outstanding across iterations under this plan versus the same program's unbounded growth under emerald_alloc alone"
    status: pending
isProject: false
---

# Plan 51 — Scope-Based Arena Allocation

This is plan 51 of the 48-57 batch implementing "Beyond the Ceiling" in
full — ten independent sibling plans (compiler-implementation
improvements plus the actor-model concurrency pillar and ADTs/Result),
each owning one distinct piece of that proposal. It is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
(that table's own Completion note calls Emerald v1 done at row 15; this
is post-v1 scope, same posture as plans 17 and 35 — this plan does not
touch `plan-of-plans.md` or any other plan file). This plan is
**foundational for the concurrency pillar**: plan 54
(actor-declarations-and-isolated-heaps), authored later in this same
batch, reuses this plan's region/arena mechanism directly as "an
actor's heap" — this plan builds the mechanism general enough for that
reuse and states the reuse point explicitly in the Decision log below,
but does not build actors itself; that is plan 54's job alone. This
plan depends on plan 50 (escape-analysis-stack-allocation, authored in
parallel in this same batch) for the escape verdict it consumes — see
Decision log for the assumed contract, verified against plan 50's own
file where that file exists at the time of writing.

Concrete proof this plan targets — a function called from a loop many
times, each call allocating several instances that never leave it:
```ruby
class Point
  x: Int64
  y: Int64
  def initialize(x: Int64, y: Int64) -> Void
    @x = x
    @y = y
  end
  def sum -> Int64
    @x + @y
  end
end

def compute_local_sum(n: Int64) -> Int64
  p1: Point = Point.new(n, n + 1)
  p2: Point = Point.new(n * 2, n * 3)
  p1.sum + p2.sum
end

total: Int64 = 0
i: Int64 = 0
while i < 100000
  total += compute_local_sum(i)
  i += 1
end
puts total
```
`p1` and `p2` are constructed, read via `.sum`, and discarded —
neither is returned, stored in a field, or passed anywhere outside
`compute_local_sum` — exactly plan 50's non-escaping shape. Under
today's `emerald_alloc` alone, all 100,000 calls' 200,000 `Point`
allocations (16 bytes each: two `Int64` fields) are `malloc`'d and
never freed — real, unbounded growth in `emerald_alloc`'s bytes-
outstanding counter as the loop runs, harmless at this program's small
per-object size but a genuine, disclosed problem at scale (a function
called millions of times, or allocating larger objects, exhausts
memory with nothing ever reclaimed). Under this plan, `compute_local_sum`
is compiled with its own region: created on entry, `p1`/`p2` bump-
allocated into it via `emerald_region_alloc` instead of `emerald_alloc`,
and the whole region destroyed in one call before the function returns
— so peak bytes-outstanding attributable to `compute_local_sum`'s calls
never exceeds "one call's worth," constant across all 100,000
iterations rather than growing by 3.2MB total and staying there.
`leaf-bounded-memory-proof` instruments exactly this contrast, since —
like plan 50's own escape proof — none of this is visible on stdout.

## Decision log

- **Verified this session: `emerald_alloc` truly never frees.** Its
  entire implementation, in `runtime/emerald_runtime.c` (owned by plans
  06/08 per that file's own header comment, `"plan-of-plans rows 06,
  08"`), is:
  ```c
  void *emerald_alloc(long long size) {
    return malloc((size_t) size);
  }
  ```
  with an adjacent comment stating plainly: `"No corresponding free —
  no GC, no lifetime tracking yet; matches inception §12 (no ownership
  system, no GC pressure, for now)."` Every other allocating helper in
  that file (`emerald_alloc_zeroed` for `Array.new`, `emerald_string_
  concat`, `emerald_int64_to_string`, `emerald_float64_to_string`) is
  either a second bare `malloc`/`calloc` wrapper or itself routed
  through `emerald_alloc` — the whole runtime's heap is a single
  monotonically growing `malloc` arena with no `free` call anywhere in
  the file except inside the unrelated `setjmp`/`longjmp` exception-
  handler bookkeeping (`emerald_pop_handler`/`emerald_free_handler`,
  which free handler *records*, never allocated Emerald objects). This
  plan does not change `emerald_alloc` itself or any call site plan 50
  proves *does* escape — it adds a second, parallel allocation path
  used only where plan 50 proves it's safe to bulk-free.
- **Region granularity: per-function-call-frame, not per-REPL-
  statement.** A coarser granularity — one arena per top-level REPL
  statement (plan 47 gives Emerald a REPL), freed after each statement
  finishes printing/evaluating — is a real alternative and would be
  simpler to trigger (one region for the whole program in the common
  compiled-binary case, since there's exactly one "REPL statement": the
  entire `main`). This plan declines it: plan 50's escape analysis is
  explicitly *intraprocedural* — its whole proof obligation is "does
  this reference escape its allocating function," which hands this plan
  an allocation's exact lifetime upper bound (the allocating function's
  own activation) for free. A per-call-frame region matches that bound
  exactly with no slack: create on entry, destroy on exit, no separate
  bookkeeping for "which statement are we in." A per-REPL-statement
  region would need its own, different analysis (whole-top-level-
  statement escape, not whole-function escape) and would only ever
  benefit the REPL entry point rather than every function everywhere —
  strictly narrower payoff for a differently-shaped analysis this plan
  doesn't have. Per-call-frame is also what composes cleanly with
  ordinary function calls of any depth (a helper called from a loop, a
  helper called from another helper, a recursive function each
  activation owning its own region) without inventing a second
  granularity concept later.
- **A genuine, disclosed limitation of per-call-frame granularity: it
  does not reclaim memory *within* one call, only *between* calls.** If
  `compute_local_sum` itself looped 100,000 times internally and
  allocated a non-escaping `Point` each iteration before discarding it,
  every one of those `Point`s lives in the *same* region (the one
  region for that single activation of `compute_local_sum`), since the
  region's lifetime is bounded by the function's activation, not by
  any inner block or loop iteration within it — the region would grow
  for the entire duration of that one call and only shrink back to zero
  when the call returns. This plan's worked example is deliberately
  structured with the loop in the *caller* (calling `compute_local_sum`
  100,000 times) rather than inside `compute_local_sum`, specifically
  because that is the shape per-call-frame granularity actually bounds.
  Finer-grained sub-regions (one per loop iteration, or per lexical
  block) are a real, larger design this plan does not attempt — stated
  here as an honest boundary of this plan's scope, not silently assumed
  away.
- **Escaping allocations permanently stay on `emerald_alloc` — this is
  not a temporary gap.** Any allocation plan 50 proves (or fails to
  prove non-escaping, which plan 50's conservative analysis treats the
  same as escaping) still goes through the existing, unmodified
  `emerald_alloc`, with all of its existing never-free behavior. Closing
  that gap for real would require either full ownership/lifetime
  tracking through the type system or a tracing garbage collector — the
  first is a substantially larger, distinct feature this batch does not
  build in this plan, and the second is explicitly against this
  project's identity ceiling (no tracing garbage collector). This plan
  states that limitation plainly, as permanent, not as a TODO.
- **Precedent: this is Zig's ordinary, non-exotic allocator-passing
  style, not an invention specific to this project.** Zig's standard
  library ships `std.heap.ArenaAllocator`, constructed by wrapping a
  backing allocator (`ArenaAllocator.init(backing_allocator)`); it
  internally keeps a linked list of growing buffers, hands out bump-
  allocated memory through the ordinary `Allocator` interface, and
  frees every buffer it ever grew in one pass via a single `deinit()`
  call — the same "one bulk free, no per-object free" shape as this
  plan's `emerald_region_destroy`. Idiomatic Zig programs construct one
  `ArenaAllocator` at the top of a function (very often `main` itself,
  for CLI tools), do all their work against it, and `defer
  arena.deinit()` — Zig's broader convention of passing an explicit
  `Allocator` value into every allocating function, rather than a
  single implicit global heap, is exactly what makes scoping an arena
  to one call's lifetime a normal, well-understood pattern rather than
  a novel one. This plan is Emerald's version of the same idea, made
  automatic (the compiler decides which functions get a region and
  which allocations use it, per plan 50's verdict) rather than manually
  threaded by the programmer, since Emerald has no user-facing allocator
  parameters to thread it through.
- **Forward-looking note for plan 54 (not built here): the region type
  is deliberately trigger-agnostic.** `EmeraldRegion`'s C API —
  `emerald_region_create`, `emerald_region_alloc`, `emerald_region_
  destroy` — carries no assumption baked in about *when* those three
  calls happen relative to a function's prologue/epilogue; they are
  three ordinary functions operating on an opaque `void *region`
  handle. This plan's own codegen work (`leaf-codegen-function-frame-
  regions`) is the part that decides to call `emerald_region_create` at
  function entry and `emerald_region_destroy` at function exit — that
  policy lives in `emerald-codegen`, not in the region's own
  implementation. Plan 54 can therefore reuse `EmeraldRegion` verbatim
  as the arena backing one actor instance's isolated heap: call
  `emerald_region_create` when an actor instance is constructed, store
  the resulting handle as part of the actor's own runtime
  representation (rather than a call-frame-local variable), route the
  actor's own non-escaping-from-the-actor allocations through
  `emerald_region_alloc` against that stored handle, and call
  `emerald_region_destroy` on actor termination instead of on function
  return — the same mechanism, a different trigger for the bulk-free.
  This plan does not decide any of plan 54's actor-lifetime semantics;
  it only avoids closing off that reuse by keeping the region's
  implementation free of call-frame-specific assumptions.
- **Tooling: plan 35 (debug info) is genuinely, narrowly affected;
  plan 17 (IDE integration) is not.** Plan 35 adds real DWARF line-
  table info to the object file `emerald-codegen` emits, walking
  generated code to attach source positions. The region-create/region-
  destroy calls this plan inserts at function entry/exit are ordinary
  internal runtime calls with no user-visible Emerald source
  expression backing them (much like the existing `emerald_push_
  handler`/`emerald_pop_handler` calls plan 11's `begin`/`rescue`
  codegen already inserts) — they should be attributed the same
  source line as the function's own `def`/`end` boundary, not given a
  synthetic DWARF variable or lexical block of their own; `leaf-
  codegen-function-frame-regions`'s acceptance criteria require
  verifying this doesn't regress plan 35's own `gdb`/`lldb`-visible
  line stepping through a function that now has a region. Plan 17 (LSP
  symbols/navigation) operates over source-level parse/sema
  information and is not touched at all by this plan — regions are a
  backend/runtime-only concept invisible above `emerald-codegen`, so
  this plan does not cite plan 17 further.

## Leaf: leaf-runtime-region-arena

### 1. Context
- Why: neither `emerald_alloc` nor `emerald_alloc_zeroed` supports bulk
  deallocation at all today (verified this session — see Decision log);
  this plan needs a genuinely new runtime primitive, not a
  reinterpretation of an existing one.
- Target state: `runtime/emerald_runtime.c` gains an `EmeraldRegion`
  struct — a linked list of growing chunks (not one `realloc`'d buffer:
  `realloc` may move memory, which would invalidate every pointer
  already handed out of the region, so growth must append a new chunk
  and bump-allocate within whichever chunk currently has room) —
  plus:
  - `void *emerald_region_create(void)` — allocates the region's
    control structure and one initial chunk (a fixed default size,
    e.g. 4096 bytes) via `malloc`.
  - `void *emerald_region_alloc(void *region, long long size)` — bump-
    allocates `size` bytes from the region's current chunk, appending a
    new chunk (sized to at least `size`, doubling growth otherwise,
    mirroring `ArenaAllocator`'s own growth policy per the Decision
    log's Zig precedent) if the current chunk lacks room; never
    returns `NULL` for a satisfiable request the same way `emerald_
    alloc` never checks `malloc`'s own `NULL` return today (consistent,
    not a new failure mode this plan invents).
  - `void emerald_region_destroy(void *region)` — frees every chunk in
    the region's list, then the control structure itself, in one call;
    this is the plan's "one bulk operation."
  - A process-wide `long long emerald_bytes_outstanding(void)`
    accounting counter, incremented by the underlying `malloc` calls
    inside both `emerald_alloc`/`emerald_alloc_zeroed` *and*
    `emerald_region_create`/`emerald_region_alloc`'s chunk growth, and
    decremented only by `emerald_region_destroy`'s frees — the shared
    instrumentation `leaf-bounded-memory-proof` reads to make the
    "bounded under regions, unbounded under `emerald_alloc` alone"
    contrast a real, measured fact rather than an assertion.
- Non-goals: no `emerald_region_free_one(ptr)`-style per-object free
  inside a region (defeats the entire point — regions are bulk-free
  only); no thread-safety (matches every other runtime helper in this
  file today — single-threaded, per the same inception §12 posture
  cited above; plan 54's actor model is exactly the future work that
  will need to revisit this, not this plan).

### 2. Acceptance Criteria
1. `cc -c runtime/emerald_runtime.c -o /tmp/emerald_runtime.o` still
   succeeds (regression on plan 06's own original acceptance criterion)
   after this leaf's additions.
2. A standalone C (or Rust `cc`-linked) test harness calling
   `emerald_region_create`, then `emerald_region_alloc` for a total
   size larger than one default chunk (forcing at least one chunk-
   growth), then `emerald_region_destroy`, runs clean under a memory
   checker (`valgrind --leak-check=full` or equivalent) with zero
   leaked bytes attributable to the region.
3. Calling `emerald_region_alloc` for N objects, reading `emerald_
   bytes_outstanding()` before and after, shows it increased by at
   least the requested bytes; calling `emerald_region_destroy`
   afterward and reading `emerald_bytes_outstanding()` again shows it
   returned to its pre-allocation value — proving the counter is a
   real, working measurement, not a stub, before `leaf-bounded-memory-
   proof` relies on it.
4. Regression: every existing `emerald_alloc`/`emerald_alloc_zeroed`
   call site in the runtime and every existing compiled example still
   behaves identically (this leaf only adds new functions and a
   counter increment inside the existing `malloc`/`calloc` wrappers, it
   does not change their return values or semantics).

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cc -c runtime/emerald_runtime.c -o /tmp/emerald_runtime.o` | clean | agent-claimed-locally |
| Leak check | `valgrind --leak-check=full <region harness binary>` | zero leaked bytes from region path | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass, no regression | agent-claimed-locally |

---

## Leaf: leaf-codegen-function-frame-regions

### 1. Context
- Why: `emerald-codegen`'s `Ctx` struct (`crates/emerald-codegen/src/
  lib.rs`, `L709-764`, verified this session) already holds one
  `FunctionValue` per runtime helper (`alloc`, `alloc_zeroed`,
  `print_i64`, …) declared once and threaded through every codegen
  function — there is no per-function-activation state in `Ctx` today
  at all, only whole-module-lifetime function handles. This leaf adds
  the first such per-activation value.
- Target state: `Ctx` gains a `region: Option<PointerValue<'ctx>>`
  field (by analogy with the existing `self_ctx: Option<(PointerValue
  <'ctx>, ...)>` pattern the struct already uses for "this value exists
  only while compiling one particular function's body"), plus declared
  `FunctionValue` handles for `emerald_region_create`/`_alloc`/
  `_destroy` alongside the existing `alloc`/`alloc_zeroed` declarations.
  `define_user_function`/`define_method`/`define_lambda` (the three
  existing per-function-body entry points, `L3761-3935`) are extended
  so that, for a function plan 50 marks as having at least one
  candidate non-escaping allocation site, the function's prologue calls
  `emerald_region_create` and binds the result into `Ctx.region` for
  that body's compilation, and every exit path — every `Stmt::Return`
  codegen site plus the implicit fall-through return at the end of
  `build_function_body` — calls `emerald_region_destroy(region)` before
  actually returning. A function with zero candidate sites (per plan
  50) skips region creation entirely — `Ctx.region` stays `None` for
  that function's body, and `leaf-codegen-arena-allocation-sites`'s
  fallback path (see that leaf) is exercised instead, so a `puts`-only
  or arithmetic-only function pays no new per-call overhead.
- Interaction with exceptions (disclosed, not solved here): plan 11's
  `begin`/`rescue` uses `setjmp`/`longjmp` to jump directly from
  `emerald_raise` to the enclosing handler's `begin` site, skipping any
  intermediate stack frames' own code entirely (verified against
  `runtime/emerald_runtime.c`'s `emerald_raise` — it never walks or
  notifies frames between the raise site and the matching handler).
  A function whose region was created but which is unwound past via
  `longjmp` before reaching its own `emerald_region_destroy` call never
  runs that call — its region leaks (its chunks are never freed). This
  is a real, disclosed gap this leaf does not solve: it is no worse
  than today's status quo (that memory was never freed under plain
  `emerald_alloc` either), and solving it properly would mean threading
  cleanup actions through `longjmp`-based unwinding, a materially
  larger feature. `leaf-codegen-function-frame-regions`'s acceptance
  criteria only cover normal (non-exceptional) control flow.

### 2. Acceptance Criteria
1. Compiling this plan's own worked `compute_local_sum` example emits,
   for `compute_local_sum` specifically, exactly one `emerald_region_
   create` call at function entry and exactly one `emerald_region_
   destroy` call preceding its single `return` — verified by inspecting
   the emitted LLVM IR text (`Module::print_to_string` or `-emit-llvm`)
   for the function.
2. A function with two `return` statements (one early, one at the end)
   and at least one candidate non-escaping allocation gets a region
   destroy call inserted before *both* return sites, not just the
   final one — a real early-return regression case, compiled and run,
   proving both paths free the region rather than only the syntactically
   last one.
3. A function with zero allocation sites at all (e.g. a pure-arithmetic
   helper) emits no `emerald_region_create`/`_destroy` calls at all —
   verified against the emitted IR, proving the skip-when-nothing-to-
   allocate path is real, not just declared.
4. Regression: `leaf-bounded-memory-proof`'s memory-checker gate (see
   that leaf) passes with zero leaks on the normal-control-flow path;
   compiling and running every prior plan's example (including plan
   11's exception examples, on their non-exceptional path) still
   produces identical output.
5. Compiling this plan's worked example with plan 35's debug-info path
   enabled (where plan 35 has landed) still allows a scripted debugger
   session to set a breakpoint on `compute_local_sum`'s first
   source line and step through it — the newly inserted region calls
   attach to the function's own entry line rather than introducing a
   spurious, unattributable line jump.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`Ctx` struct,
  `define_user_function`, `define_method`, `define_lambda`,
  `build_function_body`, `build_stmt`'s `Return` arm, the runtime-
  function-declaration section that currently declares `alloc`/
  `alloc_zeroed`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. IR-inspection tests | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass, no regression | agent-claimed-locally |

---

## Leaf: leaf-codegen-arena-allocation-sites

### 1. Context
- Why: `leaf-codegen-function-frame-regions` gives every eligible
  function a live region during its body's compilation, but nothing
  yet routes any actual allocation through it — `build_expr`'s `New`
  handling (inside `L1089-1635`, verified this session to be where
  object construction currently calls `alloc` unconditionally) still
  calls `emerald_alloc` for every instance, escaping or not.
- Target state: this leaf consumes plan 50's escape verdict. Assumed
  contract with plan 50 (verified against plan 50's own file where it
  exists at time of writing; this plan does not redefine plan 50's
  analysis, only its consumption): a conservative, intraprocedural,
  per-allocation-site judgment — keyed however plan 50 keys it (e.g. by
  `New` expression identity/span within its enclosing function) —
  answering "does this site's result ever escape its allocating
  function," where "no" is only ever asserted when plan 50 can prove
  it and any doubt defaults to "yes" (escapes). At every `New`-
  construction codegen site, this leaf looks up that site's verdict:
  `false` (proven non-escaping) *and* the enclosing function has a live
  `Ctx.region` (per the previous leaf — a function plan 50 found
  nothing non-escaping in never gets one) routes the allocation through
  `emerald_region_alloc(region, size)`; anything else — `true`,
  unproven, or no live region for this function — routes through the
  existing, unmodified `emerald_alloc(size)` call, unconditionally
  preserving today's behavior for every allocation this plan doesn't
  touch.
- Non-goals: this leaf does not change `Array.new`'s `emerald_alloc_
  zeroed` path (plan 25) or any `String` allocation path (plans 19/36)
  — plan 50's stated scope (per this plan's Depends-on) is a check for
  whether *a reference* escapes; extending that judgment to array/
  string buffers is real future work this leaf does not fold in
  silently, since neither plan 50 nor this plan claims that scope.

### 2. Acceptance Criteria
1. This plan's own worked `compute_local_sum` example, compiled and
   inspected at the IR level, shows both `Point.new` call sites routed
   through `emerald_region_alloc` against `compute_local_sum`'s region,
   not `emerald_alloc`.
2. This plan's own worked example, compiled, linked, and run for the
   full 100,000-iteration loop, prints the correct arithmetic result
   (a real executed proof the region-allocated `Point`s are read
   correctly via `.sum` before their region is destroyed — not merely
   that codegen picks the right call, but that the values are actually
   intact and correct while the region is still live).
3. A second worked example where a `New`'d instance *is* returned from
   its allocating function (a real escaping case) is, per plan 50's own
   verdict for that case, still routed through `emerald_alloc` — a
   regression proof that this leaf does not over-apply the arena path
   to anything plan 50 doesn't clear, compiled, linked, and run to
   confirm the returned instance's fields are still readable by the
   caller after the callee returns (which would be a use-after-free if
   it had wrongly gone through the now-destroyed region instead).
4. Regression: every prior plan's example compiles and runs identically
   (no allocation site this plan doesn't touch changes behavior).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`build_expr`'s `New`
  handling)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. IR-inspection + escaping-regression tests | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-bounded-memory-proof

### 1. Context
- Why: like plan 50's own escape proof, "peak memory stayed bounded" is
  not something a program's stdout can demonstrate on its own — this
  plan needs the same style of internal instrumentation plan 50 needed,
  reading `leaf-runtime-region-arena`'s `emerald_bytes_outstanding()`
  counter rather than trusting the design argument alone.
- Target state: an integration test (in `crates/emerald-codegen/src/
  lib.rs`'s existing test module, alongside the existing `*_EXAMPLE`
  compiled-and-run tests) that: (a) compiles, links, and runs this
  plan's worked `compute_local_sum` example with a reduced-but-still-
  meaningful iteration count instrumented to sample `emerald_bytes_
  outstanding()` at fixed intervals across the loop (e.g. every 10,000
  iterations, via a small additional runtime hook the test binary calls
  directly rather than anything visible from Emerald source); asserts
  every sample after the first is within a small constant tolerance of
  the first (bounded, not growing) — proving the arena path keeps peak
  memory flat across iterations. (b) A second build of the *same*
  source program compiled with `leaf-codegen-arena-allocation-sites`'s
  routing forced off (a debug-only escape hatch flag, e.g. an
  environment variable or cargo feature that makes every site fall
  back to `emerald_alloc` regardless of plan 50's verdict — a real,
  disclosed test-only knob, not shipped as a language feature) samples
  the same counter and asserts it grows roughly linearly with iteration
  count instead — the explicit "what today's `emerald_alloc`-only
  world does instead" contrast the plan's overview promises.
- This leaf is the plan's proof of the headline claim; it does not
  introduce any new allocation-routing logic itself.

### 2. Acceptance Criteria
1. Under normal (arena-routed) compilation, `emerald_bytes_outstanding()`
   sampled across the loop's iterations stays within a fixed, small
   tolerance of its value after the first sample — a real measured
   bound, not an assumption.
2. Under the forced-`emerald_alloc`-only fallback build of the identical
   source program, the same counter grows monotonically and
   substantially (proportional to iteration count) across the same
   sampling points — the disclosed contrast made concrete and measured,
   not merely asserted in prose.
3. Both builds still print the identical, correct arithmetic result on
   stdout — the memory-behavior difference between the two builds is
   invisible to a program's own observable output, exactly as expected
   (this plan changes memory reclamation, never program semantics).
4. `valgrind --leak-check=full` (or equivalent) run against the arena-
   routed binary for a bounded iteration count reports zero leaked
   bytes for anything routed through `emerald_region_alloc`; the
   forced-`emerald_alloc`-only build is *expected* to still report the
   pre-existing "still reachable, never freed" pattern for its
   `emerald_alloc` calls (matching today's status quo exactly, not a
   regression this plan introduces).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (new test module
  additions; the worked `compute_local_sum` example as a new named
  const alongside the existing `*_EXAMPLE` constants)
- **Create (test-only):** a small sampling/reporting hook exposed from
  `runtime/emerald_runtime.c` for the test harness to call directly
  (not part of the Emerald language surface)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen -- bounded_memory` | bounded-vs-unbounded contrast both hold | agent-claimed-locally |
| Leak check | `valgrind --leak-check=full <arena-routed binary>` | zero leaked bytes from region path | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cc -c runtime/emerald_runtime.c -o /tmp/emerald_runtime.o
```
