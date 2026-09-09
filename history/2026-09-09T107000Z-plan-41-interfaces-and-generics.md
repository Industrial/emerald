---
name: Interfaces and Generics (Static, Monomorphized)
overview: "A minimal nominal interface mechanism (`interface Comparable ... end`, `class Foo implements Comparable`) plus a bounded generic function (`def max[T: Comparable](a: T, b: T) -> T`), compiled via whole-program monomorphization — the batch's core proof that Emerald can offer duck-typing's ergonomic benefit (one function body usable across unrelated types) through an explicit, compile-time-checked, zero-runtime-cost mechanism, with no vtables, no runtime type tags, and no indirect calls anywhere in the emitted code."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-parser-interfaces-generics
    content: "Item::Interface(InterfaceDef); ClassDef.implements: Option<String>; Function.type_params: Vec<TypeParam>; grammar for `interface ... end`, `implements <Name>`, `def name[T: Bound](...)`"
    status: pending
  - id: leaf-sema-interfaces
    content: "Interface registry; structural conformance check at class-declaration time (Self substituted with the implementing class's own name), gated behind the explicit `implements` declaration"
    status: pending
  - id: leaf-sema-generics
    content: "Generic function registry (Type::Generic placeholder); body type-checked once against the bound interface (Self substituted with the type parameter itself); call-site substitution, consistency, and implements-bound checking"
    status: pending
  - id: leaf-codegen-monomorphization
    content: "Whole-program specialization-cache pass over call sites; one fully concrete LLVM function per (generic function, concrete type) instantiation, name-mangled, direct-called — no vtable, no indirect call"
    status: pending
isProject: false
---

# Plan 41 — Interfaces and Generics (Static, Monomorphized)

This is plan 41 of the 36–47 follow-up batch — twelve independent sibling
plans whose combined purpose is closing Emerald's language/stdlib surface
from ~10–15% of Ruby toward the batch's disclosed ~45% ceiling, without
conceding any of Emerald's identity constraints (no `method_missing`/
`eval`/`send`/reflection, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing GC, no runtime
reflection). Like plans 28–35 before it, this plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
post-v1 scope; `plan-of-plans.md` itself is updated separately, once,
after all twelve 36–47 plans are authored, and this plan does not touch
it or any other plan file. This plan is explicitly the batch's
design-heaviest member: it is the direct prerequisite for plan 42
(enumerable-stdlib), which needs a real generic-function mechanism to
express `Array[T]`-wide operations (`max`, `min`, `sort`) without
duck typing — everything below is written to be precise enough for
plan 42 to build directly on top of, not merely to satisfy this plan's
own worked example.

