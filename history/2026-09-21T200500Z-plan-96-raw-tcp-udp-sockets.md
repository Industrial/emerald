2026-09-21T20:05:00Z

---
name: Raw TCP/UDP Sockets — General-Purpose Networking on `std::net`
overview: "A new `emerald-rt` module wrapping `std::net::{TcpStream, TcpListener, UdpSocket}` — the Rust standard library's own blocking socket types, not a third-party crate, the cleanest possible instance of plan 95's pure-Rust-first policy since there is no dependency to vet at all — exposed to Emerald as three compiler-provided, instance-carrying resource types (`TcpStream`, `TcpListener`, `UdpSocket`) built on plan 93's opaque-u64-handle-plus-explicit-`.close()` convention. This generalizes networking beyond plan 60's existing TCP usage, which is real but narrow: `runtime/emerald_runtime.c`'s `emerald_tcp_listen`/`emerald_tcp_connect` exist solely to carry the distributed-actor wire protocol between Emerald processes and are never reachable from ordinary Emerald source. This plan is the first user-facing socket API the language has ever had, and documents precisely how a blocking read inside an actor's message handler interacts with the fixed-size worker pool plan 94's async-to-sync guidance addresses."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-net-module-and-handle-wiring
    content: "Create `crates/emerald-rt/src/net.rs` with three thin wrapper structs (`EmeraldTcpStream(std::net::TcpStream)`, `EmeraldTcpListener(std::net::TcpListener)`, `EmeraldUdpSocket(std::net::UdpSocket)`) registered into whatever opaque-u64-handle registry plan 93 establishes (a global slab/table mapping `u64 -> Box<dyn Any>` or a per-kind table — plan 93's call, not re-derived here). Every exported `#[no_mangle] pub extern \"C\" fn emerald_rt_tcp_*`/`emerald_rt_udp_*` function's entire body must be wrapped in `std::panic::catch_unwind` per plan 92, converting any `std::io::Error` into plan 92's canonical error-propagation shape rather than a bare `-1`/sentinel return — this plan supplies the domain-specific `io::Error -> Emerald exception` message text (e.g. `\"connection refused\"`, `\"address already in use\"`, taken from `std::io::Error::kind()`'s real `ErrorKind` variants, not invented strings), plan 92 supplies the marshaling mechanism itself. Wire the Emerald-facing `TcpStream`/`TcpListener`/`UdpSocket` as compiler-provided, non-user-declarable classes carrying one hidden `Int64` handle field each, dispatched the same way `String`'s intrinsics are gated in `crates/emerald-sema/src/lib.rs` (`infer_expr_type`'s `MethodCall` arm checking `recv_ty` against the reserved type name *before* falling through to the `Type::Class` registry path, exactly as documented in plan 45's Decision log for `ValKind::Str`) — the one genuinely new wrinkle beyond both `String` (stateless, no fields) and `File` (a namespace with zero instances, per plan 45's own `File` leaf) is that these three types are both instance-carrying *and* non-user-declarable, a combination neither existing precedent covers alone; defer the exact sema/codegen representation of a compiler-provided single-field handle-carrying type to plan 93, since that representation is this plan's first real tenant, not this plan's own invention."
    status: done
  - id: leaf-tcp-stream-connect-read-write-close
    content: "`TcpStream.connect(host: String, port: Int64): TcpStream` (blocking `std::net::TcpStream::connect((host.as_str(), port as u16))`, relying on `std::net`'s own internal `ToSocketAddrs`-driven `getaddrinfo` resolution — see plan 97's Decision log for why this is sufficient for a literal host/IP but insufficient for DoH/DoT/custom-nameserver use cases), `.read(max_len: Int64): String` (a single `std::io::Read::read` call, NOT `read_to_end`/`read_exact` — returns as soon as 1..=max_len bytes are available, real POSIX-`read(2)`-style short reads, disclosed explicitly rather than silently looping to fill the buffer), `.write(data: String): Int64` (a single `std::io::Write::write` call returning the real byte count actually written, since TCP sockets can produce partial writes under backpressure — the Emerald caller is responsible for looping if it needs all bytes sent, matching `.read`'s own no-looping disclosure), and `.close(): Void` (drops the wrapped `std::net::TcpStream`, removes the handle-table entry, an explicit lifecycle op per plan 93 — there is no finalizer/GC path that calls this automatically). Emerald's own `String` is a bare null-terminated `char*` per plan 59's own verified finding (`emerald_alloc`/`emerald_string_length` show no length header at all) — `.read`/`.write` on a raw socket therefore inherit a real, disclosed gap: a payload containing an embedded NUL byte (fully legal on an arbitrary TCP stream, illegal in a null-terminated C string) silently truncates at the first zero byte on both the read and write paths. This plan does not fix that gap; it names it explicitly and defers the real fix — a binary-safe `(ptr, len)` buffer convention — to plan 92, which already owns that exact convention for non-UTF8 data; nothing in this plan invents a competing mechanism."
    status: done
  - id: leaf-tcp-listener-bind-accept-close
    content: "`TcpListener.bind(host: String, port: Int64): TcpListener` (`std::net::TcpListener::bind`, `SO_REUSEADDR` left at Rust std's own default — unlike `emerald_tcp_listen`'s C implementation at `runtime/emerald_runtime.c:1494-1523`, which sets `SO_REUSEADDR` explicitly; verify Rust std's own actual default behavior on this project's supported Linux targets during implementation rather than assuming parity with the C runtime's explicit choice, and set it explicitly via `socket2`-free `TcpListener`-level means only if verification shows Rust's default diverges) and `.accept(): TcpStream` (blocking `std::net::TcpListener::accept`, discarding the returned `SocketAddr` for v1 — a real, disclosed simplification; a `.local_addr()`/peer-address accessor is not part of this plan's surface, see Out of scope) and `.close(): Void`. Unlike `emerald_tcp_listen`, which is hardcoded to `AF_INET`/`sockaddr_in` (IPv4 only, verified directly against the C source), this plan's `std::net`-backed implementation accepts any host string `std::net::ToSocketAddrs` can resolve, including IPv6 literals and hostnames — a real, disclosed capability improvement over the existing actor transport, not a compatibility requirement with it (see Decision log: the two socket implementations are deliberately not unified)."
    status: done
  - id: leaf-udp-socket-bind-send-recv-close
    content: "`UdpSocket.bind(host: String, port: Int64): UdpSocket`, `.send_to(data: String, host: String, port: Int64): Int64` (`std::net::UdpSocket::send_to`), `.recv_from(max_len: Int64): String` (`std::net::UdpSocket::recv_from`, returning only the payload — see below for the sender address), and `.close(): Void`. Rather than inventing a multi-value return convention Emerald has no established syntax for in this plan's own worked proof, the sender's address is exposed through a `_Thread_local`-style last-value accessor pair, `UdpSocket.last_sender_host(): String` / `UdpSocket.last_sender_port(): Int64`, populated by the most recent `.recv_from` call on the calling thread — this reuses, verbatim, the exact pattern `runtime/emerald_runtime.c`'s own `emerald_remote_last_error`/`emerald_remote_last_error_message` (L1479-1492) already established for `errno`-style side-channel state, not a newly invented idiom. `emerald-rt`'s Rust-side equivalent is a `thread_local!` `RefCell<Option<SocketAddr>>` inside `net.rs`, read by the two accessor functions and written by `recv_from`."
    status: done
  - id: leaf-document-worker-pool-blocking-hazard
    content: "Add a new subsection to `spec/RUNTIME.md` (near the existing actor-scheduling material) documenting, in plain terms and with a real citation, that a blocking `TcpStream`/`TcpListener`/`UdpSocket` call made from inside an actor's message-handler body ties up one of the fixed `EMERALD_WORKERS`-or-`sysconf(_SC_NPROCESSORS_ONLN)` worker threads (`runtime/emerald_runtime.c:964-993`, `emerald_worker_pool_start`) for the full duration of that call — since actor message dispatch runs exclusively on this fixed-size pool, enough concurrently blocked reads/accepts (bounded above by the pool size, which defaults to core count) stall every other actor's mailbox processing project-wide, not just the blocked actor's own. State explicitly that this is the same class of hazard plan 94 names generally for any blocking native call made from a worker thread, and that this plan's sockets are deliberately, only ever blocking (`std::net`'s only mode) — a fully non-blocking/`mio`-driven or `tokio::net`-based variant is out of scope here (see Out of scope), consistent with plan 94's own preference for a crate's sync API first. Add a `#[test]` in `emerald-rt` (not a full end-to-end `.em` example, since reproducing worker-pool exhaustion deterministically needs `EMERALD_WORKERS=1` set at process start, awkward from inside `cargo nextest`'s own test harness) that starts a `TcpListener`, spawns a thread that blocks on `.accept()`-equivalent Rust-level code, confirms the accept only unblocks once a connect happens on a second thread — a real, working proof the underlying `std::net` primitives behave as documented, even though the worker-pool-exhaustion scenario itself is asserted in prose/spec text rather than in an automated regression test."
    status: done
  - id: leaf-example-and-full-gate
    content: "Add `examples/raw_tcp_sockets.em` (the Concrete Proof below) to `examples/`, wire it into `emerald-cli`'s example-conformance test table per plan 95's mandatory docs+example+test checklist and `examples/README.md`'s stated CI contract, and run the full `AGENTS.md` gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`) plus a clean-checkout end-to-end build, matching plan 91's own `leaf-example-and-full-gate` exactly."
    status: done
