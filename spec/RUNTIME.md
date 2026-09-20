# Emerald — RUNTIME.md

**Status:** written retroactively, against the runtime as it actually exists
(`runtime/emerald_runtime.c`, 2,362 lines, as of plan 65) — not as a forward
design document. Inception §4 called for this file at project birth; it was
never written across 65 plans. Every claim below was checked against the
current runtime source and the current test suite this session, not carried
forward from plan records.

**Purpose:** answer, in one place, the question a user or contributor
actually has: *what does Emerald do with memory and with concurrency, at
runtime, right now* — since no other document says this and several plan
records' Decision logs each answer a fragment.

---

## 1. Memory model: no GC, no ownership system, by deliberate decision

Emerald has **no garbage collector and no borrow/ownership checker**. This
is not an oversight — it is inception §12's own instruction ("do not design
a complicated ownership/borrowing system for v1... first determine what
Ruby-like static semantics require"), taken literally and still true today.

1. **`ClassName.new` allocates via `emerald_alloc`, a thin `malloc` wrapper
   with no corresponding free.** (`runtime/emerald_runtime.c:141-147`.) Once
   an object heap-allocates this way and escapes whatever scope created it,
   nothing in the runtime ever reclaims that memory. The source comment is
   explicit about this being deliberate: "No corresponding free — no GC, no
   lifetime tracking yet."
2. **`String` concatenation, numeric-to-string conversion, and most other
   intrinsics that produce a new `String` also allocate via `emerald_alloc`**
   (`emerald_runtime.c:270-307`) and are subject to the same rule: never
   freed.
3. **The one exception is scope-based arena/region allocation** (plan 51):
   `emerald_region_create`/`emerald_region_alloc`/`emerald_region_destroy`
   bump-allocate into growable chunks and bulk-free the entire region in one
   call. Plan 50's escape analysis decides, per allocation site, whether an
   object provably doesn't outlive its creating scope; if so, codegen routes
   it through a region instead of `emerald_alloc`, and that memory *is*
   reliably reclaimed when the region is destroyed. Actor instances (§2
   below) get their own per-instance region for the same reason.
4. **A diagnostic-only counter exists to make this honest rather than
   assumed**: `emerald_bytes_outstanding()` (`emerald_runtime.c:113-139`)
   tracks total bytes allocated minus bytes freed by region destruction,
   process-wide, updated with atomic relaxed-ordering increments (safe
   under plan 55's multi-threaded actor scheduler). It exists specifically
   so a test can measure "did memory actually stay bounded" as a real
   number — see `emerald-driver/tests/region_arena.rs`'s
   `region_leak_check_counter_round_trips_and_reports_zero_leaks` — rather
   than trusting the design argument alone. That test proves region
   round-tripping is leak-free; it does not, and cannot, prove anything
   about non-region `emerald_alloc` traffic, which by design is never
   supposed to come back down.

**What this means for a user today:** a short-lived program (a CLI tool
that runs and exits, a request-scoped or actor-scoped computation that
fits inside a region) behaves fine — the OS reclaims everything on exit
regardless. A long-running program that allocates non-region heap objects
in a loop (ordinary class instances outside an arena, repeated string
building outside a region) will grow memory without bound. This is a real,
current constraint, not a hypothetical one — there is no mitigation today
beyond routing allocation-heavy code through explicit regions or actors.
Revisiting this (reference counting, a tracing collector, or a more
aggressive escape-analysis/region-inference story) is open post-v1 work;
inception §12 itself names all three as future options, deliberately
undecided. **This is no longer only a deferred question — see
`spec/OWNERSHIP.md` for a real, decided ownership/borrowing design on top
of this section's own regions.** SUPERSEDED note (plan 84): that design is
no longer "not yet implemented" in full — plan 83 shipped the sema-level
`own`/`borrow`/`borrow var` liveness checker, and plan 84 shipped real
codegen for it (`own`-parameter transfer and `borrow`/`borrow var`-
parameter passing, both genuinely zero-cost — see `spec/OWNERSHIP.md`
§9/§10 and `emerald-codegen`'s own `strip_ownership_in_type_expr`/
`bind_params` doc comments for the exact mechanism and its one disclosed,
real, unavoidable cost). This still doesn't close the gap described in
this section: code that never opts into `own`/`borrow` annotations is
exactly as unproven/unbounded as described above (`spec/OWNERSHIP.md` §4's
own "additive, not a replacement" framing) — plan 84 narrows the gap for
code that opts in, it does not eliminate it for code that doesn't.
`benchmarks/REPORT.md`'s `object_allocation` benchmark gives this
a real number: 1,000,000 `emerald_alloc`-backed instances runs measurably
slower than every other language in that report, including Crystal and
Ruby's own garbage-collected allocation — the likely mechanism (never
reusing freed slots forces the allocator to keep extending the heap) is a
disclosed hypothesis there, not a proven root cause.

