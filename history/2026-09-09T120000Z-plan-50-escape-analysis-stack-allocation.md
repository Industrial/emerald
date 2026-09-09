---
name: Escape Analysis and Stack Allocation
overview: "A conservative, intraprocedural escape analysis that stack-allocates a `ClassName.new(...)` instance instead of heap-allocating it whenever the compiler can prove the reference never leaves the current function — the first of the batch's two pure-implementation memory-model plans that reduce allocation pressure so Emerald never needs a tracing collector."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-escape-detection
    content: "A new intraprocedural prepass over one function body's `Vec<Stmt>` that finds every `Stmt::Let`-bound `ClassName.new(...)` name never returned, never stored into a field/array element, and never passed as a call argument (receiver position included) — producing a `HashSet<String>` of provably non-escaping instances, verified directly against constructed ASTs, not yet wired into codegen"
    status: pending
  - id: leaf-stack-allocation-codegen
    content: "Wire `leaf-escape-detection`'s result into a new `Stmt::Let{ name, value: Expr::New(..), .. }` special-cased arm (alongside the existing `Expr::Lambda`/`Expr::ArrayNew` special cases) that allocas `layout.size` bytes in the function's entry block instead of calling `ctx.alloc`, proven by both worked programs' compiled-and-run output staying byte-for-byte identical to the heap-allocated baseline"
    status: pending
  - id: leaf-escape-instrumentation-and-report
    content: "A non-breaking `compile_to_object_with_stats` returning `EscapeStats { stack_allocated, heap_allocated }`, plus an `--emit=escape-report` CLI flag printing it — the internal proof mechanism this plan needs since stack- vs heap-allocated instances are behaviorally indistinguishable from stdout alone"
    status: pending
isProject: false
---

# Plan 50 — Escape Analysis and Stack Allocation

This is plan 50 of the 48–57 batch implementing "Beyond the Ceiling" in
full — ten independent sibling plans, post-v1 scope, same posture as
plans 17/31/35 (it does not touch
[`plan-of-plans.md`](../../history/2026-09-08T174011Z-plan-of-plans.md)
or any other plan file). This plan and its sibling, plan 51
(scope-based arenas), are the batch's two pure-implementation
memory-model plans, and both bear directly on Emerald's identity
constraint of **no tracing garbage collector**: the only way a
GC-free language stays fast under real allocation pressure is by not
allocating on the heap in the first place whenever it's provably safe
not to. This plan attacks that from the "prove a specific reference
never escapes" angle; plan 51 attacks it from the "bound a whole
scope's allocations to one arena" angle — they are independent,
non-overlapping techniques over the same `emerald_alloc` call site.

Concrete proof this plan targets — two functions, same class,
opposite escape outcomes:

```ruby
class Point
  x: Int64
  y: Int64

  def initialize(x: Int64, y: Int64) -> Void
    @x = x
    @y = y
  end
end

def distance_squared(x: Int64, y: Int64) -> Int64
  p: Point = Point.new(x, y)
  x * x + y * y
end

def make_point(x: Int64, y: Int64) -> Point
  p: Point = Point.new(x, y)
  p
end

puts distance_squared(3, 4)
```

