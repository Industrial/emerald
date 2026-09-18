---
name: Single-File Actor Require Without --jobs
overview: "Plan 65 fixed require-splicing an actor-declaring file under --jobs N (WeakODR linkage closed the duplicate-symbol link failure) but explicitly left the no-jobs half of the gap open: examples/README.md states plainly that a bare `emerald host.em -o out` (no --jobs) still fails typecheck — `unknown type 'Counter'`, `undefined variable 'c'` — for a file that requires an actor-declaring module. Ordinary (non-actor) requires already splice correctly without --jobs per plan 23; this plan closes the actor-specific gap in that same no-jobs single-compilation-unit path."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-confirm-scope-of-the-gap
    content: "Verify directly (not assume from examples/README.md alone) whether the no-jobs require path fails specifically for actor-declaring required files, or more broadly — write a minimal non-actor require case first (plan 23's own original worked example, re-run against current HEAD) to confirm ordinary requires still splice correctly without --jobs today, isolating that this gap is actor-specific and not a broader regression in plan 23's machinery."
    status: pending
  - id: leaf-root-cause-and-fix
    content: "Root-cause why the no-jobs single-compilation-unit require path fails to resolve an actor class's type/constructor where the --jobs multi-unit path (crates/emerald-driver's parallel.rs, per plan 49) succeeds — likely candidates to check first: whether actor declarations are registered into the same AST-merge/symbol-table step plan 23's splicer already uses for ordinary classes, or whether actor-specific registration (trampolines, wire codecs) was only ever wired into the --jobs/parallel compilation path plan 65's WeakODR fix touched. Fix wherever the actual gap is found."
    status: pending
  - id: leaf-regression-test
    content: "A test compiling examples/host.em (or an equivalent minimal actor-require program) via the bare, no-jobs CLI path and asserting it typechecks and links successfully, alongside the existing --jobs regression test (an_actor_shared_across_require_d_files_links_and_runs_correctly_under_jobs, crates/emerald-driver/tests/parallel_jobs.rs) so both paths are covered rather than only the one plan 65 already fixed."
    status: pending
isProject: false
---

# Plan 69 — Single-File Actor Require Without --jobs

Plan 65's own file is explicit that it fixed only half of this bug: "A
bare `emerald host.em -o out` still fails typecheck (`unknown type
'Counter'`, `undefined variable 'c'`) — the 'no `--jobs`, no splice' gap
is unrelated and still real." `examples/README.md` repeats the same
finding. This plan is that other half, named as its own row rather than
left permanently implicit the way it has been since plan 65 shipped.

## Concrete proof this plan targets

`examples/counter_actor.em` (an `actor Counter` declaration, no
top-level statements — already exists) required by a single-file program
with no `--jobs` flag:

```ruby
require counter_actor

c = Counter.spawn(0)
c.increment
c.increment
c.increment
puts c.report
```

Run as `emerald require_actor.em -o out && ./out` — **no** `--jobs`
flag. Expected output: `3`. Today: typecheck fails before codegen is
even reached, citing `unknown type 'Counter'` and `undefined variable
'c'`.

## Decision log

- **Why this is scoped separately from plan 65 rather than reopening
  it.** Plan 65 is a shipped, verified, closed plan (backfilled into
  the plan-of-plans this session) whose own worked example and
  regression test are specifically about the `--jobs` path. Reopening a
  closed, correctly-scoped plan to add unrelated no-jobs-path work would
  blur what plan 65 actually proved; a new plan number keeps plan 65's
  own concrete proof and this one's each independently checkable.
- **Confirm-scope leaf exists because this plan does not yet know
  whether the bug is actor-specific or a symptom of something broader
  in the no-jobs require path.** `examples/README.md`'s language
  ("this genuinely didn't build earlier this same session, fixed via
  WeakODR linkage" for the `--jobs` half) is about linking, a codegen/
  driver-stage concern; the no-jobs failure is a *typecheck*-stage
  error naming an unresolved type and an unresolved variable — a
  different compiler stage entirely. It is plausible these are two
  unrelated gaps that happen to share one example file, not one bug
  with two manifestations; `leaf-confirm-scope-of-the-gap` requires
  checking plan 23's ordinary (non-actor) require path still works
  before assuming the actor case is what's special, rather than
  inheriting that assumption unchecked from two prior documents that
  both stated it without re-deriving it from source.
- **Out of scope.** This plan does not add `--jobs`-style parallel
  compilation behavior to the no-jobs path, and does not change what
  `--jobs` itself does — it closes the specific typecheck-stage gap for
  actor-declaring requires in the single-compilation-unit path plan 23
  already established for ordinary classes.
