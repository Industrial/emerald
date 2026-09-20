# Emerald — OWNERSHIP.md

**Status:** forward design document (plan 82), not a retroactive record of shipped
behavior the way `RUNTIME.md` is. It exists to give plans 83-85 a concrete, decided
target to implement against, rather than each guessing independently at a design
nobody made.

SUPERSEDED note (plan 84): the paragraph above originally said "nothing in this
file exists in the compiler yet." That's no longer true for §§9-10's plan 84 row —
`emerald-sema` (plan 83) and `emerald-codegen` (plan 84) now implement real
`own`/`borrow`/`borrow var` checking and codegen exactly as designed here. Plan 85
(§10's own remaining row) is still unimplemented. Corrected in place rather than
silently rewritten, per this project's own convention.

**Purpose:** decide what "a full Rust-style ownership/borrow checker" (this
session's explicit commitment, made ahead of `spec/RUNTIME.md` §1 and
`history/2026-09-08T173600Z-inception.md` §12's own deferral) actually means for
Emerald, concretely enough to implement.

---

## 1. The decision this document commits to, and how it was reached

Before writing anything here, four independent research passes studied Rust's
own borrow checker, Austral's linear types, Vale's region/generational-reference
model, and Pony's reference capabilities — each asked, separately and without
seeing the others' answer, "would you recommend this system's full model for
Emerald?" All four said no to their own subject, and converged on some version
of the same smaller hybrid: extend what Emerald already ships (escape analysis,
regions, plan 56's linear-use check) rather than import a second, independent
ownership system wholesale. That convergence was put to the project directly.
**The explicit decision, made with that evidence in hand, was to proceed with a
real, full Rust-style ownership/borrow checker anyway.** This document honors
that decision — Emerald gets real ownership, real borrowing, a real static
checker — while using what the research surfaced to avoid the specific,
named mistakes Rust's own implementation history shows cost the most: named
lifetime parameters as a general-purpose feature, variance, and higher-ranked
trait bounds are the parts of Rust's model the research identified as hardest
to build and learn, and least connected to why the checker exists in the first
place (proving no two live references can alias unsafely). This design gets
that same proof without importing those specific mechanisms.

## 2. Syntax: `own` / `borrow` / `borrow var`, no named lifetime parameters

```ruby
fn process(data: borrow Data): Void do
  puts data.length
end

fn mutate(data: borrow var Data): Void do
  data.append("x")
end

fn consume(data: own Data): Void do
  puts data.length
  # `data` cannot be used again by the caller after this call returns —
  # the exact rule plan 56 already enforces for cross-actor sends,
  # generalized to every `own`-typed parameter.
end
```

- **`own T`** — the callee takes ownership; the caller's binding is consumed
  and using it again is a compile error. This is plan 56's `is_cross_actor_send`
  check (Austral research's key insight: Emerald already ships a narrow,
  single-point instance of exactly this rule), generalized from "only checked
  at a cross-actor send" to "checked at every `own`-typed call argument."
- **`borrow T`** — a shared, read-only reference. Rust's `&T`, spelled as a
  keyword rather than a sigil, matching Sable's own explicit anti-arrow,
  anti-punctuation stance and its own illustrative (not-finalized) sketch in
  §31 of the Sable design brief.
- **`borrow var T`** — an exclusive, mutable reference. Rust's `&mut T`,
  spelled by reusing plan 72's `var` keyword rather than inventing a second
  mutability spelling (Rust's own asymmetric `&`/`&mut` split, or a bespoke
  `mut`) — one keyword means "this can be mutated" everywhere in the
  language, on a binding (plan 72) or on a borrow (here).
- **No named lifetime parameters (`'a`) as a general type-system feature.**
  Rust needs these because a borrow's validity is an abstract quantity that
  can be threaded through arbitrary function/struct boundaries. Emerald
  already has a concrete, shipped mechanism for "how long does this memory
  live": regions (plans 50/51, `emerald_region_create`/`_alloc`/`_destroy`).
  A `borrow`'s validity is expressed relative to the region its target was
  allocated in, not an independent lifetime variable:

  ```ruby
  fn use_within(r: Region, x: borrow Int64) do
    # legal only while `r` (the region `x`'s target lives in) is alive
  end
  ```

  This is a real, deliberate scope reduction versus Rust's model — no
  function can return a `borrow` whose validity outlives the specific region
  it names, and no struct can hold a `borrow` parameterized over an abstract,
  caller-chosen lifetime the way `struct Parser<'a>` does in Rust. What this
  buys: the entire lifetime-elision rule set, and the variance question that
  comes with making lifetime parameters generic, simply don't need to exist,
  because there is no abstract lifetime parameter to elide or vary. This is
  the single biggest complexity cut this document makes, and it is only
  possible because Emerald already has regions to anchor validity to — Rust
  had no equivalent concrete mechanism to borrow this idea from.

## 3. What Emerald's object model already sidesteps, for free

Rust's hardest structural problem — self-referential structs — exists because
an ordinary Rust value can be moved by `memcpy` anywhere its owner goes,
which invalidates any pointer into that value's own fields the moment it
moves; `Pin`/`PhantomPinned` was built almost entirely to make `async fn`
state machines (which are self-referential by construction) possible despite
this. Emerald objects are never stack values relocated by `memcpy` — every
`ClassName.new` heap- or region-allocates via `emerald_alloc`/
`emerald_region_alloc` and is accessed through a pointer from the moment it's
created (`RUNTIME.md` §1). A reference into an Emerald object's own fields is
therefore never invalidated by the object "moving," because it never does.
This problem does not need a solution here — not because it was solved, but
because Emerald's existing allocation model never created it.

## 4. Relationship to escape analysis and regions (plans 50/51) — additive, not a replacement

Escape analysis remains exactly what it is today: the decision of *where* an
allocation goes (stack, region, or `emerald_alloc`'s unmanaged heap). The
borrow checker is a second, complementary source of proof, not a competing
one: a `borrow`-annotated parameter gives the compiler an explicit, checked
guarantee that a reference does not escape its region, which lets a caller's
allocation qualify for region placement in cases plan 50's automatic,
conservative escape analysis alone couldn't prove. Code that never uses
`own`/`borrow` annotations keeps today's exact behavior — this is additive,
not a breaking change to existing escape analysis. It also does not close
`RUNTIME.md` §1's "leaks forever" gap entirely: code that never opts into
these annotations is exactly as unproven as it is today. It narrows the gap
for code that does opt in; it does not eliminate it for code that doesn't.

## 5. Relationship to actors (plans 54-57) — coexist, don't subsume

The borrow checker governs aliasing and lifetime safety *within* one actor's
region. Plan 56's existing cross-actor linear-use send-check remains a
separate, additional layer specific to the actor boundary — this mirrors
Rust's own actual layering, confirmed by this session's research: Rust's
borrow checker has no cross-thread-safety mechanism of its own at all; that
job belongs to the entirely separate `Send`/`Sync` marker-trait pair, layered
on top of, not derived from, the borrow checker. Extending plan 56 into a
real, tiered capability system (a 3-tier `iso`/`ref`/`val`, per this
session's Pony research, closing the real gap where a `ref`-shaped value can
escape an actor without ever going through the syntactic send-check) is real,
disclosed future work — its own plan when it comes, not folded into 83-85.

## 6. Relationship to generics (plans 41/58/75) — deferred, not designed here

`own`/`borrow`/`borrow var` apply to concrete, non-generic parameter
positions in v1. A generic function bounded by an ownership-related
constraint (Rust's `T: Send` equivalent) is real future work, revisited once
plan 75's multi-trait-bound generics ships — this document does not attempt
to design that interaction now, on the same "don't guess ahead of a
prerequisite landing" reasoning plan 82 itself was authored under.

## 7. Relationship to FFI

This closes the question raised earlier in this project's history (an FFI
call returning heap data needs its lifetime bound to something, or it leaks
exactly like every other unmanaged `emerald_alloc` today) using the same
vocabulary as the rest of this document, not a separate convention: an
`unsafe extern "C"` function's return type is annotated `own T` (the
returned data is transferred into a region the *caller* controls and is
responsible for) or `borrow T` bound to a region the caller guarantees
outlives the call. No new mechanism — the FFI boundary is just another
`own`/`borrow`-typed call site.

## 8. Scope staged deliberately: lexical borrow-checking first, NLL-equivalent later

The Rust research is direct evidence for how to sequence this, not just what
to build: Non-Lexical Lifetimes took roughly six years from RFC to becoming
the sole checker (2016-2022), and Polonius — a fix to a *known, already-
diagnosed* gap in NLL itself — has taken eight years and counting, with a
dedicated compiler team, and its first working version (Polonius Alpha, 2026)
still doesn't accept every program the slow reference implementation did.
That is real evidence that flow-sensitive, liveness-based borrow checking is
a multi-year problem even for expert, sustained effort — building it as
Emerald's v1 would contradict this project's own engineering rule #10 ("keep
the initial compiler small enough that one person can understand the entire
pipeline").

**Plans 83-85 therefore target Rust's own *original* (pre-2018) lexical-
scope-based checker, not NLL.** A `borrow`'s live range is its entire
enclosing lexical scope, not "from creation to last use" — a strictly more
conservative rule that will reject some obviously-sound programs (the same
real, accepted cost Rust itself lived with for years). Flow-sensitive
refinement is real, named, future work — a candidate plan for whenever this
lands and proves itself, not attempted here.

## 9. Explicitly declined for v1

- **Variance/subtyping over borrow types.** Rust's `&mut T` invariance-in-`T`
  (the mechanism that closes a real soundness hole, at the cost of being one
  of the most confusing parts of the model to learn) has no equivalent here:
  v1 borrows are invariant everywhere — the simplest possible rule. Revisit
  only if a real program needs otherwise, not preemptively.
- **Higher-ranked trait bounds / `for<'a>`-style polymorphism.** No feature
  for "this callback must work for any lifetime the caller chooses" exists in
  v1.
- **No `Drop`/destructor mechanism distinct from region-destroy.** An
  `own`-typed value's cleanup remains "when its region is destroyed," not a
  per-value destructor hook run at point of last use. Named plainly because
  it's a real, disclosed gap versus Rust's model, not an oversight: an `own`
  parameter consumed mid-function does not get RAII-style cleanup the moment
  it's dropped — only at its region's eventual destruction.
- **Named lifetime parameters as a general type-system feature** — declined
  per §2; region-bound borrow validity substitutes for every case this
  project has identified needing it so far.

## 10. Plans 83-85, rescoped against this design

- **Plan 83 (`borrow-checker-sema-enforcement`)**: implement lexical-scope
  liveness checking in `emerald-sema` for `own`/`borrow`/`borrow var` per §2
  and §8. Reject: more than one live `borrow var` of the same binding within
  its lexical scope; any `borrow` coexisting with a live `borrow var` of the
  same binding; use of a binding after it's been passed as an `own` argument;
  a `borrow` whose named region is destroyed while the borrow's lexical scope
  is still live.
- **Plan 84 (`deterministic-destruction-codegen`)**: per §9's explicit
  decline of a Rust-style `Drop` mechanism, this plan's real scope is
  narrower than its original name suggested — no new destructor codegen is
  built. Scope becomes: codegen for `own`-parameter passing and
  `borrow`/`borrow var`-parameter passing.

  **Shipped (status: done, not just designed).** Two real findings changed
  this row's scope from what was originally assumed, both in `emerald-
  codegen`:
  - `own` needed NO new codegen at all. Plan 56's `is_cross_actor_send`
    check turned out to be sema-only (no runtime invalidation mechanism
    exists to "reuse" — `emerald-sema`'s liveness check IS the entire
    enforcement). An `own` transfer is either a class's existing
    pointer-copy or a primitive's existing scalar-copy — exactly what an
    unannotated parameter already does; the callee simply receives it.
  - `borrow`/`borrow var` of an ALREADY pointer-represented type (a class
    instance, `String`/`CString`) also needed no new codegen — this
    project's object model already passes those by pointer. Only a
    `borrow`/`borrow var` of a genuinely BY-VALUE type (`Int64`/`Float64`/
    `Boolean`/`Symbol`) needed a real, new mechanism: it compiles to an
    actual LLVM `ptr` parameter (`strip_ownership_in_type_expr`'s synthetic
    marker, `bind_params`'/`build_call_arg_vals`'s own doc comments) — the
    real design decision this row anticipated needing. Two disclosed, real
    scope limits: (1) this real-pointer treatment applies to top-level
    free-function parameters only, not class/actor/module methods (would
    require also updating `build_method_call`'s own separate argument-
    building code — not attempted); (2) a `borrow var` primitive parameter
    has no legal syntax to actually be mutated yet (`emerald-sema`'s
    plan-72 "no `var` slot on a parameter" rule) — the writeback mechanism
    is built and tested directly (bypassing sema, this codebase's own
    established test idiom), but not reachable from real `.em` source
    until that separate, disclosed sema gap is closed. Zero-cost proven
    via `examples/ownership_zero_cost_benchmark.em` (statistically
    indistinguishable elapsed time, plain vs. `borrow Int64`) for the
    common case (a plain local-variable argument reuses its own existing
    stack slot); a non-local argument (a literal, a computed expression)
    pays one real, disclosed extra `alloca`+store, which no unannotated
    parameter ever needed.
- **Plan 85 (`ownership-actor-ffi-integration`)**: implement §5 (borrow
  checker and plan 56 coexisting as separate layers, not one subsuming the
  other) and §7 (FFI `own`/`borrow` typing at `extern` boundaries with
  region-transfer semantics).
