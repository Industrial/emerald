---
name: Full Exception Model
overview: "`ensure`, multiple typed `rescue` clauses tried in order (with plan 32 subtype-aware matching), a bare catch-all `rescue => e`, and `retry` — turning plan 11's proof-of-concept single-rescue setjmp/longjmp handler into a genuinely complete exception model without adding a tracing GC, unwind tables, or any reflection."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-full-exceptions
    content: "Stmt::Begin gains rescues: Vec<RescueClause> (class_name: Option<String>, var, body) and ensure: Option<Vec<Stmt>>; new Stmt::Retry; grammar grows RescueClause+, an optional EnsureClause, and a reserved `retry` keyword"
    status: pending
  - id: leaf-sema-full-exceptions
    content: "check_begin loops over rescues in source order, binds each typed clause's var (bare clauses bind nothing — no universal root class exists to type them at), threads a new in_rescue bool for retry validity exactly like the existing in_loop bool"
    status: pending
  - id: leaf-codegen-ensure
    content: "ensure_stack threaded through build_stmt/build_block like loop_stack; ensure body codegen is duplicated at try-normal-exit, rescue-match-normal-exit, the mismatch-exhausted re-raise path, and every Return/Break/Next/Raise inside the guarded region"
    status: pending
  - id: leaf-codegen-multi-rescue
    content: "N rescue clauses tried via a chained conditional-branch loop (build_case's own arm/next-check idiom); a rescue naming a superclass matches any subclass by OR-ing build_int_compare(EQ) over that type's precomputed descendant-tag set; a bare clause matches unconditionally"
    status: pending
  - id: leaf-codegen-retry
    content: "Stmt::Retry branches to a per-Begin retry_blk (tracked on a new retry_stack) that redoes push_handler+setjmp from scratch; retry never re-runs the enclosing ensure, matching Ruby's own retry semantics"
    status: pending
isProject: false
---

# Plan 38 — Full Exception Model

This is one of twelve independent sibling plans in the 36–47 follow-up
batch, whose combined purpose is closing Emerald's language/stdlib
surface to its strategic ceiling — roughly 45% of standard Ruby's
surface — without conceding any of the four identity constraints (no
`method_missing`/`eval`/`send`/reflection, no mixins/open classes/
monkey-patching, no dynamic dispatch or vtables, no tracing GC). Like
the 28–35 batch before it, this is post-v1 scope and is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md);
`plan-of-plans.md` itself will be updated separately, once, after all
twelve plans in this batch are authored — this plan does not touch it
or any other plan file. This plan owns exceptions only; the other
eleven gaps are separate sibling plans written independently in the
same batch.

This is the direct, named follow-up to two already-*implemented* prior
plans (verified this session against real, current source — plans
28–35's own commit already landed all of them, so this plan extends
working code, not a still-hypothetical design):

- **Plan 11 (exceptions)** built a single-typed-`rescue`, no-`ensure`,
  no-multi-`rescue` setjmp/longjmp handler stack. Verified against
  current source: `crates/emerald-parser/src/ast.rs:179-184`'s
  `Stmt::Begin { body, rescue_type: String, rescue_var: String,
  rescue_body }` is exactly one clause, no ensure field anywhere;
  `crates/emerald-codegen/src/lib.rs:2879-3043`'s `build_begin` pushes
  one handler, calls `setjmp` once, and branches into exactly one
  `rescue_type` tag comparison; `runtime/emerald_runtime.c`'s
  `EmeraldHandler`/`emerald_push_handler`/`emerald_handler_jmpbuf`/
  `emerald_pop_handler`/`emerald_free_handler`/`emerald_handler_tag`/
  `emerald_handler_exception_ptr`/`emerald_raise` are the real,
  currently-compiling handler-stack runtime this plan reuses unchanged.
