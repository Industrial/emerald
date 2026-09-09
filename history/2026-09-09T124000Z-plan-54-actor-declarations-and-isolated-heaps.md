---
name: Actor Declarations and Isolated Heaps
overview: "`actor Counter ... end` + `.spawn(...)` — a new flat, non-inheriting top-level declaration reusing plan 08's class field/method syntax verbatim, whose instances live in their own arena (plan 51's mechanism) instead of the shared never-freed heap. Method calls stay ordinary synchronous calls in this plan; only isolation is proven here."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-actor
    content: "ActorDef (its own struct, not ClassDef+flag) + Item::Actor + Expr::Spawn; grammar for `actor ... end` with no `<Superclass` clause at all, and `.spawn(args)` as a new reserved-keyword call form parallel to `.new`"
    status: pending
  - id: leaf-sema-actor
    content: "ClassInfo gains is_actor: bool (mirrors plan 12's is_module exactly); actor registration reuses the same classes registry; .new rejected on actors and .spawn rejected on non-actors, both directions checked"
    status: pending
  - id: leaf-codegen-actor
    content: "ClassLayout gains is_actor: bool; ActorDef is normalized to a synthetic superclass:None ClassDef so build_class_layout/build_method_owners need zero new logic; Expr::Spawn reuses Expr::New's codegen verbatim except the allocation call targets plan 51's arena constructor instead of ctx.alloc; worked two-Counters proof plus a white-box instrumentation test proving the two arenas' address ranges are disjoint"
    status: pending
isProject: false
---

# Plan 54 — Actor Declarations and Isolated Heaps

This is plan 54 of the 48–57 batch implementing "Beyond the Ceiling" in
full — the follow-up analysis's centerpiece proposal to give Emerald an
actor-model concurrency pillar borrowed from BEAM (process isolation in
place of a shared/global GC) and Pony (a statically-typed actor language
with compile-time-checked, GC-free message safety via reference
capabilities). Per the same posture plans 17/28–47 already established,
this batch is post-v1 scope and does not touch `plan-of-plans.md` or any
other plan file. This plan is the **first of four concurrency-pillar
plans in sequence**: 54 (this one, actor declarations + isolated heaps)
→ 55 (scheduler + message passing) → 56 (compile-time message safety,
Pony-style reference capabilities) → 57 (supervision trees).

**This plan's scope is deliberately staged and narrower than "full actor
concurrency."** It builds exactly three things: the `actor` declaration
form, `.spawn()` instantiation, and per-instance heap **isolation** via
plan 51's arena mechanism. It does **not** build a scheduler, a mailbox,
or asynchronous dispatch — every method call on a spawned actor instance
in this plan compiles and executes as an ordinary, **synchronous**,
in-place call, identical in every respect to a plain class method call
today. This is a deliberate staging choice, not an oversight: proving
heap isolation actually compiles, links, and runs correctly — two
independently-allocated instances whose state genuinely does not
interfere — is a smaller, independently-testable claim than "isolated
heaps *and* a working scheduler *and* async message dispatch" landing
all at once. Plan 55 is explicitly the plan that turns these
synchronous calls into real asynchronous message sends against a
mailbox; this plan does not anticipate or partially build that
machinery.

Concrete proof this plan targets:
```ruby
actor Counter
  count: Int64

  def initialize(start: Int64) -> Void
    @count = start
  end

  def increment -> Void
    @count = @count + 1
  end

  def value -> Int64
    @count
  end
end

a: Counter = Counter.spawn(0)
b: Counter = Counter.spawn(100)

a.increment
a.increment
b.increment

puts a.value
puts b.value
```
Expected output: `2\n101\n` — an ordinary stdout proof that two spawned
`Counter` instances hold genuinely independent state (`a` incremented
twice from `0`, `b` incremented once from `100`, and neither instance's
field mutation leaks into the other). Note every call here —
`Counter.spawn(0)`, `a.increment`, `a.value` — compiles to a direct,
synchronous call at its call site; nothing about this stdout output
depends on or proves heap isolation specifically, since two ordinary
class instances allocated from the same shared heap would print
identically. Heap isolation itself is invisible in stdout, which is why
this plan also requires a second, internal instrumentation proof (see
`leaf-codegen-actor`, AC4): a white-box test showing the two `Counter`
instances' backing memory actually comes from two distinct,
non-overlapping address ranges, not merely that their field values
happen not to collide.