Depends on: **plan 08** (`object-model` — classes, fields, static method
dispatch; done) and **plan 09** (`collections` — `Array[T]`; done, and
also this plan's cautionary precedent, per the Decision log's first
entry). Also interacts constructively with **plan 32** (`class-
inheritance`, done — an interface's required method may be satisfied by
an *inherited* method, for free, because interface conformance is
checked against a class's already-flattened method table) and **plan
33** (`field-access-sugar`, done — this plan's own worked example uses
`read` accessors rather than hand-writing them).

Concrete proof this plan targets — two unrelated user classes, each
implementing the same interface differently, both driving one shared
generic function body to two independently-compiled, independently-
correct results:

```ruby
interface Comparable
  def compare_to(other: Self) -> Int64
end

class Money implements Comparable
  read cents: Int64

  def initialize(cents: Int64) -> Void
    @cents = cents
  end

  def compare_to(other: Money) -> Int64
    @cents - other.cents
  end
end

class Distance implements Comparable
  read meters: Int64

  def initialize(meters: Int64) -> Void
    @meters = meters
  end

  def compare_to(other: Distance) -> Int64
    @meters - other.meters
  end
end

def max[T: Comparable](a: T, b: T) -> T
  if a.compare_to(b) >= 0
    return a
  end
  return b
end

m1: Money = Money.new(500)
m2: Money = Money.new(750)
winner_money: Money = max(m1, m2)
puts winner_money.cents

d1: Distance = Distance.new(100)
d2: Distance = Distance.new(42)
winner_distance: Distance = max(d1, d2)
puts winner_distance.meters
```

Expected output: `750\n100\n`. `500.compare_to(750) = -250 < 0`, so
`max(m1, m2)` falls through to `return b` (`m2`, 750 cents) — the
`Money` specialization's `else` path. `100.compare_to(42) = 58 >= 0`,
so `max(d1, d2)` takes `return a` (`d1`, 100 meters) — the `Distance`
specialization's `if` path. The two calls to `max` therefore exercise
*opposite* control-flow branches inside what is, textually, one shared
function body — real proof that `max$$Money` and `max$$Distance` (this
plan's mangled specialization symbols) are two separately compiled,
independently correct functions, not one generic body reached through a
shared dispatch trick.

## Decision log

- **`interface`/`implements` are genuine, general, user-declarable
  grammar — not a fourth hardcoded builtin keyword the way `Array`,
  `Hash`, and `Proc` are.** Verified this session directly against
  `crates/emerald-parser/src/grammar.lalrpop`'s `TypeName` production
  (lines 704–709): `"Array" "[" Ident "]"`, `"Hash" "[" Ident "," Ident
  "]"`, and bare `"Proc"` are three individually hand-written, literal-
  keyword grammar alternatives — there is no general mechanism by which
  an Emerald program declares its own parameterized or protocol type;
  plan 09's Decision log confirms this was a deliberate simplification
  ("a real `Type` AST node... is deferred until a second compound-type
  shape... makes the string-parsing approach genuinely awkward"), not an
  oversight. This plan does not add a fourth hardcoded case — `interface
  Comparable ... end` uses a new, general `InterfaceDef` grammar
  production that would accept `interface AnythingElse ... end` equally
  well; `Comparable` is not a reserved word anywhere in this plan's
  grammar changes, just the one interface this plan's own worked example
  happens to declare, the ordinary way any interface would be declared.
  This generality (not the specific name `Comparable`) is what makes this
  the batch's identity-defining plan rather than a one-off special case.
- **One required method per interface, enforced *structurally* by the
  grammar shape, not merely documented as a convention sema must
  separately check.** `InterfaceDef`'s grammar is `"interface" Ident
  "def" Ident "(" Params ")" "->" TypeName "end"` — a single inline
  method signature between the interface's own `interface`/`end`
  keywords, with no `FuncDef*`-style repetition the way `ClassDef`'s body
  has for methods. It is not possible to write a two-method interface in
  this plan's grammar at all; extending to multiple required methods
  later is a grammar change (adding a repetition), not a hidden
  invariant an implementer could silently violate. This directly
  matches the plan's own scope decision (see below) rather than fighting
  it.
- **`implements` is a deliberate, disclosed reintroduction of duck
  typing's ergonomic benefit through an explicit nominal declaration —
  not Ruby's actual duck typing, and the departure is named, not
  hidden.** Verified this session against the spec: `spec/TYPE_SYSTEM.md`
  §10 states outright, "No implicit 'same shape, different type'
  structural assignability — matches `GRAMMAR.md`'s no-duck-typing
  stance (inception §9)"; `spec/GRAMMAR.md` §7 states "Receiver's static
  type must declare the method; no duck typing (inception §9)"; `spec/
  SEMANTICS.md` §3.2 states "No duck-typed dispatch (inception §9): the
  receiver's declared type must have a matching method." Ruby itself has
  no `implements`-equivalent at all — a Ruby object satisfies `<=>`-based
  polymorphism simply by responding to `<=>`, checked (if at all) at
  runtime via `respond_to?`. This plan's `implements Comparable` clause
  is the hybrid that keeps Emerald statically checkable while still
  letting one function body (`max[T: Comparable]`) work across
  unrelated classes: the compiler always knows, from the explicit
  declaration, which concrete method a given call site resolves to,
  before the program ever runs — it never asks "does this value happen
  to respond to this method" at runtime, because there is no runtime
  mechanism in this codebase that could ask that question in the first
  place (no `respond_to?`, no reflection, per this project's identity
  constraints).
- **The interface's required method is named `compare_to`, not Ruby's
  literal `<=>` operator token — a real, disclosed narrowing from the
  task's own suggested spelling, forced by a verified gap, not a
  stylistic choice.** Verified this session: `crates/emerald-parser/src/
  ast.rs`'s `CompareOp` enum (`Lt, Gt, Le, Ge, Eq, Ne`) has no spaceship
  variant, and `grammar.lalrpop`'s `CompareOp` production recognizes
  only `> < >= <= == !=` — there is no `<=>` token anywhere in the
  lexer/grammar. `spec/GRAMMAR.md` §4 lists "Method-as-operator
  overloading (defining `+` etc. on a class)" as an aspirational `KEEP`
  row, but this session's read of `emerald-sema/src/lib.rs`'s
  `infer_expr_type` and `emerald-codegen/src/lib.rs`'s `build_expr`
  confirms `Expr::Add`/`Sub`/`Mul`/`Div`/`Rem`/`Compare` are fixed,
  built-in primitive operators that never look up or dispatch to a
  user-defined method of the same name — no operator-overload dispatch
  mechanism exists anywhere in this compiler today. Building `a <=> b`
  infix syntax that dispatches to a user method is a separate, larger,
  wholly undelivered feature (new token, new precedence-ladder
  production, and — the hard part — a name-based dispatch-to-user-method
  codegen path that doesn't exist for *any* operator yet). This plan's
  `Comparable` interface therefore requires an ordinary method,
  `compare_to(other: Self): Int64`, called through the already fully
  working `receiver.method(args)` `MethodCall` syntax — zero new
  operator-grammar or operator-dispatch work, and the interface
  mechanism itself is exactly as general either way.
- **`Self` is substituted twice, for two distinct purposes, and this
  plan keeps them structurally separate rather than conflating them —
  this is the plan's core type-theoretic mechanism.** (1) When sema
  checks a class's `implements` declaration (`class Money implements
  Comparable`), every `Self` in `Comparable`'s required signature is
  substituted with the concrete implementing class's own name
  (`Self → "Money"`) before comparing against `Money`'s actual declared
  `compare_to` signature — an exact-match structural check, the same
  invariant-signature discipline plan 32's override checking already
  uses (`build_flattened_class_info`'s `existing.params != sig.params ||
  existing.return_type != sig.return_type` check), not a covariant/
  contravariant one. (2) When sema type-checks a generic function's own
  body (`max[T: Comparable]`'s body, checked exactly **once**, statically,
  before any concrete type is known — this is what makes it a real
  generic rather than duck-typed re-checking per call), a method call on
  a `T`-typed value (`a.compare_to(b)`) is checked against `Comparable`'s
  required signature with every `Self` substituted with the type
  parameter **itself** (`Self → T`, a new `Type::Generic("T")` value, not
  a concrete class) — which is exactly why `a.compare_to(b)`'s argument
  `b` must *also* have inferred type `Type::Generic("T")`, not some
  concrete class, for the body to type-check. These are two different
  substitution targets solving two different problems (validating one
  concrete implementor vs. validating one generic body against an as-yet-
  unknown implementor), and every function in the `infer_expr_type`/
  `check_stmt` family gains one new threaded parameter, `generic_ctx:
  Option<(&str, &str)>` (type-param name, bound interface name), `None`
  everywhere except while checking a generic function's own body —
  additive, matching the exact pattern `self_fields: Option<&HashMap<
  String, Type>>` already established in this file for method-body-only
  context.
- **Call-site checking (per the task's design point 2) is a separate,
  later pass from body-checking, and only it ever touches a real
  concrete type.** At `max(m1, m2)`, sema infers each argument's
  concrete type (`Type::Class("Money")` for both), requires every
  argument position whose declared parameter type is the bound type
  parameter (`a: T`, `b: T`) to resolve to the *same* concrete type
  (positions disagreeing — e.g. `max(m1, d1)` — is a real, distinct
  diagnostic: "type parameter `T` resolved inconsistently... `Money` at
  argument 1, `Distance` at argument 2"), then requires that concrete
  type's `ClassInfo.implements` to equal `Comparable` by name (a class
  with no `implements` clause at all, or one declaring a *different*
  interface, fails this check with a diagnostic naming the missing
  interface) — only after both checks pass does sema substitute `T →
  Money` (or `→ Distance`) into the generic signature's raw parameter/
  return-type strings and run the substituted signature through the
  exact same `check_args` every ordinary call already uses.
- **Monomorphization, concretely: a specialization cache keyed by
  `(function name, concrete type)`, one fully concrete LLVM function
  emitted per distinct entry, resolved by a name-mangled direct call —
  explicitly *not* the vtable/dynamic-dispatch alternative other
  languages use for the same ergonomic goal.** Verified this session
  against `crates/emerald-codegen/src/lib.rs`'s `declare_user_functions`
  (lines 3438–3482): every `Item::Function` becomes exactly one
  `module.add_function(&f.name, ...)` LLVM symbol, looked up by that bare
  name from a flat `HashMap<String, (FunctionValue, ValKind)>` — there is
  no template/generic instantiation primitive in LLVM itself to lean on,
  and this codebase's only call-emission path (`Expr::Call`'s arm of
  `build_expr`, and `build_method_call`) is already, uniformly, a direct
  LLVM `call` against a statically-resolved `FunctionValue` — never an
  indirect call through a computed function pointer, anywhere in this
  codebase today. This plan's codegen leaf reuses that exact shape: for
  every distinct `(generic_fn_name, concrete_class_name)` pair actually
  called anywhere in the whole program (a new, program-wide collection
  pass — see leaf-codegen-monomorphization), it runs the *existing*,
  unmodified single-function codegen path (`bind_params`,
  `build_function_body`, `param_kinds`, `make_fn_type`) once, with the
  function's own `T`-annotated parameter/return types textually
  substituted for the concrete class name first, and registers the
  result under a mangled symbol (`max$$Money`, `max$$Distance`) rather
  than the bare generic name (which is deliberately never registered as
  a callable LLVM symbol at all). Each call site resolves to its own
  mangled symbol and emits a direct call — byte-for-byte the same call-
  emission code every non-generic call already uses. Contrast: a vtable
  approach (Java interfaces; Ruby's own `respond_to?`-checked duck
  typing) needs (a) a runtime type tag stored on every heap object, and
  (b) an indirect call through a function-pointer slot computed from
  that tag at the call site. Verified this session that (a) does not
  exist as a general mechanism anywhere in `ClassLayout`/`FieldInfo` —
  the *only* thing resembling a runtime type discriminant in this
  codebase is plan 11's integer exception-class tag, and that is
  explicitly scoped to `rescue` matching alone, not a general dispatch
  primitive. Building either (a) or (b) project-wide is precisely the
  runtime type information + indirect call this project's identity
  constraints rule out ("no dynamic/virtual dispatch or vtables... all
  method resolution stays static, from the receiver's declared type") —
  monomorphization is chosen specifically because it needs neither, at
  the cost of code size (N compiled copies of `max` for N concrete call
  types) rather than a runtime cost, which is exactly the trade this
  project's whole codegen strategy already makes everywhere else (e.g.
  plan 32's fully static, receiver-declared-type method resolution).
- **Type parameters are legal on top-level functions only — never on
  classes/structs, and never on class methods, even though this plan's
  own grammar change is textually reachable from a class-method
  `FuncDef` too.** `class Box[T] ... end` is out of scope: a generic
  class needs a *storage-layout* strategy per instantiation (a
  monomorphized struct layout per concrete `T`, interacting with plan
  32's inherited-field-offset invariant and `ClassLayout`'s whole field-
  offset scheme) — materially larger than monomorphizing one function
  body, and this plan's job is proving the call-site-substitution +
  monomorphization *mechanism* works at all via the smallest complete
  example, not building out its full surface. Generic **methods**
  (`def compare_all[T: Comparable](...)` inside a `class`/`module` body)
  are a smaller but still real, disclosed cut: `FuncDef`'s grammar
  production is shared between top-level functions and class/module
  methods (the same reuse plan 08 established for fields via `Param`),
  so the `[T: Bound]` clause is *grammatically* reachable in a method
  position too — sema's leaf-sema-generics rejects a non-empty
  `type_params` found while registering a class method or module
  function with an explicit "generic methods are not supported"
  diagnostic (matching this codebase's established "grammar admits a
  superset, sema narrows with a real diagnostic" pattern — e.g.
  `resolve_chain`'s rejection of inheriting from a module), never a
  panic and never a silent accept.
- **Exactly one type parameter per generic function, with a mandatory
  bound — no unbounded generics (`def identity[T](x: T)`), no multiple
  bounded parameters (`[T: Comparable, U: Comparable]`), no interface
  intersections (`T: Comparable + Hashable`).** All three are real,
  meaningful extensions of the same substitution mechanism this plan
  builds (an unbounded parameter simply skips the implements-check step;
  multiple parameters repeat the whole per-parameter procedure; an
  intersection bound requires the `implements`-check step to test
  membership in a *set* of interfaces instead of one name) — none of
  them is needed to prove monomorphization or interface-gated dispatch
  work at all, and `TypeParam`'s grammar (`Ident ":" Ident`, bound
  mandatory, no bare-`Ident` alternative) makes an unbounded parameter a
  parse error today rather than a silently-accepted-but-unchecked one.
- **Interfaces cannot be `implements`'d by primitive types (`Int64`,
  `Float64`, ...), and this plan's worked example deliberately uses two
  user classes rather than reaching for `Int64`'s native ordering.**
  There is no grammar position that attaches an `implements` clause to a
  primitive type name — `implements` is parsed only as part of
  `ClassDef`, and `Int64` is not a `ClassDef`. A call-site check that
  resolves `T → Int64` therefore always fails the `ClassInfo.implements`
  lookup (`classes.get("Int64")` doesn't even exist — primitives are
  resolved directly in `resolve_type`, never registered in the `classes`
  map at all), cleanly and by construction, rather than needing a
  special-cased "primitives never implement anything" rule bolted on
  separately.
- **Modules cannot declare `implements`.** Consistent with `spec/
  SEMANTICS.md` §10.4 ("Modules cannot define fields... a namespace-only
  module has no instances to hold fields on") — a module likewise has no
  instance for an interface's required method to be called *on*
  (`Comparable`'s `compare_to(other: Self)` presupposes a receiver
  instance); the grammar's `ImplementsClause?` is only reachable from
  `ClassDef`, not `ModuleDef`, so this is enforced structurally, the same
  way the one-method-per-interface rule is.

## Leaf: leaf-ast-parser-interfaces-generics

### 1. Context
- Why: no AST shape or grammar production exists for an interface
  declaration, a class's `implements` clause, or a function's bounded
  type-parameter list. Verified this session directly against
  `crates/emerald-parser/src/ast.rs` (no `Interface`/`implements`/
  `TypeParam` anywhere) and `crates/emerald-parser/src/grammar.lalrpop`
  (no `"interface"`/`"implements"` tokens; `FuncDef`'s only shape is
  `"def" Ident ParenParams "->" TypeName Stmt* "end"`).
- Target state:
  - `ast.rs`: `pub struct InterfaceDef { pub name: String, pub
    method_name: String, pub params: Vec<Param>, pub return_type: String
    }` (reuses `Param`'s `{name, ty}` shape, the same reuse plan 08
    established for class fields); `Item` gains `Interface(InterfaceDef)`;
    `ClassDef` gains `pub implements: Option<String>`; `pub struct
    TypeParam { pub name: String, pub bound: String }` (bound mandatory —
    see Decision log); `Function` gains `pub type_params:
    Vec<TypeParam>` (empty `Vec` for every existing function — additive,
    source-compatible).
  - `grammar.lalrpop`: `"interface"`/`"implements"` become reserved
    keywords (same LALR(1) reason `class`/`module`/`new`/`Array` already
    are — verified against those exact precedents in the current file).
    `InterfaceDef: InterfaceDef = { "interface" <name:Ident> "def"
    <method_name:Ident> "(" <params:Params> ")" "->" <return_type:
    TypeName> "end" => ... }`, added as a new `Item` alternative.
    `ClassDef`'s production gains an optional `<implements:
    ImplementsClause?>` immediately after the existing `SuperclassClause?`
    (`ImplementsClause: String = { "implements" <i:Ident> => i }`).
    `FuncDef`'s production gains an optional `<type_params:
    TypeParamClause?>` between `<name:Ident>` and `<parts:ParenParams>`
    (`TypeParamClause: Vec<TypeParam> = { "[" <params:TypeParamList> "]"
    => params }`; `TypeParamList` right-recursive comma list, mirroring
    `MultiAssignNames`'s own right-recursion trick; `TypeParam: TypeParam
    = { <name:Ident> ":" <bound:Ident> => TypeParam { name, bound } }`).
- Dependencies: none (purely additive to `ast.rs`/`grammar.lalrpop`).

### 2. Acceptance Criteria
1. This plan's full worked example (interface + two `implements`
   classes + one generic function + two call sites) parses end to end
   into the AST shapes described above — `InterfaceDef.method_name ==
   "compare_to"`, `params == [Param{name:"other", ty:"Self"}]`,
   `return_type == "Int64"`; `Money`'s `ClassDef.implements ==
   Some("Comparable".to_string())`; `max`'s `Function.type_params ==
   vec![TypeParam{name:"T", bound:"Comparable"}]`.
2. `def bad[T: Comparable, U: Comparable](...)` parses (the grammar's
   `TypeParamList` is a general comma list — the single-type-parameter
   restriction is sema's job, per the Decision log, not the grammar's) —
   a parser-level AST-shape test asserting `type_params.len() == 2`.
3. An interface with a malformed body (e.g. missing the inner `"->"
   TypeName`, or a second `def` before the outer `"end"`) is a real
   parse error, not a panic — the grammar structurally admits exactly
   one method, per the Decision log.
4. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, `collections.em`, the inheritance/field-access-sugar
   examples) still parses identically — this leaf only adds new optional
   clauses and one new `Item` alternative.
5. No LALRPOP build-time conflicts introduced by `"interface"`/
   `"implements"` as new reserved tokens or by `[` appearing after `def
   Ident` (verified at build time, not assumed — same bar as every prior
   grammar-touching plan).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. the 3 new AST-shape tests above | agent-claimed-locally |

---

## Leaf: leaf-sema-interfaces

### 1. Context
- Why: `emerald-sema` has no concept of an interface or an `implements`
  declaration; `ClassInfo` (verified this session, `crates/emerald-
  sema/src/lib.rs` lines 65–78) carries `fields`, `methods`, `is_module`,
  `superclass` — no `implements` field, and `check_program`'s class-
  registration passes (lines 1258–1374) never look at one.
- Target state: `ClassInfo` gains `implements: Option<String>`, set from
  `ClassDef.implements` in both the pass-1 stub (alongside the existing
  `superclass: c.superclass.clone()`) and `build_flattened_class_info`'s
  returned value. A new registry, `interfaces: HashMap<String,
  InterfaceInfo>` (`InterfaceInfo { method_name: String, params_raw:
  Vec<String>, return_type_raw: String }` — raw, unresolved type-name
  strings, since `"Self"` isn't resolvable via `resolve_type` until
  substituted), built from every `Item::Interface` in one pass alongside
  the existing class/module stub pass. A new function,
  `check_interface_conformance(class_name, iface_name, info, interfaces,
  classes) -> Result<(), Diagnostic>`, substitutes every `"Self"` in the
  interface's `params_raw`/`return_type_raw` with `class_name` itself,
  resolves the substituted strings via the existing `resolve_type`, and
  compares the result *exactly* (invariant, not covariant — same
  discipline as plan 32's override check) against `info.methods.get(
  &iface.method_name)` (the class's already-**flattened** method table —
  so an interface requirement satisfied by an *inherited* method, plan
  32's contribution, is accepted for free). Called from `check_program`
  immediately after a class with `implements.is_some()` successfully
  registers its flattened `ClassInfo`.

### 2. Acceptance Criteria
1. This plan's worked example's `Money`/`Distance` classes (each
   `implements Comparable`, each defining a correctly-shaped
   `compare_to`) type-check `Ok(())`.
2. A class declaring `implements Comparable` but never defining
   `compare_to` at all is rejected with a diagnostic naming the class,
   the interface, and the missing method.
3. A class declaring `implements Comparable` with a `compare_to` of the
   wrong shape (e.g. `def compare_to(other: Money) -> Boolean` — wrong
   return type) is rejected with a diagnostic naming the expected vs.
   found signature.
4. `class Foo implements NotAnInterface ... end` (undefined interface
   name) is rejected with a diagnostic, not a panic.
5. A **subclass** (plan 32) that itself declares no `compare_to` but
   inherits one from a superclass that does, and itself declares
   `implements Comparable`, type-checks `Ok(())` — real proof
   conformance is checked against the flattened, not the class's own-
   declared-only, method table.
6. Regression: every prior plan's class/module sema test still passes
   unmodified — a class with `implements: None` never invokes the new
   conformance check at all.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 5 new cases above | agent-claimed-locally |

---

## Leaf: leaf-sema-generics

### 1. Context
- Why: `emerald-sema`'s `Type` enum (verified this session, lines
  12–38) has no placeholder-type variant, `FunctionSig`/`sigs:
  HashMap<String, FunctionSig>` (lines 54–57, populated flatly by plain
  function name, no overload slot per the locked "no overloading in v1"
  rule — `spec/SEMANTICS.md` §3.1) has no notion of an unresolved type
  parameter, and `infer_expr_type`'s `Expr::Call`/`Expr::MethodCall` arms
  (lines 456–539) only ever resolve against concrete, already-known
  types.
- Target state: `Type` gains `Generic(String)` (a not-yet-concrete
  type-parameter reference, legal only inside the body of the generic
  function that declares it). A new `GenericFunctionSig { type_param:
  String, bound: String, params_raw: Vec<String>, return_type_raw:
  String }` and registry `generic_sigs: HashMap<String,
  GenericFunctionSig>`, populated from `Item::Function`s whose
  `type_params` is non-empty (exactly one entry required — two or more
  is a diagnostic, per the Decision log's scope cut; the ordinary `sigs`
  pass skips these functions entirely, so a generic function's name is
  never present in both registries at once). `infer_expr_type`/
  `check_stmt`/`check_function_body` gain a threaded `generic_ctx:
  Option<(&str, &str)>` parameter (type-param name, bound interface
  name), `None` everywhere except a new `check_generic_function_body`
  entry point that builds the body's initial `env` by mapping any
  parameter whose declared type-name equals the type parameter to
  `Type::Generic(name)` (bypassing `resolve_type`, which cannot resolve
  a bare `"T"`) and passes `Some((type_param, bound))` down. Inside that
  context, `infer_expr_type`'s `Expr::MethodCall` arm gains a branch: a
  call on a `Type::Generic(name)` receiver where `generic_ctx`'s name
  matches resolves against `interfaces[bound]`'s required method (with
  `Self` substituted to `Type::Generic(name)` itself, per the Decision
  log's "Self substituted twice" mechanism), not against the `classes`
  registry. Separately, `infer_expr_type`'s `Expr::Call` arm gains a
  branch (checked before the plain `sigs.get(name)` fallback) for
  `generic_sigs.get(name)`: infers every argument's concrete type,
  requires every parameter position typed with the type parameter to
  agree on one concrete `Type` (a mismatch is a diagnostic naming both
  disagreeing types and their argument positions), requires that
  concrete type to be `Type::Class(c)` with `classes[c].implements ==
  Some(bound)` (a diagnostic otherwise, naming the missing/wrong
  interface), then substitutes the resolved concrete type into
  `params_raw`/`return_type_raw`, runs the result through the existing
  `check_args`, and returns the substituted return type.

  Illustrative shape of the call-site check (prose mechanism, not final
  Rust):
  ```
  let g = generic_sigs[name];
  let arg_types = args.map(infer_expr_type);
  let concrete = the one distinct Type among positions where params_raw[i] == g.type_param;
  require concrete is Type::Class(c) and classes[c].implements == Some(g.bound);
  let effective_params = params_raw.map(|raw| if raw == g.type_param { concrete } else { resolve_type(raw) });
  check_args(name, args, &effective_params, ...);
  return if g.return_type_raw == g.type_param { concrete } else { resolve_type(&g.return_type_raw) };
  ```

### 2. Acceptance Criteria
1. This plan's worked example's `max[T: Comparable]` body type-checks
   `Ok(())` exactly once (not once per call site) — `a.compare_to(b)`
   resolves via `Comparable`'s required signature with both operands
   typed `Type::Generic("T")`.
2. Both call sites, `max(m1, m2)` and `max(d1, d2)`, type-check `Ok(())`
   independently, substituting `T → Money` and `T → Distance`
   respectively, each returning the correspondingly substituted
   `Type::Class(_)`.
3. `max(m1, d1)` (one `Money`, one `Distance` — inconsistent `T`) is
   rejected with a diagnostic naming both disagreeing types and their
   argument positions.
4. Calling a generic function bounded by `Comparable` with a class that
   declares no `implements` clause at all (or `implements` a different,
   unrelated interface) is rejected with a diagnostic naming the missing
   requirement.
5. `def bad[T: Comparable, U: Comparable](a: T, b: U) -> T ... end`
   (two type parameters) is rejected at registration time with a
   "multiple type parameters are not supported" diagnostic — not a
   panic, not a silent single-parameter fallback.
6. A `type_params`-non-empty `Function` found while registering a class
   method or module function is rejected with a "generic methods are
   not supported" diagnostic, per the Decision log's disclosed cut.
7. Regression: every prior plan's sema test suite passes unmodified —
   a non-generic `Expr::Call` never touches `generic_sigs` or
   `generic_ctx` (both default to empty/`None`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 6 new cases above | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-monomorphization

### 1. Context
- Why: nothing compiles an interface, an `implements` declaration, or a
  generic function to machine code — `declare_user_functions` (verified
  this session, `crates/emerald-codegen/src/lib.rs` lines 3438–3482)
  declares exactly one LLVM `FunctionValue` per `Item::Function`, keyed
  by its bare name; there is no name-mangling, no specialization
  concept, and no per-call-site symbol-selection logic anywhere in this
  file today.
- Target state: a new pass, `collect_generic_specializations(program,
  local_class_types) -> HashMap<String, HashSet<String>>` (generic
  function name → every distinct concrete class name it is ever called
  with, anywhere in the whole program), implemented as an AST walk in
  the same style as the existing `collect_idents_in_expr`/
  `collect_idents_in_stmt` walkers (already present for lambda free-
  variable analysis — this is a new, analogous walk, not a new walking
  primitive), resolving each call site's concrete argument type via the
  same `local_classes: HashMap<String, String>` local-variable-declared-
  class tracking `build_method_call` already relies on. For every
  `(generic_fn_name, concrete_class_name)` pair collected: substitute
  the concrete class name for the type-parameter name in the `Function`
  AST's own `params`/`return_type` strings, then run the *existing*,
  unmodified single-function codegen path (`bind_params`,
  `build_function_body`, `param_kinds`, `make_fn_type`) against that
  substituted, now fully concrete signature — reusing 100% of the
  non-generic function codegen machinery. Register the result under a
  mangled symbol, `format!("{}$${}", f.name, concrete_class_name)`
  (`max$$Money`, `max$$Distance`), in `user_func_ids` — the bare generic
  name (`max`) is never registered as a callable LLVM symbol at all. At
  each generic call site, `build_expr`'s `Expr::Call` arm resolves the
  call's own concrete argument type the same way the collection pass
  did, forms the matching mangled name, looks it up in `user_func_ids`,
  and emits a direct LLVM `call` against it — identical call-emission
  code to every non-generic `Expr::Call` today, just keyed by the
  mangled name.
- Dependencies: `leaf-sema-generics`, `leaf-sema-interfaces` (codegen
  runs on already-checked input — every instantiation codegen ever
  attempts has already been proven type-safe and interface-conformant
  by sema, matching every prior codegen leaf's stated contract).

### 2. Acceptance Criteria
1. This plan's full worked example, compiled, linked, and run, prints
   exactly `750\n100\n` — real executed proof that both specializations
   are independently correct and take opposite control-flow branches
   (see the worked example's own walkthrough above).
2. Exactly two specialized LLVM functions are emitted for `max`
   (`max$$Money`, `max$$Distance`) and no bare `max` symbol is ever
   emitted or referenced — checkable directly from the exact output
   above, since `max$$Money` and `max$$Distance` operate over
   different field layouts (`Point`-style 8-byte-slot offsets for
   `cents` vs. `meters`) and must each independently produce the
   correct result for that to be possible at all.
3. Every call to a generic function compiles to a direct LLVM `call`
   instruction against a statically-resolved, name-mangled symbol — no
   indirect call (through a computed function pointer) and no runtime
   type tag read anywhere in the emitted code for this feature, matching
   the Decision log's explicit vtable rejection.
4. An unsupported/malformed shape reaching codegen directly (e.g. a
   call site whose concrete type sema should already have rejected)
   defensively returns a descriptive `Err`, not a panic — same AC
   standard as every prior codegen plan (plan 08 AC4, plan 09 AC3, plan
   32 AC2).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`
- **Create:** `examples/interfaces_generics.em` (this plan's worked
  example, matching the project's established `examples/*.em` +
  `emerald-cli/tests/examples.rs` compiled-and-run test convention —
  verified this session against `crates/emerald-cli/tests/examples.rs`,
  which asserts exact stdout for `classes.em`, `collections.em`, etc.
  the same way)
- **Modify:** `crates/emerald-cli/tests/examples.rs` (new
  `interfaces_generics_em_prints_expected_sequence` test asserting
  `compile_and_run("interfaces_generics.em") == "750\n100\n"`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass | agent-claimed-locally |
| Workspace (real compiled-and-run proof) | `cargo test --workspace` | all pass, incl. `interfaces_generics_em_prints_expected_sequence` printing exactly `750\n100\n` | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

## Out of scope / deferred
- Generic classes/structs (`class Box[T] ... end`) — needs a per-
  instantiation storage-layout strategy (monomorphized struct layouts
  interacting with plan 32's field-offset invariant), a materially
  larger leaf than monomorphizing one function body; see Decision log.
- Generic methods on classes/modules (`def m[T: Comparable](...)`
  inside a `class`/`module` body) — grammatically reachable but
  rejected by sema with a real diagnostic; see Decision log.
- Multiple type parameters, unbounded type parameters, and interface
  intersection bounds (`T: A + B`) — real extensions of the same
  mechanism, not needed by the minimal complete example; see Decision
  log.
- Multiple `implements` per class (`class Foo implements A, B`) — the
  grammar's `ImplementsClause` is a single `Ident`, not a list; not
  needed to prove the mechanism.
- Interfaces with more than one required method — structurally
  impossible in this plan's grammar, not merely undelivered; see
  Decision log.
- An interface method whose return type is itself `Self` (e.g. a
  hypothetical `clone(): Self`) — needs return-type substitution back to
  the *caller's* concrete type at each instantiation, not just parameter
  substitution; `Comparable`'s one method returns a fixed `Int64`, so
  this plan never needs it.
- `<=>` as literal, dispatchable infix operator syntax — needs a
  project-wide operator-overload dispatch mechanism that doesn't exist
  today for any operator; see Decision log's `compare_to` naming
  decision.
- Primitive types (`Int64`, `Float64`, ...) implementing an interface —
  no grammar position exists to attach `implements` to a primitive type
  name; see Decision log.
- Explicit instantiation syntax (`max[Money](m1, m2)`) — this plan's
  call-site type inference (from argument types) is always sufficient
  for its own worked example and for plan 42's anticipated `Array[T]`-
  wide operations; an explicit-instantiation escape hatch is a real,
  separate future convenience, not required here.
