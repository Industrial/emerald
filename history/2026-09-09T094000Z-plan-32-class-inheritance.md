---
name: Class Inheritance
overview: "Single-inheritance `class Dog < Animal ... end` — inherited field layout, inherited method resolution, and same-signature override, all still statically dispatched by the receiver's declared class (no vtables/polymorphism yet)."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-inheritance
    content: "ClassDef gains superclass: Option<String>; grammar adds an optional \"<\" Ident after the class name"
    status: pending
  - id: leaf-sema-inheritance
    content: "Chain resolution (root-to-leaf), cycle detection, field-name-collision checks, method override with invariant-signature checking, flattened field/method tables per class"
    status: pending
  - id: leaf-codegen-inheritance
    content: "Layout = superclass fields first (same offsets a superclass instance would use) then own fields appended; method calls resolve to whichever ancestor actually defines the method, still picked by the receiver's static declared class"
    status: pending
isProject: false
---

# Plan 32 — Class Inheritance

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
post-v1 scope, same posture as plan 17: don't touch `plan-of-plans.md`
or any other plan file. It's one of an eight-plan follow-up batch
(28–35) covering distinct language-completeness gaps found while
authoring plans 17–27; siblings (bitwise operators, control-flow
completeness, `for`-`in`, compound/multiple assignment, field-access
sugar, blocks/`yield`, debug info) are separate plans, written by other
agents in the same batch — this plan owns class inheritance only.

