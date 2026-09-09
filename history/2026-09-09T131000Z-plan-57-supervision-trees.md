---
name: Supervision Trees
overview: "`supervise do ... end` registers a one_for_one restart policy over the actors `.spawn`ed in its body. A message handler's uncaught exception (plan 38) terminates that one actor — worker slot freed, arena bulk-freed — and, if that actor is tracked by a supervisor, the supervisor re-`.spawn`s it fresh with its original arguments; `one_for_all`/`rest_for_one` and Erlang-style restart-intensity limits are explicitly declined as future work."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-crash-isolation
    content: "An uncaught exception inside an actor's message handler is caught by a synthetic handler frame at the top of that actor's message-processing loop, reusing plan 38's push_handler/setjmp/emerald_raise mechanism verbatim; the actor terminates (worker slot freed, arena bulk-freed) without taking down the process, whether or not it is supervised"
    status: pending
  - id: leaf-supervise-declaration
    content: "`supervise do <name> = <Class>.spawn(<args>) ... end` grammar/AST/sema: body restricted to spawn-assignment statements, each tracked child's evaluated spawn arguments captured verbatim in the supervisor's own record, `supervise` evaluates to a `Supervisor` value"
    status: pending
  - id: leaf-one-for-one-restart-runtime
    content: "The Supervisor is itself compiled as an ordinary actor that reacts to an internal child-terminated system message by re-`.spawn`ing the failed child alone (one_for_one) with its captured original arguments and logging the restart; `child(name)` returns the current live reference, old references are never implicitly rebound"
    status: pending
isProject: false
---

# Plan 57 — Supervision Trees

This is plan 57, the tenth and final plan of the 48-57 batch implementing
"Beyond the Ceiling" — the follow-up analysis that proposed an actor-model
concurrency pillar with Erlang/BEAM-style "let it crash" supervision as a
genuine fifth capability class for Emerald, on top of the four already-
implemented batches (17-27, 28-35, 36-47). Like every batch before it, this
is post-v1 scope; `plan-of-plans.md` is not touched by this plan or any
other plan file. This is the fourth and last of four concurrency-pillar
plans, each owning one distinct layer of the same feature: plan 54 (actor
declarations, `.spawn`, per-actor isolated arena) → plan 55 (the worker-
thread-pool scheduler and inter-actor message passing) → plan 56 (compile-
time message-shape safety) → this plan, which is the only one of the four
that is about *failure* — what happens when a message handler doesn't
return normally.

This plan depends on **plan 55** (scheduler and message passing) and
**plan 38** (full exception model — read in full this session; its file
already exists at `history/2026-09-09T104000Z-plan-38-full-exception-
model.md`). It references **plan 54** (actor declarations, for `.spawn`
and the per-actor arena) and **plan 51** (arena, for the bulk-free-on-
termination tie-in). Plans 51 and 54 are, at the time this plan is
authored, siblings in the same not-yet-fully-written batch; where their
exact contracts matter, this plan states the assumed contract explicitly
rather than inventing unstated internals, exactly as plan 47 (repl-and-
test-framework, `history/2026-09-09T113000Z-plan-47-repl-and-test-
framework.md`) already disclosed depending on plan 17's not-yet-built
`emerald-driver` crate by its documented contract rather than its
(absent) code.

Concrete proof this plan targets — one supervisor watching two children,
proving three things in a single run: normal operation, a crash that
does not take the process down, and `one_for_one` restart scoped to
exactly the failed child:

```ruby
class Boom
end

class Worker
  count: Int64

  def initialize(seed: Int64)
    @count = seed
  end

  def handle(n: Int64)
    @count += n
    if @count == 3
      raise Boom.new
    end
    puts @count
  end
end

class Logger
  prefix: String

  def initialize(prefix: String)
    @prefix = prefix
  end

  def handle(n: Int64)
    puts @prefix
  end
end

sup = supervise do
  worker = Worker.spawn(0)
  logger = Logger.spawn("log")
end

w = sup.child(:worker)
l = sup.child(:logger)

w.send(1)          # count 1 -> 1
w.send(1)          # count 2 -> 2
l.send(1)           # sibling, unaffected -> "log"
w.send(1)          # count 3 -> raises; Worker terminates; supervisor restarts it
w2 = sup.child(:worker)
l.send(1)           # sibling still the SAME instance, still fine -> "log"
w2.send(1)         # fresh instance: 0 + 1 = 1, not 4 -> 1
```

