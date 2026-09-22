2026-09-21T20:02:00Z

---
name: Resource Handle & Lifetime Model for Native Objects
overview: "How a long-lived native resource — a TCP stream, a DB connection, a TLS session, a compression stream, needed by dozens of upcoming domain plans — is represented across the plan-92 FFI boundary: an opaque `i64` handle issued by a process-wide `Mutex<HashMap<u64, Box<dyn Any + Send>>>` registry inside `emerald-rt`, paired with an explicit `.close()` method Emerald code must call. Explicit manual close is chosen over hooking plan 82/83's `own`/`borrow` scope-exit machinery for v1, named as a real, deferred future improvement rather than assumed solved; a use-after-close is a disclosed runtime abort mirroring `emerald_hash_key_not_found`'s existing pattern, never UB; and a handle is decided to be actor-local, never shared, matching the isolated-heap model plans 54/55 already ship."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-registry-and-handle-type
    content: "Add a process-wide `static REGISTRY: OnceLock<Mutex<HashMap<u64, RegistryEntry>>> = OnceLock::new()` to a new `crates/emerald-rt/src/handle.rs`, where `RegistryEntry { value: Box<dyn Any + Send>, type_tag: &'static str, closed: bool }` (the `closed` flag, not an immediate `HashMap::remove`, is deliberate — see Decision log for the double-close/use-after-close diagnostic this enables). Add `emerald_rt_handle_alloc(value: Box<dyn Any + Send>, type_tag: &'static str) -> i64` (a monotonically increasing `u64` counter, never reused within a process's lifetime, cast to `i64` since Emerald has no unsigned integer type per plan 59's own finding), `emerald_rt_handle_get::<T>(id: i64) -> Result<&T, HandleError>`/`emerald_rt_handle_get_mut::<T>`, and `emerald_rt_handle_close(id: i64) -> bool` (marks `closed = true`, drops the boxed value in place, returns whether the handle was actually open) as the four Rust-internal primitives every resource-holding domain plan (HTTP client connections, DB pools, compression streams, TLS sessions) builds its own typed wrapper on top of, per this plan's own naming convention below."
    status: done
  - id: leaf-use-after-close-diagnostic
    content: "Wire a closed or unknown handle ID, observed inside any `emerald_rt_handle_get`/`get_mut` call a domain-plan wrapper makes, to `emerald_rt_raise_native_error` (plan 92's `leaf-native-error-and-panic-raise`) with a message naming the concrete failure (`\"use of closed <type_tag> handle\"` or `\"unknown <type_tag> handle: this process never issued id <id>\"`), producing a real, catchable `NativeError` Emerald exception — mirroring `emerald_hash_key_not_found`'s existing disclosed-abort precedent in kind (a controlled, named failure, not silent memory corruption), but using plan 92's exception channel rather than `emerald_hash_key_not_found`'s own `fprintf`+`exit(1)` process-abort, since a native-resource misuse is exactly the kind of programmer-error condition plan 92's Decision log already assigns to the `NativeError`/exception channel rather than `Result[T, E]` (it is never an anticipated, routinely-checked outcome of calling `.read()` on a stream — it is a bug in the calling Emerald code)."
    status: done
  - id: leaf-double-close-is-a-noop-not-an-error
    content: "`emerald_rt_handle_close` on an already-closed or already-unknown ID returns `false` and does not raise — only a subsequent *use* (a `.read()`/`.write()`-style method call routed through `emerald_rt_handle_get`) raises. Document this asymmetry explicitly in `handle.rs`'s module doc: idempotent `.close()` matches the disclosed convention most resource-handling stdlibs converge on (Python's own `contextlib`/file objects, Rust's own `Drop` being infallible) specifically so a `.close()` called from both an explicit call site and a future auto-close mechanism (see 'Not yet decided') never double-raises merely because cleanup ran twice."
    status: done
  - id: leaf-actor-local-handle-enforcement
    content: "Decide and enforce (see Decision log) that a handle value is actor-local: the codegen-side wrapper type Emerald source actually holds (e.g. a `TcpStream` class instance wrapping the raw `i64` id, introduced by whichever domain plan first needs one — this plan does not itself introduce a resource-holding stdlib class) is never passed as a cross-actor message argument. Concretely, extend plan 55's own cross-actor dispatch check (`crates/emerald-codegen/src/lib.rs`'s `emerald_actor_enqueue`-routing logic) with a rejection: a class whose fields include a compiler-recognized 'native handle' marker type cannot be packed into a trampoline's `argv`, a real, disclosed v1 sema/codegen diagnostic (\"native resource handles cannot cross an actor boundary\") rather than a silently-corrupting shared mutable `i64` racing across two actors' worker threads with no lock around the registry entry itself."
    status: done
  - id: leaf-example-and-gate
    content: "Add `examples/resource_handle_lifetime_proof.em` (this plan's own concrete proof below, using a trivial in-memory counter-backed 'resource' introduced only for this proof — a real I/O-backed resource is a later domain plan's job, per Out of scope) to `examples/`, wired into the CI-checked table per plan 91/92's precedent. Add Rust `#[test]`s in `emerald-rt` directly exercising `emerald_rt_handle_alloc`/`get`/`close`/double-close/use-after-close against the registry with no Emerald compilation involved, proving the registry's own concurrency safety (a `Mutex`-guarded `HashMap`, exercised from multiple Rust threads in one test) independent of whether any Emerald program ever calls it. Run the full `AGENTS.md` gate."
    status: done
