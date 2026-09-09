---
name: Automatic Actor Placement, Discovery, and Unified Failure Semantics
overview: "Node discovery becomes a single `EMERALD_PEERS`/`EMERALD_SELF` environment-variable read (no network protocol implemented); actor placement becomes Orleans-style — a logical string key, consistent-hashed over the discovered peer set, lazily activates a fresh instance on whichever node owns it, with a new minimal heartbeat driving reassignment on node death; and every cross-actor send, local or remote, changes from today's shipped fire-and-forget-or-exception call shape to a uniform `Result[Void, SendError]`, making distributed failure a value in the type system instead of a network protocol pretending to be a local call."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-automatic-discovery
    content: "EMERALD_PEERS (comma-separated host:port list) and EMERALD_SELF (which entry is this process), read once at startup — the smallest genuinely zero-implementor-code discovery mechanism, mirroring the EMERALD_WORKERS/EMERALD_REMOTE_TIMEOUT_MS getenv precedent already shipped in runtime/emerald_runtime.c; DNS-SRV, Kubernetes API discovery, and gossip membership explicitly declined as real, larger, future work"
    status: pending
  - id: leaf-virtual-actor-placement
    content: "Orleans-style on-demand virtual actors: a logical string key consistent-hashed (FNV-1a ring, small virtual-node count) over the EMERALD_PEERS set decides the owning node; ClassName.locate(key, args...) lazily spawns (reusing build_spawn_alloc) and caches on the owner, or resolves-and-routes remotely by extending the existing RESOLVE handshake; a new EMERALD_HEARTBEAT_INTERVAL_MS liveness prober marks a peer dead after missed heartbeats, at which point the ring naturally reassigns the key to a live peer, which activates a FRESH instance — state does not survive reactivation, exactly matching plan 57's real captured-spawn-args restart semantics"
    status: pending
  - id: leaf-unified-fallible-send
    content: "Every cross-actor send (local, via plan 55's emerald_actor_enqueue; remote, via plan 60's emerald_actor_dispatch) returns Result[Void, SendError] instead of today's shipped Void-typed, exception-on-remote-failure call shape; SendError { ActorTerminated, Timeout, NodeUnreachable } is a compiler-synthesized plan-52 enum, mirroring the existing ensure_remote_actor_error_class precedent; build_method_call's cross-actor dispatch arm (crates/emerald-codegen/src/lib.rs, the code presently ending in `Ok((i64_ty.const_int(0, false).into(), ValKind::Void))` at the real, shipped line 7171) is changed in place — additive at the source-program level (existing statement-position sends still compile and run unchanged, since Stmt::Expr already discards its value), breaking at the codegen-internal call-shape level (the expression's static type changes from Void to Result[Void, SendError])"
    status: pending
isProject: false
---

# Plan 65 — Automatic Actor Placement, Discovery, and Unified Failure Semantics

This plan is a direct follow-up refinement to plan 60
(`2026-09-09T134000Z-plan-60-distributed-actors.md`), authored after a
debate about whether Emerald's distributed-actor story should pursue
full location transparency — indistinguishable local and remote calls,
no visible failure path — or something narrower. It is **not** part of
the 58–64 batch's original authoring pass: those seven plans (generic
types, C FFI, distributed actors, comptime execution, design-by-
contract, purity annotations, WASM codegen) were authored together as
that batch's theoretical-ceiling exploration, and plan 60 was the
member of that batch this plan builds on. This plan is post-v1 scope,
same posture as every plan since 17: it does not touch
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) or
any other plan file — updating that table to reflect the now-larger
plan set is a separate, pending task this plan does not attempt.