`distance_squared` constructs a real `Point` (its constructor really
runs, really writes `@x`/`@y` into the instance's own memory) but
never reads `p` back out, returns it, or passes it anywhere afterward
— `p` is provably non-escaping and this plan stack-allocates it.
`make_point` constructs an identical `Point` and returns it directly —
`p` escapes by the plainest possible case (rule (a) below) and stays
heap-allocated. Compiled, linked, and run, `puts distance_squared(3,
4)` still prints `25`, unchanged from today's all-heap baseline — see
the Decision log for why observable output can never be this plan's
proof of the optimization itself, and what is instead.

## Decision log

- **Verified this session, directly against the current LLVM backend
  (`crates/emerald-codegen/src/lib.rs`), that `Expr::New` unconditionally
  heap-allocates with no escape or liveness analysis of any kind.** The
  `Expr::New(class_name, args)` arm of `build_expr` (L1518–1556) looks up
  the class's `ClassLayout`, builds `size_val = layout.size` and calls
  `builder.build_call(ctx.alloc, &[size_val.into()], "newtmp")`
  unconditionally — `ctx.alloc` is `emerald_alloc` (declared in
  `compile_to_object` at L4125, a bare `malloc`-equivalent per
  `runtime/emerald_runtime.c`, distinct from the `calloc`-backed
  `ctx.alloc_zeroed` plan 25 added for `Array.new`/`Hash[K,V]`). Every
  `.new` call plan 08 (object model) introduced, and every subclass
  instance plan 32 (class inheritance) added on top, takes this exact
  path today — there is no existing escape/liveness pass anywhere in
  this crate to build on or conflict with; this plan adds the first one.
- **Architecture: a new small dataflow prepass, not an inline check at
  the `New` call site — chosen and justified against the real
  alternative.** `build_expr`'s `Expr::New` arm has no way to see the
  rest of the function body: it is reached bottom-up, one expression at
  a time, from deep inside a recursive descent that already threads
  `vars`/`local_classes`/`local_array_elem_types`/`ctx` through every
  call (`build_expr`'s own signature carries six parameters, `build_stmt`
  eleven, both verified this session) — teaching it to look *ahead* at
  every later statement in the same body to answer "does this reference
  ever escape" would mean adding whole-function lookahead state to
  every one of those already-large recursive signatures, for a fact
  that's identical on every visit to the same call site. This codebase
  already has the answering pattern for exactly this shape of problem:
  `collect_lets` (L523–592) and `collect_lambda_infos` (L467–510) are
  both one-time prepasses over a function's `Vec<Stmt>`/`Program`,
  computed once before `build_function_body` walks statements, and their
  results (`decls: Vec<(String, ValKind)>`, `HashMap<String,
  LambdaInfo>`) are simple, read-only tables consulted by codegen as it
  goes — not new parameters threaded through every recursive call. This
  plan's escape analysis is the same shape of fact (`HashSet<String>` of
  non-escaping `Let`-bound names) computed the same way, at the same
  point in the pipeline `collect_lets`/`prealloc_lets` already occupy
  (`define_user_function` positions the builder at the function's entry
  block, then calls `collect_lets` + `prealloc_lets` at L3834–3836,
  before `build_function_body` runs at all — `define_method`,
  `define_lambda`, and `define_main` each do the analogous thing for
  their own bodies, and all four need the equivalent extension, not just
  `define_user_function`).
- **Escape rule, stated precisely and conservatively.** A `Stmt::Let`-
  bound `ClassName.new(...)` instance named `n` escapes the current
  function if, anywhere else in the same function body: (a) `n` appears
  in a `Stmt::Return`'s expression; (b) `n` appears as the `value` of a
  `Stmt::SetField` or `Stmt::SetIndex` (stored into another object's
  field or an array/hash element); (c) `n` appears as an argument to
  *any* `Expr::Call`, `Expr::New`, or `Expr::MethodCall` — **including
  receiver position.** If none of (a)/(b)/(c) hold anywhere in the
  function, `n` is provably non-escaping.
- **Receiver position counts as an argument because it mechanically is
  one in this compiler's own ABI — verified, not assumed.**
  `build_method_call` (L1740–1833) builds `let mut call_args: Vec<...> =
  vec![recv_val.into()]` and pushes the user-written arguments after it
  (L1816); `Expr::New`'s own compiler-generated `initialize` call does
  the identical thing with the freshly allocated `ptr` (L1538,
  `vec![ptr.into()]`). There is no separate "receiver slot" in the LLVM
  call this backend emits — `self`/receiver is an ordinary leading
  positional argument, indistinguishable at the call site from any other
  argument. A conservative, non-interprocedural analysis that didn't
  also treat receiver position as escaping would be unsound: nothing in
  this function's own body proves the *callee* doesn't stash that
  pointer somewhere longer-lived (a static, another object's field, its
  own return value) — proving that requires looking inside the callee,
  which is exactly the interprocedural step this plan declines (below).
  One narrow, disclosed exception: the `initialize` call `Expr::New`
  itself generates (L1532–1554) is not a rule-(c) escape of `n` — it is
  intrinsic to construction, not a source-level use, it always returns
  before `New` itself does, and skipping this exemption would make the
  whole optimization vacuous (literally every instance, escaping or not,
  triggers exactly this call as part of being built at all).