## Decision log

- **Actors reuse `emerald-sema`'s existing `ClassInfo`/`classes`
  registry and `emerald-codegen`'s existing `ClassLayout`/`classes`
  registry, tagged with a new `is_actor: bool`, rather than threading a
  brand-new `actors: HashMap<...>` parameter through the checker/codegen
  functions that already carry `classes`.** This is the exact same
  architectural move plan 12 made for modules (`ClassInfo` gained
  `is_module: bool` rather than a separate `modules` map — verified
  against `crates/emerald-sema/src/lib.rs` lines 66–85 this session:
  `struct ClassInfo { fields, methods, is_module: bool, superclass:
  Option<String> }`), and for the same reason: it's what makes
  `Counter.new(...)` and `Point.spawn(...)` genuine, correctly-rejected
  sema errors (`is_actor` gates `Expr::New` the same way `is_module`
  already gates it; a parallel `!is_actor` check gates `Expr::Spawn`)
  rather than silent type-system holes. An ordinary synchronous method
  call on an actor instance costs **zero new codegen** as a direct
  consequence of this reuse: `build_method_call` (verified,
  `crates/emerald-codegen/src/lib.rs` lines 1740–1833) resolves a
  receiver by looking it up in `local_classes`/`ctx.classes`/
  `ctx.method_owners` with no branch anywhere asking "is this actually a
  plain class instance and not an actor" — and this plan deliberately
  does not add one. That absence is the concrete, load-bearing proof
  that method calls stay ordinary and synchronous in this plan: there is
  no dispatch fork for actors to fall into yet. Building that fork
  (routing an actor call through a mailbox instead of a direct call) is
  exactly plan 55's job.
- **`ActorDef` is its own struct, not `ClassDef` plus a marker flag.**
  Verified against the real, current AST (`crates/emerald-parser/src/
  ast.rs`): `ClassDef { name: String, superclass: Option<String>,
  fields: Vec<Param>, methods: Vec<Function> }`. The natural precedent
  to weigh this against is `ModuleDef` (plan 12), which is *also* its
  own struct rather than a `ClassDef` reuse — specifically because
  modules omit `fields` (no instance state to hold them). Actors are
  the mirror-image case: they keep `fields` (per-instance state is the
  entire point of this plan) but, like modules, can never have a
  `superclass` (see next bullet) — so by the same reasoning `ModuleDef`
  used to justify omitting `fields`, `ActorDef` omits `superclass`:
  `ActorDef { name: String, fields: Vec<Param>, methods: Vec<Function>
  }`. This is a real, disclosed type-level statement that actors are
  flat, not a `ClassDef` with an unused `Option<String>` sitting in it
  by convention. The alternative — `Item::Actor(ClassDef)` with a
  grammar or sema rule that merely rejects a non-`None` `superclass` at
  runtime-of-the-checker — was considered and rejected: it would let an
  `actor Foo < Bar` shape parse successfully into a structurally
  well-formed `ClassDef` and only fail later, in sema, whereas giving
  `ActorDef` no `superclass` field at all makes `actor Foo < Bar`
  impossible to represent in the AST in the first place, matching the
  grammar-level rejection described next.
