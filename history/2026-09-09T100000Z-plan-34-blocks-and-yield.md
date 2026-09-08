---
name: Blocks and Yield
overview: "Implicit block arguments (`method(args) { |x| ... }`) and `yield` inside a method that declares a block parameter — GRAMMAR.md §§6-7/9 and SEMANTICS.md §5's block-argument mechanism, distinct from plan 10's explicit `Proc` values, proven with a user-defined `repeat`/`yield` program."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-blocks
    content: "Function.block_param: Option<String> (`&name` in a param list); a trailing `{ |params| body }` block literal attached to Call/MethodCall, desugared to an extra Expr::Lambda argument; Stmt::Yield(Vec<Expr>)"
    status: pending
  - id: leaf-sema-blocks
    content: "`yield` legal only inside a function declaring block_param; per-call-site checking of yield's arguments against the actually-attached block literal's params (no fixed per-function block signature exists to check against once, unlike an ordinary parameter)"
    status: pending
  - id: leaf-codegen-blocks
    content: "Call-site specialization: a function with block_param + yield is compiled fresh per call site that attaches a literal block, with yield lowered to a direct call into that block's synthesized function (reuses plan 10's lambda-to-function codegen)"
    status: pending
isProject: false
---

# Plan 34 — Blocks and Yield

This is one of a numbered follow-up batch (28-35) of language-completeness
plans, none of them rows in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
that table's own Completion note already marks Emerald v1 done at row 15;
this batch, like plans 17-27 before it, is new, post-v1 scope. This plan
does not touch `plan-of-plans.md` or any other plan file. Siblings in
this same batch cover bitwise operators, `elsif`/`unless`/modifier
control flow, `for...in` iteration, compound/multiple assignment, class
inheritance, field-access sugar, and DWARF debug info — this plan is
blocks/`yield` only, and doesn't re-do plan 25's `Array[T]`
iteration-protocol work or plan 30's `for...in` sibling.

Concrete proof this plan targets:

```ruby
def repeat(n: Int64, &blk) -> Void
  i: Int64 = 0
  while i < n
    yield i
    i: Int64 = i + 1
  end
end

repeat(3) { |i: Int64| puts i }
```

Expected output: `0\n1\n2\n`.

## Decision log

- **This is a genuinely different mechanism from plan 10's "Blocks &
  Closures."** Re-read
  [plan 10](../../history/2026-09-08T192515Z-plan-10-blocks-closures.md)
  in full before touching this plan: it shipped `->(params) -> Ret {
  body }`, an explicit **value** of type `Proc`, bound to a named
  top-level local and invoked via `.call(args)` — and its own Decision
  log explicitly scoped out exactly what this plan adds: "No
  block-argument-to-method syntax (`xs.each { |x| ... }`, `yield`, `def
  f(&blk)`)." `spec/GRAMMAR.md` §§6, 7, and 9 (re-verified this session)
  independently confirm both mechanisms are meant to coexist: §6's
  `Block parameter (def f(&blk))` and §7's `Block argument (method {
  |x| ... })`/`yield` rows are KEEP, separate from §9's `->(x) { ... }`
  lambda-literal row plan 10 already implemented. This plan implements
  the `&blk`/block-argument/`yield` trio; plan 10's `Proc`/`.call`
  mechanism is untouched and fully reused underneath (see below).
- **No real `Array[T]` iteration protocol exists to motivate this with a
  built-in `.each`.** There is no user-definable-or-builtin `Array#each`,
  `#map`, or `#reduce` in this compiler (plan 09 only added index access;
  a sibling plan in this batch, `for...in`, adds loop *syntax* sugar over
  array literals, not a general block-accepting method). This plan's
  concrete proof therefore defines its own tiny block-accepting method
  (`repeat`) rather than assuming `Array[T]` gained one — a real,
  disclosed scope boundary, not an oversight. Wiring a real `Array#each`
  on top of this mechanism is separate, future stdlib work.