- **Disclosed real cost of exempting nothing else: reading a
  non-escaping instance's fields back out, from outside its own
  constructor, is not possible in v1 without losing non-escaping
  status** — because the *only* way to read a field from outside a
  class is a method call (plan 33's `read` field-access sugar included:
  its Decision log states plainly that `obj.field` "synthesizes an
  ordinary zero-arg Function... into `ClassDef.methods`" — it is still a
  `Expr::MethodCall` at codegen time, still passes its receiver as
  `call_args[0]` like any other method call). This plan does not carve
  out a special "trivial accessor" exception for `read`-sugared calls,
  even though such a carve-out is narrow and arguably always sound (a
  synthesized accessor's entire body is exactly `Return(InstanceVar(f))`
  and provably never stashes `self`) — adding even that one narrow,
  bounded peek into a callee's body is a first step *toward*
  interprocedural analysis, and this plan's own scope explicitly declines
  that (next bullet), so it declines this too rather than taking it
  piecemeal. The consequence, stated plainly: v1's stack-allocation
  candidates are realistically "constructed, exercised only through
  their own constructor's side effects, then discarded" instances (this
  plan's own `distance_squared` worked example is exactly this shape) —
  not "constructed, read back out via an accessor, then discarded." That
  broader case is real, valuable, and left to the interprocedural
  follow-on below.
- **v1 does not attempt interprocedural escape analysis, and that is a
  disclosed, materially larger future feature, not an oversight.**
  Determining that a call doesn't leak its receiver/argument requires
  analyzing (or conservatively summarizing) the callee's own body,
  transitively through whatever it calls — a whole-program or
  whole-call-graph analysis. Mature JIT escape analyzers took the same
  path historically: intraprocedural, method-local escape analysis
  shipped first (recognizing non-escaping allocations only within a
  single compiled method, enabling scalar replacement / stack allocation
  for that narrower case), with call-graph-aware, interprocedural
  variants (able to see through simple non-virtual callees, e.g. via
  aggressive inlining or dedicated summaries) arriving as a distinctly
  later, larger engineering effort. This plan follows that same
  intraprocedural-first ordering deliberately, not as a shortcut.
- **Sizing correctness depends on plan 32's per-subclass field layout —
  this is the concrete reason plan 32 is a real dependency, not a
  formality.** The stack alloca this plan emits must be exactly
  `layout.size` bytes, where `layout` is the same `ClassLayout` the
  heap path already uses. `build_class_layout` (L180–204) walks
  `resolve_class_chain`'s root-to-leaf ancestor list (L147–171) and
  accumulates every ancestor's fields at 8 bytes each — a `Dog` stack
  slot must be sized for `Dog`'s full inherited layout (`Animal`'s
  fields plus `Dog`'s own), not just `Dog`'s locally declared fields.
  Getting this wrong would silently corrupt whichever inherited field
  landed past the truncated allocation's end — this plan reuses
  `ClassLayout` exactly as constructed today rather than recomputing a
  narrower size.
- **Why a plain byte-array alloca, not a typed LLVM struct, and why the
  swap is otherwise invisible to every other codegen path that touches
  an instance.** `field_ptr` (L1719–1731) already GEPs into `base_ptr`
  using `context.i8_type()` regardless of what `base_ptr` actually
  points to — fields are addressed by raw byte offset, not by a typed
  struct member index. Combined with `inkwell = { version = "0.10",
  features = ["llvm21-1"] }` (`crates/emerald-codegen/Cargo.toml`,
  verified this session) meaning every LLVM pointer is the single
  opaque `ptr` type at this LLVM version — a `builder.build_alloca`
  returning a stack `ptr` and `ctx.alloc`'s call returning a heap `ptr`
  are the same LLVM type, used identically by `load_field`,
  `build_method_call`'s receiver argument, and every other consumer of
  an instance pointer. The stack-allocation path this plan adds is a
  same-shape substitution at exactly one call site per non-escaping
  `Let`, not a new representation every downstream consumer needs to
  learn about.
- **The stack alloca is placed in the function's entry block, once per
  syntactic `Let` site, following the same precedent `prealloc_lets`
  already establishes for ordinary locals** (`define_user_function`
  positions the builder at `entry` and calls `collect_lets`/
  `prealloc_lets` *before* walking the body, L3816–3836) — this matters
  concretely for a non-escaping `New` written inside a loop body: an
  `alloca` built at the point `build_stmt` actually visits the `Let` (a
  program point that re-executes every loop iteration) would grow the
  stack frame's live-alloca count on every iteration in unoptimized
  output, since `alloca` is not popped until the function returns, not
  at the end of a loop iteration or block. Placing exactly one alloca
  per syntactic site in the entry block — reused across every dynamic
  execution of that `Let`, the identical discipline `prealloc_lets`
  already applies to ordinary named locals — avoids this entirely.