- **Actor inheritance (`actor Foo < Bar`) is declined at the grammar
  level, not merely rejected by a semantic check** — a stronger, more
  contrastable statement than plan 32's own cyclic-inheritance rejection
  (which is a real semantic error over an otherwise-parseable shape).
  `ClassDef`'s grammar production (plan 32, verified) is `"class"
  <name:Ident> <superclass:("<" <s:Ident> => s)?> Param* FuncDef*
  "end"`; `ActorDef`'s production is `"actor" <name:Ident> Param*
  FuncDef* "end"` — no optional `"<" Ident` clause exists in it at all.
  `actor Dog < Animal ... end` is therefore a syntax error, the same
  category of failure as writing `module Dog < Animal ... end` today
  (plan 12's `ModuleDef` grammar has no such clause either, and nothing
  in that plan's Decision log needed to say so explicitly, because no
  later plan added inheritance to revisit against). This plan states
  the contrast plainly because plan 32 already exists and a reader could
  otherwise reasonably ask "does `actor` get the `class < Superclass`
  syntax too?" — the answer is no, actors are a flat declaration,
  matching plan 12's namespace-module precedent (a flat bag of
  methods, now also fields), not plan 32's single-inheritance-chain
  precedent. If a future plan ever wants "actor subclassing," that is a
  new, separate grammar and semantics decision, not something this
  plan's absence of a clause quietly leaves half-open.
- **`.spawn(...)` is a new AST node, `Expr::Spawn(String, Vec<Expr>)`,
  deliberately parallel to but distinct from `Expr::New(String,
  Vec<Expr>)`** (verified, `ast.rs`: `New(String, Vec<Expr>)` is
  `ClassName.new(args)`). Reusing `Expr::New` for both and
  disambiguating later in sema by checking `is_actor` was considered and
  rejected: it would mean `Counter.new(0)` parses successfully (as the
  same AST shape as `Counter.spawn(0)`) and only fails once sema notices
  the receiver names an actor — exactly the failure-too-late shape the
  previous bullet already rejected for inheritance. Keeping `new` and
  `spawn` as textually and structurally distinct call forms means
  `Counter.new(0)` and `AnyClass.spawn(0)` are both real, disclosed,
  symmetric sema errors (`is_actor` must be `true` for `Expr::Spawn`'s
  receiver and `false` for `Expr::New`'s), not merely one direction of
  the check. `spawn` becomes a new reserved keyword for the same LALR(1)
  reason `new` already is (plan 08) and `module`/`class`/`actor`
  already are: `recv.spawn(args)` needs `spawn` lexically distinct from
  an ordinary method name so the grammar commits to this shape
  unambiguously, the same way `recv.new(args)` already does. Choosing a
  name distinct from `.new` is also a deliberate signal, independent of
  today's synchronous semantics: `.spawn()` reads as "this creates
  something with its own lifecycle," which becomes true starting in
  plan 55, even though nothing about *this* plan's compiled behavior
  differs from `.new()` beyond which allocator backs the instance.
- **Codegen normalizes each `ActorDef` into a synthetic `ClassDef {
  name, superclass: None, fields, methods }` before handing it to the
  existing `build_class_layout`/`build_method_owners`/
  `resolve_class_chain` machinery** (verified signatures, `crates/
  emerald-codegen/src/lib.rs` lines 147–228), rather than writing a
  parallel actor-only layout/owner-resolution function. Since an actor's
  chain is always exactly one class long (no superclass, ever),
  `resolve_class_chain` on a synthetic actor `ClassDef` trivially
  returns `vec![name]` and `build_class_layout`/`build_method_owners`'s
  existing root-to-leaf walk degenerates to "this class's own fields/
  methods, offset from zero" with no code change required — the exact
  8-bytes-per-field, declaration-order-offset scheme plan 08 established
  and plan 32 generalized is reused verbatim, not reimplemented. This
  mirrors the same "codegen independently re-derives everything about
  classes from the raw `Program`/`ClassDef` list, since there is no
  typed IR shared between sema and codegen" architectural fact plan 32's
  own Decision log already documented as standing since plan 06 — this
  plan doesn't introduce that fact, it relies on it. `ClassLayout` gains
  one new field, `is_actor: bool`, populated from the same registration
  pass that builds the synthetic `ClassDef`s.
- **`Expr::Spawn`'s codegen is `Expr::New`'s codegen (verified, `crates/
  emerald-codegen/src/lib.rs` lines 1518–1556) with exactly one line
  changed: the allocation call targets plan 51's arena constructor
  instead of `ctx.alloc`.** Concretely, `Expr::New` today: (1) looks up
  `ctx.classes[class_name]` for `layout.size`; (2) calls
  `ctx.alloc(size)` (a thin `malloc` wrapper — `emerald_alloc`,
  `runtime/emerald_runtime.c`, plan 08) to get a raw pointer; (3)
  resolves `initialize` via `ctx.method_owners` and calls it with that
  pointer as the leading `self` argument, if the class declares (or
  inherits) one; (4) returns the pointer as `ValKind::Ptr`. `Expr::Spawn`
  differs only at step (2): it calls a new `ctx.arena_new(size)` instead
  of `ctx.alloc(size)` — everything else, including the `initialize`
  resolution and the "no `initialize` declared, no call made"
  fallback, is identical. Since actors are flat (no chain to walk for
  `initialize`'s defining class), the `method_owners` lookup for an
  actor's own `initialize` always resolves to the actor's own name —
  `build_method_owners`'s chain-overlay generality buys nothing here,
  but costs nothing either, since it degenerates to the same single-
  entry map a hand-written actor-only version would produce; reusing it
  avoids a second, parallel "resolve this method for a flat
  declaration" helper that would just be `build_method_owners` with the
  loop trivially bounded to one iteration.
- **Depends on plan 51's assumed contract, verified against its real
  source if it has landed by execution time.** As of this plan's
  authoring, `history/` has no plan-51 file on disk yet (checked this
  session — files run through plan 47; 48–57 are being authored in
  parallel by other agents in this same batch). The assumed contract,
  per this plan's own brief: plan 51 introduces a region/arena allocator
  — a bump-allocated buffer, bulk-freed on scope exit — as an
  alternative to `emerald_alloc`'s "never free" default, "designed
  generally enough to be reused as the heap backing one actor instance,
  freed on actor termination instead of function return." This plan
  takes the narrowest possible slice of that contract: a single runtime
  entry point, assumed here as `emerald_arena_new(capacity: i64) ->
  void*` (mirroring `emerald_alloc(long long) -> void*`'s existing
  shape exactly), which allocates a fresh arena sized to hold exactly
  one actor instance's flattened fields and returns a pointer usable
  directly as that instance's `self` pointer. Whoever executes this
  plan's `leaf-codegen-actor` must re-verify this exact function name
  and signature against plan 51's real, landed source before wiring
  `ctx.arena_new` to it — the same "re-verify against real source at
  execution time" discipline plan 32 already applied to plan 16's
  in-flight Cranelift→LLVM migration.
- **No actor termination exists yet, so nothing is freed in this plan —
  the same disclosed, deliberate non-feature plan 08 already accepted
  for `emerald_alloc`.** Plan 51's fuller contract talks about an
  arena being "freed on actor termination"; this plan has no scheduler,
  no actor lifecycle, and therefore no termination event to free it on
  — an actor spawned by `leaf-codegen-actor`'s worked example lives, and
  leaks its arena, for the remainder of the process, exactly as every
  `Counter.new(...)`-allocated instance already does today. This is not
  a bug this plan silently introduces; it is the same accepted
  simplification plan 08's Decision log named for `emerald_alloc`
  ("freeing is not implemented and not needed for this milestone's
  straight-line example"), now applying to arenas too, until plan 55's
  scheduler (or a later plan) gives an actor an actual end-of-life
  moment to free at.
- **Post-authoring correction: plan 51's real, landed API is three
  functions, not the single placeholder assumed above.** Plan 51's
  actual signatures (verified against its real file): `void
  *emerald_region_create(void)` (no capacity argument — the region
  grows via internal chunking, not a single fixed-size buffer),
  `void *emerald_region_alloc(void *region, long long size)` (bump-
  allocates `size` bytes from `region`, returning a pointer), and
  `void emerald_region_destroy(void *region)`. This plan's own guessed
  `emerald_arena_new(capacity: i64) -> void*` (a single call producing
  a `self`-usable pointer directly) does not match this shape and must
  not be wired up as written. The real construction sequence
  `leaf-codegen-actor` must use instead: call `emerald_region_create()`
  once per `.spawn`, store the returned region handle as part of the
  actor's own runtime representation (a field alongside `self`, not
  discarded), then call `emerald_region_alloc(region, field_size)`
  against that handle to obtain the pointer used as `self` — two calls
  and a retained handle, not one call and a bare pointer. Every
  `ctx.arena_new`/`emerald_arena_new` reference in this plan's leaves,
  acceptance criteria, and file-structure sections below is stale
  against this corrected sequence; `ctx.region_create`/`ctx.region_
  alloc` (or equivalently named bindings to the real two functions)
  replace `ctx.arena_new` wherever it appears, and the actor's field
  layout gains one extra machine word to hold the retained region
  handle. This does not change this leaf's own acceptance criteria in
  substance (an actor still gets an isolated, arena-backed heap,
  proven the same way), only the exact runtime call shape used to get
  there.
- **This plan does not redirect an actor method's own internal
  allocations into that actor's arena — a real, disclosed gap, not a
  silent omission.** `Expr::Spawn` arena-allocates the instance's own
  field storage. If a `Counter` method body were to call, say,
  `Array.new(3)` or `OtherClass.new(...)`, that call still goes through
  the ordinary shared, never-freed `ctx.alloc`/`ctx.alloc_zeroed` path,
  unchanged — it does not know it is executing "inside" an actor at
  all, because no actor-execution-context concept exists in codegen
  yet (there is no scheduler to define what "inside an actor" even
  means beyond "the method currently executing happens to have been
  reached via a `.spawn()`-allocated `self`"). True "everything an actor
  touches lives in its own isolated heap" needs an implicit
  allocation-context threaded through every method body call, which is
  real, separate, and larger scope than proving the instance's own
  field storage is isolated — this plan's worked example never triggers
  the gap (its methods only touch `Int64` fields), and the gap is named
  here so it isn't mistaken for solved.
- **No runtime tag is written onto the pointer itself; the "this is an
  actor instance" tag plan 55/56 will need is the existing static
  `ClassLayout`/`ClassInfo.is_actor` bit, consulted by the receiver's
  statically-known declared type — not a vtable, not RTTI, not a bit
  packed into the pointer's low bits.** This project's identity
  constraints explicitly rule out dynamic/virtual dispatch, vtables, and
  runtime reflection; a pointer-level runtime tag would be exactly the
  kind of ad hoc RTTI those constraints exclude, and this compiler
  already resolves everything else statically (plan 32's Decision log:
  "there's no vtable, no runtime type tag consulted for dispatch
  anywhere in codegen"). `Expr::Spawn` therefore returns the same
  `ValKind::Ptr` `Expr::New` already returns — no new `ValKind` variant
  is added. The forward-looking "tag" plan 55/56 will actually consult
  is `ctx.classes[name].is_actor`, reached the same way
  `build_method_call`'s existing `local_classes` side-table already
  reaches a receiver's declared class name today; this plan adds the
  bit that makes that lookup possible later, without adding any new
  per-value runtime representation now.
- **Tooling citations, scoped narrowly.** Plan 35 (debug info) is not
  otherwise touched by this plan — DWARF line-table generation doesn't
  change because a pointer happens to come from an arena instead of the
  shared heap — but is worth naming once: if plan 35 (or a later
  memory-debugging extension of it) ever wants to show which heap
  region a stack frame's `self` pointer falls in, an actor instance's
  arena is a new kind of region that didn't exist before this plan, and
  whoever extends plan 35 that way should know actor arenas exist; this
  plan does not implement any debugger-visible region annotation itself.
  Plan 21 (LSP symbols and navigation) is similarly not modified here,
  but `actor` becoming a new top-level, `Item`-producing keyword means
  `emerald-driver::symbols`/`collect_symbols` (plan 21's own
  `SymbolTable`/`ClassSymbol` machinery, which today only walks
  `Item::Function`/`Item::Class`/`Item::Module`) will need an
  `Item::Actor` arm added whenever plan 21 is next revisited, the same
  way it would need one for any new top-level declaration kind — this
  plan does not modify `emerald-sema`'s `collect_symbols` itself, since
  doing so is plan 21's surface area, not this one's, and no example in
  this plan's own scope needs go-to-definition on an actor name to work.

## Leaf: leaf-ast-actor

### 1. Context
- Why: no AST shape exists for an actor declaration or a `.spawn(...)`
  call at all (verified this session against the real, current
  `crates/emerald-parser/src/ast.rs`: `Item` has `Function`, `Class`,
  `Module`, `Stmt`, `Require`, `Error` — no `Actor` variant; `Expr` has
  `New`/`MethodCall`/`Call` — no `Spawn` variant).
- Target state: `ActorDef { name: String, fields: Vec<Param>, methods:
  Vec<Function> }` (see Decision log for why this is its own struct,
  not `ClassDef` plus a flag — deliberately omits `superclass`, unlike
  `ClassDef`); `Item::Actor(ActorDef)`; `Expr::Spawn(String, Vec<Expr>)`
  (deliberately distinct from `Expr::New`, see Decision log). Grammar
  gains a new reserved keyword `actor` and production `"actor"
  <name:Ident> <fields:Param*> <methods:FuncDef*> "end"` — structurally
  identical to `ClassDef`'s production minus the optional `"<" Ident`
  clause (no actor-inheritance grammar shape exists to parse at all).
  Grammar also gains reserved keyword `spawn` and a call-site production
  parallel to `.new`'s: `<recv:Ident> "." "spawn" "(" <args:...> ")" =>
  Expr::Spawn(recv, args)`.
- Dependencies: none (additive AST/grammar change, same posture as every
  prior plan's `leaf-ast-*`/`leaf-parser-*` leaves).

### 2. Acceptance Criteria
1. `ActorDef`/`Item::Actor`/`Expr::Spawn` exist exactly as described
   above.
2. This plan's full worked example (the `Counter` actor plus both
   `.spawn(...)` call sites and both `.increment`/`.value` method calls)
   parses into the expected `Item::Actor(ActorDef { name: "Counter", ...
   })` and `Expr::Spawn("Counter", [Int(0)])` / `Expr::Spawn("Counter",
   [Int(100)])` shapes; `a.increment`/`a.value` parse as the *existing*
   `Expr::MethodCall`/`Stmt::Expr` shapes — no new AST node for the
   call sites themselves, only for `spawn` construction.
3. `actor Dog < Animal ... end` is a real, reported parse error (not a
   silently-accepted `ActorDef` with a superclass smuggled in some other
   way) — proof the grammar-level decline from the Decision log is
   real, not just documented intent.
4. `class Foo ... end` followed by `Foo.spawn(...)` (a plain class,
   `.spawn`-called) and `actor Bar ... end` followed by `Bar.new(...)`
   (an actor, `.new`-called) both parse successfully at this leaf —
   this leaf is grammar-only; rejecting these two shapes is
   `leaf-sema-actor`'s job (AC3/AC4 there), and this leaf's own
   acceptance criteria must not accidentally couple parsing to that
   later semantic check.
5. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, the inheritance/module/collections/exception examples)
   still parses identically — `actor`/`spawn` becoming reserved must not
   collide with any existing identifier used in those fixtures (verified
   directly: no existing example declares a variable, method, class, or
   field named `actor` or `spawn`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new actor/spawn parse tests and the AC3 negative case | agent-claimed-locally |

---

## Leaf: leaf-sema-actor

### 1. Context
- Why: `emerald-sema`'s `check_program`/`ClassInfo` registry has no
  concept of an actor — it cannot register `Counter`'s fields/methods,
  resolve `Counter.spawn(0)`'s constructor signature, or reject
  `Counter.new(0)`/`SomeClass.spawn(...)` as the mismatched-construction-
  form errors they should be.
- Target state: `ClassInfo` gains `is_actor: bool` alongside its
  existing `is_module: bool` (mirrors plan 12's own tagging exactly —
  see Decision log). `Item::Actor(ActorDef)` registration reuses the
  same two-pass shape `check_program` already applies to classes/
  modules (verified, `crates/emerald-sema/src/lib.rs`: a first pass
  registers names, a second resolves field/method signatures via the
  existing `function_signature`/`resolve_type` helpers unchanged) —
  concretely, a new `actor_info(a: &ActorDef, classes: &HashMap<String,
  ClassInfo>) -> Result<ClassInfo, Diagnostic>` mirroring `module_info`'s
  own directness (verified signature, line 260: `fn module_info(m:
  &ModuleDef, classes: ...) -> Result<ClassInfo, Diagnostic>`) rather
  than `build_flattened_class_info`'s chain-walking version (line 203) —
  an actor has no superclass to flatten against, so the module's
  single-pass shape is the correct one to mirror, not the class's
  chain-walking one. `Expr::New`'s existing arm gains one more rejection
  condition (`classes[name].is_actor` → reject, "actors are constructed
  with `.spawn`, not `.new`"), symmetric to its existing `is_module`
  rejection; a new `Expr::Spawn` arm in `infer_expr_type` requires
  `classes[name].is_actor == true` (reject otherwise, "`.spawn` is only
  valid on an actor"), checks `args` against the resolved `initialize`
  signature the same way `Expr::New` already does, and infers
  `Type::Class(name)` — actors do not get their own `Type` variant,
  reusing `Type::Class(String)` unchanged, since nothing downstream
  (assignment compatibility, field/param typing) needs to distinguish
  "this variable holds a class instance" from "this variable holds an
  actor instance" at the type-checking level in this plan (that
  distinction only starts to matter for plan 56's reference-capability
  checking, not for ordinary type compatibility here).
- Dependencies: `leaf-ast-actor`.

### 2. Acceptance Criteria
1. This plan's full worked example type-checks `Ok(())`.
2. `ClassInfo.is_actor` exists as described; `Counter`'s registered
   `ClassInfo` has `is_actor: true`, `fields: {"count": Int64}`,
   `superclass: None`.
3. `Counter.new(0)` (an actor, `.new`-called) is rejected with a
   diagnostic naming actors as `.spawn`-only, not silently accepted and
   not a panic.
4. Given a plain `class Foo ... end`, `Foo.spawn()` (a non-actor,
   `.spawn`-called) is rejected with a diagnostic naming `.spawn` as
   actor-only, not silently accepted and not a panic — the symmetric
   direction of AC3, proving this is a real two-way gate and not just a
   one-off special case for `.new`.
5. `Counter.spawn(0, 1)` (wrong arity against `initialize`) and
   `Counter.spawn("x")` (wrong argument type against `initialize`) are
   both rejected the same way an ordinary `Expr::New` arity/type
   mismatch already is — proof `.spawn`'s constructor-argument checking
   is real, not a rubber-stamp.
6. `actor Dog < Animal ... end` cannot even reach this leaf's checks (it
   is already a parse error per `leaf-ast-actor` AC3) — this leaf does
   not additionally need to defend against a `superclass: Some(_)`
   `ActorDef` reaching `actor_info`, since the AST makes that shape
   unrepresentable; this is called out so the absence of such a
   defensive check here isn't mistaken for an oversight.
7. Regression: every prior plan's sema test suite (`hello.em`, `Point`,
   inheritance, modules, everything already listed in `crates/
   emerald-sema/src/lib.rs`'s existing `#[cfg(test)] mod tests`) still
   passes unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 5 new cases above (AC2–AC5) | agent-claimed-locally |