isProject: false
---

# Plan 93 — Resource Handle & Lifetime Model for Native Objects

Emerald has no garbage collector, and `emerald_alloc` — verified
directly against `runtime/emerald_runtime.c` by plan 51's own Decision
log — never frees; plan 51's scope-based arenas reclaim memory in bulk
on function-frame or actor-instance exit, but that reclaims *Emerald
heap memory*, not an external OS resource a native crate is holding on
Rust's own side (a socket file descriptor, a live database connection
object, a TLS session's key material, an in-flight compression
stream's internal buffers). Freeing the Rust-side `Box<dyn Any>` a
crate returned when its owning Emerald `Point`-like wrapper instance's
arena gets bulk-freed would require the region allocator itself to know
how to run an arbitrary `Drop` implementation on an opaque blob inside
its bump-allocated chunk — a capability plan 51's region type
deliberately does not have (it frees raw bytes, not typed values with
destructors) and this plan does not retrofit onto it. This plan instead
gives every long-lived native resource its own lifecycle, entirely
independent of Emerald's region/arena story, via an explicit handle
registry inside `emerald-rt` and a method Emerald code must call by
name. It depends on plan 92 for the exception channel a misuse reports
through, and cites plan 82/83's `own`/`borrow` ownership model only to
explain why this plan deliberately does not build on top of it yet.

## Concrete proof this plan targets

```ruby
class Counter
  handle: Int64

  def initialize -> Void
    @handle = Counter.native_open
  end

  def bump -> Int64
    Counter.native_bump(@handle)
  end

  def close -> Void
    Counter.native_close(@handle)
  end
end

c: Counter = Counter.new
puts c.bump
puts c.bump
c.close

begin
  puts c.bump
rescue NativeError => e
  puts e.message
end

c.close
puts "still running"
```

`Counter.native_open`/`native_bump`/`native_close` are three trivial
`emerald_rt_handle_*`-backed exports this plan's own leaf adds purely to
exercise the registry (`native_open` calls `emerald_rt_handle_alloc`
with a boxed `i64` counter starting at `0`; `native_bump` calls
`emerald_rt_handle_get_mut`, increments, returns the new value;
`native_close` calls `emerald_rt_handle_close`) — not a real I/O
resource, since this plan's job is the lifecycle mechanism, not any
particular domain's stdlib surface (see Out of scope). Expected output:
`1`, `2` (two real, independent bumps of the same handle's boxed
state); then, after `c.close`, the `c.bump` inside `begin`/`rescue`
raises a real `NativeError` whose message names the closed handle,
caught and printed rather than corrupting memory or crashing the
process; then the second, redundant `c.close` is a silent no-op (per
`leaf-double-close-is-a-noop-not-an-error`) and `"still running"`
prints, proving double-close is genuinely harmless rather than merely
untested.

