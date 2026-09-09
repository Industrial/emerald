---
name: Operator Overloading
overview: "User classes may define methods literally named after operator tokens (`+ - * / == <=> [] []=`) and have `a + b` / `a == b` / `a[i]` / `a[i] = v` resolve to them through the exact same static, receiver-declared-type method lookup `a.foo()` already uses (plan 08's `Expr::MethodCall` resolution, verified this session against `emerald-sema`'s `infer_expr_type` and `emerald-codegen`'s `build_method_call`) — Ruby-idiomatic operator overloading obtained for free from Emerald's existing static-dispatch machinery, not a new dynamic-dispatch mechanism; builtin `Int64`/`Float64` arithmetic (plan 18) is untouched by construction."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-parser-operator-methods
    content: "`MethodName`/`MethodDef` grammar nonterminals (class-body-only) accepting `+ - * / == <=> [] []=` as method-definition names, plus the new `<=>` token; `ast.rs` needs no new variant — `Function.name` is already a plain `String`"
    status: pending
  - id: leaf-sema-operator-dispatch
    content: "Route `Expr::Add/Sub/Mul/Div`, `Expr::Compare`'s `==`/`!=`, and `Expr::Index`/`SetIndex` to a user class's operator method via a `resolve_class_operator` helper built on the existing `check_args`, only when the declared operand type is `Type::Class(_)` — builtin numeric/array/hash paths keep their existing rules unchanged"
    status: pending
  - id: leaf-codegen-operator-dispatch
    content: "`build_expr`'s Add/Sub/Mul/Div and Compare arms, plus `build_index`/`build_set_index`, delegate to the existing `build_method_call` when the receiver is a class-typed local — a defensive ASCII symbol-mangling table replaces raw operator characters in emitted LLVM function names"
    status: pending
isProject: false
---

# Plan 40 — Operator Overloading

This is plan 40 of the 36-47 follow-up batch — twelve independent sibling
plans, each owning one distinct identity-preserving gap toward the prior
analysis's ~45%-of-Ruby-surface ceiling (closing every gap compatible
with Emerald's static-dispatch/no-GC/no-reflection identity, declining
anything that would require real dynamism). It is post-v1 scope, same
posture as the 28-35 batch before it; `plan-of-plans.md` itself will be
updated separately once all twelve plans in this batch are authored —
this plan does not touch `plan-of-plans.md` or any other plan file. One
sibling in this batch (the "Comparable interface" plan, referenced below
as plan 41) will require classes to define `<=>`; this plan is what
makes defining `<=>` — and `==`, and the arithmetic/indexing operators —
on a class possible at all.

Concrete proof this plan targets — a `Vector2` class overloading `+` and
`==`, compiled, linked, and run, exact stdout asserted:

```ruby
class Vector2
  read x: Float64
  read y: Float64

  def initialize(x: Float64, y: Float64) -> Void
    @x = x
    @y = y
  end

  def +(other: Vector2) -> Vector2
    Vector2.new(@x + other.x, @y + other.y)
  end

  def ==(other: Vector2) -> Boolean
    @x == other.x && @y == other.y
  end
end

v1: Vector2 = Vector2.new(1.0, 2.0)
v2: Vector2 = Vector2.new(3.0, 4.0)
v3: Vector2 = v1 + v2
puts v3.x
puts v3.y
if v1 == v2
  puts 1
else
  puts 0
end
if v1 == v1
  puts 1
else
  puts 0
end
```

Expected output: `4\n6\n0\n1\n` — `v3 = (1+3, 2+4) = (4.0, 6.0)` (floats
print without a trailing `.0`, same convention as plan 08's `POINT_EXAMPLE`
printing `5\n` for `5.0`), `v1 == v2` is `false` (different components,
prints `0`), `v1 == v1` is `true` (prints `1`). The example deliberately
does **not** write `puts v1 == v2` directly — verified this session
against `emerald-sema/src/lib.rs`'s `Expr::Call` `"puts"` arm (~L441-455):
`puts` accepts only `Type::Int64`/`Float64`/`String` today, not
`Boolean` — the same restriction plan 18's own `BOOL_EXAMPLE` test
already works around with an `if`/`return` shape rather than a bare
`puts flag`. This plan does not extend `puts` to `Boolean` (that is
stdlib/stringification scope, not operator-overloading scope), so the
worked example routes `v1 == v2`'s `Boolean` result through the same
`if`/`else`-to-`Int64` pattern rather than assuming `puts` of a
`Boolean` already compiles.

