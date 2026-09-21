2026-09-21T20:55:00Z

---
name: Environment Variables
overview: "An `Env` compiler-known namespace — `Env.get(key): String?`, `Env.set(key, value): Void`, `Env.remove(key): Void`, `Env.keys(): Array[String]` + `Env.keys_count(): Int64` — backed entirely by plain `std::env` in `crates/emerald-rt`, zero third-party crate. Fills a second real, confirmed gap this session's own research found: no general environment-variable access exists in Emerald today. The one genuine wrinkle is not API shape but concurrency safety: `std::env::set_var`/`remove_var` were made `unsafe fn` in the Rust 2024 edition specifically because mutating the process environment while another thread reads it is unsound, and Emerald's own actor model (plan 55) runs multiple live OS worker threads — this plan serializes every `Env` call in `emerald-rt` behind one process-wide mutex to make that real hazard actually safe for a multi-threaded Emerald program, not just declared away."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-env-mutex-and-get-set-remove
    content: "In `crates/emerald-rt`, a single `static EMERALD_RT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(())` guarding every one of this plan's functions' bodies. `emerald_rt_env_get(key: *const c_char) -> *mut c_char` (nullable, `std::env::var` mapped `Ok -> owned CString`, `Err(NotPresent)`/`Err(NotUnicode)` -> null pointer, reusing plan 43/59's zero-cost nullable-pointer convention). `emerald_rt_env_set(key, value)` / `emerald_rt_env_remove(key)`, each acquiring the mutex, then calling the real `unsafe { std::env::set_var(...) }` / `unsafe { std::env::remove_var(...) }` inside the held lock — the `unsafe` block is real and load-bearing here (see Decision log), not boilerplate. Every function wrapped in `std::panic::catch_unwind` per plan 91's proven pattern, applied *outside* the mutex acquisition so a poisoned lock from a panicking holder cannot wedge every subsequent `Env` call."
    status: pending
  - id: leaf-env-keys-enumeration
    content: "`emerald_rt_env_keys() -> *mut *mut c_char` / `emerald_rt_env_keys_count() -> i64`, both also mutex-guarded, backed by `std::env::vars()` collected under the lock — the same paired-count-with-array convention plans 45/144/145 already established for every other `Array[T]`-returning intrinsic in this batch."
    status: pending
  - id: leaf-sema-and-codegen-wiring
    content: "Add the `Env` compiler-known namespace arm to `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`, matching `File`/`Dir`/`Path`/`Process`'s established hard-coded-arm shape; declare the four `emerald_rt_env_*` symbols via `module.add_function(..., Some(Linkage::External))`."
    status: pending
  - id: leaf-example-and-tests
    content: "Add `examples/environment_variables_proof.em` (the Concrete Proof below). Add `#[test]`s inside `crates/emerald-rt`: one asserting `Env.set`+`Env.get` round-trips a real value; one asserting `Env.remove` then `Env.get` returns null; one asserting concurrent `Env.set` calls from multiple `std::thread::spawn`'d threads (simulating Emerald's own multi-worker actor pool) complete without a panic or a poisoned-mutex failure, the direct proof this plan's mutex-serialization actually closes the Rust-2024-flagged soundness hole rather than merely citing it."
    status: pending
isProject: false
---

# Plan 146 — Environment Variables

This session's own research pass found the second of two real, confirmed
stdlib gaps this batch fills directly: alongside plan 145's missing
process-spawning primitive, Emerald has no general environment-variable
access at all — no way to read `PATH`, no way to check a feature flag set
by a calling shell, no way to set a value a spawned child (plan 145) should
inherit. `std::env` already provides everything the shape of this gap
needs: `var`/`set_var`/`remove_var`/`vars()`. This plan is a thin,
zero-dependency wrapper around it, with exactly one real design problem to
solve, not invent: `std::env::set_var` and `std::env::remove_var` were
reclassified as `unsafe fn` in the Rust 2024 edition specifically because
mutating a process's environment concurrently with another thread reading
it is genuine, platform-level undefined behavior on some targets (POSIX
`getenv`/`setenv` are not required to be thread-safe with respect to each
other) — and Emerald's own actor model runs a real worker-thread pool
(plan 55), meaning two actors on two different OS threads calling `Env.set`
and `Env.get` at the same moment is not a hypothetical scenario this
language's own architecture could hit, it is close to the default shape of
concurrent Emerald code. This plan takes that specific, dated (2026),
verifiable Rust-language fact as its central design constraint, not a
footnote.

## Concrete proof this plan targets

```ruby
Env.set("EMERALD_PLAN146_PROOF", "hello")
v: String? = Env.get("EMERALD_PLAN146_PROOF")
v ||= "unset"
puts v

n: Int64 = Env.keys_count
puts n > 0

Env.remove("EMERALD_PLAN146_PROOF")
gone: String? = Env.get("EMERALD_PLAN146_PROOF")
gone ||= "unset"
puts gone
```

Expected output, in order:
```
hello
true
unset
```

