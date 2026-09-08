---
name: Blocks & Closures
overview: Lambda literals that close over outer locals and are invoked via `.call(args)` — inception §19's "are blocks closures?" question, proven with a real capture/indirect-call/print program.
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-lambda
    content: "Expr::Lambda { params, return_type, body }; method calls gain an argument list (MethodCall's Vec<Expr> was already there, the grammar never populated it)"
    status: pending
  - id: leaf-sema-lambda
    content: "Type::Proc(Vec<Type>, Box<Type>) carrying its own signature; lambda-body checking; `.call` argument/return checking"
    status: pending
  - id: leaf-codegen-lambda
    content: "Free-variable capture analysis, heap-allocated env buffer (emerald_alloc), a synthesized top-level function per lambda, static `.call` dispatch"
    status: pending
isProject: false
---

# Plan 10 — Blocks & Closures

This is `blocks-closures`, row `10` of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §19's "Blocks" open questions as resolved in
`spec/SEMANTICS.md` §5: blocks are closures, capturing by reference, able
to escape their defining scope, with call-site-inferred block parameter
types and heap allocation only when necessary. Inception/GRAMMAR.md give
no single literal worked example for this feature (unlike the
`add`/`if`/`Point` milestones); this plan's concrete proof, self-designed
per the established practice from plans 07–09, is:

```ruby
x: Int64 = 10
add_x: Proc = ->(y: Int64) -> Int64 { y + x }
puts add_x.call(5)
```

Expected output: `15`.

## Decision log

This plan proves the core claim — a lambda literal captures an outer
local and is callable through an indirect boundary, printing a value that
depends on both the capture and the call argument — with several real,
disclosed scope cuts from `SEMANTICS.md` §5's full spec:

