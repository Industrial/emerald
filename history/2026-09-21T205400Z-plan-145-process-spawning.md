2026-09-21T20:54:00Z

---
name: Process Spawning & Control
overview: "A `Process.run(cmd: String, args: Array[String], argc: Int64, stdin_data: String): ProcessResult` compiler-known intrinsic backed by plain `std::process::Command` in `crates/emerald-rt` — zero third-party crate, `std::process` already does everything this plan needs (spawn, capture stdout/stderr, exit code, write-then-close stdin). Fills a real, confirmed gap: this session's own research into Emerald's current stdlib surface found no process-spawning primitive anywhere in the language today. Spawn failure (the executable does not exist / cannot be executed) raises a real Emerald exception via plan 38's mechanism; a nonzero exit code is not an error at all — it is an ordinary field on the returned `ProcessResult`, since a nonzero exit is a completely routine, expected outcome for many real programs (`grep` returning 1 for no match, `diff` returning 1 for differing files)."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-processresult-opaque-type
    content: "New `ProcessResult` opaque built-in reference type (`ValKind::ProcessResult`, the identical inert-type shape plan 59 used for `CString` and plan 144 used for `FileMetadata`): a heap-allocated struct with three fields (captured stdout bytes as a `String`, captured stderr bytes as a `String`, exit code as `Int64`), with `.stdout(): String`, `.stderr(): String`, `.exit_code(): Int64`, `.success(): Boolean` (`exit_code == 0`) as zero-argument method intrinsics."
    status: pending
  - id: leaf-emerald-rt-process-run
    content: "`emerald_rt_process_run(cmd: *const c_char, argv: *const *const c_char, argc: i64, stdin_data: *const c_char) -> *mut ProcessResult` in `crates/emerald-rt`: build `std::process::Command::new(cmd)`, `.args(...)` from the `argv`/`argc` pair, `.stdin(Stdio::piped())`, `.stdout(Stdio::piped())`, `.stderr(Stdio::piped())`; `.spawn()`, write `stdin_data`'s bytes to the child's stdin handle then drop it (closing the pipe so the child sees real EOF), `.wait_with_output()`, wrap the whole body in `std::panic::catch_unwind` per plan 91's proven pattern. On `Command::spawn()` returning `Err` (executable not found, permission denied, etc.), do not return a sentinel `ProcessResult` — signal failure across the FFI boundary per plan 92's raise-on-error convention, reusing plan 38's exception mechanism the same way plan 57 already reuses it for actor-crash isolation, so `Process.run` failing to spawn surfaces in Emerald source as a normal `raise`-able condition, catchable with an ordinary `rescue`."
    status: pending
  - id: leaf-argv-argc-marshaling
    content: "Emerald-side call-site codegen for `Process.run(cmd, args, argc, stdin_data)` passes `args`'s already-realized `char**` array pointer plus a caller-supplied `argc: Int64` alongside it — the same explicit-count-travels-with-the-array convention plan 45 already established for `ARGV`/`ARGC` and plan 144 established for `Dir.entries`/`Dir.entries_count`, forced by `Array[T]`'s continuing lack of runtime length metadata (see Decision log)."
    status: pending
  - id: leaf-sema-and-codegen-wiring
    content: "Add the `Process` compiler-known namespace arm (`Expr::Ident(n) if n == \"Process\"`) to `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`, matching `File`'s/`Dir`'s established hard-coded-arm shape; declare `emerald_rt_process_run` and the four `ProcessResult` accessor symbols via `module.add_function(..., Some(Linkage::External))`."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/process_spawning_proof.em` (the Concrete Proof below). Add `#[test]`s inside `crates/emerald-rt`: one spawning a real, always-present binary (`/bin/echo` on the CI Linux target) and asserting captured stdout; one spawning a deliberately nonexistent binary name and asserting `Command::spawn()` genuinely returns `Err` (proving the raise path has a real trigger, not a hypothetical one); one asserting stdin bytes written to a `cat`-style child are correctly echoed back through stdout, proving the write-then-close-stdin sequencing is correct and does not deadlock (a real, classic pipe-deadlock hazard when a child's stdout buffer fills before its stdin is fully drained — addressed by `wait_with_output` reading concurrently on separate threads internally, `std::process`'s own documented behavior, not something this plan's own code needs to hand-roll)."
    status: pending
isProject: false
---

# Plan 145 — Process Spawning & Control