## Decision log

- **Representation: an opaque `i64` handle backed by a process-wide
  `Mutex<HashMap<u64, Box<dyn Any + Send>>>`, not a raw pointer smuggled
  through Emerald as an `Int64`.** A raw Rust pointer cast to `i64` and
  handed to Emerald source would let Emerald code perform arithmetic on
  it, copy it, or store multiple live copies with no way for
  `emerald-rt` to ever revoke or validate one — exactly the unchecked-
  raw-pointer hazard plan 59's own `CString`/`String?` design already
  went out of its way to fence off for C interop generally ("the real
  friction is trust and lifetime, not bytes"). A registry-issued integer
  ID, looked up through a `Mutex`-guarded table on every access, gives
  `emerald-rt` a real validation point at every use (`leaf-use-after-
  close-diagnostic`) that a bare pointer never could — the ID itself is
  inert data to Emerald code; only `emerald-rt`'s own registry can turn
  it into a live reference. `Box<dyn Any + Send>` (rather than a fixed,
  hand-written `enum ResourceKind { TcpStream(...), DbConn(...), ... }`
  covering every domain this batch will ever add) is chosen because this
  plan is authored before any of the 96+ domain plans that will actually
  populate the registry exist — a fixed enum would need editing by every
  future domain plan just to add its own variant, defeating the "one
  crate, one shared convention" purpose plan 92 already established;
  `Any`'s runtime downcast (`Box<dyn Any>::downcast_ref::<T>()`) costs a
  cheap `TypeId` compare per access, paid once per resource method call,
  not per byte moved — an acceptable, disclosed cost for the isolation
  it buys.
- **`Send`, not `Sync`, and deliberately so — this is the same
  constraint `leaf-actor-local-handle-enforcement` enforces at the
  Emerald-source level, restated at the Rust-type level.** `Box<dyn Any
  + Send>` (a value safely *transferable* to another thread) rather
  than `+ Sync` (a value safely *shared by reference* across threads
  simultaneously) is the correct bound precisely because this plan
  decides handles are never concurrently accessed from two actors at
  once (see below) — only ever owned, at any moment, by whichever single
  worker thread is currently running the one actor method holding the
  handle's `i64`, the same "at most one thread inside a given actor's
  code at a time" invariant plan 55's own Decision log already
  establishes as its entire cross-actor safety argument. Requiring
  `Sync` would be a strictly stronger, unnecessary bound purchased for
  a sharing pattern this plan explicitly declines to support.
- **Explicit `.close()` is the v1 mechanism; automatic cleanup via
  plan 82/83's `own`/`borrow` scope-exit machinery is named as real,
  deferred future work, not silently assumed solved.** Verified this
  session, directly against `crates/emerald-parser/src/grammar.lalrpop`
  (lines 1985-2000): `own`/`borrow`/`borrow var` exist today purely as
  `TypeExpr` prefixes — "reachable everywhere `TypeExpr` itself is
  (params, return types, fields, `Let`s)... `emerald-sema` rejects every
  position but a function/method parameter with a real, named
  diagnostic" (the grammar file's own comment, citing plan 83's
  `spec/OWNERSHIP.md` §2). There is no shipped destructor-call
  mechanism anywhere in this codebase today that fires when an
  `own`-typed *local variable* (as opposed to a function parameter)
  goes out of scope — ownership enforcement as it exists right now is a
  sema-level linear-use/consumption check at call sites (this session's
  own git history: "track own-consumption at a generic function call
  site"), not a codegen-level scope-exit hook that could call
  `.close()` automatically the way C++ RAII or Rust's own `Drop` would.
  Building that hook — deciding what "goes out of scope" even means
  for a `Counter` instance whose `@handle` field is what actually needs
  closing, not the instance's own storage, and reconciling it with
  plan 51's region-bulk-free model, which has no per-object destructor
  call at region-destroy time either — is a substantial, separate
  design question this plan declines to answer speculatively. Explicit
  `.close()`, called by the Emerald programmer the same way a Ruby
  `File#close` or a Python context-manager-less `f.close()` already
  works today, is the honest, buildable v1 answer; **"integrate
  automatic resource cleanup with the `own`/`borrow` scope-exit point,
  once one exists"** is recorded here by name as the concrete future
  improvement, not left implicit.
- **A closed or unknown handle is a disclosed, named runtime error via
  plan 92's exception channel — never UB, and never a silent no-op that
  could mask a real bug.** This mirrors `emerald_hash_key_not_found`'s
  own precedent in *category* (verified, `runtime/emerald_runtime.c`
  lines 258-261: `fprintf(stderr, "uncaught error: Hash key not
  found\n"); exit(1);` — "a real, controlled failure path, not silent
  UB," per that function's own doc comment cited in plan 45's Decision
  log) but not in *mechanism*: `emerald_hash_key_not_found` aborts the
  whole process, appropriate for a `Hash[K,V]` miss with no exception
  system built yet at the time plan 25 shipped it. Plan 92 already
  exists now, with a real `NativeError`/`rescue`-based channel this
  plan reuses instead of a hard process abort — a use-after-close is
  exactly the "programmer-error condition a well-behaved caller usually
  isn't expected to routinely check for" category plan 53's two-channel
  model assigns to exceptions, not `Result[T, E]` (an ordinary,
  correctly-written program never calls a method on a handle it just
  closed; when one does, that is a real bug worth surfacing loudly and
  specifically, exactly as this plan's own concrete proof demonstrates
  via `rescue NativeError => e`).
- **A handle is actor-local: it may never be read by, written to, or
  passed as an argument into a different actor than the one that
  allocated it — decided and enforced at the codegen boundary, not
  merely documented.** Two real alternatives were weighed. (1) A fully
  shared handle, usable from any actor holding its `i64`, protected only
  by the registry's own `Mutex` — rejected because the `Mutex` only
  protects the *registry's bookkeeping* (which slot maps to which
  boxed value), not the *resource itself*: two actors racing a
  `.read()`/`.write()` against the same underlying `TcpStream` through
  two independent `downcast_mut::<TcpStream>()` calls would each
  briefly hold an exclusive `&mut` reference validly per the registry's
  own locking, yet still interleave reads/writes on the same socket in
  an order neither actor's own code controls — a real data race at the
  *resource* level even with zero UB at the *Rust* level, and a
  correctness hazard this plan is not willing to hand to every future
  domain plan to work around individually. (2) Handle-per-message-copy
  (cloning the underlying resource on every cross-actor send) — rejected
  because most native resources this batch will wrap (sockets, DB
  connections, TLS sessions) are not cheaply cloneable, and some
  (a mid-write compression stream) are not cloneable at all without
  losing correctness. Actor-local — a handle allocated inside one
  actor's method body stays reachable only from that same actor's own
  subsequent method calls, enforced by `leaf-actor-local-handle-
  enforcement`'s rejection of a native-handle-holding class value in
  cross-actor trampoline argument packing — directly mirrors plan
  54/55's own isolated-heap design (verified: plan 54's own proof
  requires "two independently-allocated instances whose state genuinely
  does not interfere," and plan 55's dispatch rule already treats
  anything but a literal `self` receiver of actor type as a
  must-enqueue cross-actor send) and composes with it for free — a
  native resource follows the same isolation discipline Emerald's
  concurrency model already gives every other piece of actor state,
  rather than becoming the one kind of value that quietly bypasses it.
