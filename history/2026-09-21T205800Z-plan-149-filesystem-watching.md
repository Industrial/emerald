2026-09-21T20:58:00Z

---
name: Filesystem Change Notification
overview: "A `Watcher` resource handle (`Watcher.watch(path): Watcher`, `.poll(): FsEvent?`, `.close(): Void`) backed by the `notify` crate (cross-platform: `inotify` on Linux, `FSEvents` on macOS, `ReadDirectoryChangesW` on Windows — verified this session as actively maintained, depended on by `rust-analyzer`, `cargo watch`, and `watchexec`), giving Emerald real, OS-native filesystem change notification rather than a polling-based reimplementation. Delivery is a pollable queue by default (`.poll(): FsEvent?`, draining one buffered event per call, nil when none pending) — the simplest, most deterministic shape to prove and test — with a second, actor-native entry point (`Watcher.watch_actor(path, target)`) that pushes each event directly into a target actor's mailbox from `notify`'s own dedicated background thread, the same producer role any other non-actor code already plays when it calls `.send()` on an actor reference per plan 55's scheduler; a watched path maps onto exactly one such background thread, decoupled from the worker-thread pool that actually executes `target`'s `handle` method."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-notify-crate-and-handle
    content: "Add `notify = \"6\"` to `crates/emerald-rt/Cargo.toml`. New `Watcher` opaque built-in reference type (`ValKind::Watcher`) holding a `notify::RecommendedWatcher` plus an `std::sync::mpsc::Receiver<notify::Event>` inside a Rust-side registry keyed by an opaque `i64` handle id, the same registry shape plans 147/148 already established, required for the identical reason: `notify::Watcher`'s `Drop` impl is what actually stops the underlying OS watch (unregistering the `inotify` descriptor / `FSEventStream` / `ReadDirectoryChangesW` handle), and Emerald's memory model cannot trigger that automatically."
    status: pending
  - id: leaf-fsevent-opaque-type
    content: "New `FsEvent` opaque built-in reference type (`ValKind::FsEvent`): `.path(): String`, `.kind(): Symbol` (one of `:created`, `:modified`, `:removed`, `:renamed`, `:other`, reusing plan 44's already-real, compile-time-interned `Symbol` type rather than inventing a new tag representation), mapped from `notify::EventKind`'s own real variant set (`Create`, `Modify`, `Remove`, and `Modify(ModifyKind::Name(_))` folded into `:renamed` — see Decision log for exactly how `notify`'s richer, nested `EventKind` collapses onto this plan's five-symbol surface)."
    status: pending
  - id: leaf-watch-poll-close
    content: "`emerald_rt_watcher_watch(path: *const c_char) -> i64` (construct `notify::recommended_watcher` with an `mpsc::channel` sender, call `.watch(path, RecursiveMode::Recursive)`, register in the handle registry). `emerald_rt_watcher_poll(handle: i64) -> *mut FsEvent` (non-blocking `Receiver::try_recv()`, null on `Empty`, an `FsEvent` on `Ok`). `emerald_rt_watcher_close(handle: i64): Void` (remove from the registry, dropping the `RecommendedWatcher` and stopping the OS-level watch deterministically, the same explicit-trigger pattern plans 147/148 already use). All wrapped in `std::panic::catch_unwind` per plan 91's proven pattern."
    status: pending
  - id: leaf-watch-actor-variant
    content: "`emerald_rt_watcher_watch_actor(path: *const c_char, target_mailbox: <actor-ref ABI, per plan 92>) -> i64`: identical `notify` setup, but instead of buffering into an `mpsc::Receiver` for later `.poll()`, the channel's receiving side runs on its own dedicated background thread inside `emerald-rt` that, for each event, constructs an `FsEvent` and calls the exact same mailbox-enqueue runtime entry point plan 55's own `.send()` codegen already calls — this plan adds no new message-delivery mechanism, it adds a second, non-actor *producer* thread feeding an existing consumer-side primitive, structurally identical to how any ordinary Emerald code already becomes a mailbox producer via `.send()`."
    status: pending
  - id: leaf-sema-codegen-example-tests
    content: "Add `Watcher`/`FsEvent` compiler-known namespace/type wiring to `emerald-sema`/`emerald-codegen`, matching the established hard-coded-arm shape. Add `examples/filesystem_watching_proof.em` (the Concrete Proof below, poll-based only — the actor-push variant is exercised by a Rust-side test instead, see Decision log for why). Add `#[test]`s inside `crates/emerald-rt`: one asserting a real file creation under a watched directory produces a `Create` event within a bounded wait; one asserting `.close()` genuinely stops delivery (create a file after `.close()`, assert no event arrives); one exercising `watch_actor` end-to-end against a stub mailbox-enqueue target, proving the background-thread-to-mailbox path is real, not just described."
    status: pending