Expected stdout, in order:
```
1
2
log
restarting Worker
log
1
```

Trace: the first two sends accumulate normally on the original `Worker`
instance. The `Logger` send in between proves ordinary sibling traffic is
unaffected by anything to do with `Worker`. The third `Worker` send pushes
`@count` to `3`, hits `raise Boom.new`, and produces no output of its own
— the raise happens before that handler invocation's `puts`. Plan 38's
handler-stack mechanism (reused, not reinvented — see Decision log) longjmps
to a synthetic per-iteration handler at the top of `Worker`'s own message
loop; that worker-thread slot frees the actor's arena in bulk and, because
this `Worker` is tracked by `sup`, enqueues a child-terminated notification
into `sup`'s own mailbox. The supervisor processes that notification,
prints `restarting Worker`, and re-`.spawn`s a fresh `Worker` from the
*original* captured argument (`0`), not from whatever `@count` had reached.
The `Logger` send immediately after proves `one_for_one`: it never restarted,
never logged anything about being restarted, and used the *same* reference
it always had — sibling isolation, not incidentally true but the entire
point of choosing `one_for_one` over its Erlang siblings (see Decision
log). Finally, `sup.child(:worker)` is called again to fetch the *new*
live reference (this plan explicitly declines to rebind `w` itself — see
Decision log point 2) and a fresh send against it prints `1`, not `4`: a
left-implemented restart that merely re-ran `initialize` on the *same*,
still-allocated actor object without a truly fresh arena and a truly fresh
`.spawn` would either corrupt this number or leak the crashed instance's
memory; printing `1` is the disconfirming proof that didn't happen.

## Decision log

- **The catch point is plan 38's existing handler stack, at the top of
  the actor's message loop — not a second, parallel exception mechanism.**
  Verified this session against real, current `runtime/emerald_runtime.c`
  (lines 116-179): `EmeraldHandler` is a `jmp_buf` plus `exception_tag`/
  `exception_ptr`/`prev`, threaded through a global `emerald_handler_stack`
  linked list; `emerald_push_handler` conses a new frame, Cranelift/LLVM-
  generated code calls `setjmp` directly against `emerald_handler_jmpbuf`
  (the C standard requires the call site's frame to still be live, so no
  wrapper function exists — plan 38 reconfirmed this, this plan changes
  none of it); `emerald_raise` unlinks the top handler, records the tag/
  pointer, and `longjmp`s to it, or — if the stack is `NULL` — prints
  `"uncaught Emerald exception (class tag %lld)"` to stderr and calls
  `exit(1)`. That `exit(1)` path is exactly the behavior this plan must
  *not* let an in-handler `raise` reach for an actor's message dispatch:
  codegen for every compiled actor's message-loop body (the function the
  worker-thread pool invokes once per popped message, per plan 55's
  contract) wraps that body in exactly the shape `build_begin` already
  generates for a user `begin`/bare `rescue` — `push_handler`, `setjmp`,
  branch on the result — except this handler frame is compiler-synthesized
  around the whole dispatch, invisible to Emerald source, and unconditional
  (the synthetic equivalent of plan 38's bare `rescue => e`, which plan 38's
  own Decision log already establishes matches unconditionally and binds
  nothing into `env` — here there is no `env` to bind into at all, since
  no user code names this handler). On the `setjmp`-returned-nonzero path,
  codegen reads `emerald_handler_tag`/`emerald_handler_exception_ptr` (for
  the "restarting X" / diagnostic message), calls `emerald_free_handler`
  (the handler was already unlinked by `emerald_raise`, matching plan 38's
  own documented split between `emerald_pop_handler` and `emerald_free_
  handler`), and then runs this plan's termination cleanup instead of
  resuming normal dispatch. A *user* `rescue` inside the message handler
  still intercepts first, exactly as today — this synthetic frame only
  ever fires for what would otherwise have been a true process-fatal,
  uncaught exception, so a handler that catches its own errors is
  completely unaffected by this plan.