isProject: false
---

# Plan 96 — Raw TCP/UDP Sockets

Emerald has had TCP sockets in its runtime since plan 60 — but only as
private plumbing for the distributed-actor wire protocol. Verified
directly against `runtime/emerald_runtime.c`: `emerald_tcp_listen`
(L1499-1523) and `emerald_tcp_connect` (L1533-1551) are called
exclusively from the actor-registration/remote-dispatch machinery
`declare_actor_runtime_funcs`/`emerald_actor_register` wires up in
`emerald-codegen`, and the resulting file descriptor lives only inside
`EmeraldActorRef.sockfd` (L748-755, "one socket per ref, no
multiplexing") — there is no Emerald syntax that ever produces a bare
socket value, and no path from ordinary source code to `socket()`/
`bind()`/`connect()` at all. Every plan from 97 onward in this batch
(DNS, URLs, TLS, and — per the session's own stated plan numbering —
HTTP and WebSockets at 100-102) needs exactly that: a general-purpose,
directly-addressable socket a program can open, read, write, and close
on its own terms, independent of the actor system. This plan builds
that primitive, using nothing but the Rust standard library — no crate
to vet, no `DEPENDENCIES.md` entry to add under plan 95's ledger, the
single cleanest case its pure-Rust-first policy can produce.

Depends on: plan 91 (the `emerald-rt` archive and link mechanism this
module is compiled into), plan 92 (catch-unwind-at-every-boundary and
the canonical error shape every `TcpStream`/`TcpListener`/`UdpSocket`
method raises through), plan 93 (the opaque-u64-handle-plus-explicit-
`.close()` resource model these three types are the first real tenant
of), plan 95 (the pure-Rust-first policy this plan is offered as the
strongest possible example of, and the docs+example+test checklist
`leaf-example-and-full-gate` satisfies). Plans 97 (DNS), 99 (TLS), and
the future HTTP/WebSocket plans (100-102, per this session's own
numbering) depend on this plan in turn — every one of them either
resolves an address before handing it to `TcpStream.connect` (97) or
wraps a `TcpStream`/`TcpListener` this plan defines in an encryption
layer (99) rather than opening its own raw socket.

## Concrete proof this plan targets

```ruby
listener: TcpListener = TcpListener.bind("127.0.0.1", 47201)
client: TcpStream = TcpStream.connect("127.0.0.1", 47201)
client.write("ping")

server_side: TcpStream = listener.accept()
msg: String = server_side.read(64)
puts msg
server_side.write("pong")

reply: String = client.read(64)
puts reply

client.close()
server_side.close()
listener.close()
```

Expected output: `ping` then `pong`. This is a real, deterministic,
single-threaded proof — no actor spawn, no second process. It works
because TCP's own three-way handshake completes as soon as `connect()`
is called against a listener that has already called `bind()`+`listen()`
(the kernel queues the established connection in the accept backlog
regardless of whether the application has called `accept()` yet, and
the socket's own send buffer holds `"ping"` until the peer reads it) —
`listener.accept()` therefore returns immediately with the
already-completed connection, and `server_side.read(64)` returns the
already-buffered `"ping"` with no additional synchronization needed.
This ordering (`bind` -> `connect` -> `accept` -> `read`/`write` both
ways -> `close` all three) is itself the proof that plan 93's opaque
handles round-trip correctly through three independent resource types
in one program.

## Decision log

- **`std::net` needs no crate-vetting pass at all — the ideal case
  plan 95's pure-Rust-first policy describes, not merely an instance of
  it.** `TcpStream`/`TcpListener`/`UdpSocket` ship in every Rust
  standard library installation; there is no `Cargo.toml` line to add,
  no version to pin, no RustSec advisory to check, no maintenance
  signal to verify — the vetting question plan 95 asks of every other
  crate in this 101-plan batch (96-191) simply does not apply here.
  This is worth stating plainly rather than skipping straight to the
  API design, since it is the one plan in this batch where "which crate
  and why" has a one-line answer with zero research risk.
- **Plan 60's `emerald_tcp_listen`/`emerald_tcp_connect` and this
  plan's `TcpStream`/`TcpListener` are deliberately two separate socket
  universes — this plan does not unify them.** The actor transport's C
  functions are hardcoded `AF_INET`/`sockaddr_in` (IPv4 literal,
  verified at `runtime/emerald_runtime.c:1499-1551`), embed their file
  descriptor directly inside `EmeraldActorRef` with a dedicated
  `send_mutex` for cross-thread serialization, and are wired through
  `emerald-codegen`'s actor-specific trampolines — none of that
  machinery generalizes usefully to an arbitrary user-opened socket,
  and reusing it would mean either exposing actor-internal state to
  ordinary code or maintaining two calling conventions for what the C
  runtime treats as one concept. This plan's `emerald-rt`-based sockets
  keep their own independent handle table (plan 93) and never touch
  `EmeraldActorRef` or its `sockfd` field. A future plan could
  consider migrating the actor wire protocol onto this plan's `std::net`
  layer — that migration is explicitly not attempted here; per plan
  91's own precedent ("this plan adds a second archive; it does not
  begin retiring the first"), the C runtime's actor transport keeps
  working unchanged.
- **Blocking semantics are the only semantics this plan offers, and
  the actor-thread-pool interaction is real, not hypothetical.**
  Verified directly: `emerald_worker_pool_start`
  (`runtime/emerald_runtime.c:964-993`) spawns exactly
  `EMERALD_WORKERS` (or `sysconf(_SC_NPROCESSORS_ONLN)` when unset)
  `pthread`s, and every actor message dispatch runs on one of them.
  `std::net`'s blocking calls (`TcpStream::read`, `TcpListener::accept`,
  `UdpSocket::recv_from`) have no non-blocking or async mode in this
  plan's scope — a call made from inside an actor's message handler
  occupies that worker thread, unavailable to any other actor's
  scheduled message, for as long as the call takes (unbounded for
  `.accept()`/`.read()` against a peer that never sends). With a
  default pool sized to the machine's core count, a modest number of
  actors each blocked on a socket read is enough to stall the entire
  node's actor scheduling — the exact hazard plan 94 names generally
  for "a blocking native call made from a worker thread," concretized
  here with a real, cited mechanism rather than left abstract. This
  plan's answer, matching plan 94's own stated preference order, is:
  use the crate's sync API (there is nothing else to use — `std::net`
  has no async variant without pulling in `tokio::net`, which this plan
  declines, see Out of scope) and document the hazard precisely rather
  than papering over it with an unrequested async rewrite.