This session's own research pass across Emerald's stdlib surface (the
groundwork for this entire 91-191 batch) found no process-spawning
primitive anywhere in the language: no `system()`-equivalent, no
`Command`-equivalent, nothing in `runtime/emerald_runtime.c`, nothing in
`grammar.lalrpop`, no `Process`/`Exec`/`Cmd` namespace in any prior plan's
Decision log. This is a real, confirmed gap, not a guessed one — an
Emerald program today has no way to run another program and observe its
output. This plan closes it with the smallest primitive that actually
answers the real question a spawn API exists to answer — "run this command,
give me back what it printed and how it exited" — using nothing but
`std::process::Command`, already in every Rust toolchain, needing no
third-party crate at all.

## Concrete proof this plan targets

```ruby
r: ProcessResult = Process.run("echo", ["hello", "from", "emerald"], 3, "")
puts r.stdout
puts r.exit_code
puts r.success

grep: ProcessResult = Process.run("grep", ["missing"], 1, "one\ntwo\nthree\n")
puts grep.exit_code
puts grep.success

begin
  bad: ProcessResult = Process.run("definitely-not-a-real-binary-xyz", [], 0, "")
rescue => e
  puts "spawn failed"
end
```

Expected output, in order:
```
hello from emerald

0
true
1
false
spawn failed
```

Trace: `echo hello from emerald` prints its arguments joined by spaces
followed by a newline to stdout — `r.stdout` captures exactly that string
(the `puts` after it prints an extra blank line from the trailing `\n`
already embedded in the captured output, disclosed as real observable
behavior, not a formatting bug — `Process.run` returns raw captured bytes,
it does not strip trailing newlines, mirroring `File.read`'s own
byte-for-byte fidelity). `r.exit_code` is `0`, `r.success` is `true`.
`grep missing` against piped-in text containing no `"missing"` line exits
`1` (real, standard `grep` behavior for "no lines matched") — `grep.
success` is `false`, and no exception is raised: a nonzero exit is not an
error under this plan's design. The final block spawns a binary name that
does not exist on any real `PATH`; `Command::spawn()` genuinely fails
(`ENOENT`), this plan's marshaling turns that into a raised Emerald
exception, and the `rescue` catches it — proving the spawn-failure path is
a different, exceptional code path from the nonzero-exit path directly
above it.

## Decision log

