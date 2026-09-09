---
name: Distributed, Location-Transparent Actors
overview: "An actor reference becomes a tagged LOCAL-or-REMOTE handle behind one indirection; the identical `recv.method(args)` send syntax now compiles to plan 55's existing local enqueue or a new socket-framed remote enqueue, chosen by a runtime tag check codegen emits once. v1 connects exactly two known processes over a real TCP connection to a hardcoded host:port — not a self-organizing cluster."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-actor-ref-and-addressing
    content: "EmeraldActorRef tagged handle (local arena ptr | remote node+socket+id); `.register(name, port)` and `ClassName.remote(addr, name)` as new reserved call forms parallel to `.spawn`; hardcoded host:port addressing, a RESOLVE handshake, RemoteActorError on connect failure"
    status: pending
  - id: leaf-wire-codec
    content: "Compiler-generated per-concrete-type encode/decode reusing plan 08/32's ClassLayout field-offset metadata verbatim; a wire-safety predicate extending plan 56's linear-use rule for the cross-process case; per-actor-method argument-list encoders parallel to plan 55's per-method trampolines"
    status: pending
  - id: leaf-tcp-transport
    content: "runtime/emerald_runtime.c grows a minimal POSIX-sockets TCP listener/dialer/framing layer — real new scope, since plan 45 (stdlib-strings-and-io) has no socket primitives at all, verified against its real file"
    status: pending
  - id: leaf-remote-dispatch-and-worked-proof
    content: "build_method_call's dispatch branch grows a third, runtime-tag-checked arm reusing emerald_actor_enqueue unchanged for local and a new emerald_actor_dispatch_remote for remote; drain-and-join extended so a `.register`'d process blocks instead of exiting; two real OS-process worked example with RemoteSendError on failure"
    status: pending
isProject: false
---

# Plan 60 — Distributed, Location-Transparent Actors

This is plan 60 of the 58–64 batch, a direct follow-up to the 48–57
concurrency-pillar batch (54 actor declarations + isolated heaps → 55
scheduler + message passing → 56 compile-time message safety → 57
supervision trees). It was commissioned by a debate that asked, setting
aside maturity/ecosystem/stability entirely: what is Emerald's
theoretical technical ceiling measured against C/C++/Rust/Python/Ruby/
TypeScript/Go/Haskell/Elixir combined? That debate identified
distributed, location-transparent actors — Erlang/Elixir's signature
capability, the thing BEAM is *for* — as real, unclaimed territory: none
of plans 01–57 give Emerald a way for one OS process to send a typed
message to an actor living in a different OS process, let alone a
different machine. Like every post-v1 plan before it (17, 28–47, 48–57),
this plan is not a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
and does not touch that file or any other plan file.