- **Block parameters require an explicit type annotation, same as every
  other parameter in this language** (`{ |i: Int64| puts i }`, not bare
  `{ |i| puts i }`). `spec/SEMANTICS.md` §5.3 says block parameter types
  are "inferred from the call site where the receiving method's
  signature statically pins them" (e.g. from `xs: Array[Int64]`) — but
  that inference source doesn't exist here (no method has a declared,
  checkable block-parameter *type*, only a bare `&name` marker; see
  below). Requiring an explicit annotation on the block literal itself is
  the smallest honest substitute, consistent with this compiler's
  existing "every parameter requires a type annotation" rule
  (`GRAMMAR.md` §6) and with plan 10's identical choice for lambda
  literals. Call-site type inference from a real declared block
  signature is real, deferred future work.
- **Block literals have no explicit return-type annotation and are
  always treated as `Void`-returning.** Unlike plan 10's lambda literal
  (`->(...) -> T { ... }`, explicit `T`), a block literal here is `{
  |params| body }` with no `-> T` — matching every real Ruby block
  syntax example, and matching this plan's own worked proof (a `puts`
  side effect, value never used). A block whose result *is* used (Ruby's
  `xs.reduce(0) { |acc, x| acc + x }`, `GRAMMAR.md` §9's own worked
  example) is real, disclosed future work — it needs either an explicit
  return-type annotation on the block literal or genuine call-site
  inference, neither of which this plan builds.
- **No first-class, runtime-indirect block dispatch — `yield` compiles
  via call-site specialization, not an indirect call.** This is the
  central architectural decision, and it's forced by a real constraint
  plan 10 already disclosed: "`Type::Proc(Vec<Type>, Box<Type>)` ...
  there's no separate 'lambda info' registry... the signature has to
  travel with the type value itself," and "`.call` dispatch is static,
  resolved at compile time by tracing a `Proc`-typed local back to the
  `Expr::Lambda` it was bound to... there is no first-class
  function-pointer value, no `call_indirect`." A method's `&blk`
  parameter has no such literal to trace *at the method's own
  declaration site* — it only exists once a specific call site attaches
  a specific block literal. Rather than inventing first-class Proc
  values / indirect calls (explicitly out of scope per plan 10, and a
  much larger codegen change), this plan compiles a function that
  declares `block_param` **once per call site that attaches a literal
  block**, substituting that call site's actual block body wherever
  `yield` appears in the function — the same "synthesize a function from
  a literal, call it directly" technique plan 10 already built for
  ordinary lambdas, just triggered per call site instead of per
  top-level `Let`. The real, disclosed limitation: a `yield`-using
  function can only ever be invoked with a literal block trailing the
  call, never with a `Proc` value stored in a variable and passed along
  (`&stored_blk` forwarding — `GRAMMAR.md` doesn't even list this as its
  own row; treated here as out of scope, see below) — every call site is
  effectively its own specialization, not a single reusable compiled
  function. This is a real cost, stated plainly, not hidden.
- **`yield`'s arguments are checked against the attached block's own
  declared parameters at each call site individually**, not against one
  fixed "block signature" declared once on the function — because, per
  the above, no such single fixed signature exists in this compiler's
  model. Concretely: sema conceptually re-walks the callee's body once
  per call site that attaches a block, substituting that block's param
  types wherever `yield` appears, and type-checks the callee body as if
  it were compiled fresh for that attachment (mirroring exactly what
  codegen physically does). This means two different call sites could in
  principle attach blocks of different arities/types to the *same*
  `yield`-using function and each would be checked independently — a
  real, disclosed consequence of not having first-class Proc values, not
  a bug.
- **The `&blk` marker carries no type information at all — it's a bare
  name, not a `Param { name, ty }`.** Given `Type::Proc`'s signature
  can't be pinned at the function's declaration site (see above), giving
  `&blk` a fake/unenforceable type annotation would be actively
  misleading. `Function` gains a new field, `block_param: Option<String>`,
  parsed from a trailing `"," "&" Ident` in the parameter list — not a
  `Param`, and not stored in `Function.params`.