- **Plan 32 (class inheritance)** built the only inheritance-chain
  representation this compiler has. Verified against current source:
  `crates/emerald-sema/src/lib.rs:153-182`'s `resolve_chain(name,
  classes) -> Result<Vec<String>, Diagnostic>` walks `ClassInfo.
  superclass` links and returns root-to-leaf order; `crates/emerald-
  codegen/src/lib.rs:146-170`'s `resolve_class_chain` independently
  re-derives the identical chain straight from `&ClassDef` (this
  codegen backend has no shared sema→codegen data structure — a
  standing fact since plan 06, reconfirmed by plan 32's own Decision
  log). `Ctx.class_tags: &HashMap<String, i64>` (`lib.rs:659-664`)
  already carries a doc comment stating plainly that `rescue` is
  "still exact-tag equality even after plan 32 added inheritance —
  upgrading `rescue` to subtype-aware matching is real, disclosed
  future work" — this plan is that disclosed future work.

Concrete proof — raising inside a `begin`, caught by the *second* of
two typed `rescue` clauses (proving ordering), with an `ensure` that
always prints its cleanup line (proving ensure-always-runs), all in one
compiled/linked/run trace:

```ruby
class NotFoundError
  code: Int64

  def code -> Int64
    @code
  end
end

class TimeoutError
  code: Int64

  def code -> Int64
    @code
  end
end

def risky(x: Int64) -> Int64
  if x > 100
    raise TimeoutError.new(7)
  end
  return x
end

begin
  puts risky(999)
rescue NotFoundError => e
  puts e.code
rescue TimeoutError => e2
  puts e2.code
ensure
  puts "cleanup"
