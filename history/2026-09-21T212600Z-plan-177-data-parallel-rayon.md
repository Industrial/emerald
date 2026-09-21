2026-09-21T21:26:00Z

---
name: Data-Parallel Batch Processing (rayon)
overview: "`Array.par_sort`/`Array.par_sum`/`Array.par_scale` — a small, fixed set of `rayon`-backed native functions (`rayon` 1.12, verified this session: 42.6M downloads/month, #4 in Concurrency, 43,740 dependent crates, guarantees data-race freedom by construction) that copy an Emerald array's scalar contents into a private Rust `Vec`, parallelize a closed, no-Emerald-callback computation over it entirely inside one native call, and copy the result back — never a general `rayon::prelude`/`ParallelIterator`/thread-pool surface exposed to Emerald source. This plan states, explicitly and without hand-waving, why raw rayon access from Emerald code would be a genuine safety hazard given the language's actual isolation guarantees, and why bounding rayon to specific, closed-over-primitives native functions avoids that hazard entirely."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-cargo-dependency
    content: "Add `rayon = \"1.12\"` to `crates/emerald-rt/Cargo.toml`. Run plan 95's crate-vetting checklist explicitly against it (docs + example + test) — trivial to satisfy given `rayon`'s own stated, audited invariant (\"guarantees data-race freedom\") and its scale of adoption, but the checklist still runs, not skipped as \"obviously fine\"."
    status: pending
  - id: leaf-array-marshaling-copy-in-copy-out
    content: "Add `emerald_array_to_i64_vec`/`emerald_array_to_f64_vec`-style extraction helpers to `crates/emerald-rt/src/lib.rs` that read an Emerald `Array`'s real runtime representation (re-verify the exact layout against `emerald-codegen`'s own array-construction lowering in `crates/emerald-codegen/src/lib.rs` before implementing — no `emerald_array_*` runtime function exists in `runtime/emerald_runtime.c` today, verified this session via a full function-map read of that file, so array operations are codegen-inlined, not runtime-exported; this leaf must re-derive the real layout from codegen's own IR-emission code directly, the same \"verify against real source at execution time\" discipline plan 54 applied to plan 51's arena API before wiring to it) into an owned `Vec<i64>`/`Vec<f64>`, and a matching `vec_to_emerald_array` constructor that allocates a fresh Emerald array (via the existing `emerald_alloc`-based construction path codegen already uses) and copies the Rust `Vec`'s contents back into it."
    status: pending
  - id: leaf-par-native-functions
    content: "Add exactly three `emerald_rt_fn!`-wrapped exports per plan 92's convention: `emerald_rt_array_par_sort_i64(arr: *mut c_void) -> *mut c_void` (extracts to `Vec<i64>`, `par_sort_unstable()`, returns a fresh sorted array — never in-place mutation of the caller's own array pointer, see Decision log), `emerald_rt_array_par_sum_i64(arr: *mut c_void) -> i64` (`par_iter().sum()`, a pure scalar reduction, no allocation on the way back at all), and `emerald_rt_array_par_scale_f64(arr: *mut c_void, factor: f64) -> *mut c_void` (`par_iter().map(|x| x * factor).collect()`, a fixed, compiler-shipped transform — never a caller-supplied closure). Dispatch as `Array.par_sort(self): Array[Int64]`, `Array.par_sum(self): Int64`, `Array.par_scale(self, factor: Float64): Array[Float64]` via the existing receiver-storage-kind intrinsic mechanism plan 91/45 already established."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/rayon_par_array_proof.em` (this plan's Concrete Proof below) wired into `emerald-cli/tests/examples.rs`'s CI-checked table. Add a Rust-side `#[test]` in `emerald-rt` that runs `emerald_rt_array_par_sort_i64` against a large (>100,000-element, large enough that rayon's own work-splitting heuristic actually parallelizes it, not falls back to sequential) randomly-shuffled input under Miri or a plain debug build with `RAYON_NUM_THREADS` forced to e.g. `4`, asserting the output is sorted and a permutation of the input — proving real parallel execution occurred, not merely that the sequential fallback path is correct."
    status: pending
isProject: false
---

# Plan 177 — Data-Parallel Batch Processing (rayon)