- **Grammar: `&` is a new terminal.** Checked this session: `&` does not
  appear anywhere in the current `grammar.lalrpop`. If the sibling
  "bitwise operators" plan in this batch lands `&`/`&&` first, LALRPOP's
  own longest-match token handling (already relied on for `>`/`>=`,
  `=`/`==`, etc. in this same grammar) resolves the two without conflict;
  this plan's own build gate (`cargo build -p emerald-parser`, checking
  for a clean LALRPOP table generation) is the real proof either way,
  not an assumption.
- **Block literal syntax is `{ |params| body }` only — no `do...end`
  form.** `spec/GRAMMAR.md` §9 KEEPs both Ruby forms; `do...end` is a
  purely lexical variant of the same semantics (Ruby itself only
  distinguishes them by precedence convention, which doesn't apply
  here). Disclosed as a small, low-risk, deferred syntactic addition.
- **A trailing block attaches to `Expr::Call`/`Expr::MethodCall` by
  desugaring into an extra, implicit trailing argument reusing plan 10's
  existing `Expr::Lambda` AST node** — no new "call with a block" AST
  shape. `repeat(3) { |i: Int64| puts i }` parses to `Expr::Call("repeat",
  [Int(3), Expr::Lambda { params: [Param{i,Int64}], return_type: "Void",
  body: [...] }])`, exactly as if the call had been written
  `repeat(3, ->(i: Int64) -> Void { puts i })` by hand (which, per plan
  10's own scope cut, isn't even legal today — lambdas are top-level-`Let`-
  only as *values*; this plan's block literal is the first place an
  `Expr::Lambda` is allowed to appear as a bare call argument). This
  reuses plan 10's parsing, free-variable-capture, and function-synthesis
  machinery wholesale rather than inventing a parallel one.
- **A real LALR(1) question, checked and resolved:** does an optional
  trailing `{ ... }` after a `Call`/`MethodCall`'s closing `")"` (or a
  bare `MethodCall` with no parens) collide with the lambda literal's own
  `{` usage? Verified this session: the lambda literal's `{` is *only*
  ever reachable immediately after `"->" TypeName` in `PrimaryExpr`
  (`"->" "(" Params ")" "->" TypeName "{" ...`) — a completely disjoint
  left context from "just finished parsing a `Call`'s `)`\"" or "just
  parsed a bare `Ident \".\" Ident` method call." Nothing else in this
  grammar can start with a bare `{` at a statement/expression boundary
  (no hash literals, no bare-block statements), so there is no other
  production competing for the same lookahead. Real risk is low; the
  actual proof is still `cargo build -p emerald-parser` succeeding with
  no LALRPOP conflict, not this paragraph's reasoning alone.

## Leaf: leaf-ast-blocks

### 1. Context
- Why: no grammar or AST shape exists for a trailing block argument, a
  `&blk`-style method parameter, or `yield`.
- Current state (re-verified this session against
  `crates/emerald-parser/src/grammar.lalrpop` and `ast.rs`): `FuncDef`'s
  `ParenParams` production has no block-parameter slot; `Expr::Call`/
  `Expr::MethodCall` are always fully saturated by an ordinary `Args`
  list with no trailing suffix; `Stmt` has no `Yield` variant; `&` is not
  a terminal anywhere in the grammar.