**A factual correction this plan's own verification surfaced, stated
plainly because the alternative is silently working from a stale
premise:** at authoring time, `git log --oneline -- runtime/
emerald_runtime.c` and `git merge-base --is-ancestor` both confirm
current `HEAD` is commit `851540e` ("feat(distributed-actors):
location-transparent actors over real TCP (plan 60)"), with plans 58
(generics, `42c4bf9`) and 59 (C FFI, `9914200`) also real ancestors of
`HEAD`. Plan 60 is not an unimplemented proposal here — it is real,
shipped, committed code, verified directly this session against
`runtime/emerald_runtime.c` (the `EmeraldActorRef` struct at line 673,
`emerald_actor_register` at line 1612, `emerald_actor_ref_remote` at
line 1658, `emerald_actor_dispatch` at line 1714) and against
`crates/emerald-codegen/src/lib.rs`'s real cross-actor dispatch codegen
(the `dispatch_call`/`build_raise_on_remote_send_failure` pair spanning
lines 7146–7246) and its own real test,
`crates/emerald-cli/tests/distributed_actors.rs`, which launches two
genuinely separate OS processes over a real socket. This plan therefore
verifies **every** one of its dependencies — plans 52, 53, 55, 57, and
60 alike — against real, currently-committed source, not against any
plan document's original proposal where the two differ; each citation
below names the exact file and, where it matters, the exact line.

Concrete proof this plan targets, three parts:

**(a) Discovery and placement, real multi-process, real sockets.** Three
compiled Emerald processes — `node_a`, `node_b`, `node_c` — are launched
with an identical `EMERALD_PEERS="127.0.0.1:9001,127.0.0.1:9002,127.0.0.1:9003"`
and each its own `EMERALD_SELF` (`127.0.0.1:9001`/`9002`/`9003`
respectively). A fourth `client` process, given the same `EMERALD_PEERS`
(no `EMERALD_SELF`), calls `Counter.locate("shard-7", 0)` and sends three
`increment` messages followed by `report`, all addressed only by the
logical key `"shard-7"` — the client never names a node. The consistent
hash of `"shard-7"` lands on (say) `node_b`; `node_b`'s own stdout,
captured by the test harness exactly as plan 60's own worked example
already does, prints `3`. Neither `node_a` nor `node_c` ever activates a
`Counter` for that key.

**(b) Reactivation on node failure, fresh state, no crash.** The test
kills `node_b` outright (`SIGKILL`, not a graceful shutdown — plan 60
already established there is no remote-shutdown protocol to invoke
cleanly). The client sends one more `increment` to `"shard-7"`. The
liveness prober (this plan's own addition — see `leaf-virtual-actor-
placement`) marks `node_b` dead within one heartbeat interval; the same
consistent-hash computation, run independently by the client and by
every surviving peer, now names `node_a` or `node_c` as `"shard-7"`'s
new owner; that peer lazily activates a **fresh** `Counter` from
`Counter.locate`'s originally captured spawn argument (`0`), not
`node_b`'s last-known count of `3`. The send succeeds — `Result[Void,
SendError]`'s `Ok(Void)` — and a subsequent `report` prints `1`, not
`4`: real, visible proof the reactivated instance's state reset, not a
seamless failover.

**(c) All peers unreachable — a value, not a crash.** A `client` process
is launched with `EMERALD_PEERS` pointing at three ports nothing is
listening on. `Counter.locate("shard-1", 0).increment` is called inside
a `case result when Ok(v) ... when Err(e) ...` (plan 53's real dedicated
match form). The process does not crash, hang, or exit non-zero; it
prints the caught `SendError` variant's name, `NodeUnreachable`, and
exits `0`.

## Decision log

- **The debate's core finding, restated as this plan's own design
  philosophy, not background color.** Full transparency — a distributed
  call site that is lexically and semantically indistinguishable from a
  local one, with no way for a caller to observe or handle a network
  failure differently from a bug — is a proven historical mistake.
  Waldo, Wollrath, Wyant, and Kendall's 1994 Sun Microsystems paper "A
  Note on Distributed Computing" is the canonical statement of why: it
  argues, against the object-oriented distributed-systems orthodoxy of
  its era (CORBA, DCOM, and — a few years later — Java RMI all pursued
  exactly this transparency), that a local call and a remote call are
  irreducibly different in latency, partial-failure modes, concurrency
  semantics, and memory-access cost, and that hiding this distinction
  behind a uniform syntax does not eliminate the difference — it only
  defers the moment a caller discovers it, usually in production, as an
  unhandled exception or a silent hang from a call site that looks
  exactly like every other call site around it. That paper is
  substantially why RMI-style "transparent distributed objects" lost to
  explicit-RPC and explicit-message architectures over the following
  decade. Erlang took the opposite lesson deliberately, not by
  accident: Joe Armstrong's own stated design goal (both in his 2003
  PhD thesis and repeated consistently afterward) was **syntactic
  uniformity for the send operation** — `Pid ! Message` looks identical
  whether `Pid` names a local or remote process — **paired with
  explicit, first-class failure signaling** (`{'EXIT', Pid, Reason}`
  messages, `monitor`/`link`, and the deliberate refusal to make a send
  itself block or silently swallow a partner's death). Armstrong hid
  the *addressing*, never the *failure* — that split is the exact one
  this plan draws. What this plan automates — discovery (`leaf-
  automatic-discovery`), placement (`leaf-virtual-actor-placement`), and
  wire serialization (already solved; see below) — is genuinely pure
  plumbing with no failure mode a caller needs to reason about
  differently by hand. What this plan makes visible — exactly one
  thing, `Result[Void, SendError]` (`leaf-unified-fallible-send`) — is
  the one property Waldo et al. say can never be safely hidden: a send
  can fail, and the caller decides what that means for its own program,
  the same way Armstrong's Erlang never pretended a `!` was guaranteed
  delivery.
- **Serialization is not this plan's problem — it is already solved,
  verified against plan 60's real, shipped `leaf-wire-codec`.** Every
  wire-safe type (plan 60's own Design decision 2: value types, `String`,
  and recursively wire-safe classes) already has a compiler-generated
  `ClassName_encode`/`ClassName_decode` pair and a per-method argument
  encoder (`ctx.actor_arg_encoders`, referenced at
  `crates/emerald-codegen/src/lib.rs:7139`), built once per concrete
  class name from the same `ClassLayout` field metadata plan 08/32
  already compute. This plan's placement and discovery machinery reuses
  that codec unchanged — a logical key's payload is exactly the wire-
  safe argument list `ClassName.locate` was already given, encoded by
  the identical per-method encoder an ordinary `.remote(...)` send
  already uses. Nothing here proposes a new serialization mechanism.
- **What is declined, stated as explicitly as what is built.** This
  plan does not attempt: making `.spawn`/`.locate`/a send look like an
  infallible local call (the entire point above); distributed
  transactions or any cross-node consistency guarantee beyond "exactly
  one node believes it owns a key at a time, best-effort, no split-
  brain detection" (a real, disclosed gap — two peers whose liveness
  tables have drifted, e.g. during a heartbeat-interval window right
  after a node both looks dead to one peer and alive to another, could
  transiently both believe they own the same key; this plan does not
  add a fencing token, lease, or quorum mechanism to close that window,
  the same "real, disclosed limitation, not a hidden one" standard plan
  60 itself already used for schema drift); full gossip-based cluster
  membership (peers here never learn about each other except through
  the identical `EMERALD_PEERS` string every process is separately
  started with — no peer ever tells another peer about a fourth node it
  discovered); and state persistence across reactivation (`leaf-
  virtual-actor-placement`'s own worked-example part (b) is the
  concrete proof this is a real v1 ceiling, not an oversight).
- **Leaf 1's scope: the smallest thing that is still genuinely zero-
  implementor-code.** `EMERALD_PEERS` is an environment variable, read
  once via `getenv` at process startup — not a network protocol,
  identical in kind to the two precedents already shipped in `runtime/
  emerald_runtime.c`: `EMERALD_WORKERS` (`emerald_worker_pool_start`,
  line 885) and `EMERALD_REMOTE_TIMEOUT_MS` (`emerald_remote_timeout_ms`,
  line 1295, whose own doc comment already says "cheap enough (`getenv`
  + `atol`) not to need caching" — this plan's `EMERALD_PEERS` parser
  follows the identical no-caching, read-once-at-the-call-site
  convention). DNS-SRV records, a Kubernetes API/downward-API-based
  discovery client, and a gossip protocol (SWIM, or BEAM's own
  distributed `net_kernel`) are explicitly declined here, each named
  individually because each is a real, substantially larger mechanism
  in its own right — DNS-SRV needs a resolver and a TTL/refresh policy,
  Kubernetes API discovery needs a client, auth, and a watch loop,
  gossip needs its own failure-detector and membership-convergence
  protocol — not a smaller version of what this leaf builds, a
  different thing this leaf declines to build. `EMERALD_SELF` is this
  leaf's one necessary addition beyond the task's own literal ask: a
  process told only "these are my peers" has no way to know which
  peer *it is* (its own `.register` call only knows the bare port it
  was given, not its own externally-reachable host string) — `leaf-
  automatic-discovery`'s acceptance criteria require both variables,
  and states this addition and its reason explicitly rather than
  silently expanding scope.
- **Leaf 2's placement algorithm: a real, simple, single-ring consistent
  hash — not Orleans' own directory/coordinator, and that gap is named.**
  Every process independently computes the identical ring from the
  identical `EMERALD_PEERS` string (each peer contributes a small fixed
  number of virtual points, `FNV-1a("host:port#i")` for `i` in `0..4`,
  smoothing distribution across a small peer set); a logical key's owner
  is the ring's first virtual point at or after `FNV-1a(key)`, wrapping
  around — the standard consistent-hashing successor rule, computed
  identically and independently by every process since every process
  read the same peer list. This is real Orleans-precedent behavior (on-
  demand activation, a deterministic placement function, no caller-
  chosen node) but it is explicitly **not** Orleans' own directory: real
  Orleans uses a distributed, replicated directory of grain-to-silo
  activations with its own consistency protocol so that ring
  membership *changes* (a silo joining or leaving) are coordinated and
  agreed on before activations move; this plan's ring is recomputed
  independently, locally, per-process, from whatever `EMERALD_PEERS`
  plus this leaf's own liveness table says *right now* — a real,
  smaller, disclosed simplification, not a re-implementation of
  Orleans' coordinator.
- **Leaf 2's heartbeat: added here because plan 60 explicitly declined
  it, verified against its own Decision log.** Plan 60's Design decision
  5 states this precisely: "no automatic reconnection, no supervised
  restart of the remote side, and — the concrete, disclosed reason a
  hang and a crash are not distinguishable in v1 — **no heartbeat/
  liveness protocol**: a genuinely wedged … peer … is only detected once
  a send is attempted." Reassignment in this plan depends on knowing a
  peer is dead *before* the next send to one of its keys, not only
  reactively at send time — a purely reactive design would route every
  first post-crash send to the dead owner, wait out
  `EMERALD_REMOTE_TIMEOUT_MS`, get a socket error, and only then know to
  recompute the ring, adding real, avoidable latency to the exact
  failure path the worked example's part (b) is meant to demonstrate
  working cleanly. `leaf-virtual-actor-placement` therefore adds exactly
  one small, disclosed piece of new runtime scope beyond plan 60's own:
  a background prober thread per process (started the same way
  `emerald_accept_main` already is, via `pthread_create`+
  `pthread_detach`, in `emerald_actor_register`), one lightweight
  connect-and-close probe per known peer every
  `EMERALD_HEARTBEAT_INTERVAL_MS` (a third env var, mirroring
  `EMERALD_WORKERS`/`EMERALD_REMOTE_TIMEOUT_MS`'s own precedent), marking
  a peer dead in a process-local liveness table after a small, disclosed
  consecutive-miss threshold (three). This is a per-process local
  failure detector, not a distributed consensus protocol — two
  processes can disagree about a third peer's liveness for up to a few
  heartbeat intervals, the exact split-brain-adjacent gap named above.
- **Leaf 2's reactivation semantics are plan 57's own real local-restart
  semantics, reused exactly, not a new mechanism invented for the
  distributed case.** Verified against `crates/emerald-driver/tests/
  supervisor_runtime.c`'s own `worker_respawn` (its "supervisor" mode):
  a supervised child's restart calls the identical respawn function with
  the identical originally-captured `args` array, allocating a
  brand-new arena via `new_actor`/`build_spawn_alloc` and re-running
  `initialize` from scratch — proven by that file's own real assertion
  that a post-restart `Worker` send prints `1`, not the pre-crash count
  of `3` (`crates/emerald-driver/tests/supervisor_runtime.rs`'s
  `one_for_one_supervisor_worked_example`, asserting the post-restart
  line is `"1"`, not `"4"`). A key's reactivation on a new owning peer
  is the identical pattern one level up: the new owner has never seen
  this key's prior state (it lives, if anywhere, in the dead process's
  own now-unreachable memory), so it can only ever re-run
  `Counter.locate`'s originally-supplied constructor argument, `0` — the
  same "fresh instance from captured spawn args, old state gone" outcome
  plan 57 already established locally, now crossing a process boundary
  for the first time. This plan states plainly that this is *not*
  seamless failover in the sense a stateful system with a replicated
  log or a storage-provider layer (Orleans' own real persistence
  providers, or Akka Cluster Sharding's event-sourced entities) would
  give — Emerald has neither, and building either is real, separate,
  larger future work this plan does not attempt.
- **Leaf 3's honest friction: this changes plan 55's and plan 60's own
  real, already-shipped call shape, and this plan decides explicitly
  that it is allowed to.** Verified directly against the real, current
  source: `crates/emerald-codegen/src/lib.rs`'s cross-actor dispatch
  arm (the code path every `.spawn`-based local send and every
  `.remote`-based send compiles through today, per plan 60's own Design
  decision 1b unification) ends, at line 7171, with
  `Ok((i64_ty.const_int(0, false).into(), ValKind::Void))` —
  unconditionally `Void`, regardless of outcome. Failure is signaled two
  different ways today, neither of them `Result`: a local send to a
  dead actor's `EmeraldActorHeader` is silently dropped (verified via
  `supervisor_runtime.c`'s own AC4, `"dead send did not crash"` — no
  signal reaches the sender at all), and a remote send's socket failure
  is raised as a catchable `RemoteActorError` exception via
  `build_raise_on_remote_send_failure` (lines 7184–7246), reusing plan
  11/38's `setjmp`/`longjmp` handler stack. At the runtime layer,
  `emerald_actor_dispatch` itself (`runtime/emerald_runtime.c:1714`)
  makes this concrete: its local branch is `if (!ref->is_remote) {
  emerald_actor_enqueue(...); return 0; }` — hardcoded to report success,
  architecturally incapable of reporting a local failure at all, today.
  This plan's own explicit decision: `leaf-unified-fallible-send` is
  allowed to change this shipped call shape's *type* (every cross-actor
  send's static type moves from `Void` to `Result[Void, SendError]`,
  and `RemoteActorError`'s raise-on-failure path is replaced with an
  `Err(SendError::NodeUnreachable())`/`Err(SendError::Timeout())` value
  construction at the same two sites), justified because: (1) it is
  **additive, not breaking, for every existing compiled `.em` source
  program** — verified this session that no test or example anywhere in
  the tree assigns a cross-actor send's result to a typed local (`grep`
  over `crates/emerald-cli/tests/*.rs` and `examples/` found zero `:
  Void`-typed bindings and every actor send in
  `actor_concurrency.rs`/`distributed_actors.rs` used in bare statement
  position); `Stmt::Expr` already discards whatever `build_expr`
  returns for any expression used as a statement, so an unmodified
  program that only ever fire-and-forgets a send keeps compiling and
  running identically, now silently discarding an `Ok(Void)`/`Err(...)`
  value it never asked to see, the same way it already silently
  discards today's `Void`; (2) it **is** breaking at the codegen-
  internal and type-system level — any hypothetical future source line
  that captured a send's result under the old `Void` typing would need
  to change, and this plan states that cost honestly rather than
  omitting it because no current test happens to exercise it. Migration
  is therefore scoped as: this leaf **may** touch plan 55's and plan
  60's shipped codegen (the dispatch-arm's return construction and
  `build_raise_on_remote_send_failure`'s body) directly, in place — it
  is not required to preserve a parallel old code path, because the
  additive/breaking analysis above shows nothing real depends on the
  old `Void` typing surviving.
- **`SendError` is a plan-52 enum, verified against its real, shipped
  grammar — not a plan-53 `Result`-style hardcoded type, and not
  reusing `RemoteActorError`.** `crates/emerald-parser/src/grammar.
  lalrpop`'s real `EnumVariant` rule (line 261) is `<name:Ident> "("
  <fields:TypeNameList> ")"` — parentheses are mandatory even for a
  zero-field variant, since `TypeNameList` (line 267) already permits
  zero elements (`<mut v:(<TypeName> ",")*> <last:TypeName?>`, both
  parts optional). `SendError`'s three variants are therefore spelled
  `ActorTerminated()`, `Timeout()`, `NodeUnreachable()` — a real,
  disclosed syntactic wart of an otherwise-zero-payload enum, not a
  hypothetical one. `SendError` is compiler-synthesized and inserted
  once per compilation unit that declares at least one `actor`,
  following the exact `ensure_remote_actor_error_class`/
  `remote_actor_error_class_item` idiom already shipped
  (`crates/emerald-codegen/src/lib.rs:13163`/`13211`: build an
  `Item::Enum` value directly in Rust, gate insertion on
  `items.iter().any(...)` so a program with no actors pays nothing) —
  `SendError` is never something Emerald source declares by hand, and a
  user cannot construct or subclass it (plan 52 enums are closed by
  construction, matching this plan's own "no dynamic dispatch, no open
  extension" identity constraint). It is deliberately **not**
  `RemoteActorError`: that class remains exactly as shipped, still
  raised by `.remote(addr, name)`'s own connect-time failure (a
  different moment — before any `EmeraldActorRef` exists to return a
  `Result` through — this plan does not touch `.remote`'s connect path
  at all, only the post-connection send path and `.locate`'s own new
  call form) — the two failure-signaling mechanisms now coexist for two
  genuinely different moments, the identical "orthogonal, not
  redundant" posture plan 53's own Decision log already used to justify
  `Result[T,E]` and `raise`/`rescue` coexisting.
- **`Result[Void, SendError]`'s `T=Void` is a real, disclosed edge case
  in plan 53's own shipped construction, resolved by reusing an
  existing codegen idiom rather than inventing a new one.** Verified
  against plan 53's real `leaf-codegen-result-construction`: `Expr::Ok`/
  `Expr::Err` codegen "store the built inner value at offset 8," which
  presumes a real, built expression producing a concrete `ValKind` — and
  separately, `build_method_call`'s own cross-actor-argument encoder
  (line 7112) already explicitly rejects `ValKind::Void` as "not a valid
  … kind" wherever a real value is expected. This plan does not add
  user-facing `Ok(<nothing>)` syntax to resolve this — no test or
  example anywhere needs a *caller* to write `Ok(void_expr)` by hand,
  since `T=Void`'s `Ok`/`Err` values are only ever constructed by
  codegen itself, at the one dispatch-arm call site this leaf owns.
  Codegen already has a precedent for exactly this — synthesizing an AST
  node in Rust and feeding it through the ordinary `build_expr` path
  rather than adding new IR: `build_raise_on_remote_send_failure`
  itself already does this today (`Spanned::synthetic(Expr::New(
  "RemoteActorError".to_string(), …))`, line 7228). This leaf's
  replacement code follows the identical pattern — synthesize
  `Spanned::synthetic(Expr::Ok(Box::new(Spanned::synthetic(Expr::Int(0)))))`
  (a synthetic zero standing in for `Void`'s payload, stored and never
  read back, the same "dummy i64 0" convention `i64_ty.const_int(0,
  false)` already uses for every other `Void`-typed value in this exact
  function) on the success path, and `Spanned::synthetic(Expr::Err(
  Box::new(Spanned::synthetic(Expr::New("SendError".to_string(),
  vec![Spanned::synthetic(Expr::Call("ActorTerminated".to_string(),
  vec![]))])))))`-shaped construction (exact call form to be finalized
  against plan 52's real variant-construction codegen, `crates/
  emerald-codegen/src/lib.rs:10334`'s `build_enum_case`'s construction-
  side counterpart) on each failure path — no new source-level grammar,
  no new sema rule beyond typing the call expression itself.

## Leaf: leaf-automatic-discovery

### 1. Context
- Why: `leaf-virtual-actor-placement` and `leaf-unified-fallible-send`
  both need a discovered peer set to hash over and a liveness table to
  hash *against*; plan 60 itself only ever addresses one hardcoded
  `host:port` string literal per `.remote(...)` call — there is no
  notion of "the peer set" anywhere in real, shipped source today
  (verified: no `EMERALD_PEERS`-shaped constant or `getenv` call exists
  in `runtime/emerald_runtime.c` as of `HEAD`).
- Target state: `runtime/emerald_runtime.c` gains
  `emerald_discover_peers(void)`, called once, lazily, the first time
  any of this plan's new entry points (`.locate`, the heartbeat
  prober) needs the peer set — mirroring `emerald_remote_timeout_ms`'s
  own "cheap enough not to need caching, but naturally called at most a
  few times per process" posture. It reads `EMERALD_PEERS` (comma-
  separated `host:port`, parsed with the same `emerald_parse_host_port`
  splitter plan 60 already shipped at `runtime/emerald_runtime.c:1391`,
  reused unchanged per entry) into a fixed-size array (mirroring
  `EMERALD_MAX_REGISTERED_ACTORS`'s own `#define`-a-small-cap
  convention — `EMERALD_MAX_PEERS`, e.g. 32), and reads `EMERALD_SELF`
  (one `host:port`) to identify which parsed entry is this process. An
  unset `EMERALD_PEERS` is a real, valid single-process configuration
  (`leaf-virtual-actor-placement`'s ring degenerates to one node, always
  itself); an `EMERALD_SELF` that doesn't match any entry in
  `EMERALD_PEERS` is a startup-time fatal error with a real, named
  message (mirroring `emerald_tcp_listen`'s own `emerald_set_remote_
  error`-then-return-failure convention, surfaced the first time
  `.locate` or `.register` is called), not a silent fallback.

### 2. Acceptance Criteria
1. `EMERALD_PEERS="127.0.0.1:9001,127.0.0.1:9002,127.0.0.1:9003"` plus
   `EMERALD_SELF="127.0.0.1:9002"`, read by a real compiled-and-run
   process, yields a parsed 3-element peer list with the self-index
   correctly identified as index 1 — proven via a small runtime-level
   test harness mirroring `scheduler_runtime.c`'s own hand-built-C-
   harness pattern (a new `tests/discovery_runtime.c`/`.rs` pair in
   `crates/emerald-driver`), not requiring full codegen/LLVM plumbing
   to prove the parser itself.
2. A malformed peer entry (missing `:port`, non-numeric port) is a real,
   named startup error, not a crash or silent skip — same "real
   diagnostic, not a panic" standard `emerald_parse_host_port`'s
   existing local caller (`emerald_actor_ref_remote`) already uses for
   `.remote`'s own address string.
3. `EMERALD_SELF` unset or not present in `EMERALD_PEERS` is a real,
   named startup error, surfaced the first time discovery is actually
   needed (not eagerly at process start for a program that never calls
   `.locate`/`.register` — mirrors plan 45's own lazy-`ARGV`-parse
   posture, paying nothing for programs that don't use this feature).
4. Regression: a program using only plan 60's existing `.spawn`/
   `.register`/`.remote` forms, with `EMERALD_PEERS`/`EMERALD_SELF`
   unset entirely, compiles and runs identically to `HEAD` — this leaf
   adds a new, unused-by-default code path, it does not change
   `.register`/`.remote`'s existing behavior at all.

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c` (new `emerald_discover_peers`,
  `EMERALD_MAX_PEERS`, `EmeraldPeer` struct)
- **Add:** `crates/emerald-driver/tests/discovery_runtime.c`,
  `crates/emerald-driver/tests/discovery_runtime.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Runtime harness build | `cc -O0 -g -pthread runtime/emerald_runtime.c crates/emerald-driver/tests/discovery_runtime.c -o /tmp/discovery_harness` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver --test discovery_runtime` | all pass, incl. malformed-input and missing-EMERALD_SELF cases | agent-claimed-locally |

---

## Leaf: leaf-virtual-actor-placement

### 1. Context
- Why: plan 60 only ever gives a caller a way to address an actor it
  already knows the exact `host:port` and registered name of
  (`.remote(addr, name)`); nothing decides *for* the caller which node
  should own a logical unit of work, and nothing reacts to a node dying
  by moving that ownership — the entire gap this leaf closes, using
  `leaf-automatic-discovery`'s peer set as its input.
- Target state: a new reserved call form, `ClassName.locate(key,
  args...)` (parallel to `.spawn`/`.remote`'s own reserved-call-form
  precedent — `key` a `String`, `args` the class's real `initialize`
  arguments), compiled by a new codegen path that: (1) computes
  `owner = consistent_hash(key, live_peers)` using
  `leaf-automatic-discovery`'s peer list filtered by this leaf's own
  liveness table; (2) if `owner == self`, looks `key` up in a new
  process-local `HashMap<String, EmeraldActorRef*>` cache — on a miss,
  calls `build_spawn_alloc` with `args` (byte-for-byte the same
  allocation path `.spawn` already uses) and inserts the result before
  returning it; (3) if `owner != self`, builds (or reuses, from a
  second process-local cache keyed by peer address) a remote
  `EmeraldActorRef` to `owner`, RESOLVE-ing by `key` as the registered
  name — extending `emerald_reader_main`'s existing RESOLVE handler
  (`runtime/emerald_runtime.c:1517`) so that a RESOLVE for a name this
  process doesn't yet have registered, but which its own consistent-
  hash computation says it owns, triggers the identical lazy-spawn-and-
  cache-and-register sequence as the local-owner case above, instead of
  today's unconditional `ok=0`. A new background prober thread
  (`emerald_heartbeat_main`, started from the same call site
  `emerald_accept_main` already is) probes every peer in
  `EMERALD_HEARTBEAT_INTERVAL_MS` intervals via a bare `connect()`+
  `close()`, marking a peer dead after three consecutive failures in a
  process-local liveness table `leaf-automatic-discovery`'s peer list is
  filtered through before every `consistent_hash` call.

### 2. Acceptance Criteria
1. Worked-example part (a): three `node_*` processes plus one `client`,
   real compiled binaries, real sockets, identical `EMERALD_PEERS`; the
   client's three `Counter.locate("shard-7", 0)`-addressed sends and one
   `report` land on exactly one of the three nodes (whichever the
   consistent hash names), which alone prints `3`; the other two nodes
   never activate a `Counter` for that key at all (proven by an
   instrumented build that also prints an activation-count line,
   asserted `0` on the two non-owners).
2. Worked-example part (b): killing the owning node (`SIGKILL`) and
   sending one more `increment` to the same key succeeds — `Ok(Void)` —
   and is served by a **different** node than the one killed, whose
   subsequent `report` prints `1`, not `4` — real, executed proof of
   both reassignment and the fresh-state reset `leaf-unified-fallible-
   send` makes newly visible as a real return value rather than
   something only observable by reading `node_b`'s now-unreachable
   stdout.
3. Consistent-hash stability: adding a fourth, never-contacted peer to
   `EMERALD_PEERS` (present in the env var, but never started as a real
   process, so always liveness-dead) does not change which of the three
   real nodes owns `"shard-7"` — proof the ring computation is genuinely
   deterministic per-live-peer-set, not merely per-full-peer-list.
4. Heartbeat timing: with `EMERALD_HEARTBEAT_INTERVAL_MS=50`, a killed
   peer is marked dead (observable via the next `.locate` on one of its
   keys routing elsewhere) within a small, bounded number of intervals
   (asserted `< 500ms` wall-clock from kill to reroute) — real, timed
   proof the prober runs on its own schedule, not only reactively on
   send failure.
5. Regression: `leaf-automatic-discovery`'s own quality gate, plan 60's
   full existing `crates/emerald-cli/tests/distributed_actors.rs` suite,
   and plan 57's full `supervisor_runtime.rs` suite all still pass
   unmodified — this leaf adds a new call form and a new background
   thread, it does not change `.spawn`/`.register`/`.remote`/
   supervision's existing behavior.

### 3. File & Module Structure
- **Modify:** `runtime/emerald_runtime.c` (`emerald_reader_main`'s
  RESOLVE-miss path, new `emerald_heartbeat_main`/liveness table, new
  `emerald_consistent_hash_owner`), `crates/emerald-parser/src/
  grammar.lalrpop` and `ast.rs` (`.locate` reserved call form),
  `crates/emerald-sema/src/lib.rs` (`.locate` arity/type checking
  against the class's real `initialize` signature, mirroring `.spawn`'s
  existing check), `crates/emerald-codegen/src/lib.rs` (new
  `build_locate_call`, sibling to `build_spawn_alloc`/`build_method_
  call`'s existing `.register`/`.remote` codegen)
- **Add:** `crates/emerald-cli/tests/virtual_actor_placement.rs` (the
  four-process worked-example proof)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Runtime harness | `cargo test -p emerald-driver` | all pass, incl. any new hand-built liveness/hash harness | agent-claimed-locally |
| Multi-process worked example | `cargo test -p emerald-cli --test virtual_actor_placement` | all 5 ACs above pass, real processes/sockets | agent-claimed-locally |

---

## Leaf: leaf-unified-fallible-send

### 1. Context
- Why: today, per the Decision log's verified citations, a cross-actor
  send's real compiled type is unconditionally `Void` — a local send to
  a dead actor is silently dropped with no signal at all, and a remote
  send's failure is a raised, catchable `RemoteActorError` exception,
  not a value. Neither shape lets a caller treat "this send may fail"
  as an ordinary, checkable fact about the call — exactly the gap the
  debate concluded should be the *one* thing made visible.
- Target state: `crates/emerald-codegen/src/lib.rs`'s cross-actor
  dispatch arm (the code presently spanning lines 7146–7246,
  `build_method_call`'s `dispatch_call`/`build_raise_on_remote_send_
  failure` pair) is rewritten so the arm returns a real `Result[Void,
  SendError]` IR value instead of always `(0, ValKind::Void)`: the
  success path constructs a synthetic `Expr::Ok(<dummy Void payload>)`
  (see Decision log) and feeds it through plan 53's existing `Expr::Ok`
  `build_expr` arm; each real failure path — `emerald_actor_dispatch`
  returning non-zero on the remote branch, and (this leaf's own new
  runtime addition) `emerald_actor_enqueue` gaining a real dead-actor
  check on the local branch so `emerald_actor_dispatch`'s local branch
  can finally return non-zero too, closing the "hardcoded `return 0`"
  gap named in the Decision log — constructs the matching synthetic
  `Expr::Err(SendError::ActorTerminated())` / `Err(SendError::
  NodeUnreachable())` / `Err(SendError::Timeout())` (the last two
  distinguished by `emerald_actor_dispatch`'s remote branch returning a
  richer status code than today's plain `0`/`-1`, since `Timeout` and
  `NodeUnreachable` are observably different outcomes at the socket
  layer — a connect-time or `SO_RCVTIMEO`-expired failure vs. a hard
  `ECONNREFUSED`/`EPIPE` — and this leaf's own worked-example part (c)
  needs `NodeUnreachable` specifically, not a merged catch-all).
  Method-call type inference for a cross-actor send (the sema arm that
  currently assigns every such call the type `Void`, shared unmodified
  by plan 55/56/60 alike per Design decision 1b's unification) is
  changed to assign `Type::Result(Box::new(Type::Void),
  Box::new(Type::Enum("SendError".to_string())))`.

### 2. Acceptance Criteria
1. A local send to a live actor (`c.increment` after `c: Counter =
   Counter.spawn(0)`), compiled, linked, and run, still behaves exactly
   as today when used as a bare statement (`Stmt::Expr` discards the
   `Ok(Void)` it now produces) — real regression proof this leaf is
   additive at the source level, matching the Decision log's own claim.
2. `result: Result[Void, SendError] = c.increment` (capturing the send's
   result explicitly, new source-level syntax this leaf makes possible
   for the first time) followed by `case result when Ok(v) ... when
   Err(e) ...`, compiled, linked, and run against a live local actor,
   prints whatever the `Ok(v)` arm prints — real proof the success path
   round-trips through plan 53's real match form, not just that it
   type-checks.
3. A local send to a **terminated** actor (a supervised child that has
   already crashed and not been re-registered, or a bare `.spawn`ed
   actor this leaf's own new dead-actor check recognizes as terminated)
   returns `Err(SendError::ActorTerminated())`, real and caught by a
   `case ... when Err(e) ...` arm, replacing today's silent drop — real,
   executed proof of the "hardcoded `return 0`" gap actually closed at
   the runtime layer, not merely reworded at codegen.
4. `.remote("host:closed-port", name)` used for a send after a
   successful `.remote(...)` connect (i.e., the connection drops between
   connect and a later send — simulated by killing the remote process
   mid-test) returns `Err(SendError::NodeUnreachable())`, not a raised
   exception — real proof `RemoteActorError` is no longer raised on
   this specific path, while `.remote(...)`'s own connect-time failure
   (verified still present: `crates/emerald-cli/tests/
   distributed_actors.rs`'s sibling connect-failure test, if this leaf's
   scope needs a new one written against `.remote` unchanged) still
   raises `RemoteActorError` exactly as before, per the Decision log's
   "two different moments" framing.
5. Worked-example part (c): `EMERALD_PEERS` naming three genuinely
   unreachable addresses, `Counter.locate("shard-1", 0).increment`
   wrapped in `case ... when Ok ... when Err(e) ...`, compiled, linked,
   and run, prints `NodeUnreachable`, exits `0` — no crash, no panic, no
   hang (bounded by `EMERALD_REMOTE_TIMEOUT_MS`, plan 60's own existing
   socket-timeout mechanism, reused unchanged).
6. Regression: `SendError`'s synthesized-enum insertion is gated on the
   program declaring at least one `actor` (mirroring `ensure_remote_
   actor_error_class`'s own gate) — a program with no actors at all
   compiles identically to `HEAD`, paying zero cost for this leaf.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (the dispatch arm at
  lines 7146–7246; new `send_error_enum_item`/`ensure_send_error_enum`
  mirroring `remote_actor_error_class_item`/`ensure_remote_actor_error_
  class`), `crates/emerald-sema/src/lib.rs` (cross-actor send type
  inference), `runtime/emerald_runtime.c` (`emerald_actor_enqueue`'s
  dead-actor check; `emerald_actor_dispatch`'s local branch returning a
  real status; richer remote status codes distinguishing timeout from
  hard connection failure)
- **Modify (regression only, no behavior change intended):**
  `crates/emerald-cli/tests/actor_concurrency.rs`,
  `crates/emerald-cli/tests/distributed_actors.rs`,
  `crates/emerald-driver/tests/supervisor_runtime.rs` (re-run
  unmodified against the new codegen to prove the additive claim)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Unmodified regression suites | `cargo test -p emerald-cli --test actor_concurrency --test distributed_actors && cargo test -p emerald-driver --test supervisor_runtime` | all pass, unmodified, real proof of additivity | agent-claimed-locally |
| New Result-send suite | `cargo test -p emerald-cli --test unified_send_result` | all 6 ACs above pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