end
```

Expected stdout, in order:
```
7
cleanup
```

Trace: `risky(999)` raises `TimeoutError.new(7)`, which `longjmp`s back
to this `begin`'s handler. The first clause (`NotFoundError`) checks
its tag, doesn't match, and falls through to the second clause
(`TimeoutError`) rather than exhausting and re-raising — proving
ordering. The matched clause runs, printing `7` via `e2.code`. Control
then reaches the `begin`'s shared exit, where `ensure` unconditionally
runs and prints `cleanup` — proving ensure always runs on the path this
program actually takes. (A left-to-right implementation that skipped
straight to the last-declared clause, or that only ran `ensure` on the
*non*-exceptional path, would both silently fail this trace.)

## Decision log

- **`ensure` codegen strategy: duplicate the ensure body's codegen at
  every real exit point, via a new `ensure_stack` parameter threaded
  through `build_stmt`/`build_block` exactly the way `loop_stack:
  &mut Vec<LoopTargets<'ctx>>` already threads through both (verified:
  `build_stmt`'s and `build_block`'s current signatures at `crates/
  emerald-codegen/src/lib.rs:2227-2238` and `:3051-3060` both already
  carry `loop_stack` as a plain parameter for `break`/`next` to consult
  — the exact same shape this plan reuses for `ensure_stack`). This is
  chosen over a single shared "cleanup label" every exit path jumps to
  because that idiom belongs to real unwind-based exception handling
  (LLVM's `invoke`/`landingpad`/personality-function mechanism) — and
  this codebase deliberately has none of that: `build_begin`'s entire
  control flow (`lib.rs:2879-3043`) is plain `build_conditional_branch`
  over `setjmp`'s integer result, with zero `invoke`/`landingpad` calls
  anywhere in the file (confirmed via a full signature scan of `crates/
  emerald-codegen/src/lib.rs`). With no such primitive to attach a
  shared label to, duplicating the (always small, always statically
  known) `ensure` body's codegen at each of the few real exit sites is
  the smaller, more consistent extension of a codebase that already
  duplicates other codegen shapes (e.g. `Ctx.class_tags`'s own
  precedent of "compute a small compile-time-known table once").
- **`ensure` has three real trigger sites, not two.** The prompt's
  framing names "normal fallthrough" and "matching rescue body exit,"
  but re-reading the actual current `build_begin` surfaces a third:
  the **mismatch-exhausted path**, where a caught exception's tag
  doesn't match *any* of this `begin`'s rescue clauses. Verified against
  the current `mismatch_blk` (`lib.rs:2973-2996`): it already frees the
  handler and unconditionally calls `emerald_raise` to propagate to the
  next-outer handler, with the block ending in `build_unreachable`
  immediately after — zero opportunity today for anything to run first.
  Plan 38's `build_begin` inserts the `ensure` body's codegen into this
  path too, immediately before the propagating `emerald_raise` call —
  an easy exit to silently miss (it doesn't "feel" like leaving the
  `begin` construct from inside a `rescue` clause, since none of this
  `begin`'s own clauses actually ran), but Ruby's own `ensure` fires
  here too, and correctness requires it.
- **`ensure_stack` also intercepts `Stmt::Raise`, not just `Return`/
  `Break`/`Next`.** A `raise` executed directly inside a `rescue_body`
  (re-raising a different error while handling one) compiles via
  `build_raise` (`lib.rs:2830-2869`), which calls `emerald_raise` then
  `build_unreachable` and returns `Ok(true)` — a real exit with no
  chance to clean up, structurally identical to `Return`/`Break`/`Next`
  from the `ensure_stack`'s point of view. Because the C runtime's
  `emerald_raise` already unlinks the current handler from the global
  stack the instant it fires (`runtime/emerald_runtime.c`'s
  `emerald_raise`: `emerald_handler_stack = h->prev;` runs *before* the
  `longjmp`), a `raise` inside `rescue_body` naturally targets whatever
  handler was live *before* this `begin` pushed its own — i.e. the next
  outer `begin`, exactly matching Ruby's own re-raise-propagates-outward
  behavior — but Emerald's local `ensure` still has to run first, on
  the LLVM-CFG side, before that call is emitted.
- **`retry` is exempt from `ensure_stack` replay.** It doesn't exit the
  `begin` construct at all — it jumps back to that same `begin`'s own
  entry to re-attempt it — so running the enclosing `ensure` on a
  `retry` would be wrong (Ruby doesn't run `ensure` on `retry` either,
  for the same reason: the block hasn't been exited yet). This is a
  real, easy mistake this plan's leaf-codegen-retry acceptance criteria
  explicitly test for (a `retry`-then-succeed program must print its
  `ensure` line exactly once, not once per attempt).
- **Multiple typed `rescue` clauses reuse plan 20's own `build_case`
  chaining idiom, not a new dispatch shape.** Verified against `build_
  case` (`lib.rs:2678-2795`): each `when` arm already gets an `arm_blk`
  it branches into on match and a `next_check_blk` it falls through to
  on mismatch, forming a linear chain evaluated in source order — the
  exact "try this candidate, else fall to the next one" shape N ordered
  `rescue` clauses need. Plan 38's multi-clause `build_begin` threads
  the same `arm_blk`/`next_check_blk` pattern across `rescues`, with
  the final `next_check_blk` being the existing mismatch-exhausted
  re-raise path rather than a `case`'s `else_body`.
- **Subtype-aware matching walks plan 32's real chain representation,
  not a guess.** A `rescue` naming a superclass (e.g. `rescue Animal =>
  a` catching a raised `Dog`) must match every subclass's tag, not just
  the exact one. Concretely: for every class `C` declared in the
  program, codegen already has (or can trivially derive via its own
  `resolve_class_chain`, `lib.rs:146-170`) `C`'s full root-to-leaf
  ancestor list; for a `rescue`-clause type name `T`, this plan
  precomputes once, for the whole compiled program, the set of tags
  `{ class_tags[C] : T appears in resolve_class_chain(C) }` — every
  class that either *is* `T` or descends from it. The generated tag
  check reuses the exact `build_int_compare(IntPredicate::EQ, ...)`
  chained with `build_or` idiom `build_case`'s multi-value `when` arms
  already use for OR-ing several literal matches together (verified,
  `lib.rs:2726-2739`) — one comparison per tag in the precomputed set
  instead of one per literal `when` value. No vtable, no runtime type
  walk, no reflection: the whole subtype relationship is resolved to a
  flat, compile-time-constant set of integers before any code runs,
  consistent with the no-dynamic-dispatch/no-reflection ceiling.
- **A bare `rescue => e` matches unconditionally, and Emerald
  deliberately does not model Ruby's `Exception`-vs-`StandardError`
  hierarchy.** Real Ruby only lets a bare `rescue` catch `StandardError`
  descendants (`Exception` itself, `NoMemoryError`, etc. require an
  explicit type). Emerald has no such split — this is a disclosed
  simplification, not an oversight: "all raised values are catchable by
  a bare rescue," full stop. This is not a scope cut forced by dynamism
  avoidance (a static two-tier hierarchy could in principle be built
  without any reflection); it's declined because nothing in this
  project's history needs Ruby's specific built-in-exception taxonomy,
  and inventing one now would be pure speculative surface.
- **A bare-`rescue` binding is deliberately NOT inserted into `env` at
  all.** This is the sharper, load-bearing reason the catch-all can't
  reuse a typed `rescue`'s binding mechanism: Emerald has no universal
  root class. Verified: `resolve_chain` (`emerald-sema/src/lib.rs:153-
  182`) simply stops the moment `info.superclass` is `None` — every
  class with no `< Super` clause terminates its own chain right there;
  there is no implicit `Object`/`BasicObject` every chain eventually
  reaches, unlike Ruby. A bare-`rescue`'s `e` therefore has no
  statically knowable type to bind at all (this compiler resolves every
  field/method access from the receiver's *declared* type — there's
  nothing to declare it as). Rather than inventing a new "opaque
  exception handle" type and a bespoke "you can't do anything with this"
  diagnostic, this plan simply never inserts the name into `env` — any
  attempt to reference it inside the bare clause's body falls straight
  into this compiler's existing, unmodified "undefined variable"
  diagnostic (the same one plan 31's `y = 5` case already exercises),
  reusing infrastructure instead of adding a new failure mode. A real,
  disclosed narrowing versus Ruby (where a bare-rescue's `e` is a fully
  usable `StandardError`), directly downstream of declining a universal
  root class.
- **`retry` is a new `Stmt`, legal only inside a `rescue` clause's own
  body**, checked by threading a new `in_rescue: bool` through `check_
  stmt`/`check_block` — the exact same mechanism this compiler already
  uses for `break`/`next` (verified: the existing "`` `next` outside of
  a loop``" diagnostic at `emerald-sema/src/lib.rs` around line 995
  is guarded by a threaded `in_loop: bool` parameter already flowing
  through every `check_block` call). `in_rescue` is forced `true` only
  while checking a `RescueClause`'s own `body`, left unchanged (not
  forced true) while checking the `begin`'s try `body` or its `ensure`
  body — `retry` in either of those is a diagnostic, not a panic,
  matching real Ruby's own restriction (Ruby raises `SyntaxError` for
  `retry` outside `rescue`, including inside `ensure`).
- **`retry` codegen wraps the whole handler-push+`setjmp` sequence in a
  dedicated `retry_blk`, tracked on a new `retry_stack: &mut Vec<
  BasicBlock<'ctx>>`** (mirroring `loop_stack`'s own shape once more).
  `Stmt::Retry` branches directly to the innermost `retry_blk`. This
  has to redo `push_handler`+`handler_jmpbuf`+`setjmp` from scratch
  rather than just re-entering the existing `try_blk`, because by the
  time a `rescue` body runs, this `begin`'s original handler has
  already been unlinked and freed (`build_begin`'s `match_blk` already
  calls `ctx.exc_funcs.free_handler`, `lib.rs:3007-3013`) — a `longjmp`
  can only ever target a `setjmp` call site whose stack frame is still
  live *and* whose `jmp_buf` hasn't been invalidated by that frame's own
  further execution past it in a way that reuses the buffer for
  something else; the clean, safe way to get a fresh, valid handler for
  the next attempt is to genuinely re-run the push+setjmp pair, not to
  reuse stale state.
- **Out of scope: `rescue`-without-`begin` at method-definition level**
  (Ruby's implicit method-body rescue, e.g. `def foo ... rescue ... end`
  with no explicit `begin`). This is a distinct AST-attachment-point
  feature — it would need `Function`'s own body to optionally carry
  rescue/ensure clauses directly, rather than requiring an explicit
  `Stmt::Begin` wrapper — and is real, disclosed future work, not
  attempted here.
- **Out of scope: explicit bare `raise` (no argument) to re-raise the
  currently-caught exception from inside a `rescue` body.** Plan 11's
  Decision log already restricts `raise` to a direct `ClassName.new
  (args)` expression, since the raise site's class tag must be known at
  codegen time; this plan doesn't relax that. A `rescue` body that wants
  to propagate the exception it caught must construct and raise a new
  instance explicitly (or simply not catch it via a narrower `rescue`
  type in the first place) — Ruby's zero-argument re-raise sugar is a
  real, separate, smaller follow-on feature this plan doesn't need to
  prove `ensure`/multi-`rescue`/`retry` work.
- **Out of scope: a rescue-less `begin ... ensure ... end`.** This
  plan's grammar keeps `RescueClause+` (one-or-more) exactly as plan
  11 already required — `begin` still always needs at least one
  `rescue`. Ruby allows `ensure` with zero `rescue` clauses purely for
  cleanup-around-a-block; nothing in this project's history needs that
  shape, and dropping the `+` to `*` would require deciding what an
  unhandled exception does when a `begin` has an `ensure` but literally
  nothing that could ever catch anything — a real question, not a
  trivial relaxation, deferred rather than guessed at here.

## Leaf: leaf-ast-full-exceptions

### 1. Context
- Why: plan 11's `Stmt::Begin` hard-codes exactly one rescue clause and
  has no `ensure` field at all (verified: `crates/emerald-parser/src/
  ast.rs:179-184`); there is no `Stmt::Retry` anywhere in the AST.
- Target state: a new `RescueClause { class_name: Option<String>, var:
  String, body: Vec<Stmt> }` struct; `Stmt::Begin { body: Vec<Stmt>,
  rescues: Vec<RescueClause>, ensure: Option<Vec<Stmt>> }` replacing
  the old flat fields; a new `Stmt::Retry` unit variant. Grammar:
  `RescueClause: RescueClause = { "rescue" <c:Ident> "=>" <v:Ident>
  <b:Stmt*> => RescueClause { class_name: Some(c), var: v, body: b },
  "rescue" "=>" <v:Ident> <b:Stmt*> => RescueClause { class_name: None,
  var: v, body: b } }`; `EnsureClause: Vec<Stmt> = { "ensure" <b:Stmt*>
  => b }`; `"begin" <body:Stmt*> <rescues:RescueClause+> <ensure:
  EnsureClause?> "end" => Stmt::Begin { body, rescues, ensure }`; a new
  `"retry" => Stmt::Retry` `Stmt` alternative, with `retry` reserved
  the same LALR(1) way `raise`/`begin`/`rescue` already are. `"ensure"`/
  a second `"rescue"` can't start a `Stmt`, so `Stmt*` inside any clause
  unambiguously stops there, the same way `"rescue"` already terminates
  `body`'s list today.

### 2. Acceptance Criteria
1. `RescueClause`, `Stmt::Begin { rescues, ensure, .. }`, and
   `Stmt::Retry` exist as described.
2. `begin ... rescue NotFoundError => e ... rescue TimeoutError => e2
   ... ensure ... end` parses with `rescues.len() == 2` in source
   order (`rescues[0].class_name == Some("NotFoundError")`,
   `rescues[1].class_name == Some("TimeoutError")`) and `ensure ==
   Some(...)`.
3. `begin ... rescue => e ... end` (bare, no ensure) parses with
   `rescues[0].class_name == None` and `ensure == None`.
4. `retry` parses to `Stmt::Retry` wherever a `Stmt` is syntactically
   legal (sema, not the grammar, restricts where it's *valid* — the
   same division of labor `break`/`next` already use).
5. Regression: plan 11's own original example (`MyError`, one `rescue`,
   no `ensure`) still parses, now simply yielding `rescues.len() == 1`
   and `ensure: None` — the prior shape generalized, not changed.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. 2-clause/bare/retry parse shapes | agent-claimed-locally |

---

## Leaf: leaf-sema-full-exceptions

### 1. Context
- Why: `check_begin` (`emerald-sema/src/lib.rs:1118-1146`) type-checks
  exactly one `rescue_type`/`rescue_var`/`rescue_body` triple today and
  has no notion of `ensure` or `retry` at all.
- Target state: `check_begin` iterates `rescues` in source order; for
  `class_name: Some(ty)`, resolves `ty` via `resolve_type` (must be
  `Type::Class`, else a diagnostic naming the offending clause), inserts
  `var` into `env` at that type, and type-checks `body`; for `class_
  name: None`, type-checks `body` *without* inserting `var` into `env`
  at all (Decision log — any reference to it inside that clause's body
  falls through to the existing undefined-variable diagnostic). `ensure`
  is type-checked as an ordinary block against the same `env` the
  surrounding `begin` sees. A new `in_rescue: bool` parameter threads
  through `check_stmt`/`check_block` exactly like the existing `in_
  loop: bool` (verified pattern: the current `` `next` outside of a
  loop `` check reads a threaded `in_loop`); `check_begin` forces `in_
  rescue = true` only for each `RescueClause.body`, leaving it
  unchanged for the try `body` and the `ensure` body. `Stmt::Retry`'s
  own `check_stmt` arm requires `in_rescue`, erroring `` "`retry`
  outside of a rescue body" `` otherwise.

### 2. Acceptance Criteria
1. This plan's full worked example (two typed clauses, `ensure`) type-
   checks `Ok(())`.
2. Referencing the bare clause's bound name inside its own body (e.g.
   `rescue => e ... puts e ...` with no type given to `e`) is rejected
   with the existing "undefined variable" diagnostic — proving the
   name genuinely never enters `env`, not just that it's unused.
3. `retry` used at top level, inside a `begin`'s try `body`, or inside
   an `ensure` body is rejected with a "`retry` outside of a rescue
   body" diagnostic in each case — not a panic.
4. `retry` used inside a `rescue` clause's body type-checks `Ok(())`.
5. Regression: `emerald-sema`'s existing plan-11-shaped test (single
   typed `rescue`, no `ensure`) still type-checks `Ok(())` unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 4 new cases above | agent-claimed-locally |

