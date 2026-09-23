2026-09-22T22:39:00Z

---
name: "Set[T], Deque[T], PriorityQueue[T] — the Missing Collection Primitives (inception-3 §4.3)"
overview: "inception-3's own §4.3 names a real, confirmed gap: no Set[T], priority queue, or deque exists anywhere in the shipped stdlib or the 91-191 batch — every language inception-3 surveys except pre-C++11 C++ has at least one. Unlike Array[T]/Hash[K,V] (hand-rolled, header-prefixed contiguous-buffer LLVM memory built directly in emerald-codegen — verified this session, see Decision log), this plan routes all three new types through emerald-rt's already-proven resource-handle registry (plan 93), backed directly by Rust's own std::collections (HashSet, VecDeque, BinaryHeap) — the actual best-in-class implementation for this domain, no external crate needed."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-sema-new-collection-types
    content: "crates/emerald-sema/src/lib.rs: add Type::Set(Box<Type>), Type::Deque(Box<Type>), Type::PriorityQueue(Box<Type>), resolved by resolve_type the same bracket-generic way Type::Array/Type::Hash already are (lines ~511-521); reject any element type outside each type's own supported set (see Decision log) with a real, named diagnostic at the .new call site, never a silent fallback."
    status: done
  - id: leaf-rt-set-int64-and-string
    content: "crates/emerald-rt/src/collections.rs (new file): emerald_rt_set_i64_new/add/contains/remove/count/each_i64/close and an emerald_rt_set_string_* sibling, each a std::collections::HashSet<i64> / HashSet<String> behind handle_alloc/handle_get_mut/handle_close (plan 93's exact API, crates/emerald-rt/src/handle.rs)."
    status: done
  - id: leaf-rt-deque-four-types
    content: "Same file: emerald_rt_deque_<i64|f64|string|bool>_{new,push_front,push_back,pop_front,pop_back,count,close} — four monomorphized std::collections::VecDeque<T> instantiations, one per derive-Serializable's already-established four supported primitive types (Int64/Float64/String/Boolean), each independently handle-registered."
    status: done
  - id: leaf-rt-priority-queue-int64-and-string
    content: "Same file: emerald_rt_priority_queue_<i64|string>_{new,push,pop,peek,count,close} — std::collections::BinaryHeap<i64> / BinaryHeap<String>, Rust's own default max-heap ordering exposed as-is (no min-heap variant — see Decision log). Float64 and Boolean excluded — see Decision log."
    status: done
  - id: leaf-codegen-monomorphized-dispatch
    content: "crates/emerald-codegen/src/lib.rs: for each of Type::Set/Type::Deque/Type::PriorityQueue, resolve the element-type argument at the call site and emit a call to the ONE matching emerald_rt_<kind>_<elemtype>_<method> symbol — mirrors the existing per-arm Type::Array(_) / Type::Hash(_,_) dispatch (build_array_lit line ~12880, build_hash_lit line ~7397) in spirit, but through the FFI boundary (plans 91/92) rather than inline LLVM memory ops, since these three types are handle-backed, not raw buffers."
    status: done
  - id: leaf-examples-and-gate
    content: "examples/set_deque_priority_queue.em (the Concrete Proof below), wired into emerald-cli/tests/examples.rs's checked table; new emerald-rt unit tests per collections.rs export (mirroring handle.rs's own existing test style — alloc/use/close/double-close/unknown-handle, per type); new emerald-sema tests for the three new Type variants' resolve/reject behavior; full cargo nextest run --workspace / cargo clippy --workspace --all-targets / treefmt gate."
    status: done
isProject: false
---

# Plan 193 — Set[T], Deque[T], PriorityQueue[T]