- **Behavior is provably identical either way, so this plan needs an
  internal instrumentation proof, not a stdout proof — stated
  explicitly as a deliberate adaptation of this project's usual
  compiled/linked/run acceptance pattern.** Every other plan in this
  project's history proves its feature by a program's observable
  output changing in a specific, predicted way. This plan's entire
  point is that a stack-allocated and a heap-allocated `Point` behave
  *identically* from the compiled program's own perspective — same
  field values, same method dispatch, same `puts` output. `stdout`
  alone cannot distinguish "this optimization fired" from "this
  optimization was silently skipped and the heap path ran anyway." This
  plan therefore adds a second, internal proof surface —
  `EscapeStats`/`compile_to_object_with_stats`, `leaf-
  escape-instrumentation-and-report` — specifically so "N stack, M heap"
  can be asserted directly, in addition to (never instead of) the
  ordinary compiled-and-run output-unchanged proof.
- **Debug-info interaction (plan 35) is real but not this plan's
  problem yet — verified against plan 35's own stated v1 scope, not
  assumed.** Plan 35 (`2026-09-09T101000Z-plan-35-debug-info.md`, read
  in full this session) states its own scope plainly: "v1 scope is
  line-table-only debug info, not variable/type debug info... Making
  `print sum` show a real, typed value inside a debugger needs DWARF
  variable DIEs with per-variable, per-scope location expressions... a
  real, substantially larger scope, explicitly deferred." Since plan 35
  does not yet emit any per-variable location expression for *any*
  local (stack-allocated `Int64`, heap-pointer `Point`, or otherwise),
  there is no existing debugger-visible location mechanism this plan
  could break today. The real, disclosed follow-on interaction: whenever
  plan 35 is extended to variable DIEs, a stack-allocated instance needs
  a frame-relative location expression (DWARF's `DW_OP_fbreg` family,
  describing an offset from the frame base) where a heap-allocated
  instance's location would instead describe "a pointer value, then
  follow it" — the two need genuinely different location-expression
  shapes for a debugger to print `p.x` correctly, and whichever plan
  builds variable DIEs must know which allocation strategy backs each
  local. This plan discloses that requirement here, for plan 35 (or
  whichever later plan adds variable DIEs) to pick up — it does not
  attempt to build variable DIEs itself.
- **Out of scope, explicitly:**
  - Interprocedural escape analysis (see above) — real, disclosed,
    materially larger future feature.
  - Stack-allocating `Array.new`/`Hash[K,V]` literals, lambda capture
    environments, or the ordinary `ArrayLit`/`HashLit` heap allocations
    (`build_array_lit`, `build_hash_lit`, `build_lambda_let` — L1855,
    L1655, L2229, all unconditional `ctx.alloc`/`ctx.alloc_zeroed` calls
    verified this session). These are real, structurally similar
    optimization targets but a distinct leaf's worth of work each (an
    array's size is often runtime-computed, not a fixed `layout.size`)
    — this plan is scoped to class `New` instances only, matching its
    own name and the worked example.
  - A `New` reassigned through `Stmt::Assign`/`Stmt::MultiAssign`
    (plan 31) rather than bound fresh via `Stmt::Let` — v1's escape
    detection only recognizes the `Stmt::Let{ name, value: Expr::New }`
    shape; a `New` whose result flows through a later reassignment is
    conservatively left heap-allocated. Real, small, deferred extension.
  - A `New` used as a bare, unbound statement (`Point.new(1, 2)` with
    no `Let` at all) or nested directly inside another call's argument
    list — the former is vanishingly rare in practice and not worth a
    dedicated leaf; the latter already escapes trivially under rule (c)
    the moment it's written, so there is no missed optimization by not
    special-casing it.

## Leaf: leaf-escape-detection

### 1. Context
- Why: nothing in this crate currently answers "does this local ever
  leave the function" for any name at all — this leaf builds that fact
  in isolation, verified against the AST directly, before anything in
  codegen depends on it (matches this plan's chosen architecture in the
  Decision log).
