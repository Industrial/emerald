2026-09-22T22:42:00Z

---
name: "Class-Level Static Methods — ClassName.method(args) With No Instance in Hand"
overview: "A `static fn` modifier on a class method (mirroring the existing `pure`/`comptime` MethodDef prefix modifiers, grammar.lalrpop:524) that compiles with no implicit `self` receiver and is callable as `ClassName.method(args)` — the exact, verified-missing prerequisite history/2026-09-22T033000Z-derive-serializable.md's own Decision log named for its deferred `from_json_value(v: JsonValue): Result[Self, String]` leaf, and the single highest-leverage gap inception-3-stdlib-supremacy.md §4.1 names for the whole 91-191 stdlib batch: real round-trip parse_as[T] for a user's own class, not just a dynamic JsonValue tree, for every data-format plan (118-129, 189) that will ever want it."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-grammar-static-modifier
    content: "grammar.lalrpop's MethodDef (line 515) gains a fourth optional prefix modifier, `<is_static:\"static\"?>`, slotted before `<is_pure:\"pure\"?>` (or alongside it — order among the three modifiers is a grammar detail, not a semantic one) using the exact same `Token?` mechanism `pure`/`comptime` already establish at lines 524/285. `static` is reserved the same LALR(1) way `pure`/`comptime`/`class`/`actor` already are. FuncDef (top-level functions, line 279) and ModuleDef methods do NOT gain this modifier — `static` is grammatically reachable only inside ClassDef's own MethodDef* list, the same class-only-reachability precedent MethodName's own operator-method forms (`+`, `==`, `<=>`, `[]`, `[]=`, lines 503-513) already establish for method-definition-position-only grammar. `Function` gains a new `is_static: bool` field (mirrors `is_pure`/`is_comptime` exactly), set from `is_static.is_some()`."
    status: pending
  - id: leaf-sema-no-self-binding
    content: "emerald-sema's method-registration/body-checking path passes `self_fields: None` when checking a `static`-marked method's body — reusing the EXACT mechanism a top-level FuncDef's body already uses (a top-level function has no self either), not a new self-lessness concept. A `static` method's body that references `@field` (Expr::InstanceVar) or bare `self` must be rejected with a real, named diagnostic (e.g. `` static method \`from_json_value\` may not access instance state — it has no receiver ``), the same `self_fields.is_none()`-gated rejection path an ordinary top-level function's own InstanceVar use already hits today (verify this path exists and reuse it directly; do not add a second one)."
    status: pending
  - id: leaf-sema-static-call-dispatch
    content: "New dispatch branch in infer_expr_type's method-call resolution (crates/emerald-sema/src/lib.rs, inside the giant match currently spanning L3036-L6427): when a MethodCall's receiver is `Expr::Ident(name)` AND `name` is absent from the local `env` (not a bound variable) AND `name` is present in `classes` (a known class) AND that class has a method registered with `is_static: true` matching the called name — resolve as a static call: type-check args against that method's own FunctionSig exactly like an ordinary call, and produce the method's declared return type. This is a NEW dispatch path — verified directly against derive-serializable.md's own Decision log that today `.new`/`.spawn`/`.remote`/`.locate` are the ONLY reserved bare-class-name call forms; nothing currently recognizes `ClassName.arbitrary_method(...)` at all, so this cannot regress an existing accepted program. If `name` is absent from BOTH `env` and `classes`, or present in `classes` but the method isn't `is_static`, existing diagnostics fire unchanged."
    status: pending
  - id: leaf-codegen-static-dispatch
    content: "crates/emerald-codegen/src's method-call lowering gains the matching static-call case: compile to a direct LLVM call to the class method's already-existing function symbol, WITHOUT prepending a `self` pointer as the first argument (every instance method today is presumably lowered as a free function taking `self` as an implicit first parameter — verify this exact convention directly against build_method_call/build_class_layout before assuming it, per this project's own \"verified, not assumed\" discipline, then omit exactly that one parameter for the static case). No vtable, no indirect call, no runtime dispatch of any kind — a real, disclosed continuation of this compiler's existing no-dynamic-dispatch architecture, not an exception to it."
    status: pending
  - id: leaf-example-and-gate
    content: "examples/static_methods.em (the Concrete Proof below), wired into emerald-cli/tests/examples.rs's checked table; new emerald-parser unit test(s) covering the grammar's is_static field population (mirroring derive Comparable's own analogous test shape); new emerald-sema unit tests covering (a) a static method's own InstanceVar-access rejection, (b) ClassName.static_method(...) type-checking correctly, (c) the existing .new/.spawn/.remote/.locate forms and ordinary instance .method() calls remaining completely unaffected; full cargo nextest run --workspace / cargo clippy --workspace --all-targets / treefmt gate."
    status: pending
  - id: leaf-unblock-from-json-value-followup
    content: "Not this plan's own scope, but the explicit reason it exists: once this lands, derive-serializable.md's deferred `leaf-from-json-value-deferred` becomes buildable — a future, separate plan can extend expand_derives's `\"Serializable\"` branch to also synthesize a `static fn from_json_value(v: JsonValue): Result[Self, String]`. Record that follow-up here as a named pointer, do not implement it in this plan (this plan's own Concrete Proof below deliberately does not touch JsonValue at all, to keep this plan's own surface small and independently testable)."
    status: pending