## Decision log

- **The core framing: Ruby's operator overloading is free because every
  Ruby call — even `1 + 2` — is already a dynamically dispatched method
  call; Emerald's is free for the opposite reason — every call is
  *already* statically resolved by the receiver's declared type, with
  zero vtables.** Verified directly against the actual mechanism `a.foo()`
  already uses: `emerald-sema/src/lib.rs`'s `infer_expr_type`'s
  `Expr::MethodCall` arm (~L523-539) infers the receiver's type, requires
  it to be `Type::Class(class_name)`, looks up `classes[class_name]
  .methods.get(method)` (a plain `HashMap<String, FunctionSig>` — no
  runtime type tag, no method table indirection), and type-checks the
  call's arguments against that signature. `emerald-codegen/src/lib.rs`'s
  `build_method_call` (~L1562-1655) mirrors this exactly at codegen time:
  it reads `local_classes.get(recv_name)` (a compile-time
  `HashMap<String, String>` built from the receiver's own declared
  type — a plain local, a field, or a parameter's type annotation), walks
  `ctx.method_owners` to find which ancestor in the class's flattened
  inheritance chain (plan 32) actually *defines* the method, and emits a
  single direct `build_call` to that statically-known
  `LLVMFunctionValue` — never an indirect call through a computed
  address. `declare_user_functions` (~L3438-3482) mangles every method's
  LLVM symbol as `format!("{}_{}", class_name, method_name)` — a plain
  string concatenation with **zero name-based special-casing**: `"sum"`,
  `"initialize"`, `"age"` all already go through this identical path.
  This plan's entire job is: make the grammar accept `+`, `-`, `*`, `/`,
  `==`, `<=>`, `[]`, `[]=` as valid values for that `method_name` string,
  and make sema/codegen route `a + b` (when `a`'s declared type is a
  user class) into this exact, already-existing, already-static lookup
  instead of the builtin-arithmetic path — not a new dispatch mechanism,
  literally the same one `a.foo()` already uses, extended to cover eight
  more spellings of the method name.
- **`Int64`/`Float64` arithmetic keeps its exact existing typing rule,
  completely unchanged.** Verified against plan 18's actual
  implementation: `emerald-sema`'s `check_numeric_binop` (~L272-294,
  shared by `Sub`/`Mul`/`Div`/`Rem`) and `Expr::Add`'s own inline arm
  (~L370-387) both require `lt == rt` and `lt ∈ {Int64, Float64}` (`Add`
  additionally allows `String`, plan 19); `emerald-codegen`'s `build_expr`
  `Expr::Add` arm (~L958-1001) matches on `(ValKind::Int64, Int64)`,
  `(Float64, Float64)`, `(Str, Str)`, else a defensive `Err`. This plan
  inserts exactly one new branch — "if the LHS's declared type is
  `Type::Class(_)`, route to the class's operator method instead" —
  *before* each of these existing checks; it never touches, weakens, or
  reorders the existing `Int64`/`Float64`/`String` arms. `1 + 2` and
  `1.0 + 2.0` compile to the identical `iadd`/`fadd` instructions after
  this plan as before it.
