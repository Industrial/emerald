---
name: For-In Iteration
overview: "`for <var> in [literal array] ... end`, desugared to the existing index-based `while` loop machinery — closing the gap plans 07 and 09 each independently deferred."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-for-in
    content: "Stmt::For { var: String, elements: Vec<Expr>, body: Vec<Stmt> } — grammar production restricted to a literal array-lit scrutinee only"
    status: pending
  - id: leaf-sema-for-in
    content: "Type-check the literal elements (reuse ArrayLit unification), bind `var` at the unified element type for `body`"
    status: pending
  - id: leaf-codegen-for-in
    content: "Desugar to the same index-based while-loop shape leaf-codegen-array (plan 09) already proved, reusing LoopTargets for break/next"
    status: pending
isProject: false
---

# Plan 30 — For-In Iteration

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
post-v1 scope, same posture as plan 17. This is one of a numbered
follow-up batch (28-35); it owns `for...in` only.

This is the direct, named follow-up to a gap two earlier plans each
independently found and deliberately re-deferred, in their own words.
Plan 07's control-flow Decision log deferred `for x in arr` "pending
`Array[T]`'s existence." Plan 09's collections Decision log then shipped
`Array[T]` and re-deferred the same syntax again: "`for x in arr`
iteration syntax (plan `07` deferred this pending `Array[T]`'s
existence; still deferred here — a `while`-based loop proves the same
representation claim without adding a new statement kind this plan
doesn't need)." `Array[T]` now genuinely exists — verified this session
via `crates/emerald-parser/src/ast.rs`'s `Expr::ArrayLit`/`Expr::Index`
and `crates/emerald-sema/src/lib.rs`'s `Type::Array(Box<Type>)` — so this
plan is the one that finally closes the gap two plans left open.

Note on the current build: `crates/emerald-codegen/` is presently in the
middle of unrelated, concurrent, uncommitted work by a different agent
implementing plan 16 (Cranelift vs. LLVM bake-off) — its `src/lib.rs` is
transiently deleted in the working tree as of this writing. Everything
below cites the last documented codegen implementation this session
actually read in full (a Cranelift-based `crates/emerald-codegen/src/
lib.rs` with `build_expr`/`build_stmt`/`Ctx`/`ARRAY_ELEM_SIZE`/
`build_index`/`build_array_lit`/`LoopTargets`) as the target this plan's
codegen leaf integrates with, not whatever transient state is on disk
right this moment.

## Decision log

- **No `Type::Range`, no iterator protocol — `for i in 1..10` is out of
  scope.** `spec/GRAMMAR.md` §1 marks Range literals (`1..10`, `1...10`)
  KEEP, but `emerald_sema::Type` has no `Range` variant (verified this
  session — the enum is `Int64, Float64, String, Boolean, Void,
  Class(String), Array(Box<Type>), Proc(...)`, nothing else). Only
  `for <var> in <array-literal> ... end` is in scope.
- **The single most important scope cut in this plan: only a literal
  array (`for x in [10, 20, 30]`) can be iterated — not an arbitrary
  `Array[T]`-typed variable.** `Array[T]`'s own representation, per
  plan 09's Decision log, is "a raw pointer to a contiguous buffer...
  no runtime length" — there is no length to iterate "until the end
  of" for a value received as, say, a function parameter or read back
  out of a `Let` binding. Plan 25 (stdlib expansion, already written)
  adds `Array.new(size)` but explicitly does **not** add length
  tracking either ("No length tracking, no bounds checking, no
  growable `.push`... a real growable/bounds-checked representation is
  a reasonable future increment," per its own Decision log) — so this
  gap is not closed by anything currently planned. A literal's element
  count, by contrast, is known at compile time directly from the
  `Expr::ArrayLit`'s own `Vec<Expr>` length, with zero new runtime
  machinery. This plan proves the *syntax and desugaring*, exactly the
  same "prove representation, don't add new statement kinds" ethos
  plan 09 itself used for `while`-based array traversal — it does not
  and cannot yet prove general runtime-sized iteration; that is real,
  disclosed, separate future work gated on an `Array[T]` representation
  change (length tracking), not on anything this plan controls.
- **Desugaring target: `for x in [e1, e2, ...] body end` lowers, at
  parse/sema time, to the exact index-based `while` shape plan 09's own
  `leaf-codegen-array` already proved** (`i: Int64 = 0; while i < N ...
  x bound to arr[i] ... i = i + 1 end`, `N` being the literal's known
  length) — reusing `Expr::Index`/`Stmt::While` codegen entirely. No new
  codegen surface is needed if the AST node is a genuine desugaring
  rather than a first-class new statement kind — decided here as a
  dedicated `Stmt::For` AST node that codegen lowers immediately into
  the same shape `Stmt::While` already handles, rather than a sema-time
  or parser-time source-to-source rewrite, so that a `for`-loop still
  reads as `for` in the AST for any future consumer (e.g. plan 21's LSP
  work) without needing to reverse-engineer a `while` back into a `for`
  for tooling purposes.
- **`break`/`next` inside a `for` loop reuse the existing loop-stack
  mechanism** (`LoopTargets`, already used by `while`/codegen — verified
  via this session's earlier read of `crates/emerald-codegen/src/
  lib.rs`'s signatures) rather than inventing a second loop-control
  mechanism specific to `for`.
- **The loop variable `var` is bound into the same flat `env`/`vars`
  namespace everything else uses** — consistent with this compiler's
  existing "no block scoping" decision (plan 07), the same way a
  `while`/`if` body's `Let`s already flow into the surrounding scope.
  Not a new limitation specific to `for`.
- **Empty array literals are rejected**, inheriting `ArrayLit`'s own
  existing rule (plan 09: "Empty array literals (`[]`) are rejected, not
  supported") — `for x in [] ... end` is therefore also rejected at
  sema time with the same diagnostic, not silently accepted as a
  zero-iteration loop.

## Leaf: leaf-ast-for-in

### 1. Context
- Why: no AST shape exists for `for...in`; `grammar.lalrpop` has zero
  `for`/`in` productions today (verified this session).
- Target state: `Stmt::For { var: String, elements: Vec<Expr>, body:
  Vec<Stmt> }`; grammar production `"for" Ident "in" "[" Args "]"
  Stmt* "end"` — deliberately mirroring `PrimaryExpr`'s existing
  `"[" Args "]"` array-literal shape rather than accepting an arbitrary
  `Expr` scrutinee, so a non-literal `for x in arr` is a parse error,
  not a sema error, making the scope cut visible as early as possible.
  `for`/`in` become reserved keywords (same LALR(1) reason `puts`/
  `new`/`Array`/`raise`/etc. were reserved in plans 07/08/09/11).

### 2. Acceptance Criteria
1. `Stmt::For` exists as described.
2. `for x in [10, 20, 30] puts x end` parses to `Stmt::For { var: "x",
   elements: [Int(10), Int(20), Int(30)], body: [Expr(Call("puts",
   [Ident("x")]))] }`.
3. `for x in arr end` (a bare identifier, not a literal) is rejected as
   a parse error, not accepted and deferred to sema — proving the
   literal-only restriction is enforced at the grammar level.
4. Regression: every prior plan's example still parses identically.

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

## Leaf: leaf-sema-for-in

### 1. Context
- Why: no type-checking exists for `Stmt::For`.
- Target state: `check_stmt`'s new `Stmt::For` arm unifies `elements`'
  types exactly as `infer_array_lit_type` already does for a bare
  `Expr::ArrayLit` (reject empty, reject heterogeneous), binds `var` at
  the unified element type in `env`, then type-checks `body`.

### 2. Acceptance Criteria
1. `for x in [1, 2, 3] y: Int64 = x + 1 end` type-checks `Ok(())`, with
   `x` genuinely usable at `Int64` inside `body` (not just accepted).
2. `for x in [] ... end` is rejected with the same empty-array-literal
   diagnostic `Expr::ArrayLit` already produces.
3. `for x in [1, "two"] ... end` (heterogeneous) is rejected with a
   diagnostic naming the mismatching types.
4. `break`/`next` inside `body` are accepted exactly as they already are
   inside a `while` body (reuses the existing `in_loop` flag threaded
   through `check_stmt`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-for-in

### 1. Context
- Why: nothing compiles `Stmt::For` to machine code yet.
- Target state: `build_stmt`'s new `Stmt::For` arm desugars, at codegen
  time, into the exact index-based `while` shape described in the
  Decision log: declare a fresh hidden index variable, an array-literal
  build identical to `build_array_lit`'s existing handling, a loop
  condition `index < elements.len()`, a per-iteration `Expr::Index`
  load bound to `var`, the loop `body`, then an index increment — using
  the same `LoopTargets` push/pop `while` already does, so `break`/
  `next` work with zero new control-flow logic.

### 2. Acceptance Criteria
1. `for x in [10, 20, 30] sum: Int64 = sum + x end`-shaped real program
   (summing into a pre-declared accumulator, then `puts`), compiled,
   linked, and run, prints the exact expected sum (`60`) — real executed
   proof, not just "compiles cleanly."
2. A real program using `break` inside a `for` loop (e.g. stopping after
   the second element) and one using `next` (skipping one element)
   each print the expected, distinct stdout — proving loop control
   genuinely reuses `LoopTargets` correctly, not just that the happy
   path works.
3. An unsupported shape (a `for` loop that somehow reaches codegen with
   a non-literal scrutinee — should be structurally unreachable given
   leaf-ast-for-in's grammar restriction, but verified defensively)
   returns a descriptive `Err`, not a panic — same AC standard as every
   prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run sum/break/next proofs | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `for i in 1..10` (Range iteration) — no `Type::Range`/iterator
  protocol exists; see Decision log.
- General runtime-length-aware iteration over a non-literal `Array[T]`
  value (a variable, a function parameter, an `Array.new(size)` result)
  — the core scope cut of this plan; blocked on an `Array[T]`
  representation change (runtime length tracking) that neither this
  plan nor plan 25 delivers. See Decision log.
- `for (k, v) in hash` — `Hash[K, V]` iteration is separate future work
  beyond plan 25's get/set-only scope.
- Nested destructuring in the loop variable (`for (a, b) in pairs`).
- `for` as an expression (returning a value like `if`/`while` might one
  day) — this compiler's `if` itself has no `Expr::If` form yet either
  (only `Stmt::If`), so there is no existing pattern to mirror.
