2026-09-21T21:28:00Z

---
name: Channels Beyond the Actor Mailbox
overview: "`Timer.every(interval_ms, times, target)` — a narrow, single native function proving one legitimate pattern for bridging an external, Rust-owned event source into an actor's own mailbox via an internal `crossbeam-channel` (verified this session: `crossbeam-channel` 0.5.17, released 5 Sep 2026, 42.8M downloads/month, 21,438 dependent crates, chosen over `flume` — 19.7M downloads/month, explicitly \"Casually Maintained\" per its own README badge) used only between two Rust-owned background threads inside one native module's own implementation. This plan does NOT add general-purpose channels as an Emerald language feature. No `Sender`/`Receiver`/`Channel` type is ever declared, returned, passed, or constructible from Emerald source, in this plan or as precedent for any future one — the only Emerald-visible effect of the bridge is an ordinary actor mailbox message, indistinguishable from any other cross-actor call, delivered via the exact `emerald_actor_enqueue` runtime entry point plan 55's own cross-actor dispatch already uses."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-cargo-dependency
    content: "Add `crossbeam-channel = \"0.5\"` to `crates/emerald-rt/Cargo.toml`. Run plan 95's crate-vetting checklist against it explicitly, citing the comparative choice over `flume` in this plan's own Decision log rather than re-arguing it at review time."
    status: pending
  - id: leaf-on-tick-reserved-method-convention
    content: "`emerald-sema` requires any class passed as `Timer.every`'s `target` argument to be an `actor` (sema error otherwise, mirroring plan 54's `is_actor` gating precedent) that declares a method with the exact reserved signature `on_tick(n: Int64): Void` (sema error citing the missing method by name if absent — the same \"declared, not inferred\" precedent plan 55's own \"every cross-actor-reachable method must declare no return type\" rule already established). `emerald-codegen`'s `declare_actor_trampolines` (verified present, `crates/emerald-codegen/src/lib.rs` L3834-4009) is extended to also resolve and register `on_tick`'s trampoline for every actor class that declares one, using the exact same static, compile-time resolution mechanism it already uses for every ordinary cross-actor method — no new dynamic-dispatch-by-name mechanism is introduced anywhere (see Decision log)."
    status: pending
  - id: leaf-timer-bridge-native-function
    content: "Add `emerald_rt_timer_every(interval_ms: i64, times: i64, target_actor: *mut c_void, trampoline: extern \"C\" fn(*mut c_void, *const i64)) -> i64` to `crates/emerald-rt/src/lib.rs` (`emerald_rt_fn!`-wrapped per plan 92, returning a `timer_id` handle per plan 93's registry convention). Internally: spawn one dedicated OS thread (`std::thread::spawn`, never `rayon` — a single long-lived thread, not a work-stealing pool) that loops `times` times, sleeping `interval_ms` between iterations, and on each tick sends the current tick count over a `crossbeam_channel::Sender<i64>` to a second, already-running internal dispatcher thread; that dispatcher thread's `Receiver` loop calls `emerald_actor_enqueue(target_actor, trampoline, &tick_count, 1)` directly — the identical runtime entry point plan 55's own cross-actor dispatch codegen already calls (verified present, `runtime/emerald_runtime.c` `emerald_actor_enqueue` L822-879) — turning each tick into an ordinary actor mailbox message. A second, internal `crossbeam_channel::bounded::<()>(1)` \"stop\" channel is raced against the tick channel via `crossbeam_channel::select!` inside the ticking thread's own sleep loop (a `select!` between \"time elapsed\" via `after()` and \"stop requested\", not a plain `thread::sleep`) so a shutdown can interrupt a pending sleep promptly rather than waiting out the full remaining interval."
    status: pending
  - id: leaf-join-all-and-example
    content: "Add `emerald_rt_timer_join_all()` (`Timer.join_all(): Void`), which blocks the calling thread until every `Timer.every` bridge started so far has completed its `times` ticks (a `crossbeam_channel::Receiver::recv` on a per-timer completion signal, joined in sequence) and, critically, until plan 55's own worker-pool drain (`emerald_worker_pool_drain_and_join`, verified present at `runtime/emerald_runtime.c` L1023-1055/L1080-1118) has processed every message those ticks enqueued — so `Timer.join_all()` returning guarantees every `on_tick` call has actually run, not merely that it was enqueued. Add `examples/timer_bridge_proof.em` (this plan's Concrete Proof below) wired into `emerald-cli/tests/examples.rs`'s CI-checked table, and run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 179 — Channels Beyond the Actor Mailbox