- **`Expr::Compare`'s `==`/`!=` reuse the existing AST node — no new
  `Expr::Eq`/spaceship variant.** A real, verified gap this plan closes:
  `infer_expr_type`'s `Expr::Compare` arm (~L427-436) today has **no
  restriction on the operand type beyond `lt == rt`** — two same-class
  instances (e.g. two `Point`s) already type-check as `Boolean` today,
  with no operator overload involved. But `emerald-codegen`'s matching
  `Expr::Compare` arm's exhaustive `(ValKind, ValKind)` match (~L1235-1310)
  has no case for two pointer-typed (`ValKind::Ptr`) operands and falls
  through to its final `_ => Err("codegen: comparison operands must both
  be Int64 or both Float64")` — so `p1 == p2` on two `Point`s **type-checks
  today but fails to compile**, a real sema/codegen mismatch, not a
  hypothetical one. This plan closes it specifically: when `lt` is
  `Type::Class(class_name)`, sema now requires the class to declare a
  `==` method (arity 1, param type matching the RHS, return type
  `Boolean`) rather than silently accepting the comparison the way it
  does today; codegen's `Ptr`/`Ptr` case, previously absent, delegates to
  `build_method_call` with method `"=="`. This is a real behavior change
  (tightening a previously-permissive-but-broken sema rule), not a
  purely additive one — disclosed here rather than left implicit.
- **`!=` is derived from `==`, not a ninth overload token.** Matches
  Ruby's own default (`!=` calls `==` and negates it unless a class
  overrides `!=` directly, which Emerald does not support). Sema routes
  `Expr::Compare(_, CompareOp::Ne, _)` on class-typed operands through
  the identical `resolve_class_operator("==", ...)` lookup as `Eq`;
  codegen calls the same compiled `{Class}_==` function and inverts the
  returned `i1` (`build_not`) rather than requiring — or accepting — a
  separately-defined `!=` method. This keeps the task's exact eight-token
  scope (`+ - * / == <=> [] []=`) intact while still making `!=` usable.
- **`<=>` becomes *definable* on a class in this plan; it does not
  become *callable* as an infix expression.** Verified against
  `emerald-parser/src/grammar.lalrpop`: no `"<=>"` token exists anywhere
  today (`CompareOp` only covers `> < >= <= == !=`), and `Expr`/`Stmt`
  have no spaceship-result AST shape. This plan adds `"<=>"` only at the
  method-*definition* position (`MethodName`, see below) — a class can
  write `def <=>(other: Vector2) -> Int64 ... end` and it compiles to a
  real `Vector2_<=>` function via the same zero-special-casing mangling
  described above, with zero further codegen work needed. But no
  `a <=> b` expression syntax is added to `Expr`/`CompareOp` — that
  requires a real, separate follow-up (a new `Expr::Spaceship` variant,
  a comparison-tier grammar production, sema/codegen routing) which is
  explicitly plan 41's job (its Comparable interface needs classes to be
  able to declare `<=>` before it can require and consume one) — this is
  a genuine, disclosed asymmetry: `<=>` is definable but dead code from
  this plan alone, not a hidden gap.
- **Ordering comparisons (`<`, `>`, `<=`, `>=`) on class-typed operands
  are declined here, not silently accepted.** Today's permissive
  `Expr::Compare` (see above) would let e.g. `v1 < v2` on two `Vector2`s
  "type-check" and then fail at codegen with the same generic
  `Int64`-or-`Float64` error. This plan tightens that: any `CompareOp`
  other than `Eq`/`Ne` on a `Type::Class` operand becomes an explicit
  sema diagnostic ("ordering comparison `<` is not supported on class
  `Vector2` — define `<=>`, per a future Comparable interface, not a
  direct `<` overload") rather than a confusing late codegen failure —
  Ruby itself derives `<`/`>`/`<=`/`>=` from `<=>` via the `Comparable`
  module, which Emerald has no mixin mechanism to reproduce; that
  derivation is exactly plan 41's job once `<=>` is definable (this
  plan) and callable (plan 41).