isProject: false
---

# Plan 149 — Filesystem Change Notification

Watching a directory for future changes is a fundamentally different
capability from plan 144's point-in-time `Dir.walk`/`Path.metadata`
snapshots — it needs a long-lived OS subscription (an `inotify` watch
descriptor, an `FSEventStream`, a `ReadDirectoryChangesW` handle) that
stays registered with the kernel until explicitly torn down, and it needs
some way for Emerald code to receive events as they arrive rather than as
a one-shot return value. `notify` (verified this session: actively
maintained, its own crates.io listing states real, current downstream
adoption — "As used by: cargo watch, cobalt, rust-analyzer, watchexec,
watchfiles, xi-editor") is the standard cross-platform abstraction over
all three native mechanisms; reimplementing `inotify`/`FSEvents`/
`ReadDirectoryChangesW` handling separately per platform is exactly the
kind of substantial, already-solved, easy-to-get-subtly-wrong problem
this batch's crate-first directive exists for.

## Concrete proof this plan targets

```ruby
w: Watcher = Watcher.watch("plan149_demo")
File.write("plan149_demo/new.txt", "x")

attempts: Int64 = 0
found: Boolean = false
while attempts < 200000 && !found
  ev: FsEvent? = w.poll
  if ev != nil
    puts ev.path
    found = true
  end
  attempts += 1
end
puts found

w.close
File.write("plan149_demo/after_close.txt", "y")
late: FsEvent? = w.poll
late ||= FsEvent.none
puts late.is_none
```

Expected output, in order (run from a fresh temporary working directory,
`plan149_demo/` created before `Watcher.watch` is called):
```
plan149_demo/new.txt
true
true
```

Trace: `Watcher.watch` registers a real, live OS-level watch on
`plan149_demo`. `File.write` (plan 45) creates a real file inside it;
the busy-poll loop (disclosed as a busy-wait, adequate for this
deterministic proof — see Decision log for why the richer `watch_actor`
design avoids exactly this busy-waiting in production code) drains
`.poll()` until a real `Create` event for `new.txt` arrives, printing its
path and `found = true`. After `w.close`, the underlying OS watch is torn
down; a second `File.write` produces no event this handle will ever see —
`.poll()` after close returns `nil`.

## Decision log

- **Delivery is a pollable queue in this plan's own Concrete Proof, with
  the actor-mailbox-push variant offered as a second, real entry point —
  both are genuine answers, chosen for different situations, not a
  hedge.** A pollable `.poll(): FsEvent?` is the simplest possible shape
  to make deterministic and testable in a single-run example: no timing
  assumption beyond a bounded retry loop, no dependency on the actor
  scheduler being live. `Watcher.watch_actor(path, target)` is the
  answer to this batch's own explicit question — "does a watched path
  map naturally onto one actor receiving watch-event messages?" — and
  the answer is yes, with a concrete mechanism: `notify`'s own callback
  runs on a dedicated background thread this plan spawns inside
  `emerald-rt` (one such thread per live `Watcher.watch_actor` call, not
  shared), and that thread becomes a mailbox *producer* for `target`
  using the identical enqueue entry point plan 55's own `.send()`
  codegen already calls — no new message-delivery primitive, a new
  *source* feeding an existing one. This plan's own Concrete Proof uses
  the poll variant specifically because a push-based proof would need a
  real actor spawn plus a synchronization point this plan has no
  grounded, already-established primitive to build on (no confirmed
  `sleep`/`join`/`await` construct exists in this compiler as of this
  session) — the busy-poll loop is disclosed exactly as that: adequate
  for a deterministic single-run proof, not the pattern a production
  Emerald program watching a directory should actually use, where
  `watch_actor` avoids busy-waiting entirely by design.
- **`Watcher` needs plan 93's full explicit-close resource-handle
  model — it is the clearest possible case of a long-lived subscription
  this batch's own task framing names directly.** It holds a live kernel
  registration (an open `inotify` file descriptor on Linux, an active
  `FSEventStream` on macOS) that must be explicitly torn down; unlike
  plan 148's `flock`-based `FileLock` (which the OS releases automatically
  when its owning file descriptor closes at process exit), a `notify`
  watch has no such automatic backstop this plan can rely on stating —
  `.close()` is the only way to stop it before process exit, making the
  explicit-close discipline here not merely consistent with plan 93's
  model but strictly necessary for correctness, not just hygiene.