This is the third and most tension-laden of the three plans in this
batch (177, 178, 179) confronting the same question: Emerald already has
exactly one message-passing primitive, the actor mailbox (plans 54/55) —
one trampoline-typed message per method call, delivered to exactly one
owning actor, processed under plan 55's own "at most one message in-
flight per actor" invariant, with the sending side's `emerald_actor_
enqueue` call never blocking. A general-purpose MPMC channel — a real
`crossbeam-channel`/`flume` `Sender<T>`/`Receiver<T>` pair exposed as an
Emerald type — would be a second, structurally incompatible primitive:
nothing marks a channel instance as belonging to exactly one actor, so
multiple actors (or `main`) could hold either end simultaneously, and
*receiving* from inside an actor method body has no principled
implementation at all under the current scheduler — blocking on `recv()`
would freeze that actor's one execution slot indefinitely (starving its
own mailbox, since the scheduler assumes a dequeued message's handler
runs to completion promptly), and polling would mean inventing a second
scheduling discipline running alongside plan 55's own, forking Emerald's
concurrency story into two incompatible halves. **This plan states its
scope boundary firmly: it does not add general-purpose channels as an
Emerald language feature.** The one real, narrow, justified use case it
does build — bridging an external, time- or event-driven source into an
actor's mailbox — is implemented entirely with two Rust-owned background
threads that Emerald source never sees, addresses, or can construct a
second instance of.

## Concrete proof this plan targets

```ruby
actor Ticker
  count: Int64

  def initialize
    @count = 0
  end

  def on_tick(n: Int64)
    @count = @count + 1
    puts "tick " + @count.to_s
  end
end