- **`[]`/`[]=` reuse `Expr::Index`/`Stmt::SetIndex` wholesale — no new
  AST node, extending the exact dispatch plan 09/25 already built.**
  Verified against `emerald-codegen`'s `build_index`/`build_set_index`
  (~L1822-1995): both already do a two-way dispatch on the receiver's
  name — `local_array_elem_types.get(arr_name)` (an `Array[T]`) first,
  then `local_classes.get(arr_name).and_then(parse_hash_type)` (a
  `Hash[K, V]`, which plan 25's Decision log already notes shares the
  generic `local_classes` map, "repurposed to hold `Hash[K, V]` strings
  alongside class names") — falling through to a defensive `Err` if
  neither matches. This plan adds the third, previously-defensive-only
  branch: `local_classes.get(arr_name)` holding a *real* class name
  (i.e. `parse_hash_type` returns `None`) whose class declares `[]`
  (read) / `[]=` (write) delegates to `build_method_call` with that
  method name and `[index]` / `[index, value]` as arguments — sema's
  matching `Expr::Index` arm (~L557-581) and `check_set_index`
  (~L783-817) gain the identical third case in their own existing
  three-way (`Array`/`Hash`/now-`Class`) matches.
- **Operator method names are legal only inside a `class ... end` body —
  top-level `FuncDef` and `ModuleDef` are untouched.** Verified against
  `grammar.lalrpop`: `Ident` is defined as
  `r"[A-Za-z_][A-Za-z0-9_]*"` (~L717) — categorically excludes every
  operator token — and `FuncDef`'s single production
  (`"def" <name:Ident> ...`) is shared today by top-level functions,
  module methods, *and* class methods (`ClassDef`'s `<methods:FuncDef*>`).
  Rather than widen `FuncDef` itself (which would make a free function
  literally named `+` parseable — dead code, since `Expr::Call`'s own
  `<name:Ident> "(" ...` production still requires `Ident` and could
  never call it), this plan adds a new `MethodName`/`MethodDef` pair used
  *only* by `ClassDef`'s method list, leaving `FuncDef`/`ModuleDef`
  exactly as they are. This matches Ruby's own posture (operator methods
  are instance methods on a receiver; a top-level `def +` is unusual even
  in Ruby) and avoids inventing an unreachable grammar shape.
- **LLVM symbol names for operator methods go through a defensive
  ASCII-safe mangling table (`+` → `op_add`, `-` → `op_sub`, `*` →
  `op_mul`, `/` → `op_div`, `==` → `op_eq`, `<=>` → `op_cmp`, `[]` →
  `op_index`, `[]=` → `op_index_set`), not the raw operator characters.**
  `declare_user_functions` (~L3457) builds every method's mangled name
  as `format!("{}_{}", class_name, method_name)` and passes it straight
  to `module.add_function`; LLVM's C API in principle accepts arbitrary
  byte-string function names (quoted in IR text, opaque in the object
  symbol table), but this session did not verify inkwell's/LLVM's exact
  escaping behavior for names containing `+`, `[`, `]`, `=` end-to-end
  through `compile_to_object`'s object-emission path. Rather than depend
  on unverified escaping, this plan keeps the *sema-side* lookup keys
  (`ClassInfo.methods`, `ctx.method_owners`) as the literal source token
  (`"+"`, `"[]="`, ...) — unchanged, since those are plain Rust
  `HashMap<String, _>` keys with no symbol-table constraints — and only
  translates to the safe-ASCII form at the one place a real object-file
  symbol gets emitted. A real, disclosed defensive choice, not a
  correctness requirement this plan can prove is unnecessary.
- **Non-`Ident` receivers stay unsupported for operator calls, inheriting
  `build_method_call`'s own existing restriction verbatim.**
  `build_method_call` already requires `Expr::Ident` receivers only
  ("codegen: method calls are only supported on a plain local-variable
  receiver", ~L1573-1577); `build_index`/`build_set_index` carry the
  identical restriction (~L1832-1836, ~L1912-1916). Because this plan's
  operator dispatch *is* `build_method_call` (see below), a chained
  expression like `(v1 + v2) + v3` — whose outer `+`'s LHS is not a bare
  local — hits that same pre-existing restriction and returns a
  descriptive `Err`, not a panic and not silently mis-compiled. This is
  not a new limitation this plan introduces; it is the same one every
  other method call in this compiler already has, inherited for free
  by reusing the mechanism rather than reimplementing it.
- **Declined: unary operator overloading (`-@`, `+@`, `!`, `~`).**
  `Expr::Neg`/`Expr::Not`/`Expr::BitNot` (plans 18/28) keep their exact
  existing single-operand numeric/Boolean/`Int64` rules; no class-typed
  branch is added to any of the three. The task's scope names eight
  binary/indexing tokens, not the four unary ones; the identical
  static-dispatch mechanism this plan builds would extend to them
  trivially, but no worked example needs it and adding it now would be
  unexercised generality.
- **Declined: `coerce`-style mixed-type-operand coercion (`1 + my_obj`
  calling `my_obj.coerce(1)`).** This is the one item in this plan's
  scope that is genuinely, not just conventionally, incompatible with
  Emerald's identity: Ruby's `coerce` protocol requires the runtime to
  take an arbitrary right-hand value of a type the left operand's `+`
  doesn't statically know about, and ask *that value itself*, generically,
  to reshape itself into something addable — a runtime type inspection
  and a callback into a method chosen based on a value's *actual*
  runtime type, not its receiver's declared type. That is precisely the
  reflection/dynamic-dispatch machinery Emerald's identity forbids
  outright ("no method_missing/eval/send/reflection... no dynamic/virtual
  dispatch"), not a merely-inconvenient feature this plan is skipping for
  scope reasons. `1 + my_obj` where `my_obj`'s class defines `+` remains
  a plain type mismatch in this plan (`Int64` `+` still requires an
  `Int64` RHS) — no asymmetric-type arithmetic exists here at all.
- **Declined: an implicit `to_s`-overload hook for `puts`/stringification
  contexts.** Explicitly plan 36's scope (stringification, restricted to
  builtin types only, per that plan's own framing) — this plan does not
  give user classes a `to_s` override, and `puts v3` (a `Vector2`
  instance) remains exactly the sema type error it is today (`puts`
  accepts `Int64`/`Float64`/`String` only — see this plan's own worked
  example note above on why it uses `if`/`else` instead of `puts v1 ==
  v2` directly, the same restriction applying to `Boolean` too). A real,
  disclosed limitation: this plan makes `a + b`/`a == b`/`a[i]` work for
  classes; it does not make `puts a` work for them.

## Leaf: leaf-parser-operator-methods

### 1. Context
- Why: `Ident` is `r"[A-Za-z_][A-Za-z0-9_]*"` (verified,
  `grammar.lalrpop` ~L717) — no operator token can ever lex as one — and
  `FuncDef`'s single production (used by `ClassDef`'s `<methods:
  FuncDef*>`, `ModuleDef`, and top-level functions alike) requires
  `<name:Ident>`. There is no grammar shape today for `def +(...) -> T
  ... end` anywhere, class body or otherwise.
- Target state: a new `MethodName: String` nonterminal —
  `{ <n:Ident> => n, "+" | "-" | "*" | "/" | "==" | "<=>" =>
  <op>.to_string(), "[" "]" => "[]".to_string(), "[" "]" "=" =>
  "[]=".to_string() }` — and a new `MethodDef: Function` nonterminal that
  is `FuncDef`'s exact production with `<name:MethodName>` in place of
  `<name:Ident>`. `ClassDef`'s `<methods:FuncDef*>` becomes
  `<methods:MethodDef*>`; `FuncDef` itself and `ModuleDef` are untouched
  (operator names stay class-body-only, per the Decision log). `"<=>"` is
  a genuinely new terminal (no token literal named `"<=>"` exists
  anywhere in the grammar today) — added purely at this method-name
  position, not as an `Expr`-level comparison operator (that is plan
  41's job).
- Dependencies: plan 08 (`ClassDef`/`FuncDef` grammar shape being
  extended), plan 33 (`ClassField`'s `read` sugar, used by this plan's
  own worked example to give `Vector2` its `x`/`y` accessors without a
  separate field-access mechanism).

### 2. Acceptance Criteria
1. Inside a `class Vector2 ... end` body, `def +(other: Vector2) ->
   Vector2 ... end` parses to `Function { name: "+".to_string(), params:
   [Param { name: "other", ty: "Vector2" }], return_type: "Vector2", ...
   }` — a real parser unit test asserting the exact AST shape, not just
   "it doesn't error."
2. The same holds, one test each, for `def ==(other: Vector2) ->
   Boolean`, `def [](i: Int64) -> Float64`, `def []=(i: Int64, v:
   Float64) -> Void`, `def <=>(other: Vector2) -> Int64`, `def -(...)`,
   `def *(...)`, `def /(...)` — all eight tokens from this plan's scope
   produce a `Function` whose `name` is exactly that token's string.
3. A top-level (non-class-body) `def +(a: Int64, b: Int64) -> Int64 ...
   end` is a real parse error, not accepted — operator-named methods
   stay class-body-only (Decision log).
4. No LALRPOP build-time conflicts — in particular, `"[" "]"` vs. `"["
   "]" "="` must resolve via ordinary LALR(1) lookahead against
   `ParenParams`'s following `"("`/`"->"` tokens, and the new `"<=>"`
   terminal must not collide with the existing `"<"`/`"<="` tokens
   (verified the same way `">="`/`"<="`/`"=="`/`"!="` already coexist
   with `">"`/`"<"`/`"="`/`"!"` today).
5. Regression: every prior plan's example (`hello.em` through plan 34's
   blocks/`yield` examples, and specifically plan 33's `read` field-sugar
   example and plan 32's inheritance example) still parses identically —
   `MethodDef` is strictly additive over `FuncDef`'s existing
   plain-`Ident` shape.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)
- **Unchanged (verified, no edit needed):** `crates/emerald-parser/src/ast.rs`
  — `Function.name` is already a plain `String`; no new AST variant is
  required for a method to be *named* an operator token.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new operator-method-name AST-shape tests and the top-level-rejection negative test | agent-claimed-locally |

---

## Leaf: leaf-sema-operator-dispatch

### 1. Context
- Why: `infer_expr_type`'s `Add`/`check_numeric_binop`-backed
  `Sub`/`Mul`/`Div`/`Rem` arms reject any non-`Int64`/`Float64`(/`String`
  for `Add`) operand outright; `Expr::Compare` has no restriction beyond
  `lt == rt` (a real, verified gap — see Decision log); `Expr::Index`/
  `check_set_index` have a two-way `Array`/`Hash` match with a defensive
  `Err` for anything else. None of the four route to a class's own
  method table today.
- Target state: a new helper,
  `resolve_class_operator(op: &str, class_name: &str, rhs: &Expr, env,
  sigs, classes, self_fields) -> Result<Type, Diagnostic>`, that mirrors
  `Expr::MethodCall`'s own existing arm exactly: look up
  `classes[class_name].methods.get(op)` (diagnostic "class `{class_name}`
  has no operator method `{op}`" if absent), then reuse the existing
  `check_args(op, std::slice::from_ref(rhs), &sig.params, ...)` to check
  arity and the RHS's type against `sig.params[0]`, returning
  `sig.return_type.clone()`. `Expr::Add`'s arm and `check_numeric_binop`
  (shared by `Sub`/`Mul`/`Div`/`Rem`) each gain one new branch, inserted
  *before* their existing `lt != rt` / `lt ∉ {Int64, Float64[, String]}`
  checks: `if let Type::Class(class_name) = &lt { return
  resolve_class_operator(op, class_name, rhs, ...); }`. `Expr::Compare`'s
  `Eq`/`Ne` cases gain the analogous branch (requiring the resolved
  method's return type to be `Type::Boolean`, diagnostic otherwise; `Ne`
  reuses the `"=="`-keyed lookup per the Decision log); any other
  `CompareOp` on a `Type::Class` operand becomes an explicit "ordering
  comparison not supported on class" diagnostic. `Expr::Index`'s
  `Type::Array`/`Type::Hash` match gains a third `Type::Class(class_name)`
  arm requiring a one-parameter `"[]"` method; `check_set_index`'s
  three-value-tuple match (`container, elem_ty, index_expected`) gains
  the analogous `Type::Class` arm sourced from a two-parameter `"[]="`
  method's signature, reusing every line of the existing post-match
  index-type/value-type checking code unchanged.
- Dependencies: `leaf-parser-operator-methods` (needs `Function.name` to
  actually be an operator token by the time sema sees it); plan 08's
  class registry (`ClassInfo`, `classes: HashMap<String, ClassInfo>`);
  plan 18's `check_numeric_binop`/`Expr::Add` (being extended, not
  replaced); plan 09/25's `Expr::Index`/`check_set_index` (being
  extended, not replaced).

### 2. Acceptance Criteria
1. This plan's `Vector2` worked example type-checks `Ok(())` end to end
   (`v1 + v2`, `v1 == v2`, `v1 == v1`, and every internal `@x + other.x`
   / `@x == other.x` float comparison inside the two operator methods).
2. A `Vector2` with no `+` method: `v1 + v2` is rejected with "class
   `Vector2` has no operator method `+`" — not a panic, and not silently
   falling through to the builtin `Int64`/`Float64` numeric check.
3. An argument-type mismatch — `v1 + 5` where `+` is declared
   `(other: Vector2)` — is rejected via the reused `check_args` message
   ("argument 1 to `+` has type Int64, expected Class(\"Vector2\")"),
   proving the RHS is genuinely checked against the declared parameter
   type, not accepted unconditionally.
4. A `Vector2` with no `==` method: `v1 == v2` is rejected with "class
   `Vector2` has no operator method `==`" — this is a real, intentional
   behavior change from today's verified-permissive `Expr::Compare` (see
   Decision log), disclosed and tested here rather than left as a silent
   regression risk.