- Target state: `Function` gains `block_param: Option<String>`, populated
  by an optional `"," "&" <name:Ident>` at the end of a parameter list
  (or a lone `"&" Ident` when it's the only parameter — mirror
  `ParenParams`'s existing empty/non-empty split). A new
  `BlockLiteral` grammar rule, `"{" "|" <params:Params> "|" <body:Stmt*>
  "}"`, producing an `Expr::Lambda { params, return_type: "Void".into(),
  body }` value. Both call-shaped productions (`<name:Ident> "(" Args
  ")"`, `<recv:Ident> "." <method:Ident> "(" Args ")"`, and the
  bare-`<recv:Ident> "." <method:Ident>` no-parens form) gain an optional
  trailing `BlockLiteral?`; when present, its `Expr::Lambda` value is
  pushed onto the call's existing `args: Vec<Expr>`. `Stmt::Yield
  (Vec<Expr>)`, grammar production `"yield" <args:Args>` (parens
  optional the same way `puts`/`raise` already are, or required — pick
  the simpler of the two and say which; recommend requiring at least a
  bare expression list with no parens, mirroring `puts`'s own `"puts"
  <arg:Expr>` shape, extended to a comma list via the existing `Args`
  rule). `yield` and `&` become reserved/new terminals (same LALR(1)
  reservation pattern as `puts`/`new`/`Array`/`Proc`/`raise`/`begin`/
  `rescue`).

### 2. Acceptance Criteria
1. `def repeat(n: Int64, &blk) -> Void ... end` parses with
   `block_param == Some("blk".to_string())` and `params == [Param{name:
   "n", ty: "Int64"}]` (i.e. `&blk` is not present in `params`).
2. `repeat(3) { |i: Int64| puts i }` parses to `Expr::Call("repeat",
   [Int(3), Expr::Lambda { params: [Param{name:"i", ty:"Int64"}],
   return_type: "Void", body: [Stmt::Expr(Call("puts",[Ident("i")]))] }
   ])` — the block genuinely becomes an extra trailing argument, not a
   separate field.
3. `yield i` inside a function body parses to `Stmt::Yield(vec![Ident
   ("i")])`.
4. A call with no trailing block still parses exactly as it did before
   this plan (regression: every prior plan's example, all of which use
   zero-block calls, parses identically).
5. `yield`/`&` become reserved; no LALRPOP build-time conflicts.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new block/yield parse assertions | agent-claimed-locally |

---

## Leaf: leaf-sema-blocks

### 1. Context
- Why: no type-checking exists for `block_param`, a trailing block
  argument, or `yield`; nothing currently rejects `yield` outside a
  block-accepting function, or a block/`yield` arity mismatch.
- Target state: `check_function_body` (or its caller) rejects `Stmt::
  Yield` appearing in a function whose `block_param.is_none()` — a
  straightforward, real diagnostic ("`yield` used in `repeat`, which
  declares no block parameter" — actual function name included, matching
  this project's existing diagnostic-message standard). For a `Call`/
  `MethodCall` whose last argument is an `Expr::Lambda` matching the
  callee's `block_param` slot: type-check that lambda's body exactly as
  plan 10 already type-checks any `Expr::Lambda` (its own params in
  scope, `Void`-declared body, no return-value constraint since block
  literals are always `Void` per the Decision log) — **and then
  re-type-check the callee's own body a second time, substituting this
  specific block literal's parameter types at every `Stmt::Yield` site**,
  exactly as described in the Decision log's "no single fixed block
  signature" call. A call to a `block_param`-declaring function with **no**
  trailing block is rejected with a diagnostic (calling a `yield`-using
  function unconditionally requires a block, since there is no
  first-class `Proc` value a caller could otherwise supply — see Decision
  log).

### 2. Acceptance Criteria
1. This plan's full `repeat`/`yield` example type-checks `Ok(())`.
2. `repeat(3)` (no trailing block) is rejected with a diagnostic naming
   `repeat` and stating it requires a block.
3. `repeat(3) { |i: Int64, extra: Int64| puts i }` (block arity doesn't
   match `yield`'s single-argument call sites inside `repeat`) is
   rejected with a diagnostic — real proof the call-site substitution
   check in the Decision log actually runs, not just that some check
   exists.
4. `yield 5` inside a function with no `block_param` (e.g. plain
   `add(a, b)` from earlier plans) is rejected with a diagnostic, not a
   panic.
5. Regression: every prior plan's example, all `block_param.is_none()`
   and free of `Stmt::Yield`, still type-checks identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-blocks

### 1. Context
- Why: nothing compiles `block_param`, a trailing block literal, or
  `yield` to machine code. `crates/emerald-codegen/src/lib.rs` is, as of
  this session, the last documented codegen implementation (a
  ~2200-line Cranelift-based file with `LambdaInfo`, `free_vars_in_
  lambda`, `define_lambda`, `build_lambda_let`, etc. from plan 10) — it
  is currently **mid-migration by unrelated, concurrent work** (a
  different agent implementing plan 16's Cranelift-vs-LLVM bake-off is
  actively restructuring this crate; its on-disk state is transient and
  not read fresh for this plan). This leaf's design is written against
  the architecture plan 10 documented, whichever backend crate ends up
  hosting it once plan 16 settles.
- Target state: when `compile_to_object` (or its plan-16-successor
  entry point) encounters a `Call`/`MethodCall` whose callee is a
  function/method declaring `block_param` and whose last argument is an
  `Expr::Lambda` (this plan's desugared block), it does **not** call
  the callee's ordinary, once-compiled function body. Instead it
  synthesizes a specialized copy of the callee for this call site,
  exactly the way plan 10's pre-pass already synthesizes a top-level
  function (`__lambda_{name}`) from a `Let`-bound `Expr::Lambda`: the
  attached block becomes its own synthesized function
  (`__block_{callsite_id}`, reusing plan 10's free-variable-capture +
  `emerald_alloc` env mechanism verbatim, since a block closes over its
  surrounding scope exactly like a lambda does), and every `Stmt::Yield
  (args)` inside the specialized callee body becomes a direct call to
  that synthesized block function — the same "trace to a literal, call
  it directly" static-dispatch technique plan 10 already uses for
  `.call`, applied to `yield` instead.

### 2. Acceptance Criteria
1. This plan's full example, compiled, linked, and run, prints
   `0\n1\n2\n` — real executed proof that call-site specialization,
   block-closure capture (the block reads no outer local in this
   specific proof, but the mechanism is the same capture path plan 10
   already built and exercises the identical code path), and `yield`'s
   direct-call lowering all work together.
2. A second worked example where the block *does* capture an outer
   local (e.g. an accumulator `puts`ed from inside the block) proves
   capture genuinely flows through this new call-site-specialization
   path, not just plan 10's original top-level-`Let` path.
3. An unsupported shape (a `block_param`-declaring function called
   without a trailing block reaching codegen at all — should already be
   rejected by sema per leaf-sema-blocks) defensively returns a
   descriptive `Err` if it ever reaches codegen regardless, not a panic
   — same AC standard as every prior codegen plan.

### 3. File & Module Structure
- **Modify:** whichever crate hosts `compile_to_object` once plan 16's
  concurrent bake-off work settles (`crates/emerald-codegen/src/lib.rs`
  as last documented, or its successor).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` (or successor crate) | all pass, incl. real linked-and-run `0\n1\n2\n` and the capture-proof example | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `do...end` block syntax — see Decision log; `{ }` only.
- Value-returning blocks (Ruby's `reduce`/`map`-with-a-value pattern,
  `GRAMMAR.md` §9's own `xs.reduce(0) { |acc, x| acc + x }` example) —
  needs either an explicit return-type annotation on block literals or
  real call-site type inference; this plan's blocks are always `Void`.
- Storing a `&blk` parameter as a first-class `Proc` value and forwarding
  it to another call (`&blk` re-passed onward) — needs the indirect-call/
  function-pointer machinery plan 10 explicitly deferred; this plan's
  `yield` is call-site-specialized, not indirect.
- `block_given?`.
- Any real `Array[T]`/stdlib method (`each`, `map`, `reduce`) actually
  using this mechanism — this plan proves the mechanism on a
  user-defined method only; wiring it onto `Array[T]` is separate,
  future stdlib work (not redone from plan 25).
- Multiple blocks per call, block-local variables (`{ |x; y| ... }`,
  `GRAMMAR.md` §9), keyword or default parameters on a block literal.
- Call-site type inference for block parameters (`SEMANTICS.md` §5.3's
  full "inferred from the call site" story) — this plan requires
  explicit annotations instead; see Decision log.
