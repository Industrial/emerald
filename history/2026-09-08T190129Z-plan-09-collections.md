2026-09-08T19:01:29Z

Snapshot of `.cursor/plans/collections.plan.md` — plan `09 collections`
from [`plan-of-plans`](./2026-09-08T174011Z-plan-of-plans.md) — captured
before execution began.

---
name: Collections
overview: Array[T] literals, indexing (read and write) — inception §10's unboxed-representation requirement, proven with a real allocate/index/mutate/sum program.
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-array
    content: Expr::{ArrayLit, Index}; Stmt::SetIndex; a compound TypeName grammar rule for Array[Elem] annotations
    status: pending
  - id: leaf-sema-array
    content: Type::Array(Box<Type>); array-literal unification, index/element type-checking
    status: pending
  - id: leaf-codegen-array
    content: Contiguous-buffer codegen via emerald_alloc, address arithmetic for indexed load/store
    status: pending
isProject: false
---

# Plan 09 — Collections

This is `collections`, row `09` of
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
implementing inception §10/§11's `Array[T]` requirement and
`spec/TYPE_SYSTEM.md` §8's "unboxed/packed contiguous storage" mandate.
Inception gives no literal worked example for collections (unlike the
`add`/`if`/`Point` milestones); this plan's concrete proof is:
```ruby
arr: Array[Int64] = [10, 20, 30]
sum: Int64 = 0
i: Int64 = 0
while i < 3
  sum: Int64 = sum + arr[i]
  i: Int64 = i + 1
end
arr[1] = 99
puts sum
puts arr[1]
```

## Decision log

- **No runtime length, no bounds checking.** `Array[T]` is a raw pointer to
  a contiguous buffer, exactly as `spec/TYPE_SYSTEM.md` §8 mandates for
  value-type elements — nothing more. There is no length header, so
  out-of-bounds indexing is undefined behavior (a wild read/write), same
  risk class as C arrays. This is a real, disclosed scope cut, not an
  oversight — a length-tracking/bounds-checked representation is a
  reasonable future increment once a concrete program needs it (e.g. a
  `.length` method or runtime-sized arrays), not required to prove
  inception §10's core representation claim.
- **`Hash[K, V]` is deferred entirely** — `spec/TYPE_SYSTEM.md` §8 itself
  notes Hash has no representation-strictness requirement as sharp as
  `Array[T]`'s (hashing/bucket layout is explicitly a `RUNTIME.md`
  concern). Proving one collection's unboxed representation claim is this
  plan's job; `Hash` gets its own attention when a concrete program needs
  key/value lookup.
- **Empty array literals (`[]`) are rejected**, not supported. Element
  type comes from unifying the literal's elements; an empty literal has
  none to unify, and inferring from a separate declared-type annotation
  (bidirectional inference) is more machinery than this plan's concrete
  example needs.
- **Compound type annotations (`Array[Int64]`) are grammar-level string
  concatenation, not a structured type AST node yet.** `Param.ty` stays
  `String` (`"Array[Int64]"` is one such string); `emerald-sema`'s
  `resolve_type` parses the `Array[...]` pattern itself. A real `Type` AST
  node (replacing bare `String` type annotations everywhere) is deferred
  until a second compound-type shape (e.g. `Hash[K, V]`, `T?`) makes the
  string-parsing approach genuinely awkward rather than merely inelegant.
- **Indexing a non-`Ident` array expression is unsupported in codegen**,
  same pattern as plan 08's `MethodCall` receiver restriction — codegen
  has no typed IR, so an indexed array's element Cranelift type is looked
  up via a `local_array_elem_types: HashMap<String, Type>` built from
  `Stmt::Let`'s own type annotation, which only exists for plain local
  variables.

## Leaf: leaf-ast-array

### 1. Context
- Why: no AST shape exists for an array literal, an index read, an index
  write, or a compound type annotation like `Array[Int64]`.
- Current state: `Param.ty`/`Function.return_type` are bare `Ident`
  strings; the grammar has no `[...]` literal or indexing syntax
  (verified this session).
- Target state: `Expr::ArrayLit(Vec<Expr>)`, `Expr::Index(Box<Expr>,
  Box<Expr>)`; `Stmt::SetIndex { array: Expr, index: Expr, value: Expr }`;
  a `TypeName` grammar rule producing `"Array[Elem]"`-shaped strings,
  used everywhere a type annotation currently accepts a bare `Ident`.
- Dependencies: none.
- Maestro: intended slug `leaf-ast-array`, wave 0.

### 2. Acceptance Criteria
1. `Expr::ArrayLit`/`Expr::Index`/`Stmt::SetIndex` exist as described.
2. `arr: Array[Int64] = [10, 20, 30]` parses: `Stmt::Let` with `ty ==
   "Array[Int64]"` and `value == Expr::ArrayLit([Int(10), Int(20),
   Int(30)])`.
3. `arr[i]` parses as `Expr::Index(Box::new(Expr::Ident("arr")),
   Box::new(Expr::Ident("i")))`; `arr[1] = 99` parses as `Stmt::SetIndex`.