- Target state: a new function in `crates/emerald-codegen/src/lib.rs`,
  e.g. `fn find_non_escaping_news(body: &[Stmt]) -> HashSet<String>`,
  structured in two passes: (1) collect every `Stmt::Let{ name, value:
  Expr::New(..), .. }` site in `body` (including inside nested
  `if`/`while`/`case`/`begin` blocks, mirroring `collect_lets`'s own
  existing traversal shape) as a candidate name; (2) walk the *entire*
  `body` once more, and for each candidate name still classified
  non-escaping, check every `Stmt::Return`, `Stmt::SetField`,
  `Stmt::SetIndex`, and every `Expr::Call`/`Expr::New`/`Expr::MethodCall`
  argument list (receiver included, per the Decision log) for an
  `Expr::Ident` matching that name — any hit removes it from the
  result set. The compiler-generated `initialize` call `Expr::New`
  itself triggers is not part of the *source* AST this pass walks (it's
  synthesized later, in codegen), so no special-casing is needed here to
  exclude it.

### 2. Acceptance Criteria
1. For `distance_squared`'s body (this plan's own worked example),
   `find_non_escaping_news` returns `{"p"}`.
2. For `make_point`'s body, `find_non_escaping_news` returns `{}` (`p`
   is removed by the `Stmt::Return(Some(Expr::Ident("p")))` hit).
3. A synthetic case for each of the other two rules, constructed
   directly as AST fixtures (no parser involvement required): a `Let`-
   bound instance later passed as a `Stmt::SetField`'s `value`, and one
   later passed as an ordinary `Expr::Call` argument — both return `{}`.
4. A synthetic case proving receiver position is checked: a `Let`-bound
   instance later used only as `Expr::MethodCall(Ident(name), _, _)`'s
   receiver, with an otherwise-empty argument list, is still excluded
   from the result set (proves rule (c)'s receiver-position clause is
   real, not just documented).