This plan proceeds on the same forward-declared-contract discipline
plan 55 used against plan 54 before plan 54's file existed: at
authoring time, `history/` contains plans through 57 on disk, plus this
plan's own file; plans 58, 59, and 61–64 are being authored in parallel
in this same batch and do not yet exist (verified this session via a
directory listing). In particular, **no plan 59 exists** — so this
plan's exclusion of "plan-59-style FFI handles" from what is safe to
send across a process boundary (Design decision 2 below) is stated
prospectively, the same way plan 55 stated its actor-representation
assumptions about plan 54 before that file existed, and the same way
plan 56 stated its assumptions about both plan 51 and plan 55.
Depends on, verified against real, landed source this session: plan 54
(`2026-09-09T124000Z-plan-54-actor-declarations-and-isolated-heaps.md`),
plan 55
(`2026-09-09T125000Z-plan-55-scheduler-and-message-passing.md`), and
plan 56
(`2026-09-09T130000Z-plan-56-compile-time-message-safety.md`). Also
depends on plan 45
(`2026-09-09T111000Z-plan-45-stdlib-strings-and-io.md`, `File.read`/
`File.write`/`ARGV`/`gets` — verified this session, in full, to contain
**no socket, TCP, or networking primitive of any kind**; this plan
states that gap explicitly and closes the minimal slice of it itself,
see `leaf-tcp-transport`) and plan 23 (`multi-file-compilation.md`,
`require` — the mechanism that lets two separately-compiled OS
processes share one `actor` declaration's source, load-bearing to this
plan's whole addressing story, see Design decision 4 below).

**This plan's scope is deliberately narrower than "distributed Erlang."**
It builds exactly: a tagged local-or-remote actor reference; the
identical send syntax working unchanged over both; a compiler-generated
binary wire format for message arguments; and a minimal TCP transport.
It connects **exactly two known processes**, addressed by a hardcoded
`host:port` string literal, over **one real TCP socket**. It does not
build cluster membership, node discovery, gossip, or any N-node mesh —
see Design decision 4. It does not extend plan 57's supervision trees
across the network boundary — see Design decision 5.

Concrete proof this plan targets — two separately compiled, separately
launched OS processes, `require`-sharing one actor declaration:

```ruby
# counter_actor.em — required by both processes
actor Counter
  count: Int64

  def initialize(start: Int64) -> Void
    @count = start
  end

  def increment -> Void
    @count = @count + 1
  end

  def report -> Void
    puts @count
  end
end
```

```ruby
# host.em
require "counter_actor"

c: Counter = Counter.spawn(0)
c.register("counter1", 9000)
```

```ruby
# client.em
require "counter_actor"

remote: Counter = Counter.remote("127.0.0.1:9000", "counter1")
remote.increment
remote.increment
remote.increment
remote.report
```

Expected proof: launch the compiled `host` binary first; once it is
observed listening on port 9000 (the acceptance test polls the port,
not a fixed sleep), launch the compiled `client` binary; `client` exits
once its three `increment` sends and one `report` send have been
written to the socket. `host`'s own stdout — captured by the test
harness, which owns `host`'s process lifecycle since this plan has no
remote-shutdown protocol (see Design decision 5) — prints `3`: the
receiving process's own `Counter` instance, mutated only by messages
that arrived over a real socket from a different process's address
space, holding the correct final state. This is the genuine two-process,
real-socket proof the task requires — not a same-process simulation of
two "logical" nodes.

## Decision log

- **Design decision 1 — the tagged reference, and why it costs one
  indirection, not a widened field/argv slot.** Verified against plan
  55's real, landed mechanism (its own Decision log, `leaf-actor-header-
  and-trampolines`): today, a `Counter`-typed value *is* a bare pointer
  to the instance's arena (an `EmeraldActorHeader`-prefixed allocation
  from plan 51's `emerald_region_alloc`), stored in exactly one 8-byte
  local/field/`argv` slot — the same fixed-width-word convention plan
  08 established for every field. Making that slot sometimes mean
  "local arena pointer" and sometimes "remote node address" *without*
  changing what a call site looks like — the literal ask in the task's
  point 1 — rules out widening the slot itself: every existing field
  offset (plan 08/32), the 16-word `argv` cap (plan 55), and every
  already-compiled `@peer`-shaped access would need to change size,
  which is a strictly bigger, riskier surface than this plan needs.
  Instead: from this plan onward, an actor-typed value is a pointer to
  a small, fixed-size `EmeraldActorRef` record —
  ```c
  typedef struct EmeraldActorRef {
      uint8_t  is_remote;
      void    *local_arena;      /* valid iff !is_remote: EmeraldActorHeader* */
      int32_t  node_ipv4;        /* valid iff is_remote, network byte order */
      uint16_t node_port;
      uint64_t remote_actor_id;  /* opaque id, assigned by the RESOLVE handshake */
      int32_t  sockfd;           /* valid iff is_remote: one socket per ref */
  } EmeraldActorRef;
  ```
  `.spawn` (plan 54/55, unchanged shape) now allocates one of these via
  a new `emerald_actor_ref_local(void *arena)` right after
  `emerald_region_alloc` and returns *that* pointer as the actor's
  value, instead of the raw arena pointer — one extra fixed-size
  allocation per spawn, and the actor's runtime "value" is still
  exactly one pointer-sized word everywhere it already flowed. Every
  existing local/field/`argv` slot, and every already-landed plan
  54/55/56 mechanism that stores or copies an actor-typed value, is
  unaffected in shape; only what that one pointer now points *at*
  changed. `build_method_call`'s dispatch (plan 55) and this plan's own
  new dispatch arm both start by loading `ref->is_remote` — the same
  single field, checked once, is what makes an ordinary call site not
  need to know which kind of reference it holds.
- **Design decision 1b — this is not a vtable, for the same reason plan
  55's trampoline-in-a-message-struct wasn't one.** Both processes in
  this plan's worked example are compiled from source that `require`s
  the identical `counter_actor.em` (Design decision 4) — so both know,
  entirely at compile time, `Counter`'s exact field layout and its exact
  method-declaration order. The receiving process's dispatch table (see
  `leaf-remote-dispatch-and-worked-proof`) is a plain, compiler-emitted
  array indexed by a `method_tag` chosen at compile time by declaration
  order (the identical convention plan 08/32 already use for field
  offsets) — never a lookup keyed on a runtime type discovered from the
  wire. No subclass-polymorphic override resolution is possible here
  either, since actors carry no inheritance chain at all (plan 54's own
  constraint, unchanged). The "no dynamic dispatch, no vtables"
  identity constraint holds exactly as it did after plan 55.
- **Design decision 2 — wire safety extends plan 56's linear-use rule;
  it does not replace it.** Verified against plan 56's real, landed
  rule (its Decision log's "one rule, stated precisely" bullet): a
  cross-actor message argument is legal iff it is a value type, or a
  reference type whose sending expression is the sender's provably last
  use of that local — `leaf-payload-classification`'s four buckets
  (value type / trivially-fresh reference / named-local reference /
  aliasing-shape reference, the last rejected outright). This plan does
  not touch that liveness check at all — it still runs, unchanged,
  for every cross-actor send, local or remote, because the sender-side
  hazard it guards against (the sender's own plan-51 arena reclaiming
  memory the receiver still reads through) is identical in both cases.
  What this plan *adds*, only for a send whose receiver's `EmeraldActorRef.
  is_remote` could be true — which sema cannot know statically, so the
  check below is conservative and applies to every cross-actor send
  whose receiver's *static type* is `Type::Actor(_)`, not only ones
  proven remote — is a second, independent, purely-type-driven
  predicate: **wire-safety**. A type is wire-safe iff it is one of plan
  56's value types (`Int64`/`Float64`/`Boolean`/`Symbol`), `String`, or
  a `Class(name)` whose fields are *recursively* wire-safe, walked using
  the exact same `ClassLayout`/field-list metadata plan 08 built and
  plan 32 generalized for codegen — a real, concrete reuse of existing
  compile-time metadata to answer a new compile-time question ("is this
  type sendable"), computed once per concrete class name, not a new
  runtime reflection mechanism (nothing about this check exists once
  the binary is built; it is a sema-time predicate over already-computed
  layout data, exactly like plan 56's own `check_stmt`/`check_block`
  reuse). Declared **not** wire-safe in v1, each for a concrete,
  disclosed reason: `Array[T]`/`Hash[K,V]` (plan 45 already documents
  `Array[T]` carries no runtime length metadata — nothing to encode a
  length *from* without redesigning that representation, out of scope
  here exactly as it was for plan 45); `Proc`/lambda values (a closure
  embeds a code pointer and captured-environment addresses — literally
  raw pointers meaningless in another process's address space, the
  identical hazard a future plan-59 FFI handle would pose, which is why
  this plan excludes both for the same reason even though only one of
  the two types exists yet); and any `Actor`-typed field or argument
  (sending a reference to a *third* actor as a message payload is a
  real, larger generalization — recipient-side re-resolution of a
  received `EmeraldActorRef` is not attempted by this plan, real future
  work). A wire-safety failure is a sema diagnostic at the send site,
  the same "real diagnostic, not a panic" standard every prior plan
  uses, layered on top of (never instead of) plan 56's own liveness
  diagnostic.
- **Design decision 2b — decode always reconstructs, never remaps a
  pointer, which is actually a *stronger* safety property than the
  local case.** Plan 56's whole liveness proof exists because a local
  cross-actor send can share the same OS address space with the
  receiver. A remote send cannot — there is no shared memory between
  two OS processes at all, so `ClassName_decode` (see `leaf-wire-codec`)
  unconditionally allocates a fresh instance in the *receiving*
  process's own heap via the same plain `emerald_alloc` plan 08 already
  uses (not plan 51's arena — a decoded remote payload has no sender-
  side scope to be bound to on the receiving side, so it leaks exactly
  as every other `emerald_alloc`'d value already does today, the same
  disclosed "no free() yet" posture this whole project has carried since
  plan 08, not a new gap this plan introduces) and copies every field's
  bytes into it. The sender's own copy is never touched again after
  `leaf-wire-codec`'s encoder walks it (synchronously, inside the send
  call, before the sender's `argv` goes out of scope) — so plan 56's
  liveness check is exactly what makes it sound for the sender's plan-
  51 arena to be reclaimed afterward, and the unconditional decode-side
  copy is what makes cross-process aliasing structurally impossible
  regardless.
- **Design decision 3 — TCP transport is real new scope this plan takes
  on, not something plan 45 already provides.** Verified this session,
  reading plan 45's file in full: its entire stdlib-strings-and-io
  surface is `String` intrinsics, `File.read`/`File.write` (backed by
  plain libc `fopen`/`fread`/`fwrite`), and `ARGV`/`ARGC`/`gets()` — no
  `socket`/`bind`/`listen`/`accept`/`connect` call, no `sys/socket.h`
  include, anywhere in its Decision log or leaves. `leaf-tcp-transport`
  therefore adds a minimal POSIX-sockets listener/dialer/framing layer
  to `runtime/emerald_runtime.c` itself — the same file, and the same
  "plain C, no new Cargo dependency, `cc`-compiled by `emerald-cli`'s
  existing `build.rs`" architectural choice plan 55 already made for
  the scheduler, for the identical reason: this has to be reachable
  from a user's compiled, linked, standalone binary, not from the
  compiler process. This plan does not go back and add a `Socket` class
  or any user-facing networking API to the language surface plan 45
  owns — the TCP layer built here is purely a private implementation
  detail of actor message delivery, invoked only from codegen-emitted
  calls at `.register`/`.remote`/cross-actor-send sites, never
  something Emerald source can call directly.
- **Design decision 4 — hardcoded `host:port`, no cluster membership,
  and why that gap is stated plainly.** A node in this plan is
  identified by nothing more than an IPv4 literal and a port, passed as
  an ordinary Emerald `String` to `.remote(addr, name)` — parsed at
  runtime by a small `host:port` splitter added alongside the transport
  layer. There is no node-discovery protocol, no gossip, no cluster
  membership table, no equivalent of BEAM's EPMD or cookie-based
  inter-node authentication — real, named pieces of what actual Erlang/
  Elixir distribution provides that this plan does not attempt. Nor
  does this plan give two mutually-unaware processes any way to *find*
  each other — the client must already know the exact host, port, and
  registered name of the actor it wants, hardcoded or otherwise
  supplied out-of-band (an environment variable, a command-line
  argument via `ARGV`, or literally in source, as in this plan's own
  worked example). What makes the two processes' understanding of
  `Counter` agree at all — field offsets, method-tag ordering, wire
  encoding — is that both are compiled from source that `require`s
  (plan 23) the identical `counter_actor.em`; this plan performs **no
  runtime schema negotiation or version check** between the two
  processes (no protocol-version handshake beyond the bare RESOLVE-name
  lookup in `leaf-actor-ref-and-addressing`), so two processes compiled
  from *drifted* copies of `counter_actor.em` would silently
  miscommunicate — a real, disclosed limitation, not a hidden one. This
  plan proves exactly one thing, stated in the terms the task asked for
  legibility on: **location-transparent, typed message passing between
  two known processes** — not a self-organizing cluster, and the gap
  between the two is named explicitly here rather than blurred by
  reusing "distributed actors" without qualification.
- **Design decision 5 — cross-node supervision is explicitly declined.**
  Plan 57's supervision trees (verified: restart strategies over a
  parent-child actor hierarchy) operate entirely within one process's
  worker pool and mailbox machinery; nothing in this plan extends a
  supervisor's restart authority across a socket. A remote actor's
  crash, or its host process dying outright, surfaces to the sender
  only as a **failed or timed-out send** — `emerald_tcp_send_frame`
  returning a socket error (`EPIPE`/`ECONNRESET` on an already-open
  connection, `ECONNREFUSED`/timeout on the initial `.remote(...)`
  connect) — which this plan raises as a real, catchable
  `RemoteActorError` (a plain class with a `message: String` field,
  reusing plan 11/38's existing exception machinery exactly, not a new
  mechanism) rather than silently hanging or aborting the process the
  way plan 45's `File` errors do. There is no automatic reconnection,
  no supervised restart of the remote side, and — the concrete,
  disclosed reason a hang and a crash are not distinguishable in v1 —
  **no heartbeat/liveness protocol**: a genuinely wedged (not crashed)
  peer that never closes its socket is only detected once a send is
  attempted and the process-level `EMERALD_REMOTE_TIMEOUT_MS` socket
  timeout (set once, at connect time, via `SO_SNDTIMEO`/`SO_RCVTIMEO`,
  applied to the RESOLVE handshake and every later send on that same
  socket — mirroring plan 55's own `EMERALD_WORKERS` environment-
  variable convention) elapses. A real heartbeat/liveness protocol,
  and any form of cross-node supervised restart built on top of one, is
  named here as concrete, disclosed future work — not attempted, not
  partially built.
- **Declined, matching this plan's own scope discipline elsewhere:**
  connection pooling/multiplexing (one socket per `EmeraldActorRef` —
  a real cost at scale, unnecessary for the two-process, one-remote-ref
  proof this plan targets); asynchronous/buffered outbound sends (a
  remote send's `write()` runs synchronously on the calling actor's own
  worker thread, exactly at the call site — a slow or blocked peer
  socket blocks that worker, the identical "no green threads, no hidden
  stack-switching" honesty plan 55 already committed to, now extended
  to the network path); TLS/authentication (a real, substantial, and
  entirely separate security surface); IPv6 (a straightforward, un-
  attempted future parallel to the IPv4 `sockaddr_in` path built here).

## Leaf: leaf-actor-ref-and-addressing

### 1. Context
- Why: nothing today lets an `actor`-typed local hold anything but a
  same-process arena pointer, and nothing lets a process announce an
  actor under a name another process can address.
- Target state: `EmeraldActorRef` (Design decision 1) in
  `runtime/emerald_runtime.c`; `emerald_actor_ref_local(void*) -> void*`
  used by `.spawn`'s existing codegen (one added call, plan 54/55's
  allocation and `initialize` sequencing otherwise untouched);
  `register`/`remote` become new reserved tokens, parallel to `spawn`
  (plan 54) — `crates/emerald-parser/src/ast.rs` gains `Stmt::Register
  { recv: String, name: Box<Expr>, port: Box<Expr> }` and `Expr::Remote
  { class: String, addr: Box<Expr>, name: Box<Expr> }`; `emerald-sema`
  requires `recv`'s type / `class` to be `is_actor: true` (same gate
  `.spawn`/`.new` already use) and `name`/`addr`/`port` to type-check
  `String`/`String`/`Int64`; codegen's `.register` lowers to
  `emerald_actor_register(ref, name_ptr, name_len, port)` (idempotent
  listener start, Design decision 3/`leaf-tcp-transport`) and
  `.remote(...)` lowers to `emerald_actor_ref_remote(host_ptr, host_len,
  port, name_ptr, name_len) -> void*` — a synchronous connect + RESOLVE-
  handshake (Design decision 4) that raises `RemoteActorError` on
  failure via the existing `emerald_raise` path (plan 11/38) rather
  than returning a null the caller could dereference.

### 2. Acceptance Criteria
1. `Counter.spawn(0)` still compiles, links, and runs identically to
   plan 54/55's own worked examples once its value is an
   `EmeraldActorRef*` instead of a bare arena pointer — a real
   regression check against plan 54's two-`Counter`-instances proof and
   plan 55's `PingPong`/`Spinner` proofs, all re-run unchanged.
2. `c.register("counter1", 9000)` on a `Counter` value, and a distinct
   `Widget` (non-actor `class`) receiver's `.register(...)` call,
   compile and reject respectively — sema's `is_actor` gate proven both
   directions, matching plan 54's own `.spawn`/`.new` symmetry.
3. `Counter.remote("127.0.0.1:9000", "counter1")` against a *running,
   registered* host process returns a value usable at every call site
   `Counter.spawn(...)`'s value already is (locals, fields, `argv`) —
   proven by successfully sending at least one message through it
   (real integration with `leaf-remote-dispatch-and-worked-proof`, not
   solely a parse/typecheck test here).
4. `Counter.remote("127.0.0.1:1", "counter1")` (a hardcoded closed
   port) raises `RemoteActorError`, catchable with `rescue
   RemoteActorError => e`, not a process abort and not a panic.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-sema/src/lib.rs`, `crates/emerald-codegen/src/lib.rs`,
  `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser -p emerald-sema -p emerald-codegen` | all pass, incl. `is_actor` gate both directions and `RemoteActorError` on a closed-port connect | agent-claimed-locally |

---

## Leaf: leaf-wire-codec

### 1. Context
- Why: `leaf-actor-ref-and-addressing` can open a socket, but nothing
  yet turns a compiler-known concrete type's in-memory field layout into
  bytes a different process can decode back into its own local heap.
- Target state: for every concrete `Class(name)` reachable as a
  wire-safe cross-actor argument (Design decision 2), codegen emits
  `{Name}_encode(i8* self, EmeraldWireBuf* out)` and `{Name}_decode
  (EmeraldWireBuf* in) -> i8*`, generated by walking that class's
  existing `ClassLayout` field list (plan 08's fixed 8-byte-per-field,
  declaration-order offsets, plan 32's chain-resolved generalization of
  the same scheme) exactly once per concrete class, in declaration
  order, dispatching per field on its already-known `Type`
  (`Int64`/`Float64`/`Boolean` → raw 8-byte copy; `String` → length-
  prefixed byte copy off the existing `ValKind::Str` representation;
  `Class(other)` → recurse into `Other_encode`/`Other_decode`) — no new
  runtime type information is computed anywhere; every branch is chosen
  at compile time from data plan 08/32 already produce. For every
  `actor` method reachable across the network, codegen additionally
  emits one small `{Actor}_{method}_encode_args(int64_t *argv,
  EmeraldWireBuf *out)` gluing the per-argument encoders together in
  parameter order — structurally one more per-method generated function,
  the same cardinality plan 55 already pays for its per-method
  trampolines.
- `EmeraldWireBuf` is a plain grow-on-demand byte buffer
  (`data`/`len`/`cap`, `realloc`-backed) added to `runtime/
  emerald_runtime.c` alongside `leaf-tcp-transport`'s framing code.

### 2. Acceptance Criteria
1. `LogMessage { text: String }`'s generated `LogMessage_encode`/
   `LogMessage_decode` round-trip a real instance byte-for-byte
   (encode then decode in the same process, a white-box unit test —
   verifies correctness independent of the network) including a nested
   `Class(other)` field one level deep.
2. A wire-unsafe argument (an `Array[String]`, a `Proc`, or a second
   actor reference passed as a message argument) is rejected at the
   send site with a real sema diagnostic naming the offending type —
   proven for at least one case from each of the three declined
   categories in Design decision 2.
3. Plan 56's own existing liveness diagnostic (a reused-after-send
   local) still fires unchanged on a remote send — proving this plan's
   new wire-safety check is additive, not a replacement, per Design
   decision 2.
