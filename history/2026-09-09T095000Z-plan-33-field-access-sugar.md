---
name: Field-Access Sugar
overview: "An opt-in `read` field marker that synthesizes a zero-arg accessor method, so `obj.field` works from outside a class without a hand-written accessor — inception's own `attr_reader` gap, found and disclosed while writing plan 11's exceptions example."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-read-accessor-desugaring
    content: "grammar-level `read <name>: <Type>` field marker; on sight, synthesizes an ordinary zero-arg Function (returning `@<name>`) into ClassDef.methods at parse time — no AST/sema/codegen changes beyond the grammar action itself"
    status: pending
isProject: false
---

# Plan 33 — Field-Access Sugar

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
same post-v1 posture as plan 17 and the rest of the 18–27/28–35 batches;
`plan-of-plans.md` is left untouched. It is plan 33 of a follow-up batch
(28–35) covering distinct language-completeness gaps found while
authoring earlier plans; siblings in this batch (bitwise operators,
control-flow completeness, `for`-in, compound/multiple assignment, class
inheritance, blocks/`yield`, debug info) are separate, independently
written plans and are not this plan's job.

This is the direct, named follow-up to a gap plan 11 already found and
disclosed in its own worked example:

> (Two corrections made while implementing `leaf-ast-exceptions`, before
> any sema/codegen work started: `x < 0`/`risky(-1)` don't parse... And
> `e.code` needs an explicit `code` accessor method on `MyError`;
> Ruby-like dot syntax is always a method call here, never implicit
> public field access — there's no `attr_reader`-style sugar, matching
> real Ruby's own rule that `obj.field` only works if `field` is an
> actual defined method.)

Concrete proof this plan targets:

```ruby
class Point
  read x: Int64
  y: Int64

  def initialize(x: Int64, y: Int64) -> Void
    @x = x
    @y = y
  end
end

p: Point = Point.new(3, 4)
puts p.x
```

Expected output: `3`. A second, negative proof in the same program —
`puts p.y` (a field declared without `read`) — must still be rejected
with a diagnostic, proving the sugar is genuinely opt-in per field, not
a blanket exposure of every declared field.

## Decision log

- **No `Symbol`/`:foo` literal exists in this language at all** (verified
  this session: `spec/GRAMMAR.md` §1 marks Symbol literals KEEP, but
  there is no `Symbol` type, `:name` token, or AST node anywhere in
  `crates/emerald-parser`/`crates/emerald-sema`). Ruby's real
  `attr_reader :name`/`attr_accessor :name` is a macro-like method call
  taking a `Symbol` argument — that literal syntax has nothing to attach
  to here. This plan does not attempt to port that call-syntax; instead
  it adds a field-declaration-site keyword marker, `read`, directly in
  the class body's existing field-declaration list (`read x: Int64`
  instead of `x: Int64`) — opt-in per field, smallest possible grammar
  addition, and it matches Ruby's own actual default (no field is
  auto-exposed unless a reader is explicitly requested), not a
  regression from it.
- **Reader-only. No writer, no `attr_accessor`-equivalent.** Ruby's
  `attr_accessor` generates a method literally named `field=`, called via
  `obj.field = value` from outside the object. This grammar's dot-call
  syntax (`recv "." Ident ["(" Args ")"]`, `crates/emerald-parser/src/
  grammar.lalrpop`, verified this session) has no way to write `obj.field
  = value` as a call at all — `Stmt::SetField` (`@x = value`) only exists
  for instance-internal writes inside the same method body, never for an
  external receiver. Adding external-write syntax is a real, separate
  grammar feature (a new assignment-target shape) this plan doesn't need
  — plan 11's own worked example, and this plan's proof above, only ever
  need a read from outside. Writable external accessors are deferred; see
  Out of scope.