---

## 2. Actors: isolated heaps, single-threaded mailbox, real OS threads

1. **Each actor instance owns its own region** (`emerald_actor_set_region`,
   `emerald_runtime.c:785-788`), created via the plan 51 mechanism above.
   `.spawn()` allocates the actor's state inside that region; the actor's
   `initialize` and method bodies allocate into it too. This gives each
   actor an isolated heap in the sense that its memory is a distinct region
   with its own lifetime — not in the sense of copying/serializing state
   between actors on every call (plan 56 governs what may cross actor
   boundaries; see below).
2. **Scheduling is N actors over a fixed OS-thread pool**, honestly scoped
   down from an M:N green-thread model (plan 55's own Decision log — no
   coroutine/stack-switching primitive exists in this runtime).
   `emerald_worker_pool_start`/`emerald_actor_enqueue` implement a per-actor
   mailbox with an at-most-one-in-flight-message invariant: a given actor's
   method bodies never run concurrently with each other, but different
   actors' method bodies genuinely do run concurrently on separate OS
   threads.
3. **Compile-time message safety ("Pony-lite", plan 56)** is one linear-use
   check, not a full capability system: a reference sent to another actor
   may not be used again by the sender afterward. Enforced in
   `emerald-sema` (`is_cross_actor_send`), not at runtime.
4. **An actor's uncaught exception terminates that actor and bulk-frees its
   region** (`emerald_actor_terminate`, `emerald_runtime.c:1329-1341`) —
   the exception model (plan 38) and the arena model (plan 51) compose
   directly here: "let it crash" is cheap specifically because tearing down
   one actor's memory is one region-destroy call, not a GC sweep.
5. **Supervision (`supervise` blocks, plan 57)** is `one_for_one` restart
   only: a registered child's termination is reported to its supervisor
   (`emerald_supervisor_notify_terminated`), which respawns it with its
   original spawn arguments. No `one_for_all`/`rest_for_one` strategies
   (Erlang/OTP's other standard strategies) exist.

---

## 3. Distribution: real TCP, consistent-hash placement, no consensus

1. **`ActorName.remote(addr, name)`** (plan 60) opens a real TCP connection
   (`emerald_tcp_connect`/`emerald_tcp_listen`, `sockaddr_in`-based) and
   speaks a small length-prefixed binary wire protocol
   (`EmeraldWireBuf`/`emerald_tcp_send_frame`/`emerald_tcp_recv_frame`) to
   call methods on an actor registered in a different process. Both a
   locally-spawned and a remotely-referenced actor share the same
   `EmeraldActorRef` shape, which is what "location-transparent" means
   concretely here: calling code doesn't need a different code path for
   local vs. remote.
   **Found broken, then fixed, this same session:** the only examples
   exercising this (`examples/host.em`/`client.em`) failed to link under
   `--jobs` (multi-file `require`-splicing didn't scope actor-method
   linkage correctly) — fixed via `WeakODR` linkage in `emerald-codegen`;
   see `examples/README.md`'s "Real bugs found" section. Verified
   end-to-end as two real OS processes: `host` listens, `client` sends
   three real TCP `.increment` calls plus `.report`, `host` prints `3`.
2. **`ActorName.locate(key, args...)`** (plan 65) adds automatic placement:
   `emerald_consistent_hash_owner` hashes a key against a ring of known
   peers (`emerald_fnv1a`, `EmeraldRingPoint`) to decide which node owns a
   given actor, with peer discovery and heartbeating
   (`emerald_discover_peers`, `emerald_ensure_discovery_and_heartbeat`,
   `emerald_peer_is_live`) to track cluster membership. This is the same
   technique Akka Cluster Sharding uses for shard-to-node assignment — see
   this session's landscape research — not a novel algorithm.
3. **No consensus, no split-brain handling, no replication.** Peer
   liveness is heartbeat-based with no agreement protocol; if the peer set
   a node observes disagrees with another node's view (a partition), there
   is no mechanism here to reconcile that. This is a real limitation for
   any workload that needs strong placement guarantees under partition,
   not just an unpolished edge — flagged here rather than left implicit.

---

## 4. What this document does not cover

- LLVM codegen's calling conventions, stack layout, or DWARF emission —
  that's `spec/COMPILER.md`'s domain.
- The type system's static guarantees — `spec/TYPE_SYSTEM.md`.
- Language-level exception/`Result` semantics — `spec/SEMANTICS.md` §
  Exceptions and plan 38/53's Decision logs.

This document is scoped to "what the compiled binary actually does at
runtime," and should be corrected the same way it was written — by
checking `runtime/emerald_runtime.c` and the test suite directly — the
next time either changes underneath it.