4. A value-type argument (`Int64`) and a trivially-fresh `String`
   argument both encode/decode correctly with no local-liveness
   involvement at all, matching plan 56's own bucket (1)/(2)
   classification unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (wire-safety predicate),
  `crates/emerald-codegen/src/lib.rs` (per-class encode/decode, per-
  method argument-list encoder emission), `runtime/emerald_runtime.c`
  (`EmeraldWireBuf`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema -p emerald-codegen` | all pass, incl. round-trip encode/decode and wire-safety rejection tests | agent-claimed-locally |

---

## Leaf: leaf-tcp-transport

### 1. Context
- Why: verified this session, plan 45's real file has no socket
  primitive of any kind (Design decision 3) — this plan is the first to
  need one, and must add it itself.
- Target state: `runtime/emerald_runtime.c` gains `#include
  <sys/socket.h>`, `<netinet/in.h>`, `<arpa/inet.h>`; a `host:port`
  string parser; `emerald_tcp_listen(uint16_t port) -> int` (`socket`/
  `bind`/`listen`, `SO_REUSEADDR`); a per-process accept loop, started
  idempotently by the first `emerald_actor_register` call, spawning one
  reader `pthread` per accepted connection (reusing `leaf-thread-safe-
  runtime`'s existing pthread machinery, plan 55 — no new concurrency
  primitive introduced, only new callers of the one that already
  exists); `emerald_tcp_connect(uint32_t ip, uint16_t port,
  int timeout_ms) -> int` (`socket`/`connect` with `SO_SNDTIMEO`/
  `SO_RCVTIMEO` set once, per Design decision 5); length-prefixed frame
  I/O, `emerald_tcp_send_frame`/`emerald_tcp_recv_frame` (a `u32` byte
  length, then the payload — TCP is a byte stream, not a message
  stream, stated explicitly since nothing else in this codebase has
  needed framing before); a small process-local `name -> EmeraldActorRef*`
  registry (a mutex-guarded array, populated by `emerald_actor_register`,
  consulted by the RESOLVE handshake's server side and by each reader
  thread to find the target actor for an incoming `SEND` frame, then
  calling **`emerald_actor_enqueue` unchanged** — plan 55's exact local
  mailbox mechanism, reused verbatim as the concrete "identical
  mechanism" the task requires, not a parallel remote-mailbox
  implementation).

### 2. Acceptance Criteria
1. A standalone unit test opens a listener via `emerald_tcp_listen`,
   connects via `emerald_tcp_connect` from the same process (loopback),
   and round-trips one length-prefixed frame byte-for-byte.
2. `EMERALD_REMOTE_TIMEOUT_MS` (default value stated in the test)
   genuinely bounds a connect attempt to an unreachable/black-holed
   address — measured, not merely asserted to be wired.
3. A `SEND` frame received by a reader thread and routed through the
   name registry results in exactly one `emerald_actor_enqueue` call
   with the correct decoded `argv` — verified with a fake trampoline
   counting invocations, independent of any Emerald-compiled program.
4. Concurrent connections (two `emerald_tcp_connect` calls from two
   threads against one listener) are each served by their own reader
   thread with no cross-talk — a real regression proof this reuses
   plan 55's thread-safety guarantees rather than introducing a new
   shared-mutable-state hazard.

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c`
- **Modify (only if required by this toolchain, verify first):**
  `crates/emerald-cli/build.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| Runtime unit tests | `cargo test -p emerald-cli runtime_` | all pass, incl. new TCP round-trip/timeout/routing tests | agent-claimed-locally |
| Regression | `cargo test --workspace` | all pre-existing tests, esp. plan 55's actor/exception suite, still pass | agent-claimed-locally |

---

## Leaf: leaf-remote-dispatch-and-worked-proof

### 1. Context
- Why: every prior leaf in this plan builds a piece — the tagged
  reference, the wire codec, the transport — but nothing yet decides,
  at an ordinary cross-actor call site, to use the remote path instead
  of plan 55's existing local one, and nothing proves the whole chain
  end-to-end across two real processes.
- Target state: `build_method_call`'s existing syntactic dispatch
  branch (plan 55: literal `self` → direct call; other actor-typed
  receiver → `emerald_actor_enqueue`) grows exactly one more level,
  pushed into the runtime rather than duplicated in generated IR at
  every call site: codegen now always calls one new entry point,
  `emerald_actor_dispatch(EmeraldActorRef *ref, int32_t method_tag,
  void (*trampoline)(void*, int64_t*), void (*arg_encoder)(int64_t*,
  EmeraldWireBuf*), int64_t *argv)`, which internally branches on
  `ref->is_remote`: `false` calls `emerald_actor_enqueue` exactly as
  plan 55 already does (byte-for-byte the same function, same
  mailbox), `true` calls the new `arg_encoder`, then
  `emerald_tcp_send_frame`s a `SEND` frame over `ref->sockfd`, raising
  `RemoteActorError` (via `emerald_raise`) on any socket error. This
  single call-site shape is the concrete mechanism satisfying the
  task's "ordinary call-site code doesn't need to know which kind it
  has." `define_main`'s existing drain-and-join emission (plan 55) is
  extended: a process that has called `.register` at least once sets a
  `network_active` flag that `emerald_worker_pool_drain_and_join`
  checks alongside its existing outstanding-message-counter check —
  such a process blocks at end-of-`main` instead of exiting, since it
  is now a server whose remaining work arrives over the network, not
  its own local mailboxes; a process that never calls `.register`
  drains and exits exactly as today, unchanged.

### 2. Acceptance Criteria
1. The `Counter`/`host.em`/`client.em` worked example, compiled as two
   separate binaries and run as two real OS processes by the
   acceptance test (poll-for-listening, not a fixed sleep, before
   launching the client; the test owns killing `host` after observing
   its output, per Design decision 5), produces `host`'s stdout
   containing `3` — the genuine two-process, real-socket proof.
2. The identical `emerald_actor_dispatch` call, exercised against a
   *local* `Counter.spawn(...)`-produced reference in the same test
   suite, is proven (via inspecting emitted LLVM IR, the same
   inspection technique plan 55's own `leaf-cross-actor-dispatch` AC3
   already uses) to reach `emerald_actor_enqueue` with **zero** added
   overhead beyond plan 55's own existing call shape — proving this
   plan's remote path is genuinely additive, not a regression on the
   already-proven local path.
3. Killing `host`'s process (or connecting to a closed port) mid-send
   from `client` surfaces `RemoteActorError` at the `remote.increment`
   call site, catchable by `rescue`, not a client-process abort.
4. A process with at least one `.register` call, given no incoming
   connections at all within the test's timeout, is confirmed still
   running (not exited) — proving the drain-and-join extension, not
   merely assumed from the passing worked example above.
5. Full regression: `cargo test --workspace` passes with every leaf in
   this plan applied together, including plan 54/55/56's own full
   suites re-run unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`build_method_call`'s
  dispatch arm, `define_main`'s drain-and-join extension), `runtime/
  emerald_runtime.c` (`emerald_actor_dispatch`, `network_active` flag)
- **Add:** a new test file under `crates/emerald-cli/tests/` (e.g.
  `distributed_actors.rs`) for the two-real-process integration proof,
  alongside plan 55's existing `actor_concurrency.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Two-process proof | `cargo test -p emerald-cli distributed_actors -- --test-threads=1` | `host` stdout shows `3`; `RemoteActorError` proof; drain-and-join-blocks proof | agent-claimed-locally |
| Full workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
# Real network I/O is inherently timing-sensitive: run the distributed
# suite a second time to catch anything a loaded CI runner's first pass
# happened not to exercise (port-listen polling, connect timeout margin).
cargo test -p emerald-cli distributed_actors -- --test-threads=1
```
