---
name: Scheduler and Message Passing
overview: "A fixed pool of OS worker threads and a per-actor mailbox queue turn a cross-actor method call into a real enqueued, asynchronously-processed message — honestly scoped as \"N actors over M OS threads,\" not the green-thread M:N scheduler `Beyond the Ceiling` gestured at, since Emerald has no coroutine/stack-switching primitive to build true green threads on."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-thread-safe-runtime
    content: "runtime/emerald_runtime.c grows a per-actor mutex+condvar mailbox queue, a global runnable-actor queue, pthread-based worker-pool start/drain/join, a thread-local fix for the exception-handler stack (verified single-threaded today), and emerald_current_thread_id for observability"
    status: pending
  - id: leaf-actor-header-and-trampolines
    content: "Actor arenas gain a fixed-size scheduling header prefix; codegen emits one uniform-ABI trampoline function per actor method (a plain, statically-resolved function pointer, not a vtable) plus extern decls for the new runtime functions"
    status: pending
  - id: leaf-cross-actor-dispatch
    content: "Sema requires every actor method (except initialize) to declare no return type; codegen's call-site rule is syntactic — a literal self receiver stays a direct call, any other actor-typed receiver becomes an emerald_actor_enqueue call; generated main ends with an implicit drain-and-join barrier"
    status: pending
  - id: leaf-worked-concurrency-proof
    content: "A deterministic 5-round ping-pong between two actors proves per-actor FIFO ordering under real concurrent execution; an independent-two-spinners wall-clock timing test proves real OS-thread overlap actually happened, not just interleaved sequential execution"
    status: pending
isProject: false
---

# Plan 55 — Scheduler and Message Passing

This is plan 55 of the 48-57 batch implementing "Beyond the Ceiling" in
full — the actor-model concurrency pillar borrowed from BEAM/Pony that
follow-up analysis named as Emerald's next real capability tier. It is
the second of four concurrency-pillar plans: 54 (actor declarations +
isolated heaps) → **55, this plan** (scheduler + message passing) → 56
(compile-time message safety) → 57 (supervision trees). Like every
other post-v1 plan in this repository, it is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
and does not touch that file or any other plan file.

This plan depends on plan 54's design, not on plan 54 having executed.
At authoring time `history/` contains plans through 47 only — plan 54's
own file does not yet exist on disk (verified this session via a
directory listing), so this plan works from the assumed contract stated
in its own brief: `actor Foo ... end` is a new top-level declaration
form (structurally parallel to today's `ClassDef` — a `name`, a fixed
`fields: Vec<Param>` list, and `methods: Vec<Function>`, verified
against `crates/emerald-parser/src/ast.rs`'s actual `ClassDef`/`Function`
shapes this session); `.spawn(args)` allocates a per-instance arena
(plan 51) and runs `initialize` synchronously, returning a handle to the
new instance; and — the specific handoff point plan 54 explicitly left
undone — **every method call on an actor instance, including a
cross-instance one, is still today an ordinary direct/synchronous call**,
ordinary codegen with no mailbox or thread involved anywhere. This plan
is entirely about replacing that last property for calls originating
outside the callee's own execution context. If plan 54's file exists by
execution time, `leaf-actor-header-and-trampolines`'s first acceptance
criterion is to re-verify this contract against its actual text before
writing any code, exactly as every prior plan in this project verifies
its own assumptions against real source rather than a stale summary.

## The honest scope call this plan makes

