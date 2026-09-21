2026-09-21T21:00:00Z

---
name: OS Signal Handling — Graceful Shutdown
overview: "`Signal.shutdown_requested(): Boolean` (a pollable, process-wide atomic flag) plus `Signal.on_shutdown(target): Void` (pushes one fixed notification message into a target actor's mailbox when the flag flips), both set by a single registered handler using the `ctrlc` crate's `termination` feature — chosen over the lower-level `signal-hook` as this plan's primary surface because `ctrlc` with `termination` enabled already handles exactly the three signals a graceful-shutdown feature needs (`SIGINT`, `SIGTERM`, `SIGHUP`) with one cross-platform call (Windows console-control-handler included, not just Unix), verified this session against its own real `crates.io`/`docs.rs` documentation. `signal-hook` is named explicitly as the future escape hatch for arbitrary-signal needs (`SIGUSR1`-style operational triggers) this plan's fixed three-signal set does not cover — not built here, since nothing in this plan's own Concrete Proof needs it."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ctrlc-crate-and-atomic-flag
    content: "Add `ctrlc = { version = \"3\", features = [\"termination\"] }` to `crates/emerald-rt/Cargo.toml`. A single `static EMERALD_RT_SHUTDOWN: std::sync::atomic::AtomicBool = AtomicBool::new(false)` in `crates/emerald-rt`. `emerald_rt_signal_init_once()` — idempotent (an `std::sync::Once`) — calls `ctrlc::set_handler(|| { EMERALD_RT_SHUTDOWN.store(true, Ordering::SeqCst); })`, registering once per process regardless of how many times Emerald source calls into this namespace. `emerald_rt_signal_shutdown_requested() -> bool` reads the flag. Both paths wrapped in `std::panic::catch_unwind` per plan 91's proven pattern; `ctrlc::set_handler`'s own callback runs on a real, ordinary thread context the crate itself manages (not a raw, async-signal-safe-restricted OS signal handler), so the atomic store here — and the mailbox push in the next leaf — are both safe to perform from inside it."
    status: pending
  - id: leaf-on-shutdown-actor-push
    content: "`emerald_rt_signal_on_shutdown(target_mailbox: <actor-ref ABI, per plan 92>) -> Void`: registers `target_mailbox` in a small Rust-side list; the `ctrlc` handler, in addition to setting the atomic flag, iterates that list once and pushes one fixed `:shutdown` system notification into each registered target's mailbox via the same mailbox-enqueue runtime entry point plan 149's `watch_actor` already reuses from plan 55's own `.send()` codegen — a second, independent confirmation that this is the right, general integration point for any non-actor producer needing to reach into the actor mailbox system, not a one-off invented for plan 149 alone."
    status: pending
  - id: leaf-sema-and-codegen-wiring
    content: "Add the `Signal` compiler-known namespace arm to `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`, matching the established hard-coded-arm shape; declare `emerald_rt_signal_init_once`/`emerald_rt_signal_shutdown_requested`/`emerald_rt_signal_on_shutdown` via `module.add_function(..., Some(Linkage::External))`; call `emerald_rt_signal_init_once` unconditionally at the top of generated `main`, alongside `ARGV`/`ARGC` population (plan 45's own precedent for what already runs unconditionally at `main`'s start)."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/signal_handling_proof.em` (the Concrete Proof below — see Decision log for how a proof involving a real external signal stays deterministic in a scripted test harness). Add `#[test]`s inside `crates/emerald-rt`: one asserting `emerald_rt_signal_init_once` is safe to call from multiple threads concurrently (simulating several actors each independently reaching `Signal.shutdown_requested` for the first time) and only ever registers one real handler; one asserting the atomic flag is `false` before any signal and `true` after a real, self-delivered `SIGTERM` (via `libc::raise` or `std::process::id()` + `kill`, sent from the test itself); one asserting `on_shutdown` actually enqueues into a stub mailbox target when the flag flips."
    status: pending
isProject: false
---

# Plan 151 — OS Signal Handling: Graceful Shutdown