---

## Leaf: leaf-codegen-ensure

### 1. Context
- Why: `build_begin` (`emerald-codegen/src/lib.rs:2879-3043`) has no
  `ensure` concept — nothing runs at any of its exit points beyond the
  ordinary fallthrough-to-`merge_blk` branches it already emits.
- Target state: a new `ensure_stack: &mut Vec<&[Stmt]>` parameter
  threaded through `build_stmt`/`build_block` (and every function that
  already threads `loop_stack`), holding the active `begin`'s `ensure`
  body slice for the duration of compiling its try `body` and each
  `rescue` clause's `body`. `build_begin` duplicate-emits the `ensure`
  body's codegen (via `build_block`, reusing the exact same function
  every other block already compiles through) at: (a) the try-body's
  normal-fallthrough point, before the existing jump to `merge_blk`;
  (b) each matched `rescue` clause's normal-fallthrough point, same
  place; (c) the mismatch-exhausted path, immediately before the
  propagating `emerald_raise` call (Decision log — the third, easy-to-
  miss exit). `build_stmt`'s `Return`/`Break`/`Next`/`Raise` arms each
  walk `ensure_stack` innermost-to-outermost and duplicate-emit every
  active `ensure` body's codegen before emitting their own terminator
  (the `ret`/loop-target branch/`emerald_raise` call respectively).
  `retry` does not consult `ensure_stack` at all (Decision log).