isProject: false
---

# Plan 196 — Class-Level Static Methods

inception-3-stdlib-supremacy.md §4.1 names automatic round-trip
serialization for a user's own class as "the single highest-leverage
gap" left in the whole 91-191 stdlib batch — not because `Json.parse`
is missing (plan 118 already gives Emerald a real, working `JsonValue`
dynamic tree), but because every language surveyed in that document's
§1 treats `MyClass.from_json(s)`-style round-tripping, not just a
dynamic tree, as the *actual* everyday story. `derive Serializable`
(history/2026-09-22T033000Z-derive-serializable.md, implemented and
merged this batch) closed exactly half of that gap —
`to_json_value(self): JsonValue` is real, compiled, and verified
end-to-end — and its own Decision log discloses, precisely and without
hedging, why the other half was deferred rather than attempted:

> constructing `Self` from a parsed `JsonValue` needs a `ClassName.
> from_json_value(v)` call with no receiver instance in hand, and
> `emerald-sema`'s own `Expr::New` arm (the sole place a class is ever
> constructed) proves there is no such dispatch path for a real user
> class today — `.new`/`.spawn`/`.remote`/`.locate` are the only
> reserved bare-class-name call forms, and `grammar.lalrpop`'s
> `MethodDef` admits only ordinary, implicitly-`self`-taking methods,
> no class-level static method at all.

This plan is that named prerequisite, scoped narrowly and on its own:
a real `static fn` marker and a real `ClassName.method(args)` dispatch
path, with no dependency on `JsonValue`, `derive Serializable`, or any
data-format plan at all — so it can be built, tested, and verified
completely independently, and `leaf-unblock-from-json-value-followup`
above hands off the actual `from_json_value` synthesis to whichever
plan picks it up next, the same "disclosed, not dodged, handed off
precisely" discipline plan 192's own investigation record and
derive-serializable.md's own deferred leaf both already establish.

## Concrete proof this plan targets

```ruby
class Point
  x: Int64
  y: Int64

  fn initialize(x: Int64, y: Int64): Void do
    @x = x
    @y = y
  end

  static fn origin(): Point do
    Point.new(0, 0)
  end

  static fn midpoint(a: Point, b: Point): Point do
    Point.new((a.x + b.x) / 2, (a.y + b.y) / 2)
  end
end

o: Point = Point.origin()
puts o.x
puts o.y

p1: Point = Point.new(2, 4)
p2: Point = Point.new(8, 10)
m: Point = Point.midpoint(p1, p2)
puts m.x
puts m.y
```

Expected output: `0`, `0`, `5`, `7`. `Point.origin()` and `Point.
midpoint(a, b)` are both real static methods — `Point.origin` takes no
arguments and constructs a `Point` internally via the already-existing
`.new` dispatch; `Point.midpoint` takes two `Point` arguments and reads
their fields via ordinary (non-static) instance method calls (`a.x`
requires `x` to have a `read` accessor, or this example's `Point` needs
one — the implementing plan should add `read x: Int64` / `read y:
Int64` to the fixture, mirroring plan 33's existing accessor sugar,
rather than reach into private fields from outside the class). Neither
static method touches `@x`/`@y`/`self` directly — proving
`leaf-sema-no-self-binding`'s rejection path is never even triggered by
a *correctly written* static method, only by an incorrect one.

A second, negative fixture belongs in the same test file (or a sibling
one): a `static fn` body that references `@x` directly must be
rejected at compile time with the named diagnostic from
`leaf-sema-no-self-binding`, never silently accepted and never a panic.