t: Ticker = Ticker.spawn()
Timer.every(10, 3, t)
Timer.join_all()
puts "done"
```

Expected output, byte-for-byte, every run, regardless of the real
10-millisecond interval's actual wall-clock jitter: `tick 1`, `tick 2`,
`tick 3`, `done`. The tick *count* is exactly `3` by construction (this
plan's `times` parameter is a hard limit, not a best-effort target — the
bridge thread exits after exactly three sends, mirroring plan 55's own
"a real halting condition, not a fixed iteration count baked into the
harness" posture from its ping-pong proof), so this output is
deterministic independent of scheduling speed the same way plan 55's own
5-round ping-pong proof is — only `Timer.join_all()`'s own real wall-
clock wait varies run to run, never the printed sequence.

## Decision log

- **`crossbeam-channel` over `flume`, verified and compared this
  session, not picked by reputation alone.** `lib.rs`'s real crate
  pages: `crossbeam-channel` `0.5.17` (5 Sep 2026), 42,793,516
  downloads/month, 21,438 dependent crates (2,637 directly); `flume`
  `0.12.0` (8 Dec 2025 — nine months stale relative to `crossbeam-
  channel`'s release cadence), 19,706,365 downloads/month, 8,345
  dependent crates, and — decisively — `flume`'s own README displays a
  "[Casual Maintenance Intended](https://casuallymaintained.tech/)"
  badge, the maintainer's own explicit, disclosed signal of a lower
  ongoing-maintenance commitment than `crossbeam-channel`'s active,
  same-day-as-`crossbeam`-core release cadence (plan 178's own crate,
  verified released the identical day). `crossbeam-channel` also
  reuses the exact same `crossbeam-rs` dependency family plan 178
  already vets in this batch — one trusted org to evaluate under plan
  95's checklist, not two independent ones for two adjacent plans.
- **The tension, restated precisely, and the scope boundary this plan
  holds firmly.** See the framing above — a general channel type has no
  principled non-blocking-receive story inside an actor method body
  under plan 55's scheduler. This plan's own Decision log states this
  as a firm, permanent boundary, not a temporary v1 gap: **no
  `Sender`/`Receiver`/`Channel` Emerald type is added by this plan, and
  none should be added by extending this plan's own pattern later** —
  the internal channel this plan builds exists only *between two Rust
  threads this plan itself spawns and owns*, never handed to or
  constructible from Emerald source at any point.
- **The resolution: an internal bridge, terminating in the exact same
  runtime call plan 55's own cross-actor dispatch already uses.** The
  ticking thread and the dispatcher thread are both entirely Rust-
  owned, started and joined only by `emerald_rt_timer_every`/`Timer.
  join_all` themselves; the `crossbeam_channel::Sender`/`Receiver` pair
  between them is never exposed past that boundary. The dispatcher
  thread's only externally-visible action is calling `emerald_actor_
  enqueue` — the identical, already-verified entry point (`runtime/
  emerald_runtime.c` L822-879) `Expr::Spawn`'s sibling cross-actor call
  codegen already targets for an ordinary `a.foo(args)` call — so, from
  the receiving actor's own `on_tick` method body's perspective, a
  timer-driven message and an ordinary cross-actor call are genuinely
  indistinguishable: both arrive as a claimed mailbox entry processed
  under the identical scheduler invariant, with no second delivery
  mechanism for Emerald code to reason about.
- **`on_tick(n: Int64): Void` is a fixed, reserved method-name
  convention — a deliberate design choice protecting the "no dynamic
  dispatch" identity constraint.** A naive API — `Timer.every(ms, target,
  method_name: Symbol)`, letting Emerald source name an arbitrary
  target method by symbol at runtime — was considered and rejected
  outright: resolving a symbol to a method trampoline at runtime is
  exactly the `method_missing`/`send`-style dynamic dispatch this
  project's identity constraints (verified, restated in plan 59's own
  Decision log) rule out categorically. `on_tick` fixes the target
  method's name at the grammar/sema level instead, letting `emerald-
  codegen`'s `declare_actor_trampolines` resolve it exactly the way it
  already resolves every other actor method — a compile-time, per-
  actor-class static lookup, with zero new dispatch machinery. This is
  a real, disclosed narrowing (only one fixed event-kind name exists in
  v1) in exchange for reusing plan 55's own trampoline mechanism
  completely unmodified.
- **`select!` racing "tick" against "stop" is this plan's actual, real
  justification for taking a channel dependency here at all, rather
  than a plain condvar.** A single ticking thread needs to sleep for
  `interval_ms` *unless* a shutdown is requested (process exit,
  `Timer.join_all` after the last tick), in which case it must wake
  promptly rather than finish its current sleep. Hand-rolling this
  correctly with a raw `pthread_cond_timedwait`-equivalent is exactly
  the kind of fiddly, easy-to-get-subtly-wrong synchronization code
  this batch's own directive (prefer a vetted crate over hand-rolled
  primitives) exists to avoid — `crossbeam_channel::select! { recv(tick_
  after) -> _ => ..., recv(stop_rx) -> _ => ... }` gets the race
  correct by construction, which is the real, load-bearing reason this
  plan reaches for a channel crate instead of `std::sync::Condvar`
  directly.
- **New FFI/ABI surface plan 92 did not previously cover: an actor
  reference crossing the extern boundary as a raw pointer.** Plan 92's
  own type allow-list is `{Int64, Float64, String, CString, Void}` —
  no actor/class-instance pointer type. `emerald_rt_timer_every`'s
  `target_actor: *mut c_void` parameter is real, new, disclosed surface
  this plan adds: an actor instance is already a plain heap (arena)
  pointer at the `ValKind::Ptr` level in codegen, so passing it across
  the extern boundary needs no new *runtime* representation — but it is
  the first plan in this batch to actually declare and rely on a raw
  opaque-pointer parameter/return shape in an `emerald_rt_*` signature,
  extending plan 92's convention rather than fitting inside its
  original four-type list. Cited here explicitly so a future crate-
  vetting/ABI review knows this widening exists and where it started.
- **`Timer.join_all` composes with, rather than replaces, plan 55's own
  drain-and-join barrier.** A compiled program's implicit end-of-`main`
  drain (plan 55, cited) already waits for every actor's mailbox to go
  idle before the process exits — `Timer.join_all()` is this plan's own,
  separate, *earlier* synchronization point, letting a program
  deterministically observe "all ticks have both fired and been
  processed" before continuing, rather than relying on process-exit
  timing alone; it does not change or duplicate the scheduler's own
  final barrier.
- **Out of scope.** No `Sender`/`Receiver`/`Channel` Emerald type, ever
  (the firm boundary this plan's whole framing establishes). No
  arbitrary user-chosen target-method name (`on_tick` only, in v1 — a
  future plan could generalize the reserved-name convention to further
  fixed event kinds, e.g. `on_file_change`/`on_watch_error` for plan
  149's real filesystem-watcher bridge, named explicitly as the
  motivating future use of this exact pattern, not implemented here).
  No distributed bridging (plan 60's distributed actors are untouched).
  No async/future integration (plan 94's territory, untouched). No
  cancellation of an individual in-flight `Timer.every` short of
  process-wide `Timer.join_all`/exit (a `Timer.cancel(timer_id)` is real,
  disclosed future work, not built here).