This is the direct, named follow-up to a hard boundary plan 11 already
found and explicitly leaned on: *"No class inheritance exists in this
compiler yet (plan 08's `ClassDef` has no superclass field... So
`rescue ExceptionType => e` matching is exact-type equality, not subtype
matching — which is not a scope cut at all, just what 'no inheritance'
already implies."* Re-verified this session: `crates/emerald-parser/src/
ast.rs`'s `ClassDef { name, fields, methods }` has no superclass field,
and `grammar.lalrpop`'s `ClassDef` production is `"class" Ident Param*
FuncDef* "end"` — no `"<" Ident` anywhere. `spec/GRAMMAR.md` §8 already
marks single inheritance (`class Dog < Animal`) **KEEP** ("Statically
known ancestor chain, per inception §8"), and `spec/SEMANTICS.md` §4.3
already locks the boundary this plan respects: *"Multiple inheritance is
forbidden. `class` supports single inheritance only... `struct` supports
no inheritance at all."* `spec/SEMANTICS.md` §7.1 goes further and
already assumes a class hierarchy exists for exceptions specifically:
*"`raise` can construct and throw any subclass of the exception
hierarchy"* — today there's no hierarchy for it to throw a subclass of;
this plan is the prerequisite that eventually makes that sentence true.

Concrete proof:
```ruby
class Animal
  age: Int64

  def initialize(age: Int64) -> Void
    @age = age
  end

  def age -> Int64
    @age
  end

  def describe -> Int64
    @age
  end
end

class Dog < Animal
  breed_code: Int64

  def initialize(age: Int64, breed_code: Int64) -> Void
    @age = age
    @breed_code = breed_code
  end

  def describe -> Int64
    @age + @breed_code
  end
end

a: Animal = Animal.new(5)
d: Dog = Dog.new(3, 100)
puts a.describe
puts d.age
puts d.describe
```
Expected output: `5\n3\n103\n`. `d.age` proves **inherited method
resolution** (`Dog` never declares `age`; it must resolve to `Animal`'s);
`d.describe` proves **override** (`Dog`'s own `describe` wins, and can
read both the inherited `@age` and `Dog`'s own `@breed_code`); `a.describe`
proves the base class is unaffected. No `String` literals are used (a
sibling plan in this batch, not a dependency here, is what adds those) —
every value in the proof is `Int64`, avoiding an unforced cross-plan
dependency.

**A note on the current codegen crate's state:** `crates/emerald-codegen/
src/lib.rs` was read in full earlier this session (Cranelift-based,
`build_class_layout`/`ClassLayout`/`FieldInfo`/`define_method`/
`build_method_call`, etc.) — that is the *documented*, not necessarily
*current*, codegen implementation this plan's leaf-codegen-inheritance
section describes and extends. There is unrelated, in-progress,
uncommitted work by a different agent implementing plan 16 (Cranelift
vs. LLVM bake-off) directly inside `crates/emerald-codegen/` and
`crates/emerald-codegen-llvm/` right now, mid-refactor (`crates/
emerald-codegen/src/lib.rs` is transiently deleted in the working tree
as of this writing). This plan's codegen leaf targets the last
documented shape of that crate; whoever executes this plan should
re-verify the concrete function/struct names against whatever codegen
crate structure plan 16 actually settles into before implementing.

## Decision log

- **Single inheritance only — one optional superclass name, no
  `include`/`extend` mixin composition.** Directly required by `spec/
  SEMANTICS.md` §4.3's locked decision (quoted above) and consistent
  with plan 12's own precedent of rejecting Ruby's `include`/`extend`
  mixin composition for modules ("composing method sets from multiple
  sources per instance adds method-resolution-order complexity
  disproportionate to the milestones in inception §17" — the same
  reasoning applies here, doubly so for classes).
- **No real dynamic dispatch / polymorphism.** This compiler resolves
  every method call statically by the receiver expression's *declared*
  class (verified this session: `build_method_call` takes `local_classes:
  &HashMap<String, String>`, a plain name→declared-class-name map built
  from `Stmt::Let`'s own type annotation — there's no vtable, no runtime
  type tag consulted for dispatch anywhere in codegen). This plan does
  **not** add vtables or upcasting-then-dynamic-dispatch (storing a `Dog`
  in an `Animal`-typed variable and having `.describe` still call `Dog`'s
  override through the `Animal`-declared reference) — that's a real,
  separate, and materially bigger codegen feature (a vtable or a
  class-tag-driven dispatch table, in the same spirit as plan 11's
  integer class tags for `rescue` matching, but for *every* virtual call
  site instead of one exception-catch site). What this plan *does* prove
  — inherited fields, inherited method resolution, and override, all
  picked by the value's own concrete declared type — is a real, honestly
  smaller, and independently useful slice: proving polymorphism
  separately, once this plan's layout/resolution groundwork exists, is
  future work, not silently assumed here.
- **No `super`/`super(args)` syntax.** Calling the overridden
  superclass method from an override needs its own grammar production
  and a dispatch rule ("skip my own class in the resolution chain,
  start from my superclass instead") — a real, separate, self-contained
  feature this plan doesn't need to prove field layout, method
  inheritance, or override.
- **Field layout: superclass fields first, in ancestor-to-descendant
  order, then the subclass's own fields appended** — computed by
  walking the chain from the root down before assigning offsets, one
  8-byte slot per field (this codebase's existing uniform
  representation — every field/array-element seen so far, including
  `ARRAY_ELEM_SIZE`, is a fixed 8-byte word; no packed/sub-word layout
  exists anywhere to deviate from). This ordering is the load-bearing
  invariant that makes inherited-method reuse memory-safe at all: an
  inherited method compiled once against `Animal`'s field offsets
  (e.g. `Animal::initialize` writing `@age` at offset 0) still writes
  the right slot when invoked on a `Dog` instance, because `Dog`'s
  layout is required to agree with `Animal`'s for every field `Animal`
  itself declares. This is also exactly the invariant a *future* real
  dynamic-dispatch/upcasting plan would need already in place — this
  plan lays that groundwork as a side effect of proving inheritance at
  all, without itself using it for anything beyond static calls.
- **A subclass redeclaring a field name already present in an
  ancestor is a compile error** ("field `age` already declared in
  superclass `Animal`"), not silent shadowing — the simplest rule that
  avoids any ambiguity about which slot a redeclared name would even
  refer to, and there's no concrete example in this project's history
  that needs shadowing to work.
- **An overriding method's signature must exactly match (invariant,
  not covariant/contravariant) the overridden method's parameter types
  and return type.** The smallest rule that's still meaningful type
  checking; covariant returns/contravariant parameters are real
  type-theory features this project has never needed elsewhere (every
  existing type-compatibility check in `emerald-sema` is exact-match,
  not subtyping-aware — matches that existing house style) and add
  real complexity for no example in this plan's own scope.
- **Cyclic inheritance (`class A < B` ... `class B < A`) is a real,
  caught compile error**, not an infinite loop or a stack overflow —
  chain resolution tracks a visited-set and reports the cycle by name
  the first time a class reappears while walking up from itself.
- **Codegen independently re-derives the class hierarchy from the raw
  `Program`/`ClassDef` list, the same way it already independently
  re-derives everything else about classes** (`build_class_layout`
  already operates directly on `&ClassDef`, not on any structure
  `emerald-sema` built — this codebase has no typed IR / no shared
  sema→codegen data structure anywhere, a standing architectural fact
  since plan 06, not something this plan is introducing). Concretely:
  codegen builds its own `HashMap<String, HashMap<String, String>>`
  (class name → method name → *defining* class name) by walking each
  class's chain root-to-leaf and overlaying method names, so a call to
  `d.age` where `Dog` doesn't define `age` correctly emits a call to
  `Animal`'s compiled `age` function symbol rather than a nonexistent
  `Dog`-qualified one.
- **This plan is a one-line forward pointer, not an upgrade, for plan
  11's `rescue` matching.** Plan 11's Decision log already said exact-type
  `rescue` matching "is not a scope cut at all, just what 'no
  inheritance' already implies" — once this plan lands, upgrading
  `rescue ExceptionType => e` to match `ExceptionType` *or any subclass*
  (which is what `spec/SEMANTICS.md` §7.1 actually specifies:
  `"raise` can construct and throw any subclass of the exception
  hierarchy") becomes possible. This plan does not implement that
  upgrade — it's a separate future plan's job, noted here only so it
  isn't lost.

## Leaf: leaf-ast-inheritance

### 1. Context
- Why: no AST shape exists for a superclass reference at all.
- Target state: `ClassDef { name: String, superclass: Option<String>,
  fields: Vec<Param>, methods: Vec<Function> }`; grammar production
  `"class" <name:Ident> <superclass:("<" <s:Ident> => s)?> <fields:
  Param*> <methods:FuncDef*> "end"`.

### 2. Acceptance Criteria
1. `ClassDef.superclass` exists as described; a class with no `<
   Superclass` clause parses with `superclass: None`, unchanged from
   today's shape.
2. `class Dog < Animal ... end` parses with `superclass: Some("Animal"
   .to_string())`.
3. Regression: every prior plan's class example (`Counter`, `Point`,
   `MyError`, etc.) still parses identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |

