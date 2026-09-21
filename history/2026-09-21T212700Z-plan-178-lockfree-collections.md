2026-09-21T21:27:00Z

---
name: Lock-Free Concurrent Collections (crossbeam)
overview: "`Runtime.recent_native_errors` — a single, narrow, read-only Emerald-visible surface backed by a `crossbeam-queue::ArrayQueue<String>` (from the `crossbeam` 0.8.5 umbrella crate, verified this session: released 5 Sep 2026, 9.6M downloads/month, 5,629 dependent crates) that every panic caught at plan 92's `emerald_rt_raise_native_error` funnel point pushes its message into, concurrently, from however many OS worker threads happen to be executing native calls at once. This is the plan's whole scope: crossbeam's lock-free structures live as a private implementation detail inside one diagnostic subsystem's Rust code — never as an Emerald-visible queue/stack/deque type, and never a second concurrency primitive competing with the actor mailbox. Plan 93's own resource-handle registry is named as a second, real, legitimate internal application crossbeam's lock-free primitives could serve — documented here as an option for that plan's own owner to take up, not redesigned by this plan."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-cargo-dependency
    content: "Add `crossbeam-queue = \"0.3\"` (the specific sub-crate this plan needs — `ArrayQueue`'s real home, re-exported by the `crossbeam` umbrella crate but pulled in directly here to avoid dragging in `crossbeam-deque`/`crossbeam-epoch`'s own transitive surface for a plan that needs neither) to `crates/emerald-rt/Cargo.toml`. Run plan 95's crate-vetting checklist against it explicitly."
    status: pending
  - id: leaf-global-diagnostic-ring-buffer
    content: "Add a `static NATIVE_ERROR_RING: std::sync::OnceLock<crossbeam_queue::ArrayQueue<String>> = std::sync::OnceLock::new();` (capacity 64, fixed) to `crates/emerald-rt/src/lib.rs`, lazily initialized on first use. Modify plan 92's `emerald_rt_raise_native_error(msg: *const c_char)` to push a copy of `msg`'s decoded `String` into this ring via `force_push` (crossbeam's own non-blocking, overwrite-oldest-on-full semantics — never a blocking push) as its very first action, before doing anything else (allocating the `NativeError` instance, calling `emerald_raise`) — this is the single, real hook point every native panic across the entire `emerald-rt` surface already passes through, per plan 92's own Decision log, so this plan adds exactly one line there rather than duplicating panic-conversion logic anywhere."
    status: pending
  - id: leaf-runtime-drain-function
    content: "Add `emerald_rt_runtime_recent_native_errors(limit: i64) -> *mut c_void` (`emerald_rt_fn!`-wrapped per plan 92) that pops up to `limit` entries off `NATIVE_ERROR_RING` (oldest first, via repeated `pop()`) into a `Vec<String>`, then builds a fresh Emerald `Array[String]` from it via the same array-construction path plan 177's `leaf-array-marshaling-copy-in-copy-out` establishes for scalar arrays (re-verify the exact layout for a `String`-element array specifically — a pointer-of-pointers shape, not a flat scalar buffer — against `emerald-codegen`'s real array-literal lowering before implementing). Dispatch as `Runtime.recent_native_errors(limit: Int64): Array[String]`, a reserved-namespace static call exactly like `File.read` (plan 45)."
    status: pending
  - id: leaf-concurrent-stress-test
    content: "Add a Rust-side `#[test]` in `emerald-rt` that spawns 16 real OS threads, each calling a function that deliberately panics (behind `emerald_rt_fn!`) in a tight loop for a fixed duration, then asserts `NATIVE_ERROR_RING.len() <= 64` at all times (never observed to exceed capacity — `ArrayQueue` is a real bounded structure, not an unbounded one that could OOM under a panic storm) and that `pop()` never panics or blocks under concurrent `force_push` pressure from the other 15 threads — a genuine concurrency test, not merely a single-threaded correctness check, matching this batch's own repeated-run/race-detection discipline (verify any looped assertion via `cargo nextest run`, never a compressed shell capture, per `AGENTS.md`'s own documented `ctx_shell` gotcha)."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/native_error_ring_proof.em` (this plan's Concrete Proof below) wired into `emerald-cli/tests/examples.rs`'s CI-checked table. Run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 178 — Lock-Free Concurrent Collections (crossbeam)

This is the second of three plans in this batch (177, 178, 179) that
must resolve the same real tension plan 177's own Decision log states in
full: Emerald's actor model guarantees at most one OS thread is ever
inside a given actor's code at a time, with no lock needed around actor
state as a direct result (plan 55, verified). `crossbeam`'s lock-free
collections — `ArrayQueue`/`SegQueue` (crossbeam-queue), epoch-based
reclamation (crossbeam-epoch), a lock-free skip list (crossbeam-skiplist)
— exist specifically to let *multiple threads mutate one shared
structure concurrently*, safely, without a mutex. Exposing any of them
as an Emerald-visible type would be a second, structurally different,
competing concurrency primitive: an Emerald `LockFreeQueue` instance that
two different actors on two different worker threads could both hold a
reference to and push/pop from simultaneously has no relationship to the
mailbox discipline at all — no "at most one in-flight" invariant governs
it, no per-instance isolation protects it, and no compiler check
distinguishes it from an ordinary, unsynchronized Emerald object. This
plan resolves the tension the same way plan 177 does: **crossbeam's
lock-free structures live entirely inside one native subsystem's own
Rust implementation, touched only by Rust code, never returned to or
constructible from Emerald source in any form.**

## Concrete proof this plan targets

```ruby
begin
  "trigger".fnv1a_hash_panic_for_test