4. `Array` becomes a reserved keyword (same LALR(1) reason `puts`/`new`
   were reserved in plans 07/08) — no LALRPOP build-time conflicts.
5. Regression: every prior plan's example (`hello.em`, milestone-2,
   `Point`) still parses identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Diagrams
```mermaid
flowchart TB
    Expr -->|ArrayLit| AL["Vec&lt;Expr&gt;"]
    Expr -->|Index| IX["Box&lt;Expr&gt; (array), Box&lt;Expr&gt; (index)"]
    Stmt -->|SetIndex| SI["array: Expr, index: Expr, value: Expr"]
    TypeName -->|bare| Ident
    TypeName -->|compound| ArrayBracket["\"Array\" \"[\" Ident \"]\" -> \"Array[Elem]\""]
```

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |

### 6. Implementation Notes
- Array literal grammar reuses the existing `Args` comma-list rule (same
  one function-call arguments already use) inside `"[" Args "]"`.

### 7. Risks & Rollback
- None beyond the established narrow-grammar-scope risk plans `04`/`07`/`08`
  already accepted.

---

## Leaf: leaf-sema-array

### 1. Context
- Why: `resolve_type`/`infer_expr_type` have no concept of an array type,
  element unification, or index-expression type-checking.
- Current state: `Type` has no collection variant (verified in plan 08).
- Target state: `Type::Array(Box<Type>)`; `resolve_type` parses the
  `Array[Elem]` string pattern; `infer_expr_type` handles `ArrayLit`
  (unify all elements, reject empty) and `Index` (array must be
  `Type::Array(_)`, index must be `Int64`, result is the element type);
  `check_stmt` handles `SetIndex` the same way plus a value-type check
  against the element type.

### 2. Acceptance Criteria
1. This plan's full example (allocate, sum via loop+index, mutate via
   `arr[1] = 99`, read back) type-checks `Ok(())`.
2. `[1, "two", 3]` (mixed element types) is rejected with a diagnostic
   naming the mismatching types, same standard as every prior
   type-mismatch diagnostic this project produces.
3. `[]` is rejected with a diagnostic, not treated as some default/unit
   type — matches the Decision log.
4. Indexing a non-array value (e.g. `x: Int64 = 5; puts x[0]`) is
   rejected with a diagnostic, not a panic.
5. Indexing with a non-`Int64` index is rejected with a diagnostic.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Diagrams
Not applicable — extends the existing `infer_expr_type`/`check_stmt`
dispatch already diagrammed in plans 05/07/08.

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

### 6. Implementation Notes
- `resolve_type`'s `Array[Elem]` parsing is a simple `strip_prefix`/
  `strip_suffix` pair (no nested-bracket recursion needed yet — `Elem`
  itself is always a bare type name in this plan's scope, not another
  `Array[...]`).

### 7. Risks & Rollback
- None — additive checking logic.

---

## Leaf: leaf-codegen-array

### 1. Context
- Why: nothing compiles an array literal, an indexed read, or an indexed
  write to machine code yet.
- Current state: `emerald-codegen` has no array support (verified in
  plan 08).
- Target state: `Expr::ArrayLit` allocates `elements.len() * 8` bytes via
  `emerald_alloc` and stores each element sequentially; `Expr::Index`/
  `Stmt::SetIndex` compute `array_ptr + index * 8` (via `imul`/`iadd`) and
  `load`/`store` at that address, using the element's Cranelift type
  looked up via `local_array_elem_types` (see Decision log).

### 2. Acceptance Criteria
1. This plan's full example, compiled, linked, and run, prints `60\n99\n`
   — real executed proof that allocation, indexed read (in a loop
   condition-driven sum), and indexed write all work together.
2. Indexing/allocation reuses `emerald_alloc` (plan 08) rather than a
   second allocator path.
3. An unsupported shape (indexing a non-`Ident` array expression)
   defensively returns a descriptive `Err`, not a panic — same AC4
   standard as every prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Diagrams
```mermaid
sequenceDiagram
    participant Main as main()
    participant Alloc as emerald_alloc
    Main->>Alloc: emerald_alloc(24)  # 3 x 8 bytes
    Alloc-->>Main: arr ptr
    Main->>Main: store 10 at [arr+0], 20 at [arr+8], 30 at [arr+16]
    loop i = 0..3
        Main->>Main: load [arr + i*8]; sum += value
    end
    Main->>Main: store 99 at [arr + 1*8]
    Main->>Main: puts sum (60), puts arr[1] (99)
```

### 5. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `60\n99\n` | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

### 6. Implementation Notes
- Reuses `Ctx`'s existing `alloc_func_id` (plan 08) — no new runtime
  function needed for this plan.

### 7. Risks & Rollback
- None beyond the established no-bounds-checking risk already disclosed
  in the Decision log.

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `Hash[K, V]` — see Decision log.
- Bounds checking / runtime length tracking — see Decision log.
- `for x in arr` iteration syntax (plan `07` deferred this pending
  `Array[T]`'s existence; still deferred here — a `while`-based loop
  proves the same representation claim without adding a new statement
  kind this plan doesn't need).
- Indexing arbitrary sub-expressions (only a plain local-variable
  receiver is supported) — see Decision log.