- **A real, load-bearing prerequisite this plan states rather than
  assumes: `emerald_handler_stack` must stop being a single process-global
  by the time this plan's mechanism runs.** The comment directly above its
  declaration (`runtime/emerald_runtime.c:113-115`, verified this session)
  reads "Single-threaded only (no ownership/concurrency in v1 — inception
  §12/§20), so a plain global linked list is enough" — true when plan 11
  wrote it, false the instant plan 55 puts more than one worker thread
  live. Two actor worker threads both `longjmp`ing through one shared
  global linked list would corrupt each other's control flow the moment
  they overlap: one thread's `push_handler` could hand back a frame the
  other thread's `emerald_raise` unlinks. This plan's crash-isolation
  mechanism is only correct if plan 55 has already converted `emerald_
  handler_stack` (and `emerald_push_handler`/`emerald_raise`'s references
  to it) to thread-local storage — one independent stack per worker
  thread — so each actor's synthetic per-message handler frame lives on
  its *own* thread's stack, never contending with any other actor's.
  This plan does not re-litigate that conversion (it belongs to plan 55,
  which owns the scheduler and its threads); it states the dependency
  explicitly, the same discipline plan 47 already used for its own
  not-yet-built `emerald-driver` dependency, rather than silently assuming
  a global becomes thread-safe on its own.