- **Double-close is a harmless no-op; use-after-close is a hard,
  disclosed error — this asymmetry is deliberate, not an oversight.**
  Idempotent close is the same convention Rust's own `Drop` (infallible,
  and safe to compose with an explicit earlier `drop(x)`), Python's
  file objects, and Ruby's `IO#close` (a second `close` on an already-
  closed `IO` is a documented no-op, not an `IOError`) all converge on
  independently — a program that calls `.close()` from two different
  code paths defensively (an explicit call plus, once it exists, a
  future auto-close hook — see above) should not have to reason about
  which one "wins." Using a closed resource, by contrast, is
  information a correctly-written program should never need to
  recover from silently — it is a real logic bug, not a benign
  redundancy, which is exactly the distinction `leaf-double-close-is-a-
  noop-not-an-error`'s module doc states plainly.
- **The `closed: bool` flag on a still-present `RegistryEntry`, rather
  than an immediate `HashMap::remove` on close, is what makes the two
  diagnostics above distinguishable at all.** Removing the entry
  outright on close would make a subsequent use and a use of an ID this
  process genuinely never issued indistinguishable — both would see a
  missing key. Keeping a closed, emptied entry in place (dropping its
  boxed value in-place, per `leaf-registry-and-handle-type`, so no
  actual resource stays held open) lets `emerald_rt_handle_get` report
  "use of a *closed* handle" specifically, a materially more useful
  diagnostic for the exact double-use bug this plan's own concrete
  proof demonstrates, at the cost of never reclaiming a `HashMap` slot
  for a closed handle's now-dead entry for the life of the process — a
  real, small, permanent per-handle bookkeeping cost (one `HashMap`
  entry, a handful of bytes) this plan accepts deliberately rather than
  silently.
