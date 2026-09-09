---
name: Compile-Time Message Safety
overview: "A sema-level check on cross-actor message payloads (plan 55's mechanism): legal iff the argument is a value type, or a reference type provably not read again by the sender after the send — Pony-lite, not Pony: one linear-use rule, not iso/val/ref/box/tag capability polymorphism."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-payload-classification
    content: "Classify every argument expression at a cross-actor call site into value-type (always legal), trivially-fresh reference (always legal), named-local reference (subject to the liveness check), or aliasing-shape reference (rejected outright, undecided future work)"
    status: pending
  - id: leaf-liveness-analysis
    content: "The forward, branch-merging, loop-conservative moved-set analysis over a function body's statements after a send point, integrated alongside emerald-sema's existing check_stmt/check_block traversal"
    status: pending
  - id: leaf-diagnostics
    content: "The 'sent here, used again here' diagnostic naming both sites, using plan 22's real spans where available and a structural statement-position fallback otherwise; regression proof against every prior plan's example"
    status: pending
isProject: false
---

# Plan 56 — Compile-Time Message Safety

This is plan 56 of the 48-57 batch implementing "Beyond the Ceiling" in
full — the actor-model concurrency pillar that follow-up analysis
proposed, explicitly citing Pony (a statically-typed actor language
whose compile-time reference-capability system — `iso`/`val`/`ref`/
`box`/`tag` — proves message safety without a tracing GC) as precedent.
It is the third of four concurrency-pillar plans in this batch: 54
(actors + isolated heaps) → 55 (scheduler + messaging) → **56 (this
plan, compile-time message safety)** → 57 (supervision trees). Like
plans 17 and 31 before it, this is post-v1 scope — `plan-of-plans.md`'s
own completion note already calls Emerald v1 done at row 15 — and this
plan does not touch `plan-of-plans.md` or any other plan file.

As of this session, neither plan 55 (`scheduler-and-message-passing`)
nor plan 51 (`scope-based-arena-allocation`) exists on disk yet — both
are being authored in parallel in this same batch. This plan proceeds
on the assumed contracts stated in the Decision log below; if either
lands with materially different mechanics before this plan is
implemented, reconciling those two Decision logs against this one is a
prerequisite, not a detail to paper over.

Concrete proof this plan targets — a class instance sent to a spawned
actor and never touched again by the sender:

```ruby
class LogMessage
  text: String
  def initialize(text: String) -> Void
    @text = text
  end
end

actor Logger
  def log(msg: LogMessage) -> Void
    puts msg.text
  end
end

def main() -> Void
  logger: Logger = Logger.spawn
  msg: LogMessage = LogMessage.new("hello from main")
  logger.log(msg)
end
```

`msg` is a reference-typed local (a `LogMessage` instance, per plan
51's assumed contract arena-allocated in `main`'s own scope); `logger`
is a `Logger` actor reference. `logger.log(msg)` is a cross-actor send
(per plan 55's assumed contract: it lowers to an enqueued mailbox
message, not a synchronous call). `msg` is never read again anywhere
after that statement in `main` — this plan's check passes, `main`
compiles clean, and the program links and runs, printing `hello from
main` once `logger`'s mailbox is drained. The negative counterpart —
the same program with one added line that reads `msg` after the send —
is this plan's other required proof; see leaf-diagnostics below for the
full source and the exact diagnostic it produces.

## Decision log