Closes inception-3's §4.3 (`history/2026-09-21T195000Z-inception-3-stdlib-supremacy.md`),
slotted into §5.B (Core / Collections) exactly where that document's own
tier table already reserves it: "**NEW** (§4.3) `Set[T]`, priority queue,
deque." No plan in 91-191 or anywhere else in this repo owns this —
confirmed this session via `ctx_search` across all of `history/` for
`priority queue|deque|Set\[T\]`: every real hit is an internal,
Emerald-invisible use (plan 178's `crossbeam-queue::ArrayQueue`, which
explicitly disclaims "no Emerald-visible queue/stack/deque/skip-list
type of any kind" in its own Decision log) or an incidental Rust-source
mention in an unrelated plan. This is a genuine, previously-unowned gap.

## Concrete proof this plan targets

```ruby
s: Set[Int64] = Set.new
s.add(10)
s.add(20)
s.add(10)
puts s.count
puts s.contains(20)

d: Deque[String] = Deque.new
d.push_back("a")
d.push_front("z")
puts d.pop_front()

pq: PriorityQueue[Int64] = PriorityQueue.new
pq.push(5)
pq.push(1)
pq.push(9)
puts pq.pop()
```

Expected output:
```
2
true
z
9
```

(`s.count` is `2`, not `3` — `Set[T]`'s whole point is deduplication;
`pq.pop()` returns `9`, the largest, since `PriorityQueue[T]` exposes
Rust's `BinaryHeap`'s own real default max-heap ordering unmodified —
see Decision log.)

## Decision log

- **Array[T]/Hash[K,V] are NOT emerald-rt-wrapped Rust types — verified
  directly this session, not assumed from memory.** `ctx_search`/`grep`
  across `crates/emerald-codegen/src/lib.rs` and `crates/emerald-sema/src/
  lib.rs` found `build_array_lit` (line ~12880) and `build_hash_lit`
  (line ~7397) hand-rolling LLVM memory directly: `Array[T]` is
  `[length: Int64][elements...]` behind a raw pointer from `emerald_alloc`;
  `Hash[K,V]` is `[count: Int64][pairs...]`, the identical
  header-prefixed-contiguous-buffer convention. Neither touches
  `crates/emerald-rt` at all — confirmed by listing `crates/emerald-rt/
  src/` directly: no array/hash/collections file exists there, only the
  91-191 batch's own one-crate-per-plan domain modules (`aead.rs`,
  `dns.rs`, `json.rs`, ...).