---

## Leaf: leaf-sema-inheritance

### 1. Context
- Why: `emerald-sema`'s `ClassInfo`/`class_info` builds a flat
  `fields`/`methods` map per class from that class's own declaration
  only — no notion of a superclass to merge in, resolve, or validate
  against exists.
- Target state: `ClassInfo` gains `superclass: Option<String>`. A new
  `resolve_chain(name, classes) -> Result<Vec<String>, Diagnostic>`
  walks from `name` up through each `superclass` link to the root,
  returning root-to-leaf order and erroring by name on a cycle. Class
  registration builds each class's *flattened* `fields`/`methods` maps
  by processing the chain root-to-leaf: start from an empty map, for
  each class in the chain merge in its own `fields` (erroring on any
  name already present — see Decision log) and its own `methods`
  (inserting or *replacing* an existing same-named entry — an override —
  after checking the new signature exactly matches the one it replaces).

### 2. Acceptance Criteria
1. This plan's full example (`Animal`/`Dog`) type-checks `Ok(())`.
2. `class A < B ... end` / `class B < A ... end` (mutually cyclic) is
   rejected with a diagnostic naming the cycle, not a stack overflow or
   hang (verified with a real, bounded test run, not just "doesn't
   crash in principle").
3. A subclass redeclaring an ancestor's field name (e.g. `Dog`
   re-declaring `age: Int64`) is rejected with a diagnostic.
4. An override with a mismatched signature (different parameter types
   or return type than the method it overrides) is rejected with a
   diagnostic naming both the method and the mismatch.
5. `class Dog < NotAClass ... end` (superclass name that isn't a
   declared class) is rejected with a diagnostic, not a panic.
6. Regression: every prior plan's sema test suite still passes
   unmodified (no existing class's behavior changes when it has no
   superclass).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, including the 5 new cases above | agent-claimed-locally |

---

## Leaf: leaf-codegen-inheritance

### 1. Context
- Why: nothing compiles a superclass reference, inherited-field layout,
  or inherited-method dispatch to machine code (or whatever backend
  `crates/emerald-codegen` has settled on by execution time — see this
  plan's introduction).
- Target state: class layout walks each class's chain root-to-leaf,
  laying out ancestor fields first (in ancestor declaration order) at
  the same offsets an instance of that ancestor alone would use, then
  appending the class's own fields — the load-bearing invariant from
  the Decision log. A method call resolves its target function symbol
  via a `class → method → defining_class` table built the same way
  (root-to-leaf overlay, last write per method name wins), so `d.age`
  (receiver statically `Dog`, method not declared on `Dog`) compiles a
  call to `Animal`'s method body, not a nonexistent `Dog`-qualified one.
  `Expr::New("Dog", args)` allocates using `Dog`'s full flattened size
  (ancestor fields + own fields), the same total-size computation every
  class already gets today, just computed after the chain walk instead
  of from one class's own field list.

### 2. Acceptance Criteria
1. This plan's full example, compiled, linked, and run, prints
   `5\n3\n103\n` — real executed proof that inherited-field layout,
   inherited-method resolution, and override all work together and
   don't corrupt each other's memory.
2. An unsupported/malformed shape defensively returns a descriptive
   `Err`, not a panic — same standard as every prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (or whatever module
  holds `compile_to_object`/class-layout logic by execution time — see
  this plan's introduction about the concurrent plan-16 migration)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `5\n3\n103\n` | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `super`/`super(args)` — see Decision log.
- Real dynamic dispatch / polymorphism (vtables, upcasting-then-virtual-
  call) — see Decision log; the single biggest deliberate scope cut in
  this plan.
- Multiple inheritance — locked REMOVE per `spec/SEMANTICS.md` §4.3,
  not merely deferred.
- Covariant/contravariant override signatures — see Decision log.
- Subtype-aware `rescue` matching (plan 11) — a real, separate future
  plan now unblocked by this one; not implemented here.
- Abstract classes / interfaces, visibility modifiers (`private`/
  `protected`) — no concrete example in this project's history needs
  them yet.