- **The one rule, stated precisely:** a cross-actor message argument is
  legal iff it is (a) a value type (`Int64`/`Float64`/`Boolean`, plus
  whatever variant plan 44 (`symbols`) adds to `emerald_sema::Type` for
  `Symbol` — copied at the send, so aliasing is a non-concern by
  construction), or (b) a reference type whose sending expression is
  provably the last use of that local in the sending function. This is
  **Pony-lite, not Pony**: Pony's real answer to this problem is a
  capability-polymorphic type system — four to five reference-capability
  kinds (`iso` uniquely-referenced-and-mutable, `val` immutable-and-
  shareable, `ref` ordinary mutable, `box` read-only, `tag` identity-
  only) attached to every type, with a subtyping/viewpoint-adaptation
  system deciding at every call site which capabilities may legally
  flow where, plus `recover` blocks to locally regain isolation. That
  system is what makes Pony's actors provably data-race-free under
  arbitrary aliasing, not just under a single send. This plan
  deliberately does not build that system — no `iso`/`val`/`ref`/`box`/
  `tag` annotations exist anywhere in Emerald's type grammar, no
  capability subtyping, no `recover`. It builds the smallest rule that
  is still sound for the one case Emerald's actors actually need at
  this stage: a straight-line "did the sender use this again" check.
  Pony is cited here as the aspirational fuller version of this feature,
  not as a design this plan claims to have implemented a subset of in
  any formal sense — the two type systems are not comparable in power,
  only in the problem they both aim at.