- **The whole feature is a parse-time desugaring with zero AST, sema, or
  codegen changes** — the cleanest-possible-ripple implementation, and
  the one this Decision log recommends over two heavier alternatives
  considered and rejected:
  - *Rejected: add a `readable: bool` field to `Param`.* `Param {
    name, ty }` (verified in `crates/emerald-parser/src/ast.rs`) is
    deliberately reused, unchanged, for both function parameters and
    class field declarations since plan 08's Decision log ("fields reuse
    `Param`'s `{name, ty}` shape — structurally identical to a parameter
    declaration"). Adding a class-field-only flag to it would put a
    meaningless field on every function parameter in the codebase and
    ripple into every one of plans 04–32's `Param` construction sites for
    no benefit — the marker is only ever needed transiently, during
    parsing, not as permanent AST state.
  - *Rejected: a new `FieldDecl` struct replacing `ClassDef.fields: Vec<
    Param>`.* Smaller blast radius than touching `Param` itself, but
    still a breaking structural change to `ClassDef` that every existing
    consumer of `.fields` (`emerald-sema`'s `class_info`, and whatever
    codegen crate currently owns class layout — see note below) would
    need to be updated for, to carry information that, again, is only
    needed transiently during parsing.
  - **Chosen: the grammar action for a class body's field-declaration
    list does the desugaring itself, at parse time, before `ClassDef` is
    even constructed.** Seeing the optional `read` prefix on a field
    causes the grammar action to do two things: push the field into
    `ClassDef.fields` exactly as always (a plain, unmarked `Param` —
    `ClassDef.fields`'s element type never changes), and *also* push a
    synthesized `Function { name: <field name>, params: vec![],
    return_type: <field's declared type>, body: vec![Stmt::Expr(
    Expr::InstanceVar(<field name>))] }` into `ClassDef.methods`. From
    the moment parsing finishes, a class with a `read`-marked field looks
    to every downstream consumer — `emerald-sema`'s `class_info`/
    `check_method_body`, and codegen's per-class method compilation —
    exactly like a class whose author happened to hand-write a one-line
    accessor method, because that is now, structurally, exactly what it
    is. Nothing about method resolution, `MethodCall` dispatch, or
    per-method codegen needs to know an accessor was synthesized rather
    than typed by hand.
- **The negative case (a non-`read` field stays inaccessible via
  `obj.field`) needs no new sema logic at all.** `emerald-sema`'s
  existing `MethodCall` handling already rejects a call naming a method
  the receiver's class doesn't have, with a diagnostic, not a panic (plan
  08 AC3: "a call to an undeclared method... is a diagnostic, not a
  panic" — this is exactly the path a bare `obj.field` on an unmarked
  field already takes today, per plan 11's own finding that quoted this
  gap in the first place). This plan's synthesis pass only ever *adds* a
  method for a `read`-marked field; it never touches how an absent method
  name is handled, so the rejection path plan 08 already built is the
  entire proof obligation for this plan's negative acceptance criterion.
- **Codegen note:** the last documented codegen implementation this
  session (`crates/emerald-codegen`, Cranelift-based, name-mangling
  `{ClassName}_{methodName}` per plan 08) compiles every class method
  from its `Function` node with no special-casing by origin — a
  synthesized accessor compiles through that exact path unchanged. There
  is currently unrelated, in-progress, uncommitted work by a different
  agent restructuring `crates/emerald-codegen`/`emerald-codegen-llvm`
  (plan 16's Cranelift-vs-LLVM bake-off) — this plan's design doesn't
  depend on which backend wins that bake-off, since it never touches
  codegen at all; whichever crate ends up owning `Function` codegen after
  plan 16 settles is this plan's only real dependency, and it needs no
  changes either way.
- **Forward-compatibility note re: plan 32 (class inheritance, a sibling
  in this same batch).** If/when inheritance lands, an inherited field
  marked `read` on a superclass should reasonably still produce a working
  accessor on subclass instances — this plan doesn't attempt to solve
  that (there is no inheritance to solve it against yet), but the
  parse-time-desugaring design should compose cleanly with it: however
  plan 32 makes a superclass's fields visible/inherited, this plan's
  synthesized-accessor-as-an-ordinary-method mechanism should just work
  once a subclass's effective method table includes inherited methods,
  since a synthesized accessor is indistinguishable from any other
  method by the time plan 32's own resolution logic would see it.

## Leaf: leaf-read-accessor-desugaring

### 1. Context
- Why: `crates/emerald-parser/src/grammar.lalrpop`'s `ClassDef` production
  (verified this session: `"class" <name:Ident> <fields:Param*>
  <methods:FuncDef*> "end"`) parses every field as a bare `Param` with no
  way to request an accessor; `crates/emerald-parser/src/ast.rs`'s
  `ClassDef { name, fields: Vec<Param>, methods: Vec<Function> }` has no
  concept of field visibility at all (verified this session).
- Target state: a new optional `read` prefix on a class body's per-field
  declaration (`read <name>: <Type>` alongside the existing bare `<name>:
  <Type>` form), reserved the same LALR(1) way `puts`/`new`/`Array`/
  `Proc`/`raise`/`begin`/`rescue` were each reserved in plans 07/08/09/11
  — `read` becomes a keyword token distinct from the generic `Ident`
  regex. The grammar action for each field, on seeing the prefix, pushes
  the plain field into the class's `fields` list (unchanged shape) and
  additionally synthesizes and pushes a zero-arg `Function` (see Decision
  log) into the class's `methods` list.

### 2. Acceptance Criteria
1. This plan's full worked example (`Point` with `read x: Int64` and a
   plain `y: Int64`) parses into a `ClassDef` whose `fields` is
   `[Param{x,Int64}, Param{y,Int64}]` (both plain, unmarked — `fields`'s
   element type is unchanged `Param`) and whose `methods` contains a
   synthesized `Function { name: "x", params: [], return_type: "Int64",
   body: [Stmt::Expr(Expr::InstanceVar("x"))] }` alongside the
   hand-written `initialize`.
2. The same example, type-checked, compiled, linked, and run for real,
   prints `3\n` — real executed proof `p.x` dispatches through the
   synthesized accessor exactly like a hand-written zero-arg method.
3. Negative case, same class: `puts p.y` (the plain, non-`read` field) is
   rejected by `emerald-sema` with the same "undeclared method" class of
   diagnostic plan 08 already produces for any nonexistent method name —
   not a panic, and not silently accepted. This is the plan's proof that
   the sugar is opt-in per field, not blanket field exposure.
4. Regression: every prior plan's example that declares class fields
   (`examples/classes.em`, plan 08's `Point` example, plan 11's
   `MyError`, plan 12's module/class examples) still parses and
   type-checks identically with no `read` markers present — this feature
   is purely additive syntax.
5. `read` becomes a reserved keyword with no LALRPOP build-time conflicts
   (same bar as every prior keyword-reservation plan).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests) — `crates/emerald-parser/
  src/ast.rs` is explicitly **not** modified (see Decision log: `Param`/
  `ClassDef`'s shapes are unchanged by design).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. the synthesized-`Function`-shape assertion | agent-claimed-locally |
| Sema (negative case) | `cargo test -p emerald-sema` | the non-`read`-field rejection test passes | agent-claimed-locally |
| Codegen (positive case) | `cargo test -p emerald-codegen` (or whichever crate owns class-method codegen once plan 16 settles) | real linked-and-run `3\n` | agent-claimed-locally |
| Workspace | `cargo build --workspace && cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **Writable external accessors** (`obj.field = value` from outside the
  class, Ruby's `attr_accessor`/`field=`) — see Decision log; needs a new
  assignment-target grammar shape this plan doesn't add.
- **`Symbol`-argument macro-call syntax** matching Ruby's literal
  `attr_reader :x, :y` form — there is no `Symbol` type or literal in
  this language at all; see Decision log.
- **Any visibility system beyond this one binary opt-in marker** —
  `private`/`protected`, module-level visibility, or anything richer than
  "has a `read` marker or doesn't" is separate, larger future work.
- **Interaction with class inheritance (plan 32)** — noted as a
  forward-compatibility remark in the Decision log; not designed or
  proven against here, since no inheritance exists yet for it to
  interact with.