Emerald's actor model runs a fixed-size worker-thread pool (plan 55)
executing potentially many live actors at once; a `SIGINT`/`SIGTERM`
arriving from outside the process (an operator pressing Ctrl-C, a process
supervisor sending `SIGTERM` before a hard kill) needs some way to reach
across that entire pool and ask every actor to wind down cleanly — closing
plan 148's held locks, plan 149's open watches, plan 147's open temp
files — rather than the process dying mid-write with whatever OS-level
defaults `SIGINT`/`SIGTERM` already do (terminate immediately, no
Emerald-visible chance to react at all). This plan's design deliberately
does not attempt to reach every live actor individually — no actor
registry for arbitrary broadcast exists anywhere in this project's
current design (plan 55/57 describe only supervisor-to-tracked-child
relationships, never an arbitrary all-actors broadcast) — instead, a
single process-wide atomic flag, readable from any OS thread with no
per-actor delivery step needed at all, is the real, reliable, always-
available shutdown signal; `Signal.on_shutdown(target)` is a convenience
layered on top for the common "one designated coordinator actor wants to
be pushed a message rather than poll" pattern, not a full broadcast.

## Concrete proof this plan targets

```ruby
puts Signal.shutdown_requested

# (In a real deployment: an operator sends SIGTERM/SIGINT/SIGHUP to this
# process's PID here. For a deterministic, scripted proof, this example
# is run under a test harness that sends a real SIGTERM to the running
# process shortly after it starts — see Decision log.)

attempts: Int64 = 0
while !Signal.shutdown_requested && attempts < 500000
  attempts += 1
end
puts Signal.shutdown_requested
puts "shutting down cleanly"
```

Expected output, in order, when the process genuinely receives a real
`SIGTERM` (or `SIGINT`/`SIGHUP`) shortly after starting:
```
false
true
shutting down cleanly
```

Trace: `Signal.shutdown_requested` is `false` immediately at startup (no
signal has arrived yet). The bounded busy-wait loop observes the flag
flip to `true` the moment the real, externally-delivered signal reaches
the process and `ctrlc`'s registered handler runs — a genuine,
externally-triggered state change, not a value this plan's own code sets
internally for the proof's convenience. `"shutting down cleanly"` prints
afterward, standing in for whatever real cleanup (closing locks, watches,
temp files) a production program would perform once it observes the flag.

## Decision log