- **Lambdas are only supported as a top-level `Stmt::Let`'s value**
  (`name: Proc = ->(...) -> T { ... }`) — not inside a function/method
  body, not as a call argument, array element, or return value, and never
  reassigned. This keeps every capture-environment's field layout keyed
  by a single, already-unique top-level local name (mirroring
  `define_main`'s existing "top-level statements share one flat
  environment" design) rather than needing a global lambda-numbering
  scheme or AST-node identity tracking. The free-variable/env-capture
  design underneath doesn't change if this scope cut is lifted later —
  it's an unexercised-path restriction, not an architectural limit.
- **Captures are by value (a snapshot at closure-creation time), not by
  reference.** `SEMANTICS.md` §5.1 says "by reference, matching Ruby's
  closure semantics exactly" — this is a real, disclosed deviation. True
  by-reference capture requires cell-converting every capturable local
  from an SSA `Variable` to heap/stack memory (the standard
  closure-compilation technique), a broader codegen model change than
  this plan's proof needs — nothing in the worked example observes a
  post-capture mutation of `x`.
- **Closures always heap-allocate their capture environment** via the
  existing `emerald_alloc` (plan 08). `SEMANTICS.md` §5.4's
  "heap-allocate only when necessary" escape-analysis optimization is
  deferred — legality first, optimization later, the same ordering plan
  09 used for bounds checking.
- **`.call` dispatch is static**, resolved at compile time by tracing a
  `Proc`-typed local back to the `Expr::Lambda` it was bound to — the
  same lexical-tracing technique plan 08 established for `.method()`
  calls (`local_classes` there; `local_lambda_func_ids` here). There is
  no first-class function-pointer value, no `call_indirect`, no
  reassignment-changes-which-lambda-runs behavior. A real dynamic closure
  representation (fn-ptr + env, indirect call) is deferred until a
  concrete program needs first-class lambda values passed around.
- **`Type::Proc(Vec<Type>, Box<Type>)` carries its own full signature**,
  unlike `Type::Class(String)`'s bare name — there's no separate
  "lambda info" registry to consult the way `ClassInfo` backs
  `Type::Class`, so the signature has to travel with the type value
  itself. The *source-level* annotation stays the bare reserved keyword
  `Proc` (no `Proc[(Int64) -> Int64]` compound-annotation grammar,
  matching plan 09's "string concatenation, not a structured type AST
  node" choice for `Array[Elem]`); `check_stmt`'s `Let` case special-cases
  a bare `"Proc"` declared-type string to accept any `Type::Proc(_, _)`
  actual value, deferring the real signature check to `.call`'s own
  argument list.
- **Method calls gain an argument list at the grammar level**
  (`<recv:Ident> "." <method:Ident> "(" <args:Args> ")"`) — a real,
  independently useful gap fix uncovered by this plan: `Expr::MethodCall`
  already carries a `Vec<Expr>` for arguments (plan 08's AST), but the
  grammar has never populated it with anything but `vec![]` — even
  `Point`'s own methods could never be called with arguments before this
  plan. `.call(args)` needs this regardless of Proc, so it's fixed here
  rather than worked around.
- **No block-argument-to-method syntax** (`xs.each { |x| ... }`,
  `yield`, `def f(&blk)`). `SEMANTICS.md` §5.3's call-site block-parameter
  inference presupposes iteration-protocol methods (`each`, `map`,
  `reduce`) that don't exist on `Array[T]` yet (plan 09 only added index
  access) — that's its own scope of work, not required to prove blocks
  are closures.

## Leaf: leaf-ast-lambda

### 1. Context
- Why: no AST shape exists for a lambda literal; `Expr::MethodCall`'s
  argument list has never been populated by the grammar.
- Current state: `CallExpr`'s `<recv:Ident> "." <method:Ident>` grammar
  alternative always builds `Expr::MethodCall(recv, method, vec![])`
  (verified this session, plan 08's Point example never calls a method
  with arguments).
- Target state: `Expr::Lambda { params: Vec<Param>, return_type: String,
  body: Vec<Stmt> }`; a `<recv:Ident> "." <method:Ident> "(" Args ")"`
  grammar alternative populating `Expr::MethodCall`'s existing `Vec<Expr>`
  for real; `"->" "(" Params ")" "->" TypeName "{" Stmt* "}"` grammar
  production for the lambda literal; `Proc` reserved as a bare TypeName
  keyword (same LALR(1) reason `Array`/`puts`/`new` were reserved).
- Dependencies: none.
- Maestro: intended slug `leaf-ast-lambda`, wave 0.

### 2. Acceptance Criteria
1. `Expr::Lambda` exists as described; `Expr::MethodCall`'s arg list is
   populated by real parsed arguments, not always `vec![]`.
2. `->(y: Int64) -> Int64 { y + x }` parses to `Expr::Lambda { params:
   [Param { name: "y", ty: "Int64" }], return_type: "Int64", body:
   [Stmt::Expr(Expr::Add(Ident("y"), Ident("x")))] }`.
3. `add_x.call(5)` parses to `Expr::MethodCall(Ident("add_x"), "call",
   [Int(5)])`.
4. `Proc` becomes a reserved keyword; no LALRPOP build-time conflicts.
5. Regression: every prior plan's example still parses identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |

### 5. Risks & Rollback
- The method-call-with-args alternative and the existing no-args
  alternative both start `Ident "." Ident` before diverging on whether
  `"("` follows — same "decide on the next token" shape plans 07–09 have
  used repeatedly (`Let` vs bare `Expr`, `SetIndex` vs `Index`), so this
  is expected to be conflict-free; if LALRPOP disagrees, fold the two
  alternatives into one with an optional `"(" Args ")"` suffix instead.

---

## Leaf: leaf-sema-lambda

### 1. Context
- Why: no `Type` variant represents a closure; `infer_expr_type`'s
  `MethodCall` arm rejects any non-`Type::Class` receiver outright.
- Current state: `Type` has 7 variants (through `Array`, plan 09); no
  method call has ever had arguments to check.
- Target state: `Type::Proc(Vec<Type>, Box<Type>)`; `infer_expr_type`
  handles `Expr::Lambda` (type-check the body in the outer env plus the
  lambda's own params, matching a function body's checking); the
  `MethodCall` arm branches on `Type::Proc` + `method == "call"` to
  `check_args` against the carried param types and return the carried
  return type, alongside the existing `Type::Class` path; `check_stmt`'s
  `Let` case special-cases a bare `"Proc"` declared type to accept any
  `Type::Proc(_, _)` actual value (see Decision log).

### 2. Acceptance Criteria
1. This plan's full example type-checks `Ok(())`.
2. `add_x.call(5, 6)` (wrong arity) is rejected with a diagnostic naming
   the expected/found argument counts, same standard as every prior
   arity-mismatch diagnostic.
3. `add_x.call("s")` (wrong argument type) is rejected with a diagnostic
   naming both types.
4. Calling `.call` on a non-`Proc` receiver is rejected with a
   diagnostic, not a panic.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

### 5. Risks & Rollback
- None — additive checking logic, mirrors the existing `MethodCall`/
  `check_args` shape already used for classes.

---

## Leaf: leaf-codegen-lambda

### 1. Context
- Why: nothing compiles a lambda literal, a capture, or a `.call` to
  machine code yet.
- Current state: `emerald-codegen` has no closure support.
- Target state: a pre-pass over `program.items` finds every top-level
  `Stmt::Let` whose value is `Expr::Lambda`, computes its free-variable
  capture list (a dedicated AST walk over the lambda body, collecting
  `Expr::Ident`s not shadowed by the lambda's own params or its own
  internal `Let`s), and compiles each one as an ordinary top-level
  function (`__lambda_{name}`) taking an implicit leading `env: i64`
  parameter ahead of the lambda's own declared params — inside that
  function, a captured name resolves to `load(env_ptr, its_offset)`
  instead of a `Variable`, mirroring exactly how `@field` reads already
  resolve through `self_ctx`. `Stmt::Let`'s own codegen, when the value is
  `Expr::Lambda`, allocates the env buffer via `emerald_alloc` and stores
  each captured local's current value into its slot. `.call(args)`
  resolves the callee statically via `local_lambda_func_ids` (built the
  same way `local_classes` is) and calls it with `(env_ptr, args...)`.

### 2. Acceptance Criteria
1. This plan's full example, compiled, linked, and run, prints `15\n` —
   real executed proof that capture, environment allocation, and the
   `.call` invocation all work together.
2. Environment allocation reuses `emerald_alloc` (plan 08) rather than a
   second allocator path.
3. An unsupported shape (a lambda anywhere but a top-level `Let`'s value,
   or `.call` on a receiver `local_lambda_func_ids` can't trace) defensively
   returns a descriptive `Err`, not a panic — same AC standard as every
   prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Diagrams
```mermaid
sequenceDiagram
    participant Main as main()
    participant Alloc as emerald_alloc
    participant Lam as __lambda_add_x(env, y)
    Main->>Alloc: emerald_alloc(8)  # captures: [x]
    Alloc-->>Main: env ptr
    Main->>Main: store x (10) at [env+0]
    Main->>Lam: call(env, 5)
    Lam->>Lam: load x from [env+0]; return y + x
    Lam-->>Main: 15
    Main->>Main: puts 15
```

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `15\n` | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

### 6. Risks & Rollback
- None beyond the established scope cuts already disclosed in the
  Decision log (by-value capture, static `.call` dispatch, top-level-only
  lambdas).

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- By-reference capture (cell-converted locals) — see Decision log.
- Escape analysis / stack-allocated closures — see Decision log.
- First-class lambda values (passed as arguments, stored in fields/
  arrays, reassigned, dynamically dispatched via `call_indirect`) — see
  Decision log.
- Block-argument-to-method syntax, `yield`, `def f(&blk)` — needs
  iteration-protocol methods on `Array[T]` first; not this plan's job.
- Lambdas defined inside a function/method body — see Decision log.