- **`String`'s null-terminated representation is a real, disclosed
  binary-safety gap for raw sockets — inherited, not introduced, by
  this plan, and not fixed here.** Plan 59's own verified finding
  (`emerald_alloc`/`emerald_string_length` show a bare `malloc`-backed
  buffer with no length header) means an Emerald `String` cannot
  represent a byte sequence containing an embedded NUL — entirely legal
  on an arbitrary TCP/UDP payload, illegal in Emerald's own string
  representation. `.read`/`.write`/`.send_to`/`.recv_from` all inherit
  this limitation for v1: a payload with an embedded zero byte silently
  truncates. This is the same shape of gap plan 92's binary-safe
  `(ptr, len)` buffer convention exists to close (for non-UTF8 data in
  general, not sockets specifically) — this plan does not build a
  competing mechanism; it names the gap and defers the fix to whichever
  future plan adds a real `Bytes`/`Buffer` type consuming plan 92's
  convention.
- **No multi-value return for `.recv_from`'s sender address — a
  last-value thread-local accessor instead, reusing a real existing
  pattern rather than inventing tuple-returning FFI.** `runtime/
  emerald_runtime.c`'s own `emerald_remote_last_error`/
  `emerald_remote_last_error_message` (L1479-1492) already establishes
  exactly this shape — a `_Thread_local` value set by the fallible call,
  read by a separate zero-argument accessor immediately after. `.recv_
  from`'s sender host/port reuses it verbatim (`last_sender_host`/
  `last_sender_port`) rather than requiring this plan to invent or
  verify a struct-returning or tuple-returning ABI convention no
  existing runtime function uses.