"Beyond the Ceiling" used the phrase "M:N scheduler" the way BEAM and Go
use it: a large population of lightweight, cooperatively-scheduled
*green threads*, each with its own suspendable call stack, multiplexed
over a small number of OS threads by a runtime that can pause a green
thread mid-call-stack and resume it later on a different OS thread. That
requires a genuine stack-switching primitive — either stackful
coroutines (swap the whole native stack pointer/register file, `ucontext`/
`makecontext`/`swapcontext`-style, or a hand-rolled equivalent) or a
stackless CPS/state-machine transform of the entire call graph (what
Rust's own `async`/`await` desugars into). Verified directly against
this codebase this session, on two independent axes:

- **Codegen has no such primitive.** `crates/emerald-codegen/src/lib.rs`
  is 5056 lines; its full function map (read this session) contains
  every existing control-flow lowering — loops, `case`, `begin`/`rescue`,
  blocks/`yield` — and not one function whose job is saving/restoring an
  arbitrary call stack, spawning a fiber, or transforming a function into
  resumable states. `begin`/`rescue` (plan 38) lowers to a `setjmp`/
  `longjmp` handler stack — a real but strictly *one-shot, downward-only*
  non-local jump (a handler fires at most once and never returns control
  to where it was captured), nothing like a coroutine's repeated
  suspend/resume. Plan 34's `yield` (blocks) is, by its own Decision
  log's own words, "compiled fresh per call site" via call-site
  specialization — a **compile-time inlining** of the block into the
  caller, never a runtime suspension of anything.
- **The runtime has no such primitive either, and says so.**
  `runtime/emerald_runtime.c` (179 lines, read in full this session) is
  the entire native runtime shim; its own comment on the exception
  handler stack states plainly: *"Single-threaded only (no ownership/
  concurrency in v1 — inception §12/§20)"* — a design fact this plan
  must now change (see Decision log), not merely observe. No `ucontext.h`
  include, no assembly, no `Fiber`/`Coroutine`/`async` symbol exists
  anywhere in `crates/` or `runtime/`.

Building true green threads from nothing — a stack allocator, a context-
switch primitive (likely hand-written per-target-architecture assembly
or a `ucontext` dependency), a cooperative or preemptive yield point
convention threaded through every codegen path that can block — is a
large, separate, well-scoped project of its own. This plan **declines
it explicitly** rather than reinterpreting "M:N scheduler" loosely to
paper over the gap. What this plan actually builds, and what its own
title now honestly says instead of what the framing document aspired
to, is: **a fixed pool of M OS worker threads, each pulling actor
messages off thread-safe queues and running each message's handler to
completion on whichever OS thread happened to dequeue it — N actors
over M OS threads, not green threads multiplexed onto M carriers.** This
is a deliberate, disclosed scope reduction from "Beyond the Ceiling"'s
aspirational language, stated here as a decision, not discovered later
as a shortfall.

Concrete proof this plan targets — two halves, because raw concurrency
is invisible in ordinary sequential stdout and each half proves a
different half of the claim:

**(a) Correctness under concurrency** — a deterministic 5-round
ping-pong between two actor instances of the same type, run on a
multi-worker pool:

```ruby
actor PingPong
  name: String
  limit: Int64
  count: Int64
  peer: PingPong

  def initialize(name: String, limit: Int64)
    @name = name
    @limit = limit
    @count = 0
  end

  def set_peer(other: PingPong)
    @peer = other
  end

  def hit
    @count = @count + 1
    puts @name + " " + @count.to_s
    if @count < @limit
      @peer.hit
    end
  end
end

a = PingPong.spawn("A", 5)
b = PingPong.spawn("B", 5)
a.set_peer(b)
b.set_peer(a)
a.hit
```

Expected stdout, byte-for-byte, every run, regardless of worker-pool
size: `A 1`, `B 1`, `A 2`, `B 2`, `A 3`, `B 3`, `A 4`, `B 4`, `A 5` — nine
lines, then the program's implicit end-of-`main` drain (see Decision
log) waits for both mailboxes to go idle before the process exits.
`A`'s own count reaches 5 on the chain's 9th hop (its own hits land on
odd hops 1/3/5/7/9); `B`'s reaches 4 and never gets a 5th, because `A`'s
5th hit sees `5 < 5` false and does not send again — a real halting
condition, not a fixed iteration count baked into the harness.

**(b) Real concurrent execution actually happened** — two independent,
non-communicating actors given equal-sized CPU-bound work back to back,
with no explicit synchronization between them:

```ruby
actor Spinner
  id: Int64
  total: Int64

  def initialize(id: Int64)
    @id = id
    @total = 0
  end

  def spin(iterations: Int64)
    i: Int64 = 0
    while i < iterations
      @total = @total + i
      i = i + 1
    end
    puts "spinner " + @id.to_s + " done"
  end
end

s1 = Spinner.spawn(1)
s2 = Spinner.spawn(2)
s1.spin(200000000)
s2.spin(200000000)
```