- **Termination's cleanup reuses plan 51's arena bulk-free as a second
  trigger, exactly as plan 51's own Decision log already flags it.** Per
  this batch's stated cross-plan sequencing, plan 51 (arena) frames bulk-
  free primarily around function-return-style scope exit, but explicitly
  calls out actor termination as a second, later-arriving valid trigger
  for the identical free-the-whole-arena-at-once operation — not a new
  free function, not a per-object sweep, the same bulk operation plan 54
  already runs when a spawned actor's own top-level message-loop function
  returns normally (which, per plan 54's own framing, an actor's message
  loop never actually does in steady state — it loops forever pulling
  from its mailbox). This plan is that second trigger materializing: the
  synthetic handler's catch path calls the identical bulk-free entry
  point plan 54 wired up, immediately before the worker-thread slot is
  released back to the pool (per plan 55's contract) — reuse, not a
  parallel cleanup path, and precisely why plan 54's own framing (`fires
  on function return`) was already disclosed as incomplete rather than
  wrong.
- **`supervise` is a block, reusing plan 34's block mechanism, not a
  declarative list.** A declarative form (e.g. `supervise [Worker, Logger]`
  or a config-table shape) was considered and rejected: `.spawn`'s
  argument lists are ordinary Emerald expressions (`Worker.spawn(0)`,
  not `Worker.spawn(literal_only)`), and a block body is the only shape
  already in this language capable of holding a sequence of ordinary
  statements with ordinary expression arguments — inventing a second,
  restricted expression-list mini-grammar just for supervised children
  would duplicate call syntax the language already has. The block's body
  is deliberately **not** a general statement sequence: sema restricts it
  to a flat list of `<name> = <ClassName>.spawn(<args>)` statements (a
  bare `<ClassName>.spawn(<args>)` with no binding is also legal — its
  child is simply unnamed and unreachable via `child(name)` later, a real
  but narrow gap this plan accepts rather than forcing every spawn to be
  named). Any other statement shape inside the block — a `puts`, an `if`,
  a second unrelated call — is a diagnostic, not silently ignored and not
  a panic, the same posture plan 31 already used for its own restricted-
  shape leaves.
- **Original spawn arguments are captured as already-evaluated values at
  first-spawn time, in the supervisor's own record — not re-evaluated
  expressions, and not the same actor object reused.** Concretely: when
  `worker = Worker.spawn(0)` runs inside the `supervise` block, codegen
  evaluates `0` once (trivial here, but the same rule holds for an
  arbitrary expression argument) and stores that evaluated value directly
  in a small compiler-synthesized record living on the `Supervisor`
  actor's own heap — keyed by the bound name (`"worker"`) alongside the
  class's spawn entry point. On restart, the supervisor calls that same
  spawn entry point again, passing back the *stored* value, not re-running
  the original argument expression (which might reference a now-out-of-
  scope local, or have a side effect that shouldn't repeat) and not
  attempting to resurrect or reset the crashed instance in place (its
  arena is already gone by the time restart runs — see the crash-
  isolation leaf). This is a genuinely fresh `.spawn`, identical in every
  way to the first one except that it happens later and is triggered by
  the supervisor rather than by user code reaching the `supervise` block.
- **A restarted child gets a brand-new reference; senders holding the old
  one are not transparently redirected — the real Erlang fork, resolved
  by following Erlang's own answer, not inventing a friendlier one.** Real
  Erlang does not rebind a restarted child's Pid either: a `one_for_one`
  supervisor restart produces a genuinely new process with a new Pid; the
  old Pid is permanently dead, and any message sent to it is silently
  dropped (Erlang message sends never error, dead or alive) — code that
  needs to keep reaching "whichever instance is current" registers a name
  through the supervisor and looks it up again, it does not hold a raw Pid
  across a restart. This plan takes the identical position: `sup.child
  (:worker)` is the only durable way to reach a supervised child, it
  blocks until any restart in flight for that name has completed (the
  supervisor's own mailbox serializes a `child(name)` query behind any
  `__child_terminated` notification enqueued strictly before it in real
  time, since both travel through the same single mailbox — no separate
  synchronization primitive needed), and it returns whatever reference is
  currently live. A stale reference obtained before a crash (`w` in the
  worked example) still points at a mailbox with no worker behind it after
  termination; sending to it is defined to silently drop the message,
  matching Erlang's dead-Pid-send behavior exactly, rather than raising a
  new kind of runtime error this plan would otherwise have to invent and
  justify. The alternative — an indirection layer where every actor
  reference is a mutable cell the supervisor rewrites in place on restart
  — was rejected: it would tax every single message send in the entire
  concurrency pillar (plan 55's hot path) with an extra dereference to
  support a guarantee Erlang itself doesn't provide.
- **Exactly one restart strategy is built: `one_for_one`.** A failed
  child is restarted alone; its siblings are untouched — proven directly
  by the worked example's `Logger`, which never restarts and never even
  notices `Worker`'s crash. This plan explicitly declines, as real,
  disclosed future work and not a build-it-later oversight: Erlang's
  `one_for_all` (every sibling under the same supervisor is killed and
  restarted together whenever any one of them dies) and `rest_for_one`
  (siblings started *after* the failed one in declaration order are
  restarted alongside it, earlier siblings left alone) — both real,
  well-defined strategies this plan's `Supervisor` design could grow
  into later by changing only what the `__child_terminated` handler does
  with its tracked-children record, not by touching the crash-detection
  mechanism at all. Also explicitly declined: Erlang's restart-intensity
  limit (`MaxR` restarts within `MaxT` seconds, after which the
  supervisor itself gives up and terminates — Erlang's own circuit-
  breaker against a child that crashes immediately on every restart in a
  tight loop). Without it, a child that fails deterministically on every
  fresh spawn (not just this plan's worked example, which succeeds after
  restart) would restart forever — a real, disclosed gap, not invisible:
  anyone who knows Erlang and reads this plan's `Supervisor` should
  immediately recognize the missing backoff/circuit-breaker and not
  mistake its absence for an oversight.
- **The restart chain is real and recursive by construction, but this
  plan's own tested proof stops at one level.** A `Supervisor` is not a
  distinguished runtime type with special-cased scheduling — it is
  compiled and scheduled as an ordinary actor (its own message loop reacts
  to `__child_terminated` system messages and to `child(name)` queries),
  which means nesting `supervise do ... end` inside another `supervise`
  block's own body needs no new mechanism at all: the outer supervisor's
  spawn-tracking record simply names the inner `Supervisor` as one of its
  own children, and an unrecoverable failure in the inner supervisor's own
  management logic is caught by the exact same top-of-message-loop
  synthetic handler this plan already builds for every actor, propagating
  up to the outer supervisor's `one_for_one` restart like any other child
  failure. This was genuinely checked for being "just as easy to prove"
  and rejected as this plan's own acceptance criterion anyway: a two-level
  tree's worked example needs a *second* failure mode worth demonstrating
  (the inner supervisor's management logic itself crashing, as opposed to
  a leaf actor's handler crashing) to be a meaningful proof rather than a
  relabeled repeat of the one-level case, and authoring that second
  failure mode is real, additional design work this plan declines to
  rush — disclosed here as real future proof-of-concept work, not gated
  on this plan's own quality gate.
- **Tooling tie-in: a supervised crash/restart cycle is a real, natural
  candidate for plan 47's `assert`/`assert_eq` inside a `test "..." do
  ... end` block** (`history/2026-09-09T113000Z-plan-47-repl-and-test-
  framework.md`, e.g. `assert_eq(1, w2.result)` after a restart, once
  some observable result-returning path exists) — plan 47's `assert_eq`
  is literally the existing `==`-comparison checker invoked on a
  synthetic `Compare` node, so it works unmodified against whatever plain
  `Int64`/`String`/etc. values a post-restart actor produces; the only
  thing plan 47 doesn't provide on its own is a deterministic point to
  assert *at* — which is exactly what this plan's blocking `child(name)`
  already supplies (it does not return until any in-flight restart for
  that name has completed), so a test author never needs an ad hoc sleep
  to make a supervised-restart test deterministic. This plan does not
  build that test itself; it is named here because the determinism this
  plan's `child(name)` design already provides is precisely what would
  make such a test reliable rather than flaky, and that's worth stating
  plainly rather than leaving implicit.

## Leaf: leaf-crash-isolation

### 1. Context
- Why: this is the load-bearing mechanism every other leaf in this plan
  depends on — without it, nothing terminates for a supervisor to notice.
- Target state: every compiled actor's message-dispatch function (the
  function plan 55's worker-thread pool invokes once per popped mailbox
  message) is wrapped, at codegen time, in a compiler-synthesized
  `push_handler`/`setjmp` frame identical in shape to `build_begin`'s own
  (`crates/emerald-codegen/src/lib.rs`, `build_begin`, verified this
  session at lines 2879-3043), with an unconditional (bare-`rescue`-
  equivalent) catch arm. On catch: read `emerald_handler_tag`/`emerald_
  handler_exception_ptr` for diagnostics, call `emerald_free_handler`,
  invoke plan 51's arena bulk-free entry point (assumed contract) on this
  actor's own arena, release this actor's worker-thread slot back to
  plan 55's pool (assumed contract), and — only if this actor carries a
  supervisor back-pointer (set at `.spawn` time by `leaf-supervise-
  declaration`) — enqueue a `__child_terminated` message into the
  supervisor's own mailbox via plan 55's ordinary send path (no new IPC
  primitive). `runtime/emerald_runtime.c`'s `emerald_handler_stack` is
  required, as a prerequisite from plan 55, to already be thread-local
  rather than the current single process-global (verified stale comment,
  `runtime/emerald_runtime.c:113-115`) — this leaf's own acceptance
  criteria include a regression check that would fail loudly if that
  conversion were missing.

### 2. Acceptance Criteria
1. An unsupervised actor whose message handler raises an exception with
   no matching user `rescue` terminates: compiled, linked, and run, the
   rest of the program (main thread and any sibling actors) keeps running
   and producing its own subsequent output — the crash is not fatal to
   the process, directly falsifying `emerald_raise`'s existing `h == NULL
   -> exit(1)` path for actor contexts specifically.
2. The same scenario, run with two independent actors on two different
   worker threads each raising inside their own handler at overlapping
   times, terminates each independently with its own correct exception
   tag attributed to the correct actor — proving `emerald_handler_stack`'s
   thread-local conversion genuinely isolates the two stacks rather than
   one actor's `longjmp` occasionally landing in the other's frame.
3. Regression: a plain top-level (non-actor) uncaught exception still
   exits the process via the existing, unmodified `h == NULL` path —
   this leaf adds a synthetic frame only around actor message dispatch,
   never around ordinary function/method calls outside that context.
4. Regression: every prior plan's example (including plan 38's own
   multi-rescue/ensure/retry proof) still compiles and produces identical
   output — this leaf must not change `build_begin`'s existing codegen
   for user-written `begin`/`rescue`, only add a new, separate wrapping
   around actor dispatch entry points.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (new synthetic-handler
  wrapping around each compiled actor's message-dispatch entry point),
  `runtime/emerald_runtime.c` (no new functions — this leaf only
  consumes `emerald_push_handler`/`emerald_handler_jmpbuf`/`emerald_
  handler_tag`/`emerald_handler_exception_ptr`/`emerald_free_handler`,
  all unchanged; the thread-local conversion itself is plan 55's leaf,
  not re-done here — this leaf's own acceptance criterion 2 verifies it
  from the outside).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test (crash isolation, real run) | `cargo test --workspace` | all pass, incl. the unsupervised-crash-doesn't-kill-the-process proof and the two-concurrent-actors proof | agent-claimed-locally |

---

## Leaf: leaf-supervise-declaration

### 1. Context
- Why: without a way to name which spawned actors a supervisor tracks
  and what their original arguments were, there is nothing for
  `leaf-one-for-one-restart-runtime` to restart.
- Target state: grammar gains `"supervise" "do" SuperviseStmt* "end"`
  (reusing the `end`-as-closer convention already massively overloaded
  across this grammar, per plan 47's own precedent for its `test` block);
  `SuperviseStmt` is restricted, at the sema level, to `<name:Ident> "="
  <class:Ident> "." "spawn" "(" Expr* ")"` or a bare `<class:Ident> "."
  "spawn" "(" Expr* ")"` with no binding. `Stmt::Supervise { children:
  Vec<SupervisedSpawn> }` where `SupervisedSpawn { name: Option<String>,
  class_name: String, args: Vec<Expr> }`, evaluating to a `Supervisor`
  value. Sema wires each tracked spawn's actor object with a supervisor
  back-pointer (consumed by `leaf-crash-isolation`'s termination path)
  and records each spawn's evaluated argument values in the `Supervisor`'s
  own record, keyed by name where a name exists.

### 2. Acceptance Criteria
1. `supervise do worker = Worker.spawn(0) end` parses, type-checks, and
   compiles to a `Supervisor` value bound to `sup`; `sup.child(:worker)`
   resolves to a real, live actor reference identical in capability to
   one obtained from a bare `Worker.spawn(0)` outside any `supervise`
   block.
2. A `supervise` block body containing any statement shape other than a
   (bound-or-bare) `.spawn` call — e.g. `puts "x"` — is rejected with a
   diagnostic naming the offending statement, not silently dropped and
   not a panic.
3. A bare (unnamed) `Logger.spawn("log")` inside the block compiles and
   runs, but is unreachable via `child(name)` — proving the "unnamed
   child" gap is a real, working, disclosed narrowing rather than an
   accidental crash.
4. Regression: every prior plan's example still parses and type-checks
   identically — this leaf only adds a new top-level statement shape.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-sema/src/lib.rs`, `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. restricted-body-shape diagnostic and unnamed-child cases | agent-claimed-locally |

---

## Leaf: leaf-one-for-one-restart-runtime

### 1. Context
- Why: this is the leaf that actually restarts anything — the previous
  two leaves detect termination and record what to restart with, but do
  nothing about it on their own.
- Target state: a compiled `Supervisor` is itself an ordinary actor
  (spawned by the runtime the moment a `supervise do ... end` expression
  is evaluated), whose own message loop handles exactly two internal,
  compiler-synthesized message shapes: `__child_terminated(name)` —
  triggers a fresh `.spawn` of `name`'s class using its stored, already-
  evaluated original arguments, replaces that name's entry in the
  supervisor's own record with the new reference, and prints `restarting
  <ClassName>` — and `child(name)` — a blocking query returning the
  currently-live reference for `name`, guaranteed (by ordinary mailbox
  FIFO order, per plan 55's contract) to reflect any `__child_terminated`
  enqueued strictly before it in real time. A reference obtained before a
  restart is not rewritten in place; sending to it after its actor has
  terminated is defined to silently drop the message (see Decision log).

### 2. Acceptance Criteria
1. This plan's own worked example (`Worker` + `Logger` under one
   `supervise` block), compiled, linked, and run, prints exactly:
   `1`, `2`, `log`, `restarting Worker`, `log`, `1`, in that order.
2. `one_for_one` isolation: in that same run, `Logger` never prints a
   restart line and its `l.send(1)` calls both before and after `Worker`'s
   crash produce identical output (`log`) from the same underlying
   instance — a `one_for_all`-shaped implementation bug (accidentally
   restarting every tracked child on any one failure) would be caught by
   this criterion alone.
3. State-reset proof: the post-restart `w2.send(1)` prints `1`, not `4` —
   proving the fresh instance's `@count` started from the captured
   original argument (`0`) rather than inheriting the crashed instance's
   `@count == 3`, and proving no leftover/corrupted state survived the
   restart.
4. `w` (the pre-crash reference) used again after restart does not raise
   a runtime error and does not reach the new instance — its message is
   silently dropped, matching the Decision log's declared dead-reference
   semantics exactly (not left undefined).
5. Regression: an unsupervised actor's crash (no enclosing `supervise`
   block at all) still terminates and logs nothing about restarting —
   this leaf's machinery only activates for actors carrying a supervisor
   back-pointer, verifying `leaf-crash-isolation`'s baseline behavior is
   unchanged by this leaf's addition.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (Supervisor actor's
  synthesized message-loop body, `__child_terminated`/`child` handling),
  `crates/emerald-sema/src/lib.rs` (`child(name)` return type resolution
  against the supervised class's actor-reference type)
- **No changes to:** `runtime/emerald_runtime.c` — restart is ordinary
  `.spawn` plus ordinary mailbox send/receive, both already provided by
  plans 54/55; this leaf adds no new C runtime entry points.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test (real linked-and-run worked example) | `cargo test --workspace` | all pass, incl. the exact six-line stdout trace above | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