---

## Leaf: leaf-codegen-actor

### 1. Context
- Why: nothing compiles `actor`/`Expr::Spawn` to machine code yet, and
  nothing in `emerald-codegen` allocates from anywhere but the shared,
  never-freed `emerald_alloc` heap (verified, `crates/emerald-codegen/
  src/lib.rs` lines 1518–1556: `Expr::New`'s only allocation path is
  `builder.build_call(ctx.alloc, &[size_val.into()], "newtmp")`).
- Target state, per the Decision log: (1) `ClassLayout` gains
  `is_actor: bool`; (2) each `Item::Actor(ActorDef)` is normalized into
  a synthetic `ClassDef { name, superclass: None, fields, methods }`
  before being fed into the existing `resolve_class_chain`/
  `build_class_layout`/`build_method_owners` (no new layout/owner-
  resolution logic — see Decision log for why the existing chain-walk
  degenerates correctly to a one-class chain); (3) `Ctx` gains one new
  field, `arena_new: FunctionValue<'ctx>`, declared in the object file
  the same way `ctx.alloc` already is (an `extern "C" fn(i64) -> *mut
  u8`-shaped declaration, bound at link time to plan 51's runtime
  symbol — assumed name `emerald_arena_new`, re-verify against plan
  51's real source per the Decision log); (4) `Expr::Spawn(class_name,
  args)`'s codegen is `Expr::New`'s codegen with `ctx.alloc` replaced by
  `ctx.arena_new` at the one allocation call site, otherwise byte-for-
  byte identical (including the `initialize`-via-`method_owners`
  resolution and its "no `initialize` declared" no-op fallback); (5)
  `build_method_call` is **not modified at all** — an actor's method
  calls flow through the exact same `local_classes`/`ctx.classes`/
  `ctx.method_owners` path a plain class instance's calls already do,
  since both live in the same registries (this is the concrete,
  checkable proof that method calls on an actor are ordinary
  synchronous calls in this plan: there is no code path anywhere that
  even distinguishes an actor receiver from a class receiver at
  call-site codegen).
- Dependencies: `leaf-sema-actor` (codegen runs on already-checked
  input, same contract as every prior codegen plan); plan 51's
  `emerald_arena_new`-shaped runtime primitive (see Decision log for the
  assumed contract and the required re-verification step).

### 2. Acceptance Criteria
1. This plan's full worked example, compiled, linked, and run, prints
   `2\n101\n` — real executed proof that two `.spawn`-ed `Counter`
   instances hold independent state under ordinary, synchronous
   `.increment`/`.value` calls.
2. `runtime/emerald_runtime.c` (or wherever plan 51 has already landed
   its arena primitive by execution time — verify before adding a
   duplicate) gains `emerald_arena_new(long long) -> void*`, sized to
   receive exactly one instance's flattened field storage and usable
   directly as that instance's `self` pointer — no separate "create
   arena, then sub-allocate within it" step is needed in this plan's
   scope, since exactly one object is ever placed in an actor's arena
   here (see Decision log's disclosed gap about actor methods' own
   internal allocations not yet being redirected into it).
3. Calling a method on an actor instance whose declared class is
   spelled correctly but not actually registered (a defensive,
   sema-bypassing shape — mirrors every prior codegen plan's AC4/AC2
   standard) returns a descriptive `Err`, not a panic; likewise, a
   `Expr::Spawn` targeting a class name not tagged `is_actor` in
   `ctx.classes` (should be unreachable once sema runs first, defended
   anyway) returns a descriptive `Err`.
4. **Internal instrumentation proof — two arenas are backed by disjoint
   memory ranges, not merely "print the same field values twice."**
   Add a white-box test, alongside `emerald-codegen`'s existing
   `compile_link_run`-based tests in the same file: hand-build (via
   `inkwell` directly, not by parsing `.em` source) a small synthetic
   `main` that calls `ctx.arena_new` twice at `Counter`'s real field
   size (8 bytes — one `Int64` field), `ptrtoint`s each returned pointer
   to `i64`, and passes both through the crate's own existing
   `emerald_print_i64` declaration; compile/link/run this synthetic
   module through the same object-emission-and-`cc`-link pipeline
   `compile_link_run` already uses for every other test in this file;
   parse the two printed decimal addresses from the real, captured
   stdout of the actually-run binary; assert their `[addr, addr+8)`
   ranges do not overlap. This is a real, executed, address-arithmetic
   proof of heap isolation — not a source-code-level assertion that two
   `build_call` sites exist, and not merely re-running the ordinary
   `.em`-level worked example (whose stdout, `2\n101\n`, would look
   identical whether or not the two instances were actually isolated).
5. Regression: every existing `emerald-codegen` test (functions,
   classes, inheritance, modules, lambdas, exceptions, arrays,
   arithmetic, strings, case/for-in/blocks) continues to pass unmodified
   — this leaf is additive (`Expr::Spawn` is a new match arm; `Ctx`
   gains a field nothing else reads).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`, `runtime/
  emerald_runtime.c` (or confirm plan 51 already supplies the needed
  primitive there, per AC2)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. the real linked-and-run `2\n101\n` proof (AC1) and the disjoint-address instrumentation test (AC4) | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- Any form of asynchronous dispatch, mailboxes, or a scheduler — plan
  55's job entirely, not partially started here (see Decision log and
  the plan's own opening framing).