This is the first of three plans in this batch (177, 178, 179) that must
confront the same real design tension head-on: Emerald's only existing
concurrency model, built across plans 54/55/56/57, is isolated-heap
actors — at most one OS thread ever inside a given actor's code at a
time, by construction, with **no lock around the actor's own fields at
all** (plan 55's own Decision log states this precisely: the mailbox
scheduling discipline makes touching a claimed actor's arena from a
worker thread safe with zero synchronization, because only one thread is
ever inside that actor's code at a time). `rayon`'s entire reason to
exist is the opposite: shared-memory, work-stealing parallelism where
multiple OS threads read and write *the same* backing memory
concurrently, by design. Naively exposing `rayon`'s primitives to
Emerald source — a general `Array.par_each { |x| ... }` that ran an
Emerald block body on N rayon worker threads simultaneously against one
shared array, say — would silently reintroduce, inside a single native
call, exactly the class of unsynchronized shared-mutable-state hazard
the whole actor-isolation design exists to make structurally impossible
everywhere else. This plan resolves that tension by construction, not by
convention: every `rayon`-backed native function this plan ships is
closed over primitive, copied Rust data with **zero reentrancy into
Emerald-generated code** — no Emerald block, lambda, or method is ever
invoked from inside a `rayon` worker thread, anywhere, in this plan.

## Concrete proof this plan targets

```ruby
data: Array[Int64] = [42, 7, 19, 3, 88, 1, 56, 23]

sorted: Array[Int64] = data.par_sort
total: Int64 = data.par_sum

i: Int64 = 0
while i < sorted.length
  puts sorted[i]
  i = i + 1
end
puts total
```

Expected output: `1`, `3`, `7`, `19`, `23`, `42`, `56`, `88`, then `239`
(the sum of the original eight values) — `data` itself is unmodified
(`par_sort` returns a new array, proven by `data.par_sum` still summing
the original, unsorted values correctly rather than any partially-
mutated intermediate state). The example's own element count is small
enough to run in well under a millisecond either sequentially or in
parallel — this proof is about **correctness**, not about demonstrating
real wall-clock parallel speedup (that is `leaf-example-and-gate`'s own
large-input `#[test]`'s job, following the exact same "correctness
proof in stdout, real-concurrency proof in a separate large/timed test"
split plan 55's own two-part Concrete Proof already established).

## Decision log

- **Why raw `rayon` access from Emerald code would be a genuine safety
  hazard, stated plainly rather than assumed.** Emerald's ownership
  model (plans 82/83, `spec/OWNERSHIP.md`) tracks `own`/`borrow` for
  *single-threaded* aliasing safety — a function that borrows a value
  is checked against other *sequential* uses of that same binding, not
  against simultaneous access from a second OS thread. There is no
  `Send`/`Sync`-equivalent trait or compiler check anywhere in
  `emerald-sema` that could certify "this Emerald block's captured free
  variables are safe to touch from multiple threads at once" — plan
  10/34's closure-capture mechanism (`collect_idents_in_stmt`/
  `free_vars_in_lambda`, verified present in `emerald-codegen/src/
  lib.rs`) captures free variables for *sequential*, call-site-
  specialized inlining, with no thread-safety analysis of any kind. If
  a general `Array.par_each { |x| @total += x }`-style primitive existed
  and its block body were run by `rayon` worker threads directly against
  Emerald-heap-resident captured state, two worker threads could race on
  the exact same `@total` field with no lock, no atomic, and no compiler
  diagnostic — a silent, unchecked data race in a language whose entire
  design posture (plan 59's own words, quoted directly) is "ordinary
  Emerald code can never crash, corrupt memory, or misbehave in a way
  the type checker hasn't already ruled out." Exposing raw `rayon`
  primitives would puncture that guarantee exactly the way plan 59's own
  `unsafe extern "C"` fence does — except silently, with no `unsafe`
  keyword anywhere in the offending source to mark the risk.
- **The bounded resolution: `rayon` powers specific, self-contained
  native functions whose entire body is closed over copied primitive
  data, never Emerald callbacks.** Every export this plan ships follows
  one fixed shape: (1) copy the input array's scalar contents into a
  private, `emerald-rt`-owned `Vec<i64>`/`Vec<f64>` (`leaf-array-
  marshaling-copy-in-copy-out`); (2) run a `rayon` computation entirely
  over that private `Vec` — `par_sort_unstable`, `par_iter().sum()`, a
  *fixed*, compiler-shipped `.map()` closure (never a caller-supplied
  one) — with no pointer back into any Emerald-heap object visible to
  any worker thread; (3) synchronously join (every one of these `rayon`
  calls is blocking/synchronous by construction — `par_sort_unstable`
  and `par_iter().sum()` never return until every worker thread's slice
  of work is done) and, only then, copy the private result back into a
  freshly-allocated Emerald array or scalar return value. By the time
  the native call returns and any Emerald code (on any actor, any
  thread) can see the result, `rayon`'s own worker threads have already
  exited — there is no window during which Emerald-visible, Emerald-
  heap-resident data is being concurrently touched by more than the one
  calling thread. This is not a policy Emerald code has to honor; it is
  a structural property of what these three functions' Rust bodies
  literally do.
- **Copy-in/copy-out is a real, disclosed cost, not free — and is
  necessary regardless of `rayon`, not a tax `rayon` specifically
  imposes.** Any native function crossing the FFI boundary already needs
  to marshal an Emerald `Array`'s contents into a Rust-native
  representation to operate on it at all (the same category of
  necessary conversion plan 92's `(ptr, len)` convention documents for
  binary data) — `rayon`'s specific requirement of an owned, exclusively-
  `&mut`-borrowed Rust slice (`par_sort_unstable` needs `&mut [T]`,
  provably non-aliased by Rust's own borrow checker, which an Emerald-
  heap pointer arriving raw across FFI cannot honestly provide without a
  copy) makes the copy non-optional here specifically, but it would be
  needed in some form for *any* Rust-native array algorithm crossing
  this boundary, `rayon`-backed or not. `par_sum`'s case is cheaper: no
  result copy-back is needed at all, since the reduction collapses to a
  single scalar.
- **In-place mutation was considered and declined for `par_sort`.**
  Because the calling actor (or `main`) holds its own single execution
  slot for the full duration of a synchronous native call (plan 55's own
  invariant — no other message can be dequeued for that actor until the
  call returns), it would in fact be *safe*, on pure data-race grounds,
  for `par_sort` to sort the caller's array in place rather than
  allocating a fresh one: no other Emerald thread can observe the array
  mid-sort regardless. This plan declines the in-place option anyway,
  for a narrower, disclosed reason: an in-place native mutation of an
  Emerald-heap array from Rust means the Rust side must reconstruct and
  honor the array's exact real layout (length header, capacity,
  element stride) precisely enough to write back into it safely — a
  materially larger, more error-prone surface than allocating a fresh
  array through the same construction path `Expr::ArrayLit` already
  uses. A future plan, once `emerald-rt`'s array-marshaling helpers are
  battle-tested, could reconsider in-place mutation as a real, disclosed
  follow-up; this plan takes the simpler, safer default first.
- **Why this plan's scope is three functions, not a general parallel-
  iterator surface.** `Array.par_each`/`.par_map` with an Emerald block
  parameter, `Array.par_filter` with an Emerald predicate,
  `Hash.par_values`, and any other "take a callback, parallelize the
  callback" shape are all declined **for this plan specifically**
  because every one of them would need to call back into Emerald-
  generated code from inside a `rayon` worker thread — reopening the
  exact hazard this Decision log's first bullet describes. A
  fundamentally different design (e.g., proving via the ownership
  system that a captured block's free variables are provably `Send`-
  disjoint per rayon task, or restricting parallel callback bodies to a
  new, checked "pure, no-shared-capture" block subset) would be a real,
  substantial, separate piece of type-system work this plan does not
  attempt — named here as a deliberate non-goal, not an oversight.
- **`rayon` 1.12, verified current and dominant in the ecosystem, not
  assumed.** `lib.rs`'s real crate page: latest release `1.12.0` (14 Apr
  2026), 42,623,870 downloads/month, used in 43,740 dependent crates
  (11,232 directly), MIT/Apache, #4 in the Concurrency category, written
  and maintained by the original Rust project contributors (Niko
  Matsakis, Josh Stone) with 98 contributors — its own README states its
  central, load-bearing guarantee plainly: "Rayon is a data-parallelism
  library for Rust... It also guarantees data-race freedom." That
  guarantee is exactly what makes the copy-in/copy-out design above
  sound: `rayon` itself cannot introduce a data race *within* its own
  worker-thread computation over the private `Vec` it's handed; this
  plan's own job is only to make sure nothing Emerald-visible is ever
  handed to it in the first place.