- **This plan deliberately does NOT follow that precedent for the three
  new types — it goes through emerald-rt's handle registry instead —
  and that choice has real precedent of its own, not invented for this
  plan.** Plan 09's own Decision log (`history/2026-09-08T190129Z-
  plan-09-collections.md`), written the same session `Array[T]`'s
  unboxed-buffer requirement was proven, already discloses that `Hash[K,
  V]` does NOT carry that same representation-strictness mandate —
  "`spec/TYPE_SYSTEM.md` §8 itself notes Hash has no representation-
  strictness requirement as sharp as `Array[T]`'s." `Set[T]`/`Deque[T]`/
  `PriorityQueue[T]` inherit that same lighter mandate; nothing in
  `inception.md`/`spec/TYPE_SYSTEM.md` requires them to be raw unboxed
  memory the way `Array[T]` specifically was required to be.
- **"Best in class Rust lib" for this domain is Rust's own `std::
  collections` — no external crate.** `HashSet`, `VecDeque`, and
  `BinaryHeap` are already the ecosystem's own uncontested best answer
  (this is exactly the category Rust's own minimal-`std` bet, cited in
  inception-3 §1's own comparison table, gets right without an external
  crate). Hand-deriving open-addressing/chaining hash-set logic or a
  binary-heap in raw LLVM IR the way `build_array_lit` hand-rolls a
  contiguous buffer would be substantial, real, and unnecessary
  engineering risk for a data structure Rust's own standard library
  already implements correctly and efficiently. Routing through plan
  93's resource-handle registry (`crates/emerald-rt/src/handle.rs`
  — `handle_alloc(Box<dyn Any + Send>, type_tag) -> i64`,
  `handle_get`/`handle_get_mut`/`handle_close`, verified directly by
  reading the file this session) reuses a mechanism already proven safe
  across actor boundaries (plan 93's own stated purpose) rather than
  inventing a second one.
- **Narrow initial element-type scope for `Set[T]`/`PriorityQueue[T]` —
  Int64 and String only, disclosed and deliberate, not an oversight.**
  `Float64` is excluded from both: Rust's own `f64` does not implement
  `Eq`/`Hash`/`Ord` (NaN breaks all three), so `HashSet<f64>`/
  `BinaryHeap<f64>` are not expressible in safe Rust without a wrapper
  type — this is not a self-imposed Emerald limitation, it is the same
  one Rust itself has, and this plan declines to paper over it with an
  unsound `PartialOrd`-as-`Ord` cast. `Boolean` is excluded as a
  genuinely low-value `Set`/`PriorityQueue` element (a `Set[Boolean]`
  has at most 2 members) — out of v1 scope, not permanently declined.
  `Deque[T]` carries none of `Set`/`PriorityQueue`'s hashing/ordering
  constraints (it does neither), so it is scoped instead to all four of
  `derive Serializable`'s own already-proven, already-supported
  primitive types (Int64, Float64, String, Boolean —
  `history/2026-09-22T033000Z-derive-serializable.md`'s own precedent
  for "exactly these four field types, disclosed narrow v1 scope").
- **`PriorityQueue[T]` is a max-heap, matching Rust's own `BinaryHeap`
  default exactly, not Python's `heapq` min-heap default — a real,
  disclosed choice, not an unstated assumption.** A min-heap variant
  (trivially available by pushing `std::cmp::Reverse`-wrapped values
  internally) is out of scope for this plan; a caller needing min-heap
  semantics over `Int64` can negate keys themselves today, the same
  class of disclosed, narrow cut plan 09 itself made for `Array[T]`'s
  missing bounds checking.
- **Monomorphized per-element-type dispatch, not a boxed generic value
  representation.** Each supported `(collection kind, element type)`
  pair gets its own concrete Rust instantiation and its own
  `emerald_rt_<kind>_<elemtype>_<method>` export (e.g.
  `emerald_rt_set_i64_add` vs. `emerald_rt_set_string_add`), selected at
  the codegen call site once the generic type argument is resolved —
  consistent with how this language's generics are already monomorphized
  elsewhere (per this session's own broader stdlib survey), and directly
  mirroring `Type::Array(Box<Type>)`/`Type::Hash(K,V)`'s own existing
  per-arm dispatch in `emerald-sema` (`infer_expr_type`'s `Type::Array`/
  `Type::Hash` arms, verified at lines ~6311/6320 this session) — not a
  new dispatch mechanism invented for this plan.
- **Out of scope.** No `Set`/`Deque`/`PriorityQueue` literal syntax (no
  new grammar productions) — construction is `.new` plus `.add`/
  `.push_*` only, matching every emerald-rt-backed domain type across
  the entire 91-191 batch (none of which introduce new literal syntax
  either). No `for x in set`/iteration-protocol integration — deferred
  to whichever future plan actually lands inception-3 §4.2's real
  `Iterable[T]` interface (a separate new plan this same session
  authors); until then these three types expose only explicit accessor
  methods, the same disclosed limitation `Array[T]`/`Hash[K,V]`
  themselves carried before plan 74's enumerable stdlib landed. `.union`/
  `.intersection`/`.difference` on `Set[T]` and any richer `Deque`/
  `PriorityQueue` API surface beyond this plan's own listed methods are
  real, reasonable follow-ups, not attempted here — the same "prove the
  core shape first" discipline plan 09 itself applied to `Array[T]`.

## Total quality gate
```bash
cargo build --workspace
cargo nextest run --workspace
cargo clippy --workspace --all-targets
treefmt
```

## Update (2026-09-23, same-day session): implemented, all five leaves done

All three types landed exactly per the Decision log's own design —
`Set[Int64|String]`, `Deque[Int64|Float64|String|Boolean]`,
`PriorityQueue[Int64|String]`, monomorphized per-`(kind, element type)`
pair into 54 concrete `emerald_rt_<kind>_<elemtype>_<method>` exports
(`crates/emerald-rt/src/collections.rs`, new file — `HashSet<T>`/
`VecDeque<T>`/`BinaryHeap<T>` behind plan 93's `crate::handle` registry,
no external crate), wired through `emerald-sema` (`Type::Set`/`Type::
Deque`/`Type::PriorityQueue`, `resolve_type`'s three new arms rejecting
an unsupported element type right there — the one shared path every
annotation position funnels through — and a new `Stmt::Let` special
case for `.new()`, the fourth expected-type-providing shape alongside
`Array.new`/`Ok`/`Err`/a real generic class's own `.new()`) and
`emerald-codegen` (`local_classes` tagged with the `mangle_type_expr`
form, e.g. `"Set$Int64"`, dispatched by re-parsing that tag — the same
`Regex`/`XmlReader` shape, just with a second axis, the element type,
that `Regex` itself never needed).

Two real, disclosed grammar corrections found only by actually trying
to compile the plan's own Concrete Proof, not assumed from its prose:
(1) a zero-argument `.new` call needs explicit `()` in this grammar —
the plan's own `Set.new`/`Deque.new`/`PriorityQueue.new` are written
`Set.new()`/`Deque.new()`/`PriorityQueue.new()` in the real example and
tests, matching `examples/resource_handle_lifetime_proof.em`'s own
already-disclosed rule. (2) `puts` accepts only `Int64`/`Float64`/
`String` — `s.contains(20)`'s `Boolean` result needs string
interpolation (`puts "#{s.contains(20)}"`), matching `examples/
regex_dates.em`'s own already-disclosed finding. Neither changes the
Concrete Proof's own predicted output (`2`, `true`, `z`, `9`).

One real, disclosed implementation-time simplification, not in the
original Decision log: `Set[T]#each` does NOT have `emerald-rt` call
back into an Emerald closure across the FFI boundary — no such
mechanism exists anywhere in this crate (`Array[T]`/`Hash[K,V]`'s own
`.each` is hand-rolled entirely in `emerald-codegen`, since both are
raw contiguous buffers, never crossing the FFI boundary at all).
Instead, `emerald_rt_set_<elem>_each` materializes a real `Array[T]`
snapshot (the exact same `[len: i64][elements...]` layout `build_array_
lit`/`regex.rs`'s own `.find_all`/`.split` already use), and codegen's
`.each` dispatch arm reuses `build_array_each`'s own established loop-
and-call-block shape directly over that snapshot. The block still runs
once per element — just in `HashSet::iter`'s own real, insertion-order-
unspecified order, a disclosed (and, for a `Set`, inherent) limitation.

One real, disclosed unit-testing finding (not anticipated up front):
`raise_native_error` never returns via an ordinary, catchable Rust
panic — `crates/emerald-rt/src/lib.rs`'s own `emerald_raise` test-build
stub calls `std::process::abort()` outright (a real `longjmp`-shaped
divergence across what's genuinely a C ABI boundary, not something Rust
panic machinery can safely unwind through) — confirmed directly by
hitting a real `SIGABRT` mid-implementation. `collections.rs`'s own
use-after-close/unknown-handle unit tests therefore verify those two
diagnostics the same way `handle.rs`'s OWN tests already do — a direct
`handle_get_mut`/`handle_close` call, never through one of this
module's own `raise_native_error`-calling public wrappers. The real
end-to-end raise path stays verified only by a real `.em` program run
through the real CLI, matching `regex.rs`'s own already-established
precedent (that module's own tests carry an identical omission).

Full concrete proof verified end to end via `examples/
set_deque_priority_queue.em`, producing exactly the predicted `2`,
`true`, `z`, `9` — confirmed by actually compiling and running the
binary through `emerald-cli`'s own test harness
(`set_deque_priority_queue_em_prints_expected_sequence`,
`crates/emerald-cli/tests/examples.rs`), not assumed from the codegen
reasoning alone.

New tests: 9 `emerald-sema` unit tests (the Concrete Proof itself
type-checking end to end; `Set[Float64]`/`Set[Boolean]`/
`PriorityQueue[Float64]`/`Deque[Symbol]` each rejected with a real,
named diagnostic; all four `Deque[T]` primitive element types accepted;
a `Set[Int64] = Deque.new()` kind mismatch rejected; `Set.new(1)`'s
spurious argument rejected; `.each` accepting a matching `Proc`;
`Set#push` — a `Deque`/`PriorityQueue`-only method name — rejected on a
`Set` receiver with a real diagnostic, not a panic); 24 `emerald-rt`
unit tests, one alloc/use/close(/double-close/unknown-handle where
raising is actually testable) cluster per each of the 8 concrete
`(kind, element type)` pairs, mirroring `handle.rs`'s own established
test style; 1 `emerald-cli` end-to-end example test.

Full workspace gate: `cargo nextest run --workspace -j4` — 1136/1137
passing, 2 skipped, the sole failure (`dns_resolution_em_prints_
expected_sequence`) a real, pre-existing, network-dependent test
unrelated to this plan (no DNS/network access in this sandbox,
confirmed reproducible on its own, untouched by this change).
`cargo clippy -p emerald-sema -p emerald-codegen -p emerald-rt -p
emerald-cli --all-targets` clean. `cargo clippy --workspace --all-
targets` (the full, unscoped run) fails, but provably NOT from this
plan's own code: the one reported error (`missing fields
bigint_add, bigint_factorial, bigint_from_i64...` in a `Ctx` literal at
`emerald-codegen/src/lib.rs:22733`) is inside another, concurrently-
in-flight, uncommitted session's own uninished `BigInt`/`Decimal` work
(`crates/emerald-rt/src/bignum.rs`/`decimal.rs`, `Cargo.toml`/
`DEPENDENCIES.md` additions for `num-bigint`/`rust_decimal`) sharing
this working tree at the same time — none of those paths were read,
edited, or staged by this plan. `treefmt` clean (0 changed on every
file this plan owns).