- **Out of scope.** No specific I/O-backed resource type (`TcpStream`,
  a DB connection pool, a TLS session) — this plan's own `Counter`
  proof is a deliberately trivial in-memory stand-in; the first domain
  plan that actually needs a socket or a connection builds its own
  typed wrapper class on top of this plan's four Rust-internal
  primitives, per the naming convention plan 92 already established
  for its own exported symbol names. No automatic/RAII-style cleanup
  hooked to scope exit or `own`/`borrow` (named above as real, deferred
  future work, not attempted here). No cross-process or cross-restart
  handle persistence — every handle is only ever valid for the
  lifetime of the single OS process that issued it, matching
  `emerald_alloc`'s own single-process-lifetime memory model. No
  handle pooling/reuse-after-close (a closed ID is never reissued to a
  new resource, by construction of the monotonic counter) — this
  trades a small amount of unreclaimed ID space for the guarantee that
  a stale ID a buggy program kept around can never silently start
  referring to an unrelated, newer resource.

## Not yet decided (blocking EXECUTE)

1. Whether the monotonic `u64` handle counter needs any wraparound
   handling — at one allocation per nanosecond, `u64` overflow is
   effectively unreachable within any real process's lifetime, but this
   plan does not formally prove that bound and leaves the counter as a
   plain wrapping `u64` add, deferring an explicit panic-on-overflow
   decision to whichever future review finds this assumption load-
   bearing.
2. Whether `leaf-actor-local-handle-enforcement`'s codegen-level
   rejection needs a matching sema-level diagnostic (caught before
   codegen, with a source span) rather than only a codegen-time
   failure — the concrete proof above never exercises a cross-actor
   handle-passing attempt, so this plan does not have a worked example
   to verify either error-reporting path's actual quality against; the
   executing leaf should default to a sema-level check (consistent with
   how plan 55 itself gates `Expr::Spawn`/`Expr::New` in sema, not
   codegen) unless a concrete blocker surfaces.
3. The exact shape of the future `own`/`borrow` auto-close integration
   named above — deliberately left undesigned here; a later plan
   revisiting it should treat this plan's explicit `.close()` mechanism
   as the thing auto-close eventually *calls*, not something it
   replaces, so that a program written against explicit `.close()`
   today keeps working unmodified once auto-close exists.

## Update (2026-09-22, same-day session): implemented, all five leaves done