Both `spin` calls are cross-actor sends issued back to back from `main`
with no wait between them; `main`'s only synchronization point is the
implicit drain-and-join at the very end. Run once with `EMERALD_WORKERS=1`
(one worker thread — no concurrency possible, both spins strictly
serialize) and once with the default pool size (`>= 2` workers on any
multi-core CI/dev box), the *same compiled binary*, the same program: if
the two workers' total wall-clock time is close to the *single-spin*
time rather than the sum of both, the two spins genuinely overlapped on
separate OS threads — something no purely sequential (even cleverly
interleaved single-threaded) execution could produce. `leaf-worked-
concurrency-proof` states the exact assertion and margin.

## Decision log

- **Scope call: "N actors over M OS threads," not green threads.** See
  the dedicated section above — verified against real source, not
  reargued here. Stackful/stackless cooperative green threads, and the
  context-switching runtime primitive they need, are named explicitly as
  separate, larger, disclosed future work; this plan does not build any
  scaffolding toward them (no stack-allocator type, no yield-point
  convention threaded through codegen) that a later plan would have to
  work around.
- **Mailbox and worker pool live in `runtime/emerald_runtime.c`, in
  plain C with pthreads — not a new Rust-side Cargo dependency.**
  Verified this session: `crates/emerald-cli/build.rs` already compiles
  `runtime/emerald_runtime.c` via the `cc` crate (already a dependency)
  into a static archive and `include_bytes!`s it into the shipped
  `emerald-cli` binary so a copied-alone binary can still link a user's
  program; LLVM-emitted object code already calls into that file's
  functions as plain `extern "C"` symbols (`emerald_push_handler`,
  `emerald_raise`, etc. — plan 38). The scheduler has to live somewhere
  reachable from a user's *compiled, linked, standalone* program, not
  inside the `emerald-cli` compiler process itself — extending the one
  file that mechanism already exists for is strictly less machinery than
  standing up a second Rust crate, compiling it to its own staticlib, and
  teaching `build.rs` a second embedding path for no benefit over the
  first. A mutex+condvar-guarded singly-linked queue (per actor, for its
  mailbox; one more, global, for the pool's runnable-actor queue) is
  plain C99 with `pthread_mutex_t`/`pthread_cond_t` — no lock-free MPSC
  crate, no new Cargo dependency at all. Had this plan instead chosen a
  Rust-hosted concurrent-queue crate (`crossbeam-channel` or similar), it
  would owe the same vetting discipline plan 46 established when it
  added `toml`+`serde` to `emerald-cli` (confirm current stable version,
  justify ubiquity, decide whether the new code lives inside an existing
  crate or a new one) — cited here for the tooling-integration record
  even though this plan's own design never needs to invoke it, since zero
  new Cargo dependencies cross the workspace boundary.
- **Dispatch rule and the actual safety argument: at most one in-flight
  message per actor, by construction, not by locking the actor's own
  state.** Calling a method on an actor reference builds a message
  (resolved trampoline function pointer + packed argument values, see
  next bullet) and appends it to that actor's mailbox; a worker thread
  only ever pulls the *next* message off *one* actor's mailbox after
  that actor has been placed on the shared global runnable queue, and an
  actor is placed on that queue only when transitioning from "idle, no
  thread owns it" to "claimed by exactly one worker" — a transition
  gated by one `scheduled` flag read-and-set under that actor's own
  mailbox mutex. The flag is cleared again only after the worker that
  set it finishes running the method body **and** finds the mailbox
  empty; if messages arrived meanwhile, the actor is re-appended to the
  runnable queue (still `scheduled = true` throughout) rather than ever
  having two workers observe it as claimable at once. The three actor
  states — idle, runnable-but-not-yet-picked-up, and currently-executing
  — are mutually exclusive by this construction, which is the entire
  safety argument for touching a given actor's isolated arena (plan 51)
  from a worker thread with **no lock around the actor's own fields at
  all**: only one thread is ever inside that actor's code at a time,
  full stop, so plan 54's per-instance-arena isolation composes with
  this scheduler for free rather than needing its own synchronization
  layer.
- **A message's trampoline is a statically-resolved function pointer,
  not a vtable slot — this does not reopen the "no dynamic dispatch"
  identity constraint.** Every cross-actor call site's receiver has a
  fully known static type at compile time (no runtime type discovery,
  no subclass-polymorphic override resolution — actors are not part of
  any inheritance hierarchy per plan 54's own constraint), so codegen
  already knows exactly which method body a given `a.foo(args)` targets,
  the same way `build_method_call` resolves an ordinary method call
  today. This plan has codegen additionally emit one small **trampoline
  function per actor method** with a uniform ABI — `void trampoline(i8*
  self, i64* argv)` — that unpacks `argv` per that specific method's
  already-known static parameter types and calls the real, ordinary,
  already-compiled method function. The message struct carries a plain
  pointer to *that one, statically-chosen* trampoline — data flowing
  through a queue, the same category of thing as a C `qsort` comparator
  argument, never a runtime lookup keyed on the receiver's dynamic type.
  No vtable, no indirect-through-inheritance dispatch is introduced
  anywhere by this plan.
- **A cross-actor-reachable method must declare no return type; `self`
  calls are exempt from the *dispatch* rule but not from this one.**
  Because a cross-actor call is now genuinely asynchronous, its call
  expression cannot produce a real return value synchronously — this
  plan does not add futures/promises/an `ask` pattern (real, substantial,
  separate design surface plan 54 didn't need and this plan doesn't
  either). Rather than let a method's return type depend on *how* it's
  invoked (a real value when called via `self`, silently discarded when
  called across actors — a footgun, not a feature), sema requires every
  method declared on an `actor` to omit its return type entirely, with
  exactly one exception: `initialize`, which `.spawn` still calls
  directly and synchronously before any mailbox exists to send anything
  to. This mirrors Pony's own "behaviours are always asynchronous and
  always return `None`" rule — independent real-world precedent for the
  same constraint, not an ad hoc restriction invented for this plan.
- **The dispatch rule itself is syntactic: a literal `self` receiver is
  a direct call; any other actor-typed receiver expression is an
  enqueue, even one that happens to alias `self` at runtime.** Sema/
  codegen cannot generally know, for an arbitrary actor-typed expression,
  whether it refers to the currently-executing instance (e.g. an actor
  that stashed a reference to itself in a field and handed it out) — a
  true "is this dynamically me" check would need a runtime identity
  comparison this plan doesn't otherwise need anywhere. The rule is
  instead purely syntactic and decidable at compile time: the receiver
  is the literal token `self` (or an implicit-self call inside an actor
  method body) → ordinary direct call, unchanged from today; anything
  else — a field, a parameter, a local, an expression — whose static
  type is an actor type → build an `emerald_actor_enqueue` call instead.
  Deliberately conservative (a `self`-aliasing edge case gets treated as
  cross-actor and pays a real enqueue even though it happens to be safe
  to call directly) in exchange for a rule with no runtime check and no
  ambiguity at any call site.
- **This plan's one mandatory, disclosed fix to *existing* code: the
  exception handler stack becomes thread-local.** `runtime/
  emerald_runtime.c`'s `emerald_handler_stack` is today a single
  `static` global linked list, explicitly justified in its own comment
  by "single-threaded only." The moment this plan's worker pool exists,
  two actor methods on two different OS threads can both `raise`/
  `rescue` (plan 38) concurrently, and a shared mutable global handler
  stack would let one thread's `push_handler`/`raise` corrupt the
  other's — a real, silent correctness bug this plan would otherwise
  introduce as a side effect, not a hypothetical. `leaf-thread-safe-
  runtime` changes exactly one declaration, `static EmeraldHandler
  *emerald_handler_stack` → `static _Thread_local EmeraldHandler
  *emerald_handler_stack`, and nothing else about the exception
  mechanism; every existing single-threaded exception test keeps passing
  unchanged (thread-local storage behaves identically to a plain global
  from a single thread's point of view). By contrast, `emerald_alloc`/
  `emerald_alloc_zeroed` need **no** change — verified this session that
  they call straight through to `malloc`/`calloc`, and glibc's allocator
  is already thread-safe by default; this plan does not add locking
  there because there is nothing unsafe to lock.
- **Fixed worker count from `sysconf(_SC_NPROCESSORS_ONLN)`, overridable
  via `EMERALD_WORKERS`.** A sensible default with zero configuration
  for the common case, and a deliberate escape hatch this plan's own
  concurrency proof needs directly: forcing `EMERALD_WORKERS=1` is what
  makes the timing test's "serial" baseline reproducible on any machine
  regardless of its real core count, rather than inventing a second,
  parallel single-threaded code path just for testing.
- **Declined: work-stealing between per-worker queues.** A *single*
  shared global runnable-actor queue with all M workers as consumers
  already load-balances — an idle worker blocks on the shared condvar
  and the next actor any worker pushes onto the queue goes to whichever
  worker wakes first, with no actor ever pinned to a particular worker's
  private queue that could starve while another sits empty. Per-worker
  queues plus a Chase-Lev-style work-stealing deque are a real throughput
  optimization once lock contention on one shared queue is measured to
  matter — not a correctness requirement, and not something this plan's
  own worked proof needs to demonstrate. Declined as unnecessary
  complexity for v1, not overlooked.
- **Declined: mailbox back-pressure / size limiting.** Every mailbox is
  an unbounded linked list in v1 — a sender never blocks and never
  observes a "mailbox full" condition. This is a real, disclosed
  simplification with a real cost (a producer that out-sends a slow
  consumer indefinitely grows unbounded heap, with no GC to reclaim
  processed message nodes' memory pressure feedback either) — bounded
  mailboxes with a defined overflow policy (block the sender? drop the
  message? a distinct exception?) are genuine future design surface this
  plan does not need answered to prove the core dispatch-and-scheduling
  mechanism works.
- **What a message may actually contain is explicitly not this plan's
  question.** `argv` is a fixed-size array of raw 64-bit words — an
  `Int64` as-is, an `Float64` bit-cast, a pointer's raw address — copied
  by value into the message node at enqueue time, capped at a small
  fixed arity (16 words) with a compile-time diagnostic past that cap.
  Whether copying a pointer this way is *safe* for every argument type
  (a pointer into another actor's own isolated arena being handed across
  the isolation boundary this way is exactly the hazard plan 56, compile-
  time message safety, exists to close) is explicitly out of scope here
  — this plan plumbs whatever bits the call site already computed
  through the queue unchanged; it does not validate, deep-copy, or
  restrict what those bits may point to.
- **An implicit drain-and-join barrier is compiler-inserted at the end
  of generated `main`, not a language-visible `await`/join primitive.**
  Without it, `main`'s last statement returning could exit the process
  while actor mailboxes still hold unprocessed messages, making output
  nondeterministic on process-teardown timing rather than on program
  logic — exactly the kind of hidden nondeterminism this plan's own
  worked proof needs to *not* have. `define_main` (existing codegen
  function, `crates/emerald-codegen/src/lib.rs`) emits one extra call,
  `emerald_worker_pool_drain_and_join()`, as the last thing generated
  `main` does before its `ret`. This is runtime bookkeeping inserted by
  the compiler, not a new expression or statement a user ever writes —
  Emerald still has no `async`/`await` keyword anywhere in its grammar
  after this plan.

## Leaf: leaf-thread-safe-runtime

### 1. Context
- Why: every other leaf in this plan needs a real mailbox, a real
  worker pool, and a runtime that is actually safe to call into from
  more than one OS thread at once — none of which exists today (verified
  this session: `runtime/emerald_runtime.c` is 179 lines, single-
  threaded by its own comment, with no `pthread` include at all).
- Target state: `runtime/emerald_runtime.c` gains `#include <pthread.h>`
  and `#include <unistd.h>`; an `EmeraldMessage` node (`trampoline` fn
  pointer, a fixed `int64_t argv[16]`, `next`); an `EmeraldActorHeader`
  (a `pthread_mutex_t` guarding `head`/`tail`, an `int scheduled` flag,
  and a `next_runnable` link for the global queue); `emerald_actor_init_
  header(void *arena_base)` (called once per `.spawn`, right after
  allocation, before `initialize` runs); `emerald_actor_enqueue(void
  *self, void (*trampoline)(void*, int64_t*), int64_t *argv, int64_t
  argc)` (mallocs a node, copies `argv`, appends under the target
  actor's own mailbox mutex, and — only on the idle→runnable transition
  — pushes the actor's header onto the global runnable queue and signals
  the shared condvar); a global runnable queue (`pthread_mutex_t`/
  `pthread_cond_t`-guarded, plus an outstanding-message counter for
  drain); `emerald_worker_pool_start(void)` (spawns
  `sysconf(_SC_NPROCESSORS_ONLN)` pthreads, or the `EMERALD_WORKERS`
  environment variable's value when set and `> 0`, each running
  `emerald_worker_main`); `emerald_worker_main` (the pop-actor / pop-one-
  message / run-trampoline-to-completion / requeue-or-idle loop from the
  Decision log); `emerald_worker_pool_drain_and_join(void)` (waits on the
  outstanding-message counter reaching zero, then signals shutdown and
  `pthread_join`s every worker); `emerald_current_thread_id(void)`
  returning `(long long)(intptr_t)pthread_self()`. `static
  EmeraldHandler *emerald_handler_stack` becomes `static _Thread_local
  EmeraldHandler *emerald_handler_stack` — the one required change to
  existing code (see Decision log).

### 2. Acceptance Criteria
1. Every existing `emerald_runtime.c`-backed behavior (string ops,
   `Array.new`/`Hash` allocation, `raise`/`rescue`) still passes its
   existing tests unchanged after the `_Thread_local` change, run
   single-threaded exactly as today — a real regression check, not an
   assumption that thread-local storage is transparent.
2. A standalone C (or `cc`-compiled Rust `libtest`) unit test enqueues
   50 no-op messages across 5 distinct fake actor headers from a single
   thread, starts the pool with `EMERALD_WORKERS=4`, and asserts every
   message's trampoline runs exactly once and `drain_and_join` returns
   only once the outstanding-message counter is genuinely zero (not on a
   fixed sleep).
3. Two messages enqueued back-to-back onto the *same* actor's mailbox
   (from the same calling thread, no interleaving) are observed by the
   worker running them to fire in the order they were enqueued (a
   counter each trampoline increments and asserts against, i.e. message
   2's trampoline sees message 1's effect already applied) — this is the
   FIFO-ordering property `leaf-worked-concurrency-proof`'s ping-pong
   test depends on, exercised here at the runtime layer directly, before
   any codegen is involved.
4. `emerald_current_thread_id()` called from two different worker
   threads (via two enqueued messages guaranteed to run concurrently by
   forcing `EMERALD_WORKERS=2` and blocking each trampoline on a barrier
   until both have started) returns two different values.

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c`
- **Modify (only if `pthread_*` symbols require it on this toolchain —
  verify first, don't assume):** `crates/emerald-cli/build.rs` (adding a
  `-pthread` compile/link flag to the existing `cc::Build`, only if a
  bare build fails without it)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean (compiles `emerald_runtime.c` with no warnings-as-errors regression) | agent-claimed-locally |
| Runtime unit tests | `cargo test -p emerald-cli runtime_` | all pass, incl. new mailbox/pool tests | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pre-existing tests (esp. exception-model tests) still pass | agent-claimed-locally |

---

## Leaf: leaf-actor-header-and-trampolines

### 1. Context
- Why: `emerald_actor_enqueue` needs a real header prefix on every
  spawned actor's arena to hang its mailbox/scheduling state off of, and
  a real, statically-resolved function pointer per actor method to
  enqueue in the first place — neither exists until codegen emits them.
- Target state: whatever `.spawn` codegen plan 54 lands allocates
  `sizeof(EmeraldActorHeader)` extra bytes ahead of the instance's own
  field layout (mirroring how `ClassLayout`/`FieldInfo` already compute
  fixed byte offsets for ordinary fields — this is the same category of
  fixed-offset arithmetic, one level up), calls `emerald_actor_init_
  header` on the raw allocation, and hands `initialize` a `self` pointer
  offset past the header exactly the way `field_ptr` already offsets
  past a class's own leading fields. `declare_exception_runtime_funcs`
  gets a sibling, `declare_actor_runtime_funcs`, declaring
  `emerald_actor_enqueue`/`emerald_worker_pool_start`/`_drain_and_join`/
  `emerald_current_thread_id` as `extern` `FunctionValue`s the same way.
  For every method on an `actor` declaration, codegen emits one
  additional trampoline `FunctionValue` (uniform ABI `void(i8*, i64*)`)
  that unpacks `argv` per that method's own static `Param` types
  (`inttoptr`/`bitcast` as needed, mirroring `local_llvm_type`'s
  existing `ValKind`-to-LLVM-type mapping) and tail-calls the real
  method function — structurally one more per-method `FunctionValue`,
  the same shape of work `declare_lambda_functions`/`define_lambda`
  already do per lambda.

### 2. Acceptance Criteria
1. A spawned actor's field reads/writes (`@name`, etc.) compile and run
   identically to before this leaf once the header-prefix offset is
   applied — a real regression check that the header doesn't silently
   corrupt field layout (compare against plan 54's own field-access
   tests, re-run unchanged).
2. For a two-method actor, codegen produces exactly two trampoline
   functions in the emitted LLVM module (inspected via `module.print_to_
   string()` in a codegen unit test) — real, generated, addressable
   functions, not stubs.
3. A trampoline called directly (bypassing the mailbox entirely, from a
   small test harness that hand-builds an `argv` array and calls the
   trampoline's function pointer) produces the same observable result as
   calling the underlying method function directly — proving the
   argument-unpacking is correct independent of the scheduler.
4. `declare_actor_runtime_funcs`'s declared signatures link successfully
   against `leaf-thread-safe-runtime`'s actual C implementations (a real
   `compile_to_object` + link + run, not just a signature-shape
   assertion).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (actor arena layout /
  header offset, `declare_actor_runtime_funcs`, per-method trampoline
  emission)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. new trampoline-emission and direct-trampoline-call tests | agent-claimed-locally |

---

## Leaf: leaf-cross-actor-dispatch

### 1. Context
- Why: nothing yet decides, at a given call site, whether to call a
  method body directly or to enqueue it — plan 54 left every actor
  method call as an ordinary direct call (see this plan's opening
  section); this leaf is the actual behavior change the plan is named
  for.
- Target state: `crates/emerald-sema/src/lib.rs`'s method-declaration
  checking (near `check_method_body`/`function_signature`) rejects an
  `actor`-declared method with a non-empty return type, except
  `initialize`, with a real diagnostic naming the offending method — not
  a panic, matching this project's standing diagnostic convention.
  `build_method_call` (`crates/emerald-codegen/src/lib.rs`) gains the
  syntactic branch from the Decision log: if the receiver `Expr` is
  exactly `Expr::Ident("self")` (or the call is an implicit-self call
  inside an actor method body), compile it exactly as today; otherwise,
  if the receiver's static type (from sema's `Type`/`ClassInfo`,
  extended with an actor flag) is an actor type, compile a call to
  `emerald_actor_enqueue` passing that method's trampoline pointer and a
  packed `argv` built from the call's own arguments (reusing the
  existing per-argument `build_expr` machinery, packed via the same
  `ValKind`-aware bit patterns `build_hash_lit`/`build_array_lit` already
  use for heterogeneous storage) instead of a direct call. `define_main`
  emits `emerald_worker_pool_start()` before the program's first
  statement and `emerald_worker_pool_drain_and_join()` as the last thing
  before `main`'s `ret`.

### 2. Acceptance Criteria
1. An actor method declared with a return type (e.g. `def get -> Int64`)
   is rejected by sema with a real diagnostic; the same method with no
   return type is accepted.
2. `initialize` on an actor is accepted with no return type required and
   is *not* subject to the enqueue rule even in principle — verified by
   confirming codegen never builds an `emerald_actor_enqueue` call for
   it (it's only ever invoked from `.spawn`'s own generated code).
3. A call through a stored actor-typed field (this plan's own `@peer.hit`
   shape) compiles to an `emerald_actor_enqueue` call, verified by
   inspecting the emitted LLVM IR for the call site — not merely by the
   program's eventual output.
4. A call through the literal `self` receiver inside an actor method
   body compiles to the same direct-call instruction sequence
   `build_method_call` already produces for an ordinary class method —
   zero enqueue overhead for the same-actor case.
5. Regression: every prior plan's example (including plan 54's own
   worked actor example, once it exists) still parses, type-checks, and
   — for non-actor code — compiles identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (actor-method return-type
  rule), `crates/emerald-codegen/src/lib.rs` (`build_method_call`'s
  dispatch branch, `define_main`'s pool start/drain calls)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema -p emerald-codegen` | all pass, incl. new return-type-rule and dispatch-branch tests | agent-claimed-locally |

---

## Leaf: leaf-worked-concurrency-proof

### 1. Context
- Why: this plan's whole claim — real asynchronous, concurrently-
  scheduled, still-deterministic-per-actor message passing — is only
  proven once a real program compiles, links, runs, and produces the
  exact evidence the task requires: correctness *and* genuine OS-thread
  overlap, argued separately because the ping-pong example alone cannot
  prove the second half (see below).
- Target state: this plan's two worked examples (`PingPong`, `Spinner`)
  land as new const `*_EXAMPLE` strings with round-trip
  compile-link-run tests, following the exact convention already used
  by every prior codegen example (`INHERITANCE_EXAMPLE`,
  `BLOCKS_EXAMPLE`, etc., verified present in `crates/emerald-codegen/
  src/lib.rs`'s own test module); a new integration test in
  `crates/emerald-cli/tests/` (alongside the existing `benchmarks.rs`,
  `examples.rs`, `hello_em.rs`, `portability.rs` — verified present this
  session) runs the compiled `Spinner` binary twice, once with
  `EMERALD_WORKERS=1` and once with the default pool, timing each via
  `std::time::Instant` around the `std::process::Command` invocation
  exactly the way `benchmarks.rs` already times compiled output.

### 2. Acceptance Criteria
1. `PingPong`, compiled, linked, and run, prints exactly `A 1`, `B 1`,
   `A 2`, `B 2`, `A 3`, `B 3`, `A 4`, `B 4`, `A 5` (nine lines, in that
   exact order) on every run, repeated 20 times in the test to catch
   scheduling-order flakiness rather than asserting it once — genuine
   per-actor FIFO correctness under a real multi-worker pool (the test
   forces `EMERALD_WORKERS=4` specifically so a single-worker pool can't
   trivially "prove" ordering that only holds by accident of running
   everything on one thread).
2. Explicitly documented in the test (as a code comment, not just this
   plan): the `PingPong` example, by construction, never has both actors
   simultaneously runnable — `A` cannot process hop 3 until it has sent
   and `B` has fully processed hop 2 — so it is **only** a correctness/
   ordering proof, never offered as the concurrency proof.
3. The `Spinner` timing test asserts the multi-worker run's total
   wall-clock time is less than 1.6x a single `spin(200000000)` call's
   own measured time (itself measured directly, once, at `EMERALD_
   WORKERS=1`, as the baseline unit) — a real, generous-margin assertion
   that is only satisfiable if the two `spin` calls actually executed
   concurrently on separate OS threads; a purely serialized execution
   (any single-threaded interleaving, however clever) would cost close
   to 2x, failing the assertion. The test is marked to skip (not fail)
   on a detected single-core CI runner, disclosed as a real environment-
   dependent limitation of a wall-clock-based proof.
4. As supplementary, best-effort evidence (not the primary concurrency
   proof — disclosed as probabilistic, since actor-to-thread assignment
   isn't fixed): the `PingPong` example additionally calls `current_
   thread_id()` (a small new diagnostic builtin wired through sema/
   codegen the same way `puts` is special-cased, lowering to
   `emerald_current_thread_id`) and logs it per hop when run with
   `EMERALD_WORKERS=4`; across repeated runs the test observes at least
   2 distinct thread ids somewhere in the 20-run aggregate.
5. Full regression: `cargo test --workspace` still passes with all four
   leaves' changes applied together, not just each leaf's own isolated
   test suite.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`PINGPONG_EXAMPLE`,
  `SPINNER_EXAMPLE` consts + round-trip tests, `current_thread_id()`
  builtin wiring), `crates/emerald-sema/src/lib.rs` (`current_thread_id()`
  builtin type-checking)
- **Add:** a new test file under `crates/emerald-cli/tests/` (e.g.
  `actor_concurrency.rs`) for the process-level timing comparison

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Correctness (repeated) | `cargo test -p emerald-cli actor_concurrency -- --test-threads=1` | ping-pong ordering holds across 20 repetitions | agent-claimed-locally |
| Concurrency proof | same command | timing assertion passes on a multi-core runner; skip is disclosed and logged on a single-core one | agent-claimed-locally |
| Full workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
# Concurrency correctness is inherently scheduling-sensitive: run the
# actor-specific suite a second time under a different worker count to
# catch anything the default run's timing happened not to exercise.
EMERALD_WORKERS=8 cargo test -p emerald-cli actor_concurrency
```
