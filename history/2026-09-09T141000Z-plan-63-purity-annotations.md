---
name: Purity Annotations
overview: "A `pure` function modifier — a single, checked boolean property, transitively verified over the whole call graph — proving a function contains no I/O, no actor `.spawn`/message send, no field/index mutation, and calls only other provably-`pure` functions. The Haskell-inspired, monad-transformer-free cousin of the `Effect<A,E,R>` design this project already rejected: no environment channel, no monadic composition, just a checked annotation. Proves the safety property a future optimization could exploit for memoization/parallelism; does not build that exploitation."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-pure
    content: "`pure` becomes a new reserved keyword parsed as an optional prefix on FuncDef/MethodDef (top-level functions, module functions, and class methods — actor methods parse it too, since grammar stays general, but sema rejects it there); Function gains `is_pure: bool`"
    status: pending
  - id: leaf-sema-purity-check
    content: "FunctionSig gains `is_pure: bool`; a new whole-program pass computes the call graph's SCC condensation over every `pure`-claimed function, checks each SCC once (cycle members assumed pure against each other), and rejects any function whose body contains an I/O intrinsic, an actor spawn/message send, a field/index mutation, or a call to a non-pure callee — codegen is untouched, `pure` is 100% a sema-time property"
    status: pending
  - id: leaf-concurrency-proof
    content: "Two actors, spawned onto plan 55's real OS worker pool, both call the same top-level `pure` function with different arguments and zero synchronization around the shared call — a real compiled-and-run proof of correct, deterministic results under genuine concurrent execution, plus the mirror negative proof: a `pure`-claimed function that actually performs I/O or sends a message is rejected with a diagnostic naming the exact violation"
    status: pending
isProject: false
---

# Plan 63 — Purity Annotations

This is plan 63 of the 58-64 theoretical-maximum batch — a set of
independent siblings authored after "Beyond the Ceiling" (plans 48-57)
landed in full, each pursuing one further post-v1 capability without
conceding this project's identity constraints (no `method_missing`/
`eval`/`send`/reflection, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing GC). Like every batch
before it, this is post-v1 scope and does not touch
[`plan-of-plans`](2026-09-08T174011Z-plan-of-plans.md) or any other
plan file — that table's own Completion note already calls Emerald v1
done at row 15, and a separate pass updates it once all of 58-64 are
authored, not this document.

This plan has a debate behind it worth stating plainly. An earlier
design pass considered a full Effect.ts/`id_effect`-style
`Effect<A, E, R>` monadic effect system — algebraic effect tracking,
a dependency-injection environment channel `R`, composition combinators,
a competing concurrency runtime layered under Emerald's own actor
scheduler (plan 55) — and **rejected it outright**: too much machinery,
a structural/variance type system Emerald's nominal `Type` enum
(`crates/emerald-sema/src/lib.rs` L22-115, verified this session — no
row/effect polymorphism anywhere in it) was never built for, and a
second concurrency story competing with the one plan 54/55 already
ship. A follow-up debate then asked whether anything smaller and
genuinely useful survived that rejection, and landed on this: a
Haskell-inspired but *monad-transformer-free* `pure` annotation — not
`IO`-as-a-type, not a `Monad`/`Functor` typeclass hierarchy, just a
single checked boolean fact about an ordinary function, verified the
same way this compiler already verifies everything else (a real,
whole-program static analysis with real diagnostics, never a naming
convention). **This is not a resurrection of `Effect<A,E,R>`.** There is
no environment/DI channel here (a `pure` function's dependencies are
exactly its ordinary parameters, nothing injected), no monadic
composition sugar (no `.flatMap`/`.map`/`do`-notation of any kind — a
`pure` function is called exactly like any other function), and no new
runtime type (`Void`, `Int64`, `String`, ... stay exactly what they are
today; nothing is wrapped in an `Effect`/`IO` box). `pure` is metadata
on a `Function`, checked once at compile time, erased by the time
codegen runs.

**Critical scoping decision, stated once, not relitigated below: `pure`
is a single boolean property, not an effect-row system.** This plan does
not track *which* effects a function performs (Koka/Haskell-style `IO
a`/effect rows would need a genuinely new type-level dimension —
polymorphism over effect sets, row-polymorphic function types); it
checks exactly one thing, transitively: does this function (and
everything it calls) ever touch the fixed, enumerated set of
side-effecting operations this compiler already knows how to name? A
function either qualifies or it doesn't. There is no partial credit, no
`Pure & Reads[File]`-style refinement.