`crates/emerald-rt/src/handle.rs` created: `handle_alloc`/`handle_get`/
`handle_get_mut`/`handle_close`, backed by a process-wide
`Mutex<HashMap<u64, RegistryEntry>>` (`RegistryEntry.value: Option<Box
<dyn Any + Send>>` — `None` after close, exactly the "keep a closed,
emptied entry in place" design this plan's own Decision log specifies).
Six real Rust `#[test]`s, including one spawning 8 real OS threads
racing 1000 increments each against the same handle through a real
`Mutex`, proving the registry's own concurrency safety independently
of any Emerald compilation. Three `emerald_rt_handle_counter_*` exports
prove the mechanism end to end through `examples/resource_handle_
lifetime_proof.em`, printing exactly this plan's own predicted
sequence (`1`, `2`, `use of closed counter handle`, `still running`).

Two real, disclosed corrections to this plan's own original Concrete
Proof sketch, found only by running it (the same "found only by
running it" pattern plans 91/92 already established):

- **This language has no user-declarable class-static-method syntax at
  all.** The plan's own sketch called `Counter.native_open`/
  `native_bump`/`native_close` as `ClassName.method` static dispatch
  from within `Counter`'s own methods — `fn self.native_open` fails to
  parse (`Unrecognized token '.'`, expected `"("`/`":"`/`"["`). Fixed
  by making all three ordinary compiler-intrinsic FREE FUNCTIONS
  instead (`handle_counter_open()`/`handle_counter_bump(h)`/
  `handle_counter_close(h)`), dispatched by exact name in both
  `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s
  `build_expr`/`build_stmt`, exactly the mechanism `puts`/`gets`/
  `is_valid_int` already use — a real, disclosed narrower mechanism
  than the plan's own sketch assumed, not a new one invented for this.
- **A zero-argument `.new`/free-function call still needs explicit
  `()`.** `Counter.new` (no parens) and `Receiver.spawn` (no parens,
  found while testing `leaf-actor-local-handle-enforcement` below) both
  fail to parse; `Counter.new()`/`Receiver.spawn()` are required.

**`leaf-actor-local-handle-enforcement`, implemented as a real,
enforced sema-level check** (per this plan's own "Not yet decided"
item 2, which named sema — not codegen — as the default): a
compiler-synthesized `NativeHandle` newtype over `Int64` (registered in
`emerald-sema`'s `classes` map and `emerald-codegen`'s `NEWTYPE_
UNDERLYING` thread-local, the exact same "compiler-synthesized, no
source declaration" precedent `NativeError` already established in
plan 92) — a future resource-holding domain plan declares its own
handle field as `NativeHandle` instead of a bare `Int64` to opt into
this enforcement. `emerald-sema`'s `type_contains_native_handle` walks
a class's own (recursively, cycle-guarded) field types; `infer_expr_
type`'s cross-actor `Expr::MethodCall` arm calls it against every
argument's inferred type and rejects with `"native resource handles
cannot cross an actor boundary"` before ever reaching codegen — real,
disclosed narrowing found by testing it: `emerald-codegen`'s own
`is_wire_safe_class_field` (extended too, per the leaf's own literal
text) turned out to be consulted only by the wire-encode/decode codec
path (apparently reserved for a future remote-actor dispatch mode),
never by the actual LOCAL same-process cross-actor `argv`-packing path
(`build_actor_enqueue_call`, which just bit-casts/pointer-casts any
`Ptr`-kind argument into a raw `i64` slot regardless) — so the codegen-
level change alone, verified directly, compiled a `NativeHandle`-
holding class across an actor boundary with ZERO rejection and no
runtime error either. The sema-level check is what actually blocks it;
the codegen-level `is_wire_safe_class_field` change stays too (correct
and harmless, and does the intended job for whatever future remote-
dispatch path eventually consults it), but is not, by itself,
sufficient — a fact this plan's own text does not anticipate, found
only by building both and testing the actual compiled behavior of one
before adding the other. Two `emerald-sema` regression tests (reject/
accept) cover this, since the plan's own Concrete Proof intentionally
never exercises an actor at all (`Counter` is a plain class).

Full workspace gate: `cargo nextest run --workspace` (941/941, 2
skipped — 9 new: 6 in `emerald-rt`, 2 in `emerald-sema`, 1 in
`emerald-cli`), `cargo clippy --workspace --all-targets` (clean),
`treefmt` (0 changed), `cargo audit` (same 5 pre-existing, triaged
warnings as plan 95 — no new finding).