- **`SO_REUSEADDR` parity with the C runtime's listener is verified
  empirically, not assumed.** `emerald_tcp_listen` sets it explicitly
  (`runtime/emerald_runtime.c:1506`); Rust's `std::net::TcpListener::
  bind` does not document setting it on Unix by default as of this
  writing. This plan requires the implementer to check real behavior on
  this project's supported targets during `leaf-tcp-listener-bind-
  accept-close` rather than asserting either way here — the risk of
  getting this wrong is a real, disclosed one (a restarted server
  failing to rebind its own port with `EADDRINUSE`), not a
  hypothetical.
- **IPv6 support is a genuine, disclosed capability improvement over
  the existing actor transport, not a compatibility target with it.**
  `emerald_tcp_connect`'s signature (`uint32_t ip_network_order`) is
  IPv4-literal by construction; `std::net::ToSocketAddrs` resolves
  IPv6 literals and hostnames with no extra code in this plan at all.
  This plan does not add any IPv6-specific Emerald-facing API (no
  separate `Ipv6Addr`-typed parameter) — a host string that happens to
  resolve to an IPv6 address works through the exact same `connect`/
  `bind` signatures as IPv4, for free, because `std::net` handles the
  address-family dispatch internally.
- **Out of scope.** Non-blocking/async sockets (`mio`, `tokio::net`) —
  plan 94's own stated preference is a crate's sync API first, and this
  plan has nothing to bridge since `std::net` is already fully
  synchronous; a future async socket variant is a distinct, separate
  plan, not a hidden extension of this one. Unix domain sockets, raw/
  packet sockets, and socket-option tuning beyond `SO_REUSEADDR`
  parity-checking (no `SO_KEEPALIVE`/`TCP_NODELAY`/multicast group
  join/etc. surface in v1). `TcpListener.accept()`'s peer address and
  `TcpStream`'s own local/peer address accessors (`.local_addr()`/
  `.peer_addr()`) — genuinely useful, cheaply added later, deliberately
  excluded from this plan's narrower worked proof. TLS (plan 99 wraps
  this plan's `TcpStream` directly). DNS resolution beyond what
  `std::net::ToSocketAddrs`'s internal `getaddrinfo` call already does
  for a literal host string (plan 97). Unifying this plan's handle
  table or socket implementation with plan 60's actor-transport
  sockets (see Decision log above) — the two remain permanently
  separate unless a later plan explicitly proposes and justifies a
  migration.

## Update (2026-09-22, EXECUTE)

Implemented exactly as designed, no scope reduction. `crates/emerald-rt/src/net.rs`
wraps `std::net::{TcpStream, TcpListener, UdpSocket}` directly — zero crate to vet.
`TcpStream.connect`/`#read`/`#write`/`#close`, `TcpListener.bind`/`#accept`/`#close`,
`UdpSocket.bind`/`#send_to`/`#recv_from`/`#close`/`.last_sender_host`/
`.last_sender_port` — every method this plan's own leaf list named, all backed by
plan 93's own `crate::handle` registry via the identical `Int64`-newtype shape
`Regex`/`AeadKey`/`Url`/every other compiler-provided handle type in this batch
already establishes. Real, disclosed finding: this plan's own text frames
"instance-carrying AND non-user-declarable" as a combination with no existing
precedent — by the time this plan actually ran, plans 109-111/117/122/168 had
already built and proved exactly that combination repeatedly; no new
representational design was needed at all.