5. A `Let`-bound instance never referenced again anywhere in the body
   after its own `Let` is included in the result set — the base case
   the worked `distance_squared` example itself exercises.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (new function + a
  dedicated `#[cfg(test)]` module section for AST-fixture-level tests,
  same idiom as this file's existing test module)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-codegen` | all pass, incl. the five fixture cases above | agent-claimed-locally |

---

## Leaf: leaf-stack-allocation-codegen

### 1. Context
- Why: `leaf-escape-detection`'s result is inert until codegen actually
  branches on it — this leaf is the real behavior change, and per the
  Decision log, observable program output must stay unchanged; only the
  allocation strategy changes.
- Target state: `find_non_escaping_news` is called once per function/
  method/lambda body, at the same point `collect_lets` already is
  (`define_user_function` L3834–3836, and the analogous call sites in
  `define_method`, `define_lambda`, `define_main`), producing a
  `non_escaping: HashSet<String>` alongside the existing `decls`. A new
  prealloc step — a sibling to `prealloc_lets`, e.g.
  `prealloc_stack_objects`, also run at `entry` before
  `build_function_body` — builds one `builder.build_alloca(context
  .i8_type().array_type(layout.size as u32), name)` per name in
  `non_escaping` (looking up `layout` via `ctx.classes[class_name]`,
  `class_name` known from the `Let`'s own declared type) and records
  each resulting `PointerValue` in a new `object_allocas: HashMap<String,
  PointerValue<'ctx>>` passed alongside `vars`. `build_stmt` gains a new
  match arm, ordered before the existing generic `Stmt::Let { name, ty,
  value }` arm (alongside the existing `Expr::Lambda`/`Expr::ArrayNew`
  special cases at L2695–2745): `Stmt::Let { name, value: Expr::New(class_name,
  args), .. } if object_allocas.contains_key(name)` — this arm skips the
  `ctx.alloc` call entirely, uses the pre-built stack `PointerValue` as
  `ptr`, still runs the exact same `initialize`-dispatch logic
  (L1529–1554) unchanged, and stores `ptr` into the `Let`'s own local
  slot exactly as the heap path does.

### 2. Acceptance Criteria
1. `distance_squared`, compiled, linked, and run via `puts
   distance_squared(3, 4)`, prints `25` — identical to what the
   unmodified heap-only baseline already prints today (regression proof
   that behavior didn't change).
2. `make_point`, compiled and linked into a program that calls it and
   reads a field back through it (e.g. via a `read`-sugared accessor,
   plan 33), still produces the same field values it did before this
   leaf — proving the *escaping* path is untouched.
3. Every existing `Point`/class-instance-using example in this file's
   test module (the inception `Point` example, `classes.em`'s `Counter`/
   `Point`, plan 11's exceptions example, plan 32's inheritance example)
   still compiles, links, and runs to its existing expected output
   unmodified — this leaf must not change any prior plan's observable
   result.
4. A stack-allocated instance's fields are read/written correctly
   through the same `field_ptr`/`load_field` machinery used for heap
   instances — verified by a test that constructs a non-escaping
   instance, mutates one of its fields via a method call before
   discarding it, and asserts the mutation is visible within that same
   method call's own subsequent read (proving the byte-array alloca is
   addressed identically to a heap allocation, not merely allocated and
   ignored).
5. A non-escaping `New` written inside a `while` loop body, executed
   multiple iterations, does not produce a different alloca per
   iteration (verified by inspecting the emitted LLVM IR text for the
   compiled function — exactly one `alloca` instruction for that site,
   not one per potential loop trip) — the concrete proof for the
   entry-block-placement Decision-log point.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`define_user_function`,
  `define_method`, `define_lambda`, `define_main`, `build_stmt`'s
  `Stmt::Let` arms, a new `prealloc_stack_objects`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Workspace regression | `cargo test --workspace` | all pass, no prior example's output changes | agent-claimed-locally |
| IR inspection (loop case) | inspect emitted LLVM IR (`Module::print_to_string`) for the while-loop fixture | exactly one `alloca` for the non-escaping site | agent-claimed-locally |

---

## Leaf: leaf-escape-instrumentation-and-report

### 1. Context
- Why: per the Decision log, stack- vs. heap-allocation is invisible
  from a compiled program's own stdout — this leaf is the acceptance
  mechanism that actually proves `leaf-stack-allocation-codegen` fired,
  and it must not change `compile_to_object`'s existing public
  signature (still used by `emerald-cli` and every existing test as
  `Result<(), String>`).
- Target state: a new `pub struct EscapeStats { pub stack_allocated: u64,
  pub heap_allocated: u64 }` and a new `pub fn
  compile_to_object_with_stats(program: &Program, out_path: &Path) ->
  Result<EscapeStats, String>` that does exactly what `compile_to_object`
  does today, additionally threading an `&mut EscapeStats` accumulator
  through `build_stmt`'s `Let`/`New` arms (incremented once per
  `Expr::New` site actually compiled — in the stack arm added by
  `leaf-stack-allocation-codegen`, and in the pre-existing heap arm)
  and returning it. `compile_to_object` itself becomes a thin wrapper
  that calls `compile_to_object_with_stats` and discards the stats —
  a non-breaking, additive change verified against every existing
  caller. `emerald-cli`'s `main.rs` gains a new `--emit=escape-report`
  flag (checked the same way the existing `-o` flag is parsed) that,
  when present, calls `compile_to_object_with_stats` instead of
  `compile_to_object` and prints `"{stack_allocated} instances
  stack-allocated, {heap_allocated} heap-allocated"` to stderr before
  proceeding with linking as normal.
- Explicit non-choice: no shared global/thread-local counter. `Ctx` and
  friends are threaded per-compilation already; a global counter would
  race across parallel `cargo test`/`cargo nextest` runs compiling
  different programs concurrently in the same process — the
  `&mut EscapeStats` accumulator threaded alongside `vars`/
  `local_classes` avoids that entirely, at the cost of one more `&mut`
  parameter on the already-long `build_stmt`/`build_function_body`
  signatures (an accepted, disclosed cost, not hidden).

### 2. Acceptance Criteria
1. `compile_to_object_with_stats` on the worked `distance_squared`
   program returns `EscapeStats { stack_allocated: 1, heap_allocated: 0
   }` — the plan's headline internal proof.
2. `compile_to_object_with_stats` on the worked `make_point` program
   returns `EscapeStats { stack_allocated: 0, heap_allocated: 1 }` — the
   contrasting proof.
3. `compile_to_object` (the pre-existing, unmodified-signature function)
   still compiles and links every existing test in this file
   unmodified — proving the wrapper refactor is behavior-preserving for
   every current caller.
4. A CLI-level test (`crates/emerald-cli/tests/`) invoking the built
   binary with `--emit=escape-report` against a small `.em` file
   containing both worked functions asserts the printed report names
   both counts correctly.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`EscapeStats`,
  `compile_to_object_with_stats`, `compile_to_object` becomes a thin
  wrapper), `crates/emerald-cli/src/main.rs` (`--emit=escape-report`
  flag handling)
- **Create:** a new test file or an addition to an existing one under
  `crates/emerald-cli/tests/` for the CLI-level `--emit=escape-report`
  proof

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. the two `EscapeStats` assertions and the CLI report test | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