rescue NativeError => e
  puts e.message
end

errors: Array[String] = Runtime.recent_native_errors(5)
puts errors.length
puts errors[0]
```

Expected output: the deliberate panic's message (`native panic in
fnv1a_hash_panic_for_test`, matching plan 92's own Concrete Proof's
identical `rescue` line byte-for-byte), then `1` (exactly one entry now
in the ring buffer — this example's own process has triggered exactly
one native panic before this point), then the same message a second
time, retrieved from `Runtime.recent_native_errors` — proof that the
`crossbeam_queue::ArrayQueue` push this plan adds to `emerald_rt_
raise_native_error` actually captured the concurrent write plan 92's own
panic-to-exception conversion triggered, and that `Runtime.
recent_native_errors` can read it back out as an ordinary Emerald
`Array[String]` with no queue/channel/lock-free type ever appearing in
source. The genuine multi-thread concurrency claim (many OS threads
pushing into the same `ArrayQueue` at once, with no lock and no lost/
corrupted entries) is proven separately, by `leaf-concurrent-stress-
test`'s own Rust-side test — stdout from a single-threaded `.em` example
cannot demonstrate concurrent-write correctness on its own, the same
"correctness in stdout, concurrency in a dedicated test" split plan 55
and plan 177 both already use.

## Decision log

- **`crossbeam` 0.8.5, verified current, not assumed.** `lib.rs`'s real
  crate page: latest release `0.8.5` (5 Sep 2026 — the same day as
  `crossbeam-channel`'s own `0.5.17` release, plan 179's own pick, a
  real signal of coordinated, active maintenance across the whole
  `crossbeam-rs` GitHub org rather than one abandoned sub-crate),
  9,647,925 downloads/month, used in 5,629 dependent crates (1,432
  directly), MIT/Apache, tagged `lock-free`/`atomic`/`rcu`/`garbage` on
  its own listing — exactly the standard, most heavily reviewed
  lock-free toolkit in the Rust ecosystem, maintained by contributors
  closely tied to the Rust project itself (Alex Crichton, Aaron Turon,
  Jeehoon Kang, Taiki Endo). This plan takes `crossbeam-queue`
  specifically (not the full umbrella crate) for `ArrayQueue` — a
  bounded, `Send + Sync`, lock-free (Michael-Scott-derived) MPMC ring
  buffer, exactly the shape this plan's one real use case needs.
- **The tension, and why a general lock-free collection type would be
  unsound to expose, stated concretely.** An Emerald-visible
  `LockFreeQueue`'s `.push`/`.pop` would need to be callable from
  whichever actor holds a reference to it, on whichever OS worker thread
  is executing that actor's method body at the time — but nothing in
  `emerald-sema`'s type system marks a value as "safe to alias across
  threads" the way Rust's own `Send`/`Sync` traits do (the identical gap
  plan 177's own Decision log names for closures). Unlike plan 177's
  `rayon` functions, where the hazard is a hypothetical *callback*
  design this plan declines to build, here the entire *value itself*
  (a queue instance) would need to cross actor boundaries and be held
  concurrently — there is no narrower, "closed over primitives" version
  of a general concurrent queue type the way there is for a sort or a
  sum; a queue's whole purpose is being shared and mutated over time,
  which is precisely the shared-mutable-state shape the actor isolation
  model exists to rule out for ordinary Emerald values.
- **The resolution: `crossbeam-queue::ArrayQueue` as one diagnostic
  subsystem's private internal state, never an Emerald-visible type.**
  `NATIVE_ERROR_RING` is a single, `emerald-rt`-private, process-global
  `static` — no Emerald source, in this plan or any future one, ever
  receives a handle, pointer, or reference to it. The only two
  operations Emerald code can trigger are (1) an *implicit* push,
  reached only by causing a native panic (an event Emerald code cannot
  target or control precisely, since which specific native call panics
  is a Rust-side bug, not an Emerald-level decision), and (2) an
  explicit, read-only, snapshot-and-copy drain via `Runtime.
  recent_native_errors`, which returns an ordinary, fully-owned
  `Array[String]` — a value with no further connection to the ring
  buffer once returned. Emerald code can observe *that* concurrent
  writes happened; it can never *perform* one, hold a reference to the
  structure doing it, or construct a second instance of one.
- **Why this specific application (a panic-diagnostics ring buffer) is
  a genuine, motivated use of a lock-free structure, not a contrived
  excuse to justify the dependency.** Per plan 55's own scheduler model,
  many actors run concurrently across many OS worker threads; per plan
  92's own convention, every native call across the eventual ~100-plan
  `emerald-rt` surface is `catch_unwind`-wrapped and funnels a panic
  through the exact same `emerald_rt_raise_native_error` call. A real
  panic storm — several actors' native calls panicking within the same
  short window, on different worker threads, simultaneously — needs a
  structure that many threads can push into concurrently with no lock
  contention becoming itself a bottleneck or, worse, a source of lock-
  ordering bugs in the compiler's own runtime. A plain
  `Mutex<VecDeque<String>>` would also be *correct* here (this is a
  low-frequency diagnostic path, not a hot loop) — this plan's real
  justification for `ArrayQueue` over a mutex is demonstrating the
  vetted, idiomatic tool for exactly this shape of problem now, so a
  later, genuinely hot-path use (plan 93's registry, next bullet) has a
  proven, tested pattern in this codebase to point to rather than
  inventing one under pressure.
- **Plan 93's resource-handle registry is a second, real, higher-value
  candidate application — named here as an option, not redesigned by
  this plan.** Plan 93 will need an "opaque `i64` handle -> live Rust
  resource" table (`SmtpTransport` in plan 176, and every future
  stateful native wrapper) accessed by many OS worker threads
  concurrently allocating/looking-up/freeing handles across every actor
  in a running program — a materially hotter path than this plan's own
  panic-diagnostics ring, and one where lock contention on a single
  `Mutex<HashMap<i64, T>>` could become real under load. This plan
  explicitly does not modify plan 93's design or presume its landed
  shape (plan 93 may not have executed yet, the same "verify against
  real source at execution time" caveat plan 54 stated for plan 51
  applies identically here) — it documents, as a citable option, that
  `crossbeam`'s lock-free primitives (a sharded map, or a
  `crossbeam_queue::SegQueue`-backed free-list for handle-ID recycling)
  are available and already vetted by this plan, should plan 93's own
  owner want them.
- **`force_push`, not blocking `push` — a deliberately lossy, best-
  effort diagnostic aid, disclosed plainly.** `ArrayQueue::force_push`
  overwrites the oldest entry when the ring is full rather than
  blocking or returning an error the panic-handling path would have to
  do something with; a burst of more than 64 concurrent native panics
  within one short window silently drops the oldest ones. This is a
  real, accepted tradeoff: `NATIVE_ERROR_RING` is a best-effort recent-
  history aid for interactive debugging, never a durable, complete audit
  log — a real logging/observability story (persisted, unbounded,
  ordered-with-timestamps) is explicitly out of scope.
- **No interaction with plan 55's scheduler or plan 60's distributed
  actors.** `NATIVE_ERROR_RING` is a single process-local `static`; a
  distributed Emerald program (plan 60) has one independent ring per
  OS process, with no cross-node aggregation — `Runtime.
  recent_native_errors` only ever reports panics from native calls that
  happened to execute in the calling process.
- **Out of scope.** No Emerald-visible queue/stack/deque/skip-list type
  of any kind, no `crossbeam-deque` (work-stealing deque, the structure
  `rayon`'s own scheduler is internally built on — plan 177 takes
  `rayon` as a whole dependency rather than hand-building a scheduler
  from this primitive) integration, no `crossbeam-epoch` memory-
  reclamation use in this plan specifically (reserved for a future
  plan that actually needs hazard-pointer-style reclamation — this
  plan's own `ArrayQueue` needs none, being a fixed-capacity ring with
  no dynamic node freeing), no redesign of plan 93's registry (option
  only, as stated above), and no persisted/unbounded error log.
