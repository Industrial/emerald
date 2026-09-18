---
name: Ownership Model Design
overview: "Design-only — no code. Reopens spec/RUNTIME.md §1 and inception.md §12's explicitly-deferred question, per this session's decision to commit to a full Rust-style borrow checker rather than the lighter escape-analysis extension. Sable's own brief admits (§47) it hasn't settled the syntax either, only the goal — this plan's job is to actually decide what plans 83-85 will build, against real, currently-shipped Emerald constraints: the actor-per-instance-region model (plan 54), FFI's not-yet-designed region-bound return convention (raised but not planned in this session's earlier FFI conversation), plan 72's shipped mutability semantics, and generics (plan 41/58, extended by plan 75). Output is a new spec document (or a new section of spec/RUNTIME.md), not a compiler change."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-survey-real-prior-art
    content: "Before designing anything, survey how existing systems languages actually solved this, honestly assessed for what transfers and what doesn't: Rust's own borrow checker (NLL, the region-inference literature it's built on), Austral's linear types (a smaller, more tractable ownership discipline than full Rust lifetimes, explicitly designed for simplicity), Vale's region-based memory management (closer in spirit to Emerald's existing arena model than Rust's lifetimes are), and Pony's reference capabilities (directly relevant since Emerald's plan 56 already ships a Pony-lite linear-use check for cross-actor sends — is that mechanism extensible into the new model, or does it get superseded?)."
    status: pending
  - id: leaf-decide-syntax
    content: "Decide concrete own/borrow syntax (Sable's own §31 offers only illustrative, explicitly-not-finalized examples: fn process(data: borrow Data), fn consume(data: own Data)) — or a different spelling if the survey above surfaces a better-fitting precedent. Record the actual grammar, not a sketch."
    status: pending
  - id: leaf-reconcile-with-regions-and-actors
    content: "Decide explicitly whether the borrow checker replaces plans 50/51's escape-analysis/region-arena mechanism, subsumes it as an implementation detail underneath real ownership tracking, or coexists with it as a distinct, lower-level tool. Decide how an actor's per-instance region (plan 54) interacts with borrowed references to actor state — can a borrowed reference to actor-owned data ever legally cross the actor boundary plan 56's linear-use check already restricts, or does ownership analysis subsume that check entirely?"
    status: pending
  - id: leaf-reconcile-with-mutability-and-generics
    content: "Decide how borrow/own interacts with plan 72's var/immutable-by-default bindings (does a borrow require the underlying binding to be var to be borrowed mutably, mirroring Rust's & vs &mut?) and with generic type parameters (plan 41/58/75) — can a generic function be bounded by an ownership-related constraint the way Rust bounds by Send/Sync?"
    status: pending
  - id: leaf-reconcile-with-ffi-ownership
    content: "This session's earlier FFI/stdlib conversation raised — but did not plan — the question of how a Rust-crate-backed stdlib call's returned heap data gets its lifetime bound to a region rather than leaking. Decide whether that FFI-ownership question is answered by this same ownership model (a borrowed/owned reference crossing the FFI boundary) or needs its own separate convention; if the latter, say so plainly rather than silently assuming this plan covers it."
    status: pending
  - id: leaf-spec-document-and-scope-plans-83-85
    content: "Write the actual design output — a new spec/OWNERSHIP.md or a new section of spec/RUNTIME.md — and use its conclusions to write real, concrete scope descriptions for plans 83 (sema enforcement), 84 (codegen/destruction), and 85 (actor/FFI integration) in the plan-of-plans table, replacing their current 'scope set by this plan' placeholders."
    status: pending
isProject: false
---

# Plan 82 — Ownership Model Design

This plan is deliberately not an implementation plan. Committing to "a
full Rust-style borrow checker" (this session's decision) without first
answering the questions Sable's own brief admits are still open (§47:
"How much of Rust's ownership model should be visible?") would mean
plans 83-85 are guessing at a design nobody has actually made. This
plan makes it.

## What "done" looks like — no worked-example program, a document instead

Unlike every other plan in this batch, this plan's deliverable is not a
compiling `.em` program — it's a spec document precise enough that
plan 83 can be scoped and authored directly from it, the same way
plan 65 could cite exact file/line evidence because plans 51-57's real
shipped shape was already settled before it was written. This plan's
job is to make plans 83-85 possible to write with that same precision,
not to write them itself.

## Decision log

- **Why this can't be skipped or compressed into plan 83.** Every
  other design decision in this session's batch (immutability, Option
  vs. nullable, chaining) had a source document that already stated
  the target shape concretely enough to write a concrete-proof example
  against. Ownership is the one place Sable's own brief explicitly
  declines to do that — §47 lists "ownership syntax," "closure
  captures... ownership and mutability," and "lifetime exposure" as
  open questions in its own words. Treating this as a normal plan with
  a worked example would mean inventing Sable's own undecided design
  inside a plan meant to implement it, which is backwards.
- **Real, named tension with the existing region/arena model.** Plans
  50/51 already give Emerald a working, shipped, tested memory
  discipline (escape analysis → stack allocation or arena-scoped
  allocation) that this session's original "keep it locked" framing
  (from earlier in this conversation, before Sable was introduced)
  assumed would remain permanent. A real borrow checker is a different
  and larger mechanism than region inference — Rust itself has no
  region-arena escape hatch of this shape; its allocator is ordinary
  heap allocation disciplined entirely by the borrow checker. Whether
  Emerald ends up with *both* mechanisms serving different purposes, or
  the borrow checker fully subsumes what regions did, is exactly the
  kind of question `leaf-reconcile-with-regions-and-actors` exists to
  answer deliberately rather than let drift.
- **Prior-art survey comes first, deliberately, before syntax.**
  Copying Rust's own lifetime syntax wholesale would contradict Sable's
  own stated goal (§31: "investigate whether all of Rust's ownership
  and lifetime syntax needs to be exposed to the programmer") and this
  project's own inception-level engineering rule #4 ("Do not import
  another language's type system wholesale"). Austral and Vale in
  particular are named because they represent real, shipped attempts
  at *simplifying* Rust's model rather than reproducing it — exactly
  the design space this plan needs to search before defaulting to
  "just do what Rust does."