Depends on: plan 06 (function declarations — `Function`/`FuncDef`, the
node this plan adds a field and a keyword to), plan 45
(`2026-09-09T111000Z-plan-45-stdlib-strings-and-io.md` — the exact I/O
intrinsic surface `pure` must forbid, re-verified against its real text
and the real `grammar.lalrpop`/`runtime/emerald_runtime.c` this
session), plan 54
(`2026-09-09T124000Z-plan-54-actor-declarations-and-isolated-heaps.md`
— `actor`/`.spawn`, `is_actor`, `Expr::Spawn`), and plan 55
(`2026-09-09T125000Z-plan-55-scheduler-and-message-passing.md` — the
real worker pool and cross-actor dispatch rule this plan's concurrency
proof reuses verbatim, and the exact mechanism `pure` must recognize as
a "message send"). Plans 56/57 (compile-time message safety,
supervision trees) have also landed by this session (verified: both
files exist in `history/`, and `emerald-sema`'s
`check_message_safety*`/`expr_moved_read` family, L3826-4213, and
`runtime/emerald_runtime.c`'s `EmeraldSupervisor`/
`emerald_supervisor_register_child` already exist) — cited for
orientation only; this plan does not touch either.

Concrete proof this plan targets — a real, compiled-and-run function
reused by two OS threads with nothing protecting it:

```ruby
pure def fib(n: Int64) -> Int64
  if n < 2
    n
  else
    fib(n - 1) + fib(n - 2)
  end
end

actor Worker
  def run(n: Int64) -> Void
    puts fib(n)
  end
end

w1: Worker = Worker.spawn()
w2: Worker = Worker.spawn()
w1.run(30)
w2.run(31)
```