### 2. Acceptance Criteria
1. A `begin risky_call rescue Foo => f puts f.code ensure puts
   "cleanup" end` program where `risky_call` does *not* raise, compiled,
   linked, and run, still prints `cleanup` — proving `ensure` runs on
   the non-exceptional path, not only when a `rescue` actually fires.
2. This plan's full worked example, compiled, linked, and run, prints
   exactly `7\ncleanup\n` — the headline proof.
3. A program with an inner `begin ... rescue WrongType => w ... ensure
   puts "inner" end` nested inside an outer `begin ... rescue RightType
   => r puts r.code ensure puts "outer" end`, where the raised
   exception's type matches neither the inner clause but does match the
   outer one, compiled, linked, and run, prints `inner` *before* the
   outer clause's own output — proving `ensure` fires on the mismatch-
   exhausted re-raise path, in the correct order.
4. A `return` statement executed inside a matched `rescue` clause's
   body, inside a `begin` that also has an `ensure`, compiled, linked,
   and run, still prints the `ensure` line before the function actually
   returns — proving the early-exit interception works, not just the
   two block-level fallthrough paths.
5. Regression: plan 11's original single-rescue, no-`ensure` example
   still compiles, links, and runs, printing `99\n` unchanged.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. the 4 new linked-and-run cases above | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-multi-rescue