- **`ctrlc` with the `termination` feature, not `signal-hook`, is this
  plan's primary and only implemented surface — a decision made on a
  real, verified fact about what `ctrlc` already covers, not a
  simplification made in ignorance of `signal-hook`'s existence.**
  Verified this session, directly against `ctrlc`'s own `crates.io`/
  `docs.rs` documentation: "Handling of SIGTERM and SIGHUP can be enabled
  with [the] termination feature. If this is enabled, the handler
  specified by `set_handler()` will be executed for SIGINT, SIGTERM, and
  SIGHUP" — a single, cross-platform (Windows console-control events
  included, not merely a Unix `signal()` wrapper) call already covers the
  entire real signal set a graceful-shutdown feature needs. `signal-hook`
  is a real, actively maintained (v0.4.4, April 2026), lower-level crate
  — its own description states "safe and correct Unix signal handling"
  — capable of registering arbitrary signals (`SIGUSR1`, `SIGUSR2`,
  custom operational triggers like "reload config"), but it is Unix-only
  and strictly more machinery than this plan's own three-signal graceful-
  shutdown scope needs. This plan names `signal-hook` explicitly as the
  natural extension point a future plan should reach for if Emerald ever
  needs to register a signal outside `ctrlc`'s fixed `{SIGINT, SIGTERM,
  SIGHUP}` set — not built here, because nothing in this plan's own
  Concrete Proof needs a signal `ctrlc` doesn't already cover.
- **`ctrlc`'s handler runs in an ordinary thread context the crate
  itself manages, not inside a raw, async-signal-safe-restricted OS
  signal handler — this is exactly why it is safe for this plan's
  handler to perform an atomic store and a mailbox push, both of which
  would be genuinely unsound inside a true POSIX signal handler.** A raw
  signal handler may only call a narrow, POSIX-specified list of
  async-signal-safe functions — allocating memory, acquiring most locks,
  and pushing into a data structure are all disallowed there. `ctrlc`'s
  entire reason to exist, rather than a bare `libc::signal` call, is to
  defer the actual user-supplied callback to a safe execution context;
  this plan's handler body (an atomic store, iterating a small Rust
  `Vec` of registered mailbox targets, calling the same mailbox-enqueue
  function ordinary code calls) is ordinary, unrestricted Rust code by
  the time it runs, not signal-handler-restricted code — a real,
  load-bearing property of the crate this plan depends on, verified
  against its own stated design purpose, not assumed.
- **The atomic flag, not a full broadcast to every live actor, is this
  plan's real answer to "how does a signal handler reach across the
  worker-thread pool" — a deliberate, stated choice against inventing an
  actor registry this project does not otherwise have.** Every actor's
  own `handle` loop can check `Signal.shutdown_requested()` at any
  natural yield point (the top of its own message loop, say) with zero
  new cross-thread machinery beyond a single `AtomicBool::load`, readable
  identically from every worker thread in plan 55's pool with no locking
  and no per-actor addressing needed at all. Building a true "push a
  message into literally every live actor's mailbox" broadcast would
  need a live-actor registry this project's actor model does not
  currently expose anywhere (plan 57's own Decision log describes only
  supervisor-to-tracked-child relationships, explicitly declining even
  `one_for_all` sibling-broadcast as future work, let alone an
  unrelated, project-wide broadcast) — inventing one as a side effect of
  this plan's signal-handling feature would be a materially larger,
  separate undertaking this plan does not attempt.
- **`Signal.on_shutdown(target)` connects to plan 57's supervision trees
  through ordinary, already-established mechanics, not through any new,
  unconfirmed internal Supervisor behavior this plan would have to
  invent.** Plan 57 states a `Supervisor` "is not a distinguished runtime
  type with special-cased scheduling — it is compiled and scheduled as
  an ordinary actor." A program can therefore define its own top-level
  shutdown-coordinator actor (an ordinary `class ShutdownCoordinator ...
  def handle(sig: Symbol) ... end`), register it via `Signal.on_shutdown
  (coordinator)`, and have that coordinator's own handler call whatever
  cleanup its own tracked children need — including, if it holds a
  `Supervisor` reference from its own `supervise do ... end` block,
  reaching tracked children via `sup.child(name)` (plan 57's own
  documented query mechanism) to run their cleanup. This plan does not
  assert any new capability inside `Supervisor`'s own compiler-generated
  message loop to receive `:shutdown` directly — the connection to
  supervision trees is made entirely through ordinary application code a
  program author writes, using primitives plan 57 already ships.
- **Idempotent registration (`std::sync::Once`) is not defensive
  boilerplate here — `ctrlc::set_handler` itself documents that a
  second, unguarded call is a real, observable failure, not a silent
  overwrite.** Verified this session against `ctrlc`'s own `docs.rs`
  page: `set_handler` (via `try_set_handler` underneath) "will error (on
  Unix) if another signal handler exists for the same signal(s) that
  `ctrlc` is trying to attach the handler to" — calling it twice from two
  independent call sites does not quietly replace the first registration,
  it returns `Err`, which the common `.expect("Error setting Ctrl-C
  handler")` idiom (used in `ctrlc`'s own documented examples) turns
  straight into a panic. Nothing in this plan's namespace design stops
  multiple, independent parts of an Emerald program from reaching
  `Signal` first — every actor's own startup code, potentially — so
  without a guard, whichever caller happens to run second would crash
  the process attempting to enable graceful shutdown, a real, ironic
  failure mode this plan avoids entirely: `emerald_rt_signal_init_once`'s
  `std::sync::Once` guarantees the real `ctrlc::set_handler` call
  executes exactly once per process, regardless of how many times or
  from how many threads Emerald source reaches into `Signal`.
- **Out of scope.** Arbitrary signal registration beyond `SIGINT`/
  `SIGTERM`/`SIGHUP` (the `signal-hook` escape hatch named above, not
  built); sending signals to other processes (a natural pairing with
  plan 145's spawned children — `Process.kill`/`.terminate` — a distinct,
  small follow-up this plan does not attempt); any automatic cleanup of
  plan 147/148/149's own resources when the flag flips — this plan
  provides the *signal*, not automatic resource teardown, which stays
  each program's own responsibility exactly as plan 93's model already
  requires explicit `.close()` calls generally; any change to
  `runtime/emerald_runtime.c`.

## Not yet decided

1. Whether `Process.run`'s (plan 145) blocking wait for a spawned child
   should itself check `Signal.shutdown_requested()` and forward a
   termination signal to the child — a real, plausible refinement this
   plan's own scope does not need to resolve, since this plan's Concrete
   Proof involves no spawned children.