- **Spawn failure raises; a nonzero exit code does not — a deliberate,
  load-bearing split, not an arbitrary choice.** These are two genuinely
  different classes of event. A spawn failure (`ENOENT`, permission
  denied, `ENOMEM`) means the child process never ran at all — there is
  no `ProcessResult` to construct, no stdout/stderr/exit-code triple that
  makes sense to hand back, and it almost always indicates a programmer
  or environment mistake (a typo'd binary name, a missing `PATH` entry)
  that the overwhelming majority of call sites want to know about
  immediately rather than silently limp past. A nonzero exit code means
  the child ran to completion and told you, through its own normal exit
  protocol, that something about *its* work didn't succeed — `grep`
  finding no match, `diff` finding a difference, `test` evaluating false
  — outcomes every caller of those specific programs already expects and
  frequently wants to branch on directly, not have wrapped in exception
  handling. Forcing every `Process.run` call site to `rescue` a routine
  `grep`-found-nothing outcome would make the common case verbose for no
  safety benefit; forcing every call site to manually check a "did spawn
  succeed" flag before touching any field would make the rare, genuinely
  exceptional case easy to silently ignore. This plan's split — an
  exception for "never ran", a plain field for "ran and told you how it
  went" — puts the friction exactly where each event's own frequency and
  severity actually justify it.
- **`args`/`argc` is the third confirmed sighting of `Array[T]`'s missing
  length metadata forcing an explicit companion count, and this
  occurrence is a genuinely new variant of the pattern: the count
  crosses the FFI boundary as a call *argument*, not merely as a second
  return value.** Plan 45 established the pattern for a return
  (`.split`/`.split_count`); plan 144 confirmed it for another pair of
  returns (`Dir.entries`/`Dir.entries_count`). Passing an already-
  constructed `Array[String]` *into* `Process.run` exposes the same wall
  from the opposite direction — `emerald_rt_process_run`'s Rust body has
  no way to know how many `char*` elements the `argv` pointer it received
  actually points at without a caller-supplied `argc`. This plan accepts
  the same disclosed asymmetry plan 45 already accepted for `ARGV`/`ARGC`
  (there, too, a globally-populated `Array[String]` travels everywhere
  paired with an `Int64` count, never alone) rather than inventing a
  length-prefixed array representation as a side effect of one spawn
  function — that would be a `Array[T]` representation change affecting
  every existing consumer of arrays in this compiler, a far larger,
  cross-cutting redesign this plan has no mandate to attempt.
- **Piping/chaining is real but deliberately buffered through Emerald,
  not a live concurrent OS pipe between two children.** `Process.run`
  is synchronous and blocking: it writes the entirety of `stdin_data`
  to the child before waiting for it to exit, and returns only once the
  child has fully finished and its output is fully captured into two
  `String` values. "Chaining commands" (`a | b` in shell terms) is
  achieved entirely in Emerald source by feeding one call's `.stdout`
  into the next call's `stdin_data` argument — `b_result =
  Process.run("wc", ["-l"], 1, a_result.stdout)` — never by wiring two
  live child processes' file descriptors directly together so both run
  concurrently. This is a genuine simplification with a genuine cost (no
  streaming — the full intermediate output sits in memory as a `String`,
  and the two children never overlap in time the way a real shell
  pipeline's do) accepted deliberately: true concurrent OS-pipe chaining
  needs a live, still-running child on each end of the pipe — a real
  resource under plan 93's handle model (open pipe file descriptors, a
  process that must eventually be waited on to avoid a zombie) — which
  this plan's single blocking `Process.run` entry point does not
  provide. A future `Process.spawn(...): ProcessHandle` giving
  non-blocking access to a live child's stdin/stdout as it runs, closable
  and waitable per plan 93's model, is a distinct, larger follow-up this
  plan explicitly does not attempt.
- **The child inherits the parent's environment by default, which is
  exactly the behavior plan 146 relies on and this plan does not
  reimplement.** `std::process::Command::new(...)` inherits the calling
  process's environment unless explicitly cleared or overridden — this
  plan takes that default as-is; whatever `Env.set` (plan 146) has done
  to the current process's environment before a `Process.run` call is
  visible to the spawned child automatically, with zero extra plumbing
  in this plan's own `emerald_rt_process_run` body. Plan 146's own
  Decision log states the converse risk plainly: environment variables
  set for secrets purposes propagate into every child a program spawns
  by default — this plan is the concrete mechanism that inheritance
  travels through, referenced there, not re-litigated here. Explicit
  per-child environment override/clearing (`Command::env`/`env_clear`)
  is real, small, and declined here only because this plan's own
  Concrete Proof does not need it — a natural, minimal follow-up leaf a
  future plan could add without touching this plan's core `Process.run`
  signature.
- **No shell is ever invoked — `Command::new(cmd).args(argv)` runs `cmd`
  directly via `execvp`-family semantics, never through `/bin/sh -c`.**
  This is a deliberate security property, not an oversight: passing
  arguments through a shell means shell metacharacters in any
  user-influenced argument (`;`, `` ` ``, `$(...)`, `|`) get interpreted
  rather than passed through literally, a classic and well-documented
  shell-injection hazard. `std::process::Command`'s own default behavior
  already avoids this — arguments are passed to the target program's own
  `argv`, never substituted into a shell command line — and this plan
  does nothing to override that default. A caller who genuinely wants
  shell semantics (globbing, pipes, redirection) can still get them by
  explicitly spawning `Process.run("/bin/sh", ["-c", "..."], 2, "")` — the
  hazard becomes visible and opt-in at the call site rather than silently
  present in every `Process.run` call by default.
- **Captured stdout/stderr are raw bytes reinterpreted as `String`,
  inheriting `String`'s own existing UTF-8 assumption rather than a new
  one this plan invents.** `spec/TYPE_SYSTEM.md` already declares
  `String` UTF-8 encoded; a child process's real stdout is not
  guaranteed valid UTF-8 (a miscompiled binary, a corrupted pipe, a
  program that intentionally emits raw bytes). This plan's
  `emerald_rt_process_run` uses `String::from_utf8_lossy` (replacing any
  invalid byte sequence with `U+FFFD`) rather than failing the whole
  call over a handful of bad bytes in captured output — a disclosed,
  deliberately forgiving choice, matching the spirit of `.upcase`/
  `.downcase`'s own already-disclosed byte-wise-ASCII-only limitation
  (plan 45) rather than inventing a stricter contract this plan alone
  would have to justify.
- **Out of scope.** Non-blocking/streaming spawn with a live, pollable
  `ProcessHandle` (needs plan 93's resource-handle model — noted above,
  not attempted); sending signals to a spawned child after it starts
  (`SIGTERM`/`SIGKILL` — the natural pairing is plan 151's signal work,
  once a live handle exists to send to, which this plan does not
  produce); process groups, session leaders, or daemonization; resolving
  `cmd` against `PATH` with any customization beyond what `Command::new`
  already does (`std::process::Command` already searches `PATH` the same
  way a shell's `execvp` does when `cmd` contains no `/`, which is
  sufficient for this plan's own Concrete Proof and needs no extra code);
  any interaction with `runtime/emerald_runtime.c` — this plan is pure
  `emerald-rt`/`std::process`, zero C runtime changes.