### 1. Context
- Why: `build_begin` compares exactly one `rescue_tag` (`lib.rs:2894-
  2897, 2960-2963`) and has no subtype awareness — `Ctx.class_tags`'s
  own doc comment (`lib.rs:659-664`) already flags exact-tag equality
  as a disclosed limitation to be lifted later.
- Target state: `build_begin` takes `rescues: &[RescueClause]` and, per
  the Decision log, chains them via `build_case`'s own `arm_blk`/`next_
  check_blk` idiom in source order. Before codegen starts, the caller
  (`compile_to_object`) precomputes, once, `rescue_tag_sets: HashMap<
  String, Vec<i64>>` mapping every class name `T` that appears as some
  `rescue`'s `class_name` to the tags of every class `C` where `T`
  appears in `resolve_class_chain(C, class_defs)` — i.e. `T` itself and
  every descendant. Each typed clause's match condition OR's `build_
  int_compare(EQ)` across that clause's precomputed tag set (reusing
  `build_case`'s own `build_or`-chaining pattern, `lib.rs:2726-2739`);
  a bare clause (`class_name: None`) branches to its match block
  unconditionally, with no tag comparison and no store into any
  variable pointer (its `var`, per the sema leaf, was never allocated a
  slot). The final `next_check_blk` in the chain is the existing
  mismatch-exhausted re-raise path.

### 2. Acceptance Criteria
1. This plan's full worked example, compiled, linked, and run, proves
   ordering: raising `TimeoutError` is *not* caught by the first
   (`NotFoundError`) clause and *is* caught by the second, printing `7`
   (not silently matching the wrong clause or erroring).
2. Reusing plan 32's own `Animal`/`Dog` inheritance example: a class
   hierarchy where `Dog < Animal`, a function that raises `Dog.new(...)`
   inside a `begin`, caught by `rescue Animal => a`, compiled, linked,
   and run, prints the expected field value read off `a` — proving
   subtype-aware matching genuinely works end to end, not just that a
   same-named clause still matches.
3. A bare `rescue => e` positioned after one or more typed clauses that
   don't match the raised type still catches it, compiled, linked, and
   run, printing whatever the clause's body prints — proving the
   catch-all is genuinely unconditional, not silently requiring some
   type to line up.
4. An exception whose class matches *no* clause (all typed clauses
   mismatch, no bare clause present) still propagates as fully uncaught
   — process exits nonzero via the existing `emerald_raise`'s "uncaught
   Emerald exception" path — unchanged from plan 11's own behavior.
5. Regression: plan 11's original one-clause example still compiles,
   links, and runs, printing `99\n`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. the 4 new linked-and-run cases above | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-retry

### 1. Context
- Why: no `Stmt::Retry` codegen exists; nothing in `build_begin` today
  supports re-entering a `begin` construct after a partial attempt.
- Target state: a new `retry_stack: &mut Vec<BasicBlock<'ctx>>`
  parameter, threaded through `build_stmt`/`build_block` alongside
  `loop_stack`/`ensure_stack`. `build_begin` appends a `begin.retry`
  basic block at its very top (before the current `push_handler` call),
  jumps into it unconditionally as its first instruction, and pushes it
  onto `retry_stack` for the duration of compiling every `rescue`
  clause's body (popped afterward). `retry.begin`'s own body is exactly
  today's `push_handler`/`handler_jmpbuf`/`setjmp`/branch sequence,
  unchanged — `retry`'s only new behavior is giving something for
  `Stmt::Retry` to branch back to. `Stmt::Retry`'s `build_stmt` arm
  emits `builder.build_unconditional_branch(*retry_stack.last().ok_or
  ("codegen: `retry` outside of a rescue body")?)` and returns `Ok
  (true)` (terminated) — a real `Err`, not a panic, if `retry_stack` is
  empty (defends a sema-bypassing direct codegen call the same way
  `resolve_class_chain`'s own cycle/undefined-name `Err`s already do).

### 2. Acceptance Criteria
1. A program that raises inside its `begin` body for the first two
   attempts (tracked via an outer counter) and succeeds on the third,
   using `retry` inside its `rescue` clause to re-attempt, compiled,
   linked, and run, prints `3` — real executed proof `retry` actually
   re-enters the `begin` construct rather than looping forever or
   erroring.
2. The same program with an `ensure puts "cleanup"` added prints
   `cleanup` exactly once (after the third, successful attempt) — not
   once per attempt — proving `retry` does not re-trigger the enclosing
   `ensure` on its way back (Decision log).
3. A defensive-only test constructs `Stmt::Retry` directly (bypassing
   sema) with an empty `retry_stack` and asserts codegen returns a
   descriptive `Err`, not a panic.
4. Regression: plan 11's original example still compiles, links, and
   runs, printing `99\n`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. the retry-then-succeed and retry-with-ensure linked-and-run cases | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