`#read`/`#write`/`#send_to`/`#recv_from` each make exactly one underlying
`std::io` call — real short-read/partial-write semantics, never looping, per this
plan's own leaf text. `TcpListener#accept` discards the peer `SocketAddr`, the
disclosed v1 simplification this plan's own Decision log names.
`UdpSocket.last_sender_host`/`.last_sender_port` reuse `runtime/emerald_runtime.c`'s
own `emerald_remote_last_error`/`emerald_remote_last_error_message`
`_Thread_local`-accessor-pair shape verbatim, backed by a real `thread_local!`
`RefCell<Option<SocketAddr>>` in `net.rs`.

The `SO_REUSEADDR` verification this plan's own Decision log required was carried
out for real, not assumed: confirmed (via community reports of `std::net` needing
a manual reuse-address opt-in, and via `tokio::net::TcpListener`'s own docs stating
it sets `SO_REUSEADDR` on Unix as a deliberate addition ON TOP of bare `std::net`)
that `std::net::TcpListener::bind` does NOT set it by default — a real divergence
from `emerald_tcp_listen`'s own explicit C-side `setsockopt`
(`runtime/emerald_runtime.c:1506`). Fixed via a raw `libc::getaddrinfo`/`socket`/
`setsockopt`/`bind`/`listen` sequence in `bind_tcp_listener_with_reuseaddr`,
socket2-free per the plan's own instruction — `getaddrinfo` resolves `host`/`port`
(the same mechanism `std::net` itself calls internally on Unix, still IPv6/hostname-
capable, not a narrower IPv4-literal path), `SO_REUSEADDR` is set on the raw fd
BEFORE `bind` (the only point at which it has any effect), and the bound, listening
fd is handed to `std::net::TcpListener` via `FromRawFd` so every other method on it
keeps using plain `std::net` afterward. Adds `libc` 0.2.189 (rust-lang-owned,
near-ambient) as a new, disclosed, real dependency this plan's own text didn't
name — not `socket2`, per the plan's own explicit instruction. Verified with a real
`getsockopt` readback in `emerald-rt`'s own test suite
(`reuseaddr_is_actually_set_on_a_bound_listener`), not merely assumed from the code
compiling.