- **`rayon`'s global thread pool is a real, shared, process-wide
  resource — but not an Emerald-visible one, and sharing it across
  concurrently-executing actors introduces no new hazard.** Two
  different actors on two different OS worker threads (per plan 55's
  own scheduler) could legitimately call `.par_sort` at the same moment,
  each on its own private array; both calls draw worker threads from
  `rayon`'s one process-global pool. This is analogous to, not in
  tension with, plan 55's own OS-thread-pool sharing across actors: each
  `rayon` call's worker threads only ever touch the one private `Vec`
  that specific call allocated (Rust's own borrow checker enforces this
  at compile time inside `emerald-rt`'s own code — two `Vec<i64>`s never
  alias), so contention here is a pure performance question (how many
  physical cores `rayon`'s pool has to divide across concurrently active
  calls), never a correctness one.
- **Out of scope.** No `Array.par_each`/`.par_map`/`.par_filter` or any
  other Emerald-callback-taking parallel primitive (see above — a real,
  separate, larger design question), no general `rayon::prelude` surface
  or `ParallelIterator` exposed to Emerald source in any form, no
  in-place array mutation (see above), no parallel `Hash`/`Enumerable`
  operations beyond the two array functions this plan ships, no thread-
  pool sizing/configuration control exposed to Emerald (`rayon`'s
  default global pool sizing is used as-is), and no interaction
  whatsoever with plan 55's actor scheduler or plan 60's distributed
  actors — this plan's three functions are indistinguishable, from the
  scheduler's own point of view, from any other synchronous native call
  that happens to take slightly longer.