Trace: `Env.set` then `Env.get` round-trips the real value through the real
process environment (not an in-process shadow table — a second, independent
process could observe it via `/proc/self/environ` while this one is still
running, a genuine OS-level environment mutation, not a simulated one).
`Env.keys_count` is compared with `0` rather than printed as a raw number
since a real process environment's exact key count varies by shell/CI
environment and is not itself the property being proven — that calling
`Env.keys_count` at all returns a positive count (proving real enumeration
against the real environment, not a stub) is. `Env.remove` then `Env.get`
returns `nil`, defaulted through plan 43's `||=` exactly as plan 59's
`CString`/`String?` proof already did for a real `NULL`.

## Decision log

- **The Rust-2024-edition `unsafe fn` reclassification of `set_var`/
  `remove_var` is treated as a real safety signal this plan must actually
  close, not paper over with a bare `unsafe` block.** Verified this
  session: the Rust Edition Guide's own "Newly unsafe functions" page
  states plainly that calling `std::env::set_var`/`remove_var` "can be
  unsound to call... in a multithreaded program due to safety
  limitations of the way the process environment is implemented on some
  platforms" — a real, dated, first-party language change, not a
  third-party opinion. This plan's response is a single process-wide
  `std::sync::Mutex<()>` inside `emerald-rt`, held across the full body
  of every `Env.get`/`set`/`remove`/`keys` call (reads included, not just
  writes — a concurrent `getenv` racing a concurrent `setenv` is exactly
  the unsound combination the edition guide describes, so read-only
  `Env.get` must serialize against `Env.set`/`remove` too, not merely
  against other writers). This is the one place in this whole 91-191
  batch, so far, where the standard `catch_unwind`-only boundary plan 91
  established is not sufficient by itself — a mutex is genuinely new
  machinery this plan adds on top of it, justified by a real,
  cross-referenced upstream Rust decision rather than defensive
  over-engineering.
- **The mutex is acquired, used, and released entirely inside each
  `emerald_rt_env_*` function body — `catch_unwind` wraps the call from
  outside the lock, not inside it — specifically so one thread's panic
  cannot permanently wedge every future `Env` call via mutex poisoning.**
  `std::sync::Mutex::lock()` returns `Err(PoisonError)` once a prior
  holder has panicked while holding it; naively propagating that as a
  second panic from inside an already-panicking `catch_unwind` body would
  be a real, silent lock-out of the entire `Env` namespace for the rest
  of the process's life. This plan's functions instead use `.lock().
  unwrap_or_else(|poisoned| poisoned.into_inner())` — deliberately
  recovering the guard even from a poisoned lock, since environment
  variables have no invariant a partial write could corrupt beyond
  "value wasn't fully written," a real, accepted, disclosed risk
  narrower than a wedged-forever namespace would be.
- **The security caveat this plan states — and does not attempt to
  solve — is real and documented, not invented for flavor.** Environment
  variables are a genuinely common secret-leak surface: a value set via
  `Env.set` is, by `std::process::Command`'s own default inheritance
  behavior (plan 145's own citation of this exact fact), visible to
  every child process a program spawns from that point on unless
  explicitly cleared per-child; and environment dumps are a routine
  accidental disclosure vector in crash reports, verbose logging, and CI
  job output (widely documented practice: GitHub Actions/GitLab CI mask
  *known* secret env vars in their own log output, but any value derived
  from one, or any variable never registered as a secret, is not
  automatically protected). This plan does not add masking, redaction,
  or a "sensitive" variable classification — doing so honestly would
  need a real secrets-management design (which variables are sensitive,
  who decides, what "masked" means across `puts`/logging/crash dumps)
  far beyond a get/set/enumerate wrapper. The caveat is stated here as a
  documented, disclosed risk a program author must manage themselves —
  the same posture plan 59 already took toward FFI's own undefined-
  behavior risk: name the hazard plainly rather than build a false sense
  of safety around it.
- **`Env.get` returns `nil` for both "the variable is genuinely unset"
  and "the variable's value is not valid Unicode" — a real, disclosed
  conflation, not an oversight.** `std::env::var` distinguishes
  `VarError::NotPresent` from `VarError::NotUnicode(OsString)`; `String`
  in this compiler is UTF-8-only (per `spec/TYPE_SYSTEM.md` §9, already
  cited by plan 45), so a non-Unicode environment value has no honest
  `String` representation to hand back regardless of which `VarError`
  variant produced it. Both collapse to the same `nil`, matching the
  same all-failures-become-`nil` posture plan 144 already took for
  `Path.metadata`/`Path.read_link` rather than inventing a second
  nullable-vs-error distinction this plan's narrow surface does not
  need. A caller who genuinely needs to distinguish "unset" from
  "set but unrepresentable" has no way to under this plan — a real,
  accepted, narrow gap.
- **`Env.keys_count`/`Env.keys` is the fourth confirmed sighting of the
  `Array[T]`-needs-a-companion-count pattern, and needs no more
  discussion than that — plans 45/144/145 already carry the full
  justification.** Cited here for completeness, not re-argued.
- **Out of scope.** Reading/setting another process's environment (not
  possible from user space on any of this project's target platforms
  without a debugger-level API, and out of scope regardless); `.env`
  file parsing/loading (a real, common convenience — `dotenv`-style — but
  a distinct, separate feature layered *on top of* `Env.set`, not part
  of this plan's own get/set/enumerate primitive); any secrets-masking
  or redaction mechanism (see the security-caveat bullet above); any
  change to `Process.run`'s (plan 145) own default environment-
  inheritance behavior or to `runtime/emerald_runtime.c`.