- **`notify`'s own richer, nested `EventKind` (with platform-specific
  sub-variants for renames, metadata-only changes, and access events) is
  deliberately collapsed onto a five-symbol surface — a real, disclosed
  loss of fidelity, not an oversight.** `notify::EventKind` distinguishes
  far more than "created/modified/removed" — rename-from vs. rename-to
  halves, metadata-only modifications (permission changes with no
  content change), access events on some platforms. This plan's
  `FsEvent.kind` folds all of that down to `:created`/`:modified`/
  `:removed`/`:renamed`/`:other`, matching Symbol's own already-
  established role (per plan 44) as a lightweight, compile-time-interned
  tag rather than exposing `notify`'s full platform-dependent event
  taxonomy as a new, larger Emerald-visible enum. A program needing the
  finer-grained distinction (e.g. telling a rename's from-half apart
  from its to-half) has no way to under this plan — declined here
  specifically because Emerald's own filesystem-event vocabulary should
  not have to track `notify`'s own internal, platform-varying event
  model in full.
- **Recursive watching is the only mode this plan implements —
  `RecursiveMode::Recursive`, always — a real, deliberate narrowing of
  `notify`'s own exposed API surface.** `notify` also supports
  `RecursiveMode::NonRecursive` (watch only the named path itself, not
  its subdirectories); this plan's single `Watcher.watch(path)` entry
  point always requests recursive watching, matching this plan's own
  Concrete Proof's expectation that a file created inside the watched
  directory produces an event. A non-recursive variant is a small,
  natural, declined-for-now follow-up (an additional boolean parameter,
  or a second `Watcher.watch_shallow` entry point) — not attempted here
  since nothing in this plan's own proof needs it.
- **`notify`'s own documented reliability caveat under very high event
  volume is inherited as-is, not solved by this plan.** `notify`'s own
  `docs.rs` page discloses that watching an extremely large number of
  files can cause the underlying OS mechanism to drop or coalesce events
  under load (a real, platform-level limitation — `inotify`'s own queue
  has a finite, kernel-configurable size, and a full queue silently
  drops further events per `inotify(7)`'s own documented behavior, not
  something `notify` itself can work around). This plan does not attempt
  event-loss detection or an application-level backpressure mechanism —
  a real, disclosed limitation inherited from the underlying crate and
  OS primitive, not a gap this plan's own code introduces.
- **Out of scope.** Watching multiple paths through a single `Watcher`
  handle (this plan's `.watch(path)` is one path per handle; a program
  needing several watched paths calls `Watcher.watch` once per path and
  holds several handles — declined as an added-complexity convenience,
  not a capability gap, since nothing prevents a caller from doing this
  today with the primitives given); debouncing/coalescing rapid
  successive events into one (`notify`'s own ecosystem offers a separate
  `notify-debouncer-mini`/`notify-debouncer-full` crate for exactly this,
  a natural, independent future addition this plan does not fold in);
  any change to plan 144's `Dir`/`Path` namespace, plan 55's scheduler,
  or `runtime/emerald_runtime.c`.