5. A minimal class declaring `[]`/`[]=` (e.g. a `Bag` wrapping an
   `Array[Int64]`, delegating `[](i)`/`[]=`(i, v)` straight to the
   underlying array) type-checks a read, a write, an index-type
   mismatch (rejected), and a missing-`[]=`-method case (rejected) —
   covering both the read and write sides of the new `Type::Class` arm.
6. Regression: plan 18's `ARITHMETIC_EXAMPLE`/`SHORT_CIRCUIT_EXAMPLE`,
   plan 09's `ARRAY_EXAMPLE`, and plan 25's `HASH_EXAMPLE` all still
   type-check identically to before this plan — every new branch is
   reached only when the operand's inferred type is `Type::Class(_)`, so
   `Int64`/`Float64`/`Array`/`Hash` operands never enter it.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. new operator-dispatch positive/negative tests | agent-claimed-locally |

---

## Leaf: leaf-codegen-operator-dispatch

### 1. Context
- Why: `build_expr`'s `Expr::Add`/`Sub`/`Mul`/`Div` arms and its
  `Expr::Compare` arm's exhaustive `(ValKind, ValKind)` match have no
  case for class-typed (`ValKind::Ptr`) operands beyond a defensive
  `Err`; `build_index`/`build_set_index`'s two-way dispatch
  (`local_array_elem_types` then `local_classes` + `parse_hash_type`)
  has the same gap. `build_method_call` (~L1562-1655) already implements
  every piece of machinery needed — receiver-name-to-class-name lookup
  via `local_classes`, inheritance-aware method-owner resolution via
  `ctx.method_owners`, and a direct static `build_call` — for the
  ordinary `a.foo()` case.
- Target state: `Expr::Add`'s arm gains a branch, checked before its
  existing `(ValKind, ValKind)` match: `if let Expr::Ident(name) =
  lhs.as_ref() { if local_classes.contains_key(name) { return
  build_method_call(context, builder, lhs, "+",
  std::slice::from_ref(rhs.as_ref()), vars, local_classes,
  local_array_elem_types, ctx); } }` — literally delegating to the
  existing method-call codegen with `"+"` as the method name and `[rhs]`
  as the argument list. `Sub`/`Mul`/`Div` gain the identical branch with
  `"-"`/`"*"`/`"/"`. `Expr::Compare`'s `Eq` case gains the analogous
  branch calling `build_method_call(..., "==", ...)`; its `Ne` case
  calls the same `"=="` method and then `builder.build_not` (or an XOR
  with a constant `true`) on the returned `i1`. `build_index` gains a
  third branch — after the existing `local_array_elem_types` and
  `local_classes`+`parse_hash_type` checks both miss, if `local_classes
  .get(arr_name)` names a real class with a registered `"[]"` method,
  delegate to `build_method_call(..., "[]", &[index.clone()], ...)`;
  `build_set_index` gains the mirror branch for `"[]="` with `&[index,
  value]`. A new `mangled_operator_symbol(op: &str) -> &'static str`
  table (`"+"` → `"op_add"`, `"-"` → `"op_sub"`, `"*"` → `"op_mul"`,
  `"/"` → `"op_div"`, `"=="` → `"op_eq"`, `"<=>"` → `"op_cmp"`, `"[]"` →
  `"op_index"`, `"[]="` → `"op_index_set"`) is consulted only inside
  `declare_user_functions`'/`define_method`'s mangled-name construction
  for `Item::Class` methods, replacing the raw `m.name` with its
  ASCII-safe form when `m.name` is one of these eight tokens (falling
  back to `m.name` unchanged for every ordinary method) — every other
  lookup (`ctx.method_owners`, `ClassInfo.methods`) keeps using the
  literal operator-token string as its key, unaffected.
- Dependencies: `leaf-sema-operator-dispatch` (codegen runs on
  already-checked input, same contract as every prior codegen plan);
  `build_method_call`/`build_index`/`build_set_index`/
  `declare_user_functions` (plans 08/09/25/32, all being extended, not
  replaced).

### 2. Acceptance Criteria
1. This plan's `Vector2` worked example, compiled, linked, and run,
   prints exactly `4\n6\n0\n1\n` — real executed proof that `+`
   allocates a genuine new `Vector2` via a real call into a compiled
   `Vector2_op_add` function, and that `==` genuinely calls into a
   compiled `Vector2_op_eq` function rather than doing a raw pointer
   comparison (which would make `v1 == v1` trivially true via identity
   but would have no way to make `v1 == v2` correctly false based on
   field values alone — the printed `0` for `v1 == v2` is only possible
   through a real per-field value comparison).
2. A minimal class declaring `[]`/`[]=` (same `Bag`-style example as the
   sema leaf's AC5), compiled, linked, and run, produces a real
   read-then-write-then-read-back proof analogous to plan 09's own
   `ARRAY_EXAMPLE` (`60\n99\n`-style), proving `obj[i]`/`obj[i] = v`
   genuinely delegate to the class's `[]`/`[]=` methods.
3. An unsupported shape — `(v1 + v2) + v3` (a non-`Ident` LHS receiver
   on the outer `+`) — defensively returns a descriptive `Err`, not a
   panic, inherited directly from `build_method_call`'s own pre-existing
   receiver restriction (Decision log) — same AC standard as every prior
   codegen plan.
4. Regression: plan 18's Example A/B, plan 08's `POINT_EXAMPLE`, plan
   09's `ARRAY_EXAMPLE`, and plan 25's `HASH_EXAMPLE`, compiled, linked,
   and run, still print their exact original output — the new
   class-dispatch branches are strictly additive checks inserted before
   the existing `(ValKind, ValKind)` match arms and the existing
   `local_array_elem_types`/`local_classes`+`parse_hash_type` checks,
   never replacing or reordering them.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `Vector2` (`4\n6\n0\n1\n`) and `[]`/`[]=` proofs | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- Unary operator overloading (`-@`, `+@`, `!`, `~`) — see Decision log;
  the same mechanism would extend trivially, but no worked example needs
  it.
- `coerce`-style mixed-type-operand coercion — declined outright as a
  genuine identity-ceiling violation (requires runtime inspection of an
  arbitrary value's actual type), not a scope cut. See Decision log.
- An implicit `to_s` overload hook for `puts`/stringification — plan
  36's scope, not this plan's. See Decision log.
- `a <=> b` as a callable infix expression, and any `<`/`>`/`<=`/`>=`
  derivation from it (Ruby's `Comparable` module) — `<=>` becomes
  definable on a class in this plan; making it callable, and deriving
  ordering operators from it, is plan 41's job. See Decision log.
- Chained/non-`Ident`-receiver operator calls (e.g. `(v1 + v2) + v3`) —
  inherits `build_method_call`'s pre-existing plain-local-receiver
  restriction; not a new limitation this plan introduces.