## Decision log

- **Why an explicit `static` keyword, not inference from whether a
  method's body happens to reference `self`/`@field`.** `pure` and
  `comptime` are both already explicit, opt-in prefix modifiers on
  `MethodDef`/`FuncDef` (grammar.lalrpop:285/524) rather than inferred
  from a body's own content — `static` follows that exact precedent for
  the same reason both of those do: a method whose body doesn't
  currently reference `self` is not necessarily *intended* to be
  callable without an instance (a future edit adding a `self` reference
  should be a real compile error at the call site, `ClassName.method
  (args)`, not a silent behavior change from "static call" to "you now
  need an instance"). Explicit, grammar-level, checked once, matches
  this compiler's own established idiom exactly.
- **Why `self_fields: None` and not a new "static context" concept in
  sema.** Verified directly against this compiler's own existing
  split: a top-level `FuncDef` already has no `self` at all, and
  whatever mechanism currently prevents a top-level function from
  referencing `@field`/`self` (the `self_fields: Option<&HashMap<
  String, Type>>` parameter threaded through `infer_expr_type`/
  `check_stmt` being `None` for a top-level function's own body) is the
  exact same mechanism a `static` method's body needs. Reusing it
  directly, rather than inventing a second "no self here" pathway, is
  both less code and — more importantly — guarantees a `static` method
  body is checked by literally the same, already-battle-tested rules a
  top-level function's body is, with no new diagnostic surface to get
  subtly wrong.
- **Why the dispatch check is receiver-is-`Ident`-not-in-`env`-but-in-
  `classes`, not a new grammar production.** `ClassName.method(args)`
  is, at the grammar level, structurally identical to `some_var.method
  (args)` — both are an `Ident` receiver followed by `.method(...)`,
  already parsed today by the same production ordinary instance method
  calls use. The real distinguishing fact — "is `ClassName` a bound
  local variable, or a known class name with no local shadowing it" —
  is only knowable in sema, which already has both `env` (bound locals)
  and `classes` (registered class names) in hand at every call site
  that needs this check. No grammar change is needed for the *call*
  side at all — only for the *declaration* side (`static fn`, above).
  This mirrors this grammar's own repeatedly-stated "grammar stays
  general, sema narrows" split (cited explicitly in this same file's
  comments for `DeriveClause`, `NewtypeDef`, and `TypeParam`'s bound-
  less case).
- **Why no vtable, no indirect call — direct LLVM call only.** This is
  not a new architectural decision this plan is introducing; it is a
  continuation of a decision already made and load-bearing everywhere
  else in this compiler (GRAMMAR.md's permanently-declined `vtables /
  dynamic dispatch`, cited directly in inception-3-stdlib-supremacy.md
  §4's own "permanent, disclosed declines" list). A static method has
  exactly one possible target function, known at compile time, with no
  receiver-type polymorphism question to resolve at all (there is no
  receiver) — so a direct call is not merely permitted by this
  constraint, it is the *only* shape that makes sense here regardless
  of the constraint.
- **Explicit non-goals, named rather than left ambiguous.** No static
  FIELDS or class-level variables of any kind (only static METHODS);
  no static initialization order question (there is nothing to
  initialize); no change to `.new`/`.spawn`/`.remote`/`.locate`'s own
  existing reserved-form handling, which remains completely separate
  and untouched; no attempt at `from_json_value` itself (see
  `leaf-unblock-from-json-value-followup`) or any other consumer of
  this mechanism — this plan proves the mechanism works in isolation,
  on a fixture (`Point`) that has nothing to do with serialization.

## Not yet decided (real open question, disclosed rather than assumed)

Whether a local variable is permitted to shadow a class name at all
today (e.g. `point: Int64 = 5` in a scope that also has a class named
`point` — lowercase-vs-uppercase convention likely makes this rare in
practice, but this compiler's own grammar does not appear to enforce a
capitalization rule on class names structurally) is not verified by
this plan. If shadowing is currently legal, `leaf-sema-static-call-
dispatch`'s own "absent from `env`" check already resolves the
ambiguity correctly in `ClassName.method(args)`'s favor only when no
local named `ClassName` is in scope — but the reverse case (a real
local variable happens to share a class's exact name AND that class
has a same-named static method) needs the implementing agent to verify
directly which existing diagnostic, if any, already governs this kind
of shadowing, rather than assuming this plan's own dispatch check is
the first place such a collision could ever be observed.