- Compile-time message safety / Pony-style reference capabilities — the
  identity constraint this project has already committed to (no
  reflection, no dynamic dispatch) is upheld by this plan's static-only
  `is_actor` tag, but the *checking* of what may safely be shared
  between actors is plan 56's job.
- Supervision trees / actor failure and restart semantics — plan 57's
  job; this plan has no notion of an actor "failing" independent of an
  ordinary uncaught exception today.
- Freeing an actor's arena — no actor termination event exists yet (see
  Decision log); this plan leaks arenas exactly as plan 08's
  `emerald_alloc` always has, disclosed, not silently different.
- Redirecting an actor method's own internal allocations (`Array.new`,
  `OtherClass.new`, lambdas) into that actor's own arena — a real,
  disclosed gap (see Decision log); only the spawned instance's own
  field storage is proven isolated in this plan.
- Actor inheritance in any form — declined at the grammar level (see
  Decision log), not merely deferred; a future plan proposing it would
  need to add back a superclass-shaped clause `ActorDef`'s grammar
  currently has no room for at all.
- `super`/multiple actors composing behavior via mixins — never
  proposed for classes either (plan 32); doubly out of scope for the
  flatter actor form.
- `emerald-sema::collect_symbols`/LSP go-to-definition/completion
  awareness of `Item::Actor` (plan 21) and any DWARF/debugger-visible
  actor-arena region annotation (plan 35) — both named as forward
  pointers in the Decision log, neither implemented here.