`w1.run(30)`/`w2.run(31)` are both cross-actor sends (plan 55's
syntactic dispatch rule — neither receiver is a literal `self`), each
landing in `Worker`'s mailbox and, under plan 55's real worker pool,
each potentially executed on a *different* OS thread with no lock, no
mutex, no actor-mailbox serialization between the two `fib` calls
themselves (only each actor's own single message is serialized against
*itself* — `fib` is a plain top-level function call from inside that
message's handler, entirely outside the mailbox mechanism). Expected
values: `fib(30) = 832040`, `fib(31) = 1346269` — both printed, each
exactly once; see `leaf-concurrency-proof` for why relative order is
deliberately not asserted (the same reason plan 55's own two independent
`Spinner`s don't assert one either) and for the separate wall-clock
proof that the two calls genuinely overlapped on real hardware, not
merely "happened to be safe because the scheduler ran them serially
anyway."

The mirror negative proof, same session:

```ruby
pure def bad(x: Int64) -> Int64
  puts x
  x
end
```

rejected at compile time with a diagnostic naming `puts` and the span
of the `puts x` statement — not a generic "this function is not pure"
message, a real pointer at the violating call.

## Decision log

- **Eligibility: `pure` is checkable on a top-level `def`, a `module`
  function, and an ordinary class method — declined on an actor
  method.** Grammar-level, `pure` parses in exactly the same two
  positions `MethodDef`/`FuncDef` already exist in (`ActorDef`'s
  `<methods:MethodDef*>` reuses the identical production `ClassDef`
  does — verified, `grammar.lalrpop`), so `pure def run(...) -> Void
  ... end` inside `actor Worker` is grammatically indistinguishable
  from the class case; sema is where this plan narrows it, the same
  "grammar stays general, sema rejects" discipline `check_method_body`
  already applies to defaults/splat/tuple-returns found on a method
  (verified, `crates/emerald-sema/src/lib.rs` L4275-4306) — a `pure`
  found on an `ActorDef` method gets the identical treatment, a real,
  named diagnostic, not a silent no-op. The reason isn't grammar
  laziness: an actor's own fields are mutated through exactly the
  mechanism this plan forbids inside `pure` (`Stmt::SetField`, see
  below), and plan 55's safety argument for touching those fields
  without a lock is a *different*, already-adequate mechanism (at most
  one thread ever inside a given actor instance's code at a time, by
  construction of the mailbox/`scheduled`-flag protocol) — admitting
  actor methods into this plan's checked set would either have to
  reject every actor method with fields (making `pure` useless there)
  or silently trust a second, unrelated safety mechanism to cover the
  gap, which is exactly the kind of unstated-assumption stacking this
  project's Decision logs consistently refuse to do. A future plan
  wanting `pure` on actor methods is free to make that argument
  explicitly; this one doesn't need to.
- **Checking mechanism: a whole-program call-graph pass over every
  `pure`-claimed function's strongly-connected-component condensation,
  each component checked once, cycle members assumed pure against each
  other.** Purity is not decomposable per function body the way
  ordinary type-checking or plan 56's `check_message_safety` are
  (verified: `check_function_body`/`check_method_body`,
  `crates/emerald-sema/src/lib.rs` L4215-4340, each check one function
  in isolation using only already-resolved `sigs`/`classes` — purity
  instead needs to know, for a call `f()` inside `g`'s body, whether
  `f` itself is pure, which can depend on functions declared *later* in
  the file, including `f` itself). This plan therefore adds a new,
  dedicated pass, run once from `check_program` after every
  `check_function_body`/`check_method_body` call has already succeeded
  for the whole program (mirroring exactly when plan 56's
  `check_message_safety` already runs relative to ordinary
  type-checking — after, using the finished, trustworthy `env`/`sigs`/
  `classes`, never interleaved with it). Concretely: build a directed
  graph whose nodes are every `pure`-claimed function/method and whose
  edges are "calls" (an `Expr::Call`/`Expr::MethodCall`/whatever a
  body's forbidden-construct walk finds pointing at another
  user-defined function), compute its strongly-connected components
  (a self-contained ~40-line Tarjan or plain DFS-based SCC pass — no
  existing SCC utility anywhere in this file's function map, verified,
  and the graph is small enough that no new Cargo dependency is
  justified for it), and process components in reverse-topological
  order (callees' components decided before their callers'). A
  singleton component with no self-loop is decided directly — every
  callee it can reach is either already known-pure (an earlier
  component) or already known-not-pure, so its own forbidden-construct
  walk (below) has a definite answer for every call site. A
  multi-member or self-looping component (mutual or self recursion —
  `fib` calling itself is the simplest possible case, a
  strongly-connected component of size one with a self-edge) is checked
  with **cycle-internal calls provisionally assumed pure**: every
  member's forbidden-construct walk runs once treating a call to any
  fellow cycle member as satisfying "callee is provably pure," and the
  whole component is accepted together only if *every* member passes
  under that assumption — one relaxed pass per component, not an
  iterative fixed point converging over several rounds, because
  accepting a cycle only ever needs this one optimistic assumption to
  hold consistently (nothing tighter can ever be discovered by
  iterating further: if every member passes assuming its cycle-mates
  are pure, the assumption is self-consistent and accepted as a block;
  if any member fails even under the most permissive assumption
  possible about its own cycle, no weaker assumption exists that would
  rescue it, so the whole component is rejected together, with a
  per-member diagnostic naming that member's own actual violation).
  This is a deliberately simpler rule than the fully general
  "conservative rule" alternative the task brief also offered (e.g.
  rejecting every mutually-recursive `pure` claim outright, no
  exceptions) — rejected here because it would make `fib` itself, the
  single most natural `pure` example in a language with no loops-only
  discipline, uncheckable, for no soundness gain (the optimistic
  single-pass rule is exactly as sound: a cycle is accepted if and only
  if it is genuinely self-consistent).
- **Forbidden inside a `pure` function body, enumerated precisely, each
  verified against real source this session:**
  1. **I/O intrinsics from plan 45** — `puts` (verified,
     `grammar.lalrpop` L439: `"puts" <arg:Expr>` desugars to
     `Stmt::Expr(Expr::Call("puts".to_string(), vec![arg]))` — a
     compiler intrinsic checked directly in `infer_expr_type`, never a
     `sigs` entry); `gets()` (`Expr::Call("gets", ...)`, plan 45's
     `leaf-argv-and-gets`, backed by `emerald_gets` in
     `runtime/emerald_runtime.c`); `File.read(path)`/`File.write(path,
     content)` (`Expr::MethodCall` whose receiver is literally
     `Expr::Ident("File")`, plan 45's `leaf-file-io`, backed by
     `emerald_file_read`/`emerald_file_write` — the latter verified
     directly this session at `runtime/emerald_runtime.c` L516-524).
     `ARGV`/`ARGC` (plain `Expr::Ident("ARGV"|"ARGC")` reads, seeded
     from the process's real `argv`/`argc` per plan 45's
     `leaf-argv-and-gets`) are forbidden too, on the same footing as an
     I/O call even though they're not syntactically a call: a `pure`
     function whose result can silently vary with how the compiled
     binary happens to have been invoked is not pure by any definition
     this plan wants to make, and codegen already treats them as
     ordinary pre-seeded locals (`define_main`), not literals — nothing
     about a `pure` function's *type* distinguishes "reads `ARGV`" from
     "reads a genuinely constant global," so this plan closes the gap
     by name rather than leaving it as a silent hole. String/Array/Hash/
     Enumerable intrinsics (`.length`, `.upcase`, `.split`, indexing,
     `.map`/`.select`/`.reduce`/`.each`, etc.) are **not** forbidden —
     plan 45's own Decision log states, verified, "no method on *any*
     type mutates its receiver in place today" (the only mutation
     mechanisms in this whole compiler are `Stmt::SetField`/
     `Stmt::SetIndex`, both plain statements, never method calls), so
     every one of these intrinsics is already side-effect-free by
     construction and needs no special-casing — the checker allows
     anything it doesn't specifically recognize as forbidden, and
     these were never candidates.
  2. **Actor `.spawn` and message sends from plan 54/55** —
     `Expr::Spawn(String, Vec<Expr>)` (verified, `ast.rs`, plan 54's
     `.spawn(...)`) is forbidden unconditionally. A message send is,
     per plan 55's own Decision log verified this session, syntactic
     and decidable at compile time: any `Expr::MethodCall` whose
     receiver's statically inferred type is `Type::Class(n)` with
     `classes[n].is_actor == true` **and** whose receiver is not the
     literal token `self`. Since `pure` is never legal on an actor
     method (see above), no eligible `pure` function body ever has a
     `self` that refers to an actor in the first place — so for every
     function this plan can actually check, the rule collapses to
     "any `MethodCall` on an actor-typed receiver is forbidden,
     unconditionally," with no `self`-aliasing edge case to reason
     about at all. Codegen's own real target for this shape is
     `emerald_actor_enqueue`/`build_actor_enqueue_call` (verified,
     `crates/emerald-codegen/src/lib.rs` L5842-5932) — this plan's
     checker never calls that function or touches codegen at all, it
     only needs to recognize the same *shape* sema already has enough
     type information to recognize.
  3. **`extern "C"` FFI** — no plan 58-62 (or any other plan on disk)
     introduces one; verified this session via a full `history/`
     listing (plans run contiguously 01-57, and this plan is 63 of the
     new 58-64 batch — no FFI plan exists yet) and directly against
     `grammar.lalrpop` (no `extern` terminal anywhere). This category
     is genuinely vacuous today, not silently skipped: if a future
     plan adds `extern "C"` calls, it inherits the obligation to add
     itself as this list's fourth forbidden category, stated here so
     that obligation is discoverable rather than assumed.
  4. **Field and index mutation — a necessary fourth rule beyond the
     three the brief named, added and justified here.** `Stmt::SetField`
     (`@x = value`) and `Stmt::SetIndex` (`arr[i] = value`) are, per
     plan 45's own verified claim above, the *only* two mutation
     mechanisms this entire compiler has. Every heap-referencing type —
     `Array[T]`, `Hash[K,V]`, every class/actor instance — compiles to
     a bare `ValKind::Ptr` (verified, `value_kind_for_type`,
     `crates/emerald-codegen/src/lib.rs` L88-111: every type name that
     isn't one of `Float64`/`Int64`/`Boolean`/`String`/`Nil`/`Symbol`
     falls through to `ValKind::Ptr`), meaning any of these types
     passed as a parameter is passed *by reference*, aliasing whatever
     the caller holds. A `pure` function that forbade I/O and actor
     sends but still permitted `Stmt::SetField`/`Stmt::SetIndex` could
     take an `Array[Int64]` parameter and mutate the caller's own
     array in place — invisible to this plan's other three checks,
     and a real, live data race the moment two concurrent callers ever
     passed overlapping arrays. Forbidding both statement kinds
     directly (and transitively — a `pure` function may not call a
     non-`pure` function that does this either, already covered by the
     ordinary callee-purity rule) closes this without needing to
     classify parameter types at all. The alternative considered —
     leave `SetField`/`SetIndex` legal but scope `pure` to reject any
     function whose parameter or return type is `Array`/`Hash`/a class/
     an actor — was rejected: it's more machinery to specify and check
     than banning two statement kinds outright, and it wouldn't even
     close the hole fully (a `pure` function can still `Array.new(n)`
     a fresh array *locally* and hand it back as its return value; a
     caller that later aliases and mutates *that* returned array
     through two different variables is a real, if narrower, case
     parameter-type-scoping alone doesn't touch either).
  5. **Transitively, any call to a function not itself marked/provably
     `pure`.** The whole point of the checked-boolean-property claim:
     `FunctionSig` (`crates/emerald-sema/src/lib.rs` L139-162) gains an
     `is_pure: bool` field, populated by `function_signature`
     (L452-490) straight from `Function.is_pure` — since `ClassInfo`
     already stores each class's/module's methods as
     `HashMap<String, FunctionSig>` (verified, L177), a class method's
     purity is available through the exact same lookup a bare
     top-level call already uses, no second registry.
  6. **This is not the whole classical definition of purity, and this
     plan says so rather than overclaiming.** A `pure` method that only
     *reads* `@field` (no `SetField`/`SetIndex` anywhere in its body)
     passes every rule above — but its result can still depend on
     hidden receiver state a *different*, non-`pure` caller might be
     concurrently mutating on the very same plain (non-actor) object,
     since only actor instances carry plan 55's single-writer-per-
     instance guarantee; an ordinary class instance has no such
     protection built in anywhere in this compiler. This plan's
     concurrency payoff proof (`leaf-concurrency-proof`) is
     deliberately built from a top-level function over plain `Int64`
     parameters specifically to sidestep this — no heap-referencing
     parameter, so no aliasing question can even arise for the case
     this plan actually demonstrates. Exploiting `pure` safely for an
     *arbitrary* instance method against a shared, plainly-mutable
     object is a real, disclosed gap this plan's three-plus-one checked
     categories do not close; it is not silently assumed solved.
- **This plan proves the safety property; it does not build the
  optimization that exploits it.** No automatic memoization (a cache
  keyed on a `pure` function's arguments, reused across calls) and no
  automatic parallelization (the compiler deciding, on its own, to run
  two `pure` calls concurrently) exist anywhere in this plan's leaves —
  `leaf-concurrency-proof`'s two `Worker` actors are *explicit*,
  hand-written Ruby source calling `fib` from two independently
  scheduled message handlers; nothing about `pure` itself causes any
  call anywhere to run any differently than it would un-annotated.
  Building a real optimization pass that *automatically* memoizes or
  parallelizes `pure` calls is real, substantial, disclosed future
  work — a natural (if distinct) companion to plan 48's per-file
  content-hash query cache and plan 49's leveled parallel-compilation
  worker pool. Those two plans cache/parallelize the *compiler's own*
  work across files; a future `pure`-driven optimization pass would
  cache/parallelize the *compiled program's* runtime calls across
  arguments — a related idea (both exploit a proven independence
  property to buy speed for free) but a genuinely different axis, not
  something this plan's own scope quietly slides into. This plan's
  contribution to that eventual work is exactly the checked fact a
  memoizer/auto-parallelizer would need as its soundness precondition —
  nothing more.

## Leaf: leaf-ast-pure

### 1. Context
- Why: no notion of "this function has no side effects" exists anywhere
  in the AST today — `Function` (verified, `crates/emerald-parser/src/
  ast.rs` L477-508) carries `name`/`params`/`return_type`/`body`/
  `block_param`/`splat_param`/`type_params`, nothing purity-shaped.
- Target state: `Function` gains `pub is_pure: bool` as its final field.
  `grammar.lalrpop`'s `FuncDef` and `MethodDef` productions each gain an
  optional leading `<is_pure:"pure"?>` before their existing `"def"`
  token, threading `is_pure: is_pure.is_some()` into the constructed
  `Function` literal (mirrors exactly how `TypeParamClause?` already
  works as an optional prefix-ish clause on the same productions).
  `"pure"` becomes a new reserved keyword the identical LALR(1) way
  `actor`/`spawn`/`module` already are (verified: LALRPOP auto-registers
  a quoted literal string as its own terminal — plan 54's own Decision
  log states no separate lexer change was needed for `actor`, and none
  is needed here either). `ModuleDef`'s `<methods:FuncDef*>` and
  `ActorDef`'s `<methods:MethodDef*>` both reuse the same productions
  this leaf touches, so `pure` parses inside `module`/`actor` bodies
  too — deliberately left general at the grammar level; sema narrows it
  (see `leaf-sema-purity-check`).

### 2. Acceptance Criteria
1. `pure def square(x: Int64) -> Int64 x * x end` parses to
   `Function { is_pure: true, ... }` (verified directly against the
   parsed AST, not just "compiles").
2. An ordinary `def square(x: Int64) -> Int64 ... end` (no `pure`)
   still parses with `is_pure: false`.
3. `pure` parses identically inside a class body (`class Circle ...
   pure def area -> Float64 ... end end`) via the same `MethodDef`
   production.
4. Regression: every prior plan's example (`HELLO_EM` through
   `COUNTER_ACTOR_EXAMPLE`) still parses byte-for-byte identically —
   this leaf only adds one new optional grammar prefix, it changes no
   existing production's accepted language.
5. `cargo build -p emerald-parser` reports no new LALR(1) conflicts.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new `is_pure` AST-shape assertions | agent-claimed-locally |

---

## Leaf: leaf-sema-purity-check

### 1. Context
- Why: `is_pure: bool` on the AST is inert without a checker — anyone
  could write `pure def f ... puts "lied" ... end` today and have it
  compile clean. This leaf is the entire load-bearing claim of the
  plan: `pure` becomes a real, checked, whole-call-graph property.
- Target state: `FunctionSig` (`crates/emerald-sema/src/lib.rs`
  L139-162) gains `is_pure: bool`, populated by `function_signature`
  (L452-490) from `Function.is_pure`. A new pass,
  `check_purity(program, sigs, classes) -> Result<(), Vec<Diagnostic>>`,
  runs from `check_program` (L4825+) after every `check_function_body`/
  `check_method_body` call has already succeeded for the whole program
  (same ordering plan 56's `check_message_safety` already established
  relative to ordinary type-checking, cited above). It: (a) rejects any
  `pure` found on a method belonging to a `classes[name].is_actor ==
  true` entry, with a real diagnostic, before doing anything else; (b)
  builds the call graph over every remaining `pure`-claimed function/
  method, computes its SCC condensation, and processes components in
  reverse-topological order per the Decision log's rule; (c) for each
  function being decided, walks its body with a new
  `check_purity_stmt`/`check_purity_expr` pair (shaped like
  `check_message_safety_stmt`/`check_message_safety_expr_stmt`,
  `crates/emerald-sema/src/lib.rs` L3859-4012, the closest existing
  precedent for "walk a checked function body looking for a specific
  forbidden pattern using already-inferred types") flagging every
  construct enumerated in the Decision log, returning the first
  violation found as a `Diagnostic` carrying that construct's own real
  span. Zero changes anywhere in `crates/emerald-codegen` — `is_pure`
  never influences code generation; it is checked and then erased,
  exactly like `TypeParam` bounds already are today.

### 2. Acceptance Criteria
1. `pure def fib(n: Int64) -> Int64 ... fib(n-1) + fib(n-2) ... end`
   (self-recursive) is accepted — the size-one cyclic-component rule
   exercised directly.
2. A genuine mutual-recursion pair, both `pure`-claimed, calling only
   each other and arithmetic, is accepted together.
3. The same mutual-recursion pair, with one member additionally calling
   `puts`, is rejected as a whole component — both members get their
   own diagnostic, the one naming `puts` pointing at that exact call.
4. `pure def bad(x: Int64) -> Int64 puts x; x end` is rejected; the
   diagnostic names `puts` and the statement's real span.
5. `pure def bad2(w: Worker) -> Void w.run(5) end` (`Worker` an actor)
   is rejected; the diagnostic names the cross-actor send.
6. A `pure` function/method containing `Stmt::SetField` or
   `Stmt::SetIndex` is rejected either way, with a diagnostic naming
   which statement and where.
7. A `pure` function calling an ordinary (non-`pure`) function is
   rejected; the diagnostic names the non-`pure` callee.
8. `pure` declared on an `ActorDef` method is rejected at registration,
   independent of what that method's body contains.
9. Regression: every prior compiled example still type-checks
   identically — `check_purity` is a strict no-op over any program that
   never writes the word `pure`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` only.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. every positive/negative case above | agent-claimed-locally |
| Workspace regression | `cargo test --workspace` | all pass, no prior example's behavior changes | agent-claimed-locally |

---

## Leaf: leaf-concurrency-proof

### 1. Context
- Why: a checked property nobody ever exploits is a paper claim. This
  leaf is the real, compiled-and-run demonstration that `pure`'s
  guarantee is actually safe to lean on for unsynchronized concurrent
  reuse — reusing plan 55's already-landed worker pool rather than
  inventing a second concurrency primitive just for this proof (a
  direct `std::thread::spawn` proof from inside the Rust test suite
  was considered and rejected: it would prove the *Rust-level* call is
  thread-safe, not that an actual compiled *Emerald program* can safely
  reuse a `pure` function across two independently scheduled call
  sites with zero source-level synchronization, which is the claim this
  plan actually needs to survive contact with reality).
- Target state: the worked example from this plan's own opening section
  — `fib` (`pure`), two `Worker` actors, two cross-actor `run` sends —
  added as a real compiled-and-run test, plus a companion pair of
  `EMERALD_WORKERS=1` vs. default-pool-size runs of the *same compiled
  binary*, reusing plan 55's own `leaf-worked-concurrency-proof`
  wall-clock-margin technique verbatim (cite it directly: run once
  serialized, once with real parallelism available, and compare total
  wall-clock time against the single larger call's own time, not the
  sum). The negative examples from `leaf-sema-purity-check` (AC4/AC5)
  are re-verified end-to-end here through the real `emerald` CLI path
  (a real non-zero exit code and a real stderr diagnostic from a
  freshly invoked compiler process), not merely as `Result::Err`
  assertions inside a unit test.

### 2. Acceptance Criteria
1. Compiled once and run: both `832040` and `1346269` appear in stdout,
   each exactly once. Relative order is deliberately **not** asserted —
   the two `run` sends are independent, non-communicating cross-actor
   calls with no ordering dependency between them, the identical
   situation plan 55's own `Spinner` proof already established doesn't
   get a fixed order either (cited directly, not re-argued).
2. The same compiled binary run twice — once with `EMERALD_WORKERS=1`,
   once with the default (`>= 2`-worker) pool — prints the identical,
   correct pair of values both times: purity's safety guarantee holds
   regardless of whether the two `fib` calls actually overlapped on
   separate OS threads or ran strictly serialized.
3. The default-pool run's total wall-clock time is close to
   `fib(31)`'s own single-call time rather than the sum of `fib(30)`'s
   and `fib(31)`'s — the same margin-based assertion plan 55's own
   `Spinner` proof uses — real, positive evidence the two calls
   genuinely overlapped on separate OS threads at least once, not just
   "happened not to visibly break."
4. `pure def bad(x: Int64) -> Int64 puts x; x end`, compiled via the
   real `emerald` CLI entry point, exits non-zero and prints a stderr
   diagnostic naming `puts` and its source location — a real
   end-to-end compiler-process proof, not a unit test's `Result::Err`.
5. `pure def bad2(w: Worker) -> Void w.run(5) end`, compiled the same
   way, exits non-zero and prints a stderr diagnostic naming the
   cross-actor send.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (new compiled-and-run
  test constant, alongside `COUNTER_ACTOR_EXAMPLE`), `crates/
  emerald-cli` (or its existing test harness, wherever plan 55's own
  `EMERALD_WORKERS` timing proof already lives) for the two negative,
  real-CLI-process cases.
- **No changes anywhere in `runtime/emerald_runtime.c`** — this leaf
  exercises `emerald_worker_pool_start`/`emerald_actor_enqueue`/
  `emerald_worker_pool_drain_and_join` exactly as plan 55 already built
  them.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. the `fib`/`Worker` compiled-and-run proof | agent-claimed-locally |
| Concurrency margin | manual two-run (`EMERALD_WORKERS=1` vs. default) timing comparison | default run's wall-clock close to the single larger call, not the sum | agent-claimed-locally |
| Negative CLI proof | real `emerald <file>` invocation on each `bad`/`bad2` source | non-zero exit, stderr names the violation | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