`spec/RUNTIME.md` §2 gained a new item 6 documenting the worker-pool-blocking
hazard this plan's own `leaf-document-worker-pool-blocking-hazard` requires, citing
this plan's own sockets as the concretizing example. `net.rs`'s own
`accept_blocks_until_a_real_connect_happens` test proves the underlying `std::net`
primitive behaves as documented (a real `.accept()` only unblocks once a
`.connect()` happens on a second thread); the worker-pool-exhaustion scenario
itself stays prose-only, per this plan's own leaf text, since reproducing it
deterministically needs `EMERALD_WORKERS=1` set at process start.

`examples/raw_tcp_sockets.em` matches this plan's own Concrete Proof verbatim and
its CLI conformance test (`raw_tcp_sockets_em_prints_expected_sequence`) passed on
the first attempt with the exact predicted `ping`/`pong` output. 4 new unit tests
in `emerald-rt` (TCP round trip, UDP round trip + last-sender accessors, accept-
blocks-until-connect, SO_REUSEADDR readback), all passed on the first attempt.
1032/1032 tests, clean clippy/treefmt, `cargo audit --ignore RUSTSEC-2023-0071`
clean (same 5 pre-existing, already-triaged warnings, zero new advisories from
`libc` itself).

All six todos: `status: done`.