- **Why a linear-use check is the right proxy for ownership transfer,
  tied concretely to plan 51/54's arena model:** plan 51's assumed
  contract gives every non-escaping allocation a scope-bound arena
  lifetime; plan 54's assumed contract gives each actor its own
  isolated heap. A class instance allocated in `main`'s arena and handed
  to another actor's mailbox is only safe if `main`'s arena reclaiming
  that memory (when `main`'s scope ends) can never race with the
  receiving actor still reading through its own copy of the reference.
  The runtime mechanics of actually moving ownership of the allocation
  from the sender's arena into the receiver's heap (or into a message-
  owned allocation) are plan 51/55's job, not this plan's — this plan
  supplies only the static precondition that makes such a transfer
  sound: proof that the sender holds no remaining path to the object
  after the send. "Provably last use" is exactly that proof, in the
  cheapest form that's still real: if the sender's own function body
  never reads the local again, the sender cannot race the receiver
  through that local, full stop. This is intentionally *not* a proof
  that no alias exists anywhere in the program (a field could have
  copied the reference out earlier) — see leaf-payload-classification's
  aliasing-shape rejection below for how this plan closes that gap by
  restricting the checked shape, not by proving non-aliasing.
- **Where this runs in the pipeline:** `emerald-sema`, in the same pass
  as existing type-checking, not a separate pass over already-checked
  code. Concretely (verified this session against
  `crates/emerald-sema/src/lib.rs`): `check_stmt` (L893-1223, the
  per-statement dispatcher already threaded with `env: &mut
  HashMap<String, Type>` through every `Stmt` variant) and `check_block`
  (L1362-1387, the sequential-statement driver `check_function_body`/
  `check_method_body`/every branching construct's own body ultimately
  calls) are where a cross-actor `Expr::MethodCall` argument list is
  already visited for ordinary type-checking (`check_args`, L781-807).
  This plan's check rides along the same traversal rather than
  re-walking the AST separately: `check_block` gains a second piece of
  state threaded exactly like `env` already is — see leaf-liveness-
  analysis for its shape — updated at every `Stmt::Expr(Expr::
  MethodCall(...))` that resolves to a cross-actor send, and consulted
  at every `Expr::Ident` read anywhere in `infer_expr_type`.
  `emerald_sema::Diagnostic` today (`lib.rs:43-45`) is a bare `{
  message: String }` with no position field at all — the same
  disclosed gap plans 13 and 17 already accepted and plan 22
  (`sema-diagnostic-spans`) is the named fix for. This plan's
  diagnostics use plan 22's real `(usize, usize)` byte spans when that
  plan has landed by the time this one is implemented (pointing
  independently at the send site and the later use site, exactly the
  "moved, used again on line N" shape plan 22's own worked example
  demonstrates for a type mismatch); until then, this plan's own
  `Diagnostic::message` text names the local and both statement roles
  structurally (see leaf-diagnostics) rather than silently claiming a
  line number that doesn't exist yet.
- **What makes a call a "message send" at all — plan 55's assumed
  contract:** a `receiver.method(args)` (`Expr::MethodCall`) whose
  receiver's static `Type` is an actor (plan 54's assumed contract:
  either a dedicated `Type::Actor(String)` variant or a `ClassInfo`-
  shaped registry entry flagged `is_actor: true` — this plan does not
  need to guess which, only that sema can ask "is this receiver's type
  an actor" the same way it already asks "is this receiver's type a
  known class" for `MethodCall`/`New`). Plan 55's assumed contract lowers
  such a call to an enqueued mailbox message (method tag plus argument
  values) rather than an ordinary synchronous call; this plan's check
  runs purely at the sema layer, before that lowering choice is made in
  codegen, and only needs the actor-typed-receiver fact to decide the
  call's argument list is a message payload subject to this rule. A
  same-actor or plain-class method call is completely unaffected — this
  plan adds zero new restrictions to ordinary, non-actor method calls.
- **Control flow: a real, disclosed conservative approximation, not
  full path-sensitivity.** The naive rule "any use after the send,
  textually" is wrong in two directions Emerald's existing `if`/`while`/
  `for`/`case`/`begin` constructs actually exercise, so this plan
  commits to one concrete, conservative resolution for each:
  - **Branches (`if`/`else`, `case` arms, `rescue` clauses):** a send in
    one branch and a use in a *different, mutually exclusive* branch is
    safe — the two can never execute on the same run, so nothing races.
    Each branch is checked starting from the moved-set snapshot at the
    branch point, independently of its sibling branches.
  - **After the branch construct rejoins:** conservative merge, stated
    plainly because it's a real over-approximation, not a hidden one —
    once the branches rejoin, the moved-set carried into the following
    code is the *union* of what every branch did, not the intersection.
    If *any* branch sends local `x`, code after the whole `if`/`case`
    that reads `x` is rejected — even on the runs where the branch that
    actually executed never touched `x` at all. This is deliberately
    the same "assume the worst reachable path" posture true path-
    sensitive analysis would improve on; this plan does not attempt
    that improvement (see below).
  - **Loops (`while`, `for`, `for...in` range):** a loop body can run
    zero, one, or many times, so "textually after" has no fixed meaning
    inside one. This plan's rule: a send anywhere inside a loop body is
    checked against every other use anywhere else in that same loop
    body, regardless of which comes first in source order (a second
    iteration could execute the send before a later-in-text use from
    the first iteration's perspective actually runs) — the loop body is
    treated as if concatenated with itself once for this purpose. A
    send that occurs anywhere inside a loop body also poisons that local
    for all code after the loop exits, exactly like the branch-merge
    rule above, since a later iteration could reach the send before the
    loop's final exit.
  - This is a real, load-bearing decision, not a placeholder: it means
    this plan will reject some genuinely safe programs (a send in an
    `if` branch immediately followed, after the `if`, by code that
    provably only runs when the *other* branch was taken — undetectable
    without real path sensitivity). That is accepted, disclosed
    conservatism, matching this plan's own "smallest useful rule"
    mandate — a false rejection is a compile error the programmer can
    work around (restructure the send, or don't reuse the name); a
    false acceptance would be a real, silent memory-safety hole.
- **What counts as a "reference type" for this rule — String included,
  deliberately.** `Int64`/`Float64`/`Boolean`/`Symbol` are the only
  types this plan treats as copied value types. Notably `String` is
  *not* on that list, even though nothing else in Emerald's type system
  currently distinguishes it from a scalar: under plan 51's assumed
  arena model a `String` value is heap/arena-backed like a class
  instance, not a fixed-size `Copy` scalar, so it gets the same
  last-use scrutiny as any `Class`/`Array`/`Hash`/`Proc` value. `Nil`
  carries no data at all (nothing to own, nothing to race), so it's
  trivially safe and isn't gated by this rule either way, but it isn't
  being called a "value type" for this rule's purposes — it's simply
  outside the rule's concern.
- **Explicitly declined, real future work, not silently out of scope:**
  - Full Pony-style capability polymorphism — multiple reference-
    capability kinds, capability subtyping/viewpoint adaptation,
    `recover` blocks. This is the single largest piece of real,
    substantial future work this plan leaves on the table; see the
    first bullet above for why it isn't attempted here.
  - Extending the last-use carve-out beyond bare named locals to field
    reads (`@field`) or index reads (`arr[i]`) as message arguments —
    both can alias a binding that outlives the send (a field stays
    reachable via `self`; an array element stays reachable via the
    array), and proving otherwise needs real alias analysis this plan
    doesn't attempt. See leaf-payload-classification: these shapes are
    rejected outright, not analyzed.
  - Compile-time cycle/deadlock detection across actors. A real, Pony-
    adjacent concern (mailbox back-pressure and cross-actor call cycles
    can deadlock a scheduler), but a distinct static-analysis problem
    from "is this one message payload safe to send" — out of scope for
    this plan, not attempted piecemeal here, and not assumed to be plan
    57's job either (supervision trees are a runtime-recovery answer to
    actors *crashing*, not a static answer to actors *deadlocking*).
  - Interprocedural freshness proofs for method-call-result arguments
    (`logger.log(factory.build())`) — see leaf-payload-classification;
    this plan rejects these outright rather than attempting to prove a
    callee's return value is unaliased.

## Leaf: leaf-payload-classification

### 1. Context
- Why: the one rule only says what happens to a *named local*; every
  message argument in a real program is one of several distinct
  expression shapes, and the rule has to say what happens to each
  before the liveness analysis (leaf-liveness-analysis) has anything to
  operate on.
- Target state, per argument expression at a cross-actor send site:
  1. **Value type** (`Int64`/`Float64`/`Boolean`/`Symbol`, whatever the
     expression shape) — always legal, no further check.
  2. **Trivially-fresh reference** — `Expr::New`, `Expr::ArrayLit`,
     `Expr::HashLit`, `Expr::ArrayNew`, `Expr::Lambda`, `Expr::
     StringLit`, `Expr::Interpolate` — always legal: each constructs a
     value with no prior binding, so there is nothing else that could
     read it after the send by construction. `Expr::Nil` is likewise
     always legal (no data to own).
  3. **Named-local reference** — a bare `Expr::Ident(name)` whose type
     is not a value type — subject to leaf-liveness-analysis.
  4. **Aliasing-shape reference** — `Expr::InstanceVar`, `Expr::Index`,
     `Expr::Call`, `Expr::MethodCall` (as the argument expression
     itself, not as the enclosing send) — rejected outright with a
     dedicated diagnostic ("message arguments must be a value, a freshly
     constructed value, or a local variable used for the last time —
     `@field`/`array[i]`/a nested call result cannot be proven unaliased
     by this compiler"). Not analyzed, not partially supported — a
     real, disclosed limitation, not silently miscompiled.
- Why bucket (4) exists at all rather than only supporting bucket (3):
  the user-facing rule text ("a reference type whose sending expression
  is provably the last use of that local") is about locals specifically
  — an `@field` or `arr[i]` read has no "local" for the liveness
  analysis to track use-after-send against, and this plan does not
  invent alias tracking for `self`'s fields or array contents to make
  one up. Rejecting outright is the conservative-safe answer, matching
  this plan's own "smallest useful rule" mandate.

### 2. Acceptance Criteria
1. Every value-type argument (four `Type` cases plus `Nil`) to a
   cross-actor send compiles regardless of expression shape or prior
   use of any local it happens to reference.
2. Every trivially-fresh-reference argument shape (7 `Expr` variants
   listed above) compiles unconditionally, including when constructed
   from locals internally (e.g. `LogMessage.new(some_string)` — the
   `LogMessage.new(...)` result itself is fresh even though `some_string`
   is a local; `some_string`'s own last-use status is checked separately
   as `New`'s own argument, recursively, by the same classification).
3. `@field`, `arr[i]`, a bare `Call`/`MethodCall` result passed directly
   as a message argument are each rejected with the dedicated
   diagnostic from bucket (4), naming the argument's position and the
   send's method name.
4. Regression: every prior plan's example (none of which use actors,
   since plan 54 hasn't landed) is completely unaffected — this leaf
   only adds behavior at call sites whose receiver type is an actor,
   which cannot exist in any pre-batch-48 example.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (`check_args` or a new
  sibling `check_message_args`, called instead of `check_args` once
  `check_stmt`'s `MethodCall` handling determines the receiver is
  actor-typed).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. all four argument-shape cases above | agent-claimed-locally |

---

## Leaf: leaf-liveness-analysis

### 1. Context
- Why: leaf-payload-classification identifies *which* arguments need a
  last-use proof; this leaf is the proof itself.
- Target state: a `HashMap<String, MoveRecord>` (`MoveRecord` holding
  whatever site-identifying data leaf-diagnostics needs — see there)
  threaded through `check_block`/`check_stmt` exactly the way `env:
  &mut HashMap<String, Type>` already is, scoped per function body
  (fresh at `check_function_body`/`check_method_body`'s entry, not
  shared across functions — a send in one function says nothing about
  a same-named local in another). Two operations:
  - **Record a move:** at a cross-actor send whose argument is a
    named-local reference (bucket 3), after confirming the local isn't
    already in the moved-set (a local sent twice is caught by the
    ordinary "used again" diagnostic on the *second* send — no separate
    double-send diagnostic needed), insert `name → MoveRecord{ site }`.
  - **Check a read:** every `Expr::Ident(name)` visited anywhere by
    `infer_expr_type` (field access target, another call's argument,
    another send, a `Return`, a `Compare`/`Add` operand, everything —
    "any use" means literally any AST position that reads the
    identifier) consults the moved-set first; if present, that's the
    rejection, using the recorded site plus the current position.
  - **Branch merge:** `check_stmt`'s `If`/`Case`/`Begin` handling
    (L893-1223, L1231-1285, L1299-1359) forks the moved-set into a
    clone per branch, checks each branch against its own clone, then
    unions all branches' resulting moved-sets (plus whatever was
    already moved before the branch) into the moved-set carried into
    subsequent statements — implementing the Decision log's conservative
    merge rule directly as a `HashMap` union.
  - **Loop conservatism:** `check_stmt`'s `While`/`For`/`ForRange`
    handling checks the loop body once against a moved-set clone, then
    checks the body a *second* time against the moved-set produced by
    the first pass (catching a second-iteration use of a first-
    iteration send) before unioning the result into the post-loop
    moved-set — a direct, mechanical implementation of "treat the body
    as concatenated with itself once."
- Why a `HashMap` keyed by plain `String` name, not a de Bruijn-style
  scope-aware index: Emerald has no shadowing today (verified this
  session — `check_stmt`'s `Let` case rejects redeclaring an existing
  `env` key as a diagnostic, the same rule this plan's moved-set keys
  off of), so a bare name is already an unambiguous key everywhere
  `env` itself uses one; this leaf doesn't need a richer key than the
  type-checker's own `env` does.

### 2. Acceptance Criteria
1. This plan's own top-level worked proof (`msg` sent once, never read
   again) type-checks `Ok(())`.
2. A send followed immediately by a read of the same local in the same
   straight-line block is rejected (the core case; full diagnostic text
   is leaf-diagnostics' concern).
3. A send inside one `if` branch and a read of the same local only in
   the *other* branch is accepted — the required proof that branches
   are not naively merged as "textually after."
4. A send inside one `if` branch and a read of the same local in the
   code *after* the whole `if`/`else` is rejected, even though only one
   branch could have executed it — the required proof of the
   conservative merge rule, not full path sensitivity.
5. A send on a loop's first statement and a read on the loop's last
   statement (same body, same iteration) is rejected — the required
   proof loops don't get an accidental pass from "used before its own
   send, textually."
6. A read of an already-sent local placed *after* the loop entirely is
   rejected — the required proof a send anywhere in a loop body poisons
   the local for all code following the loop.
7. Regression: every prior plan's example still type-checks identically
   (none touch the moved-set machinery at all, since none contain an
   actor-typed receiver).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (`check_stmt`,
  `check_block`, `check_case`, `check_begin`, `infer_expr_type`'s
  `Expr::Ident` arm).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. all seven cases above | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-diagnostics

### 1. Context
- Why: leaf-liveness-analysis detects the violation; this leaf makes it
  legible — naming both the send site and the illegal later-use site,
  the concrete requirement this plan was given.
- Target state: `MoveRecord` carries enough to render "local `{name}`
  was sent to an actor here and cannot be used again" pointing at the
  send, plus a second diagnostic (or a single multi-span one, if plan
  22's `Diagnostic` shape supports secondary spans by the time this is
  implemented — if not, two `Diagnostic`s, both returned from
  `check_program`, same as any other plan that already reports more
  than one error) pointing at the illegal use. Precision is real but
  disclosed exactly per whether plan 22 has landed, the same discipline
  plan 22 itself used for its own diagnostic classes:
  - **If plan 22 has landed:** both diagnostics carry the real
    `(usize, usize)` byte span of the send expression and the use
    expression respectively — a direct instance of plan 22's own stated
    target state ("Type-mismatch diagnostics ... point at the actual
    mismatched sub-expression's own span"), generalized to this plan's
    two-site case.
  - **If plan 22 has not landed:** `Diagnostic.message` (the only field
    that exists) names the local, the sending method call's callee name
    and argument position, and the using statement's structural position
    (its index within the enclosing function body, e.g. "statement 3 of
    `main`") — enough to disambiguate the two sites in a short function
    without fabricating a line number the compiler cannot actually
    compute yet. This is the same disclosed-gap posture plans 13 and 17
    took before plan 22 existed at all.

### 2. Acceptance Criteria
1. **Positive proof** (this plan's top-level worked example, verbatim):
   compiles, links, and runs, printing `hello from main`.
2. **Negative proof** — the same program with one line added:
   ```ruby
   def main() -> Void
     logger: Logger = Logger.spawn
     msg: LogMessage = LogMessage.new("hello from main")
     logger.log(msg)
     puts msg.text
   end
   ```
   is rejected at compile time with two diagnostics (or one two-span
   diagnostic, per whichever shape is available): one naming the send
   site (`logger.log(msg)`) as where `msg` was sent, one naming the use
   site (`msg.text` inside `puts msg.text`) as the illegal later read —
   verified directly against the returned `Vec<Diagnostic>` from
   `check_program`, not just "it errors."
3. The diagnostic text (or span target, once plan 22 lands) is
   distinguishable from an ordinary undefined-variable or type-mismatch
   diagnostic — a reader must be able to tell "this failed because of
   the message-safety rule," not a generic error.
4. Regression: `cargo test --workspace` — every prior plan's example
   (`hello.em`, `Point`, collections, exceptions, modules, inheritance,
   blocks, the full exception model) still parses, type-checks, and
   (where applicable) runs identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (`Diagnostic`
  construction sites added by leaf-liveness-analysis); `crates/
  emerald-cli` only if its miette rendering needs a new diagnostic
  category label — no behavior change to rendering itself.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run positive proof and the rejected-with-two-site-diagnostic negative proof | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
