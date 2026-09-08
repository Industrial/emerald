---
name: Standard Library Expansion
overview: "Four independent, already-disclosed gaps closed: true/false boolean literals, a narrowly-scoped nil (not the full T? nullable-type system), Hash[K, V] literals/get/set, and Array.new(size) — each a real, executed proof, not a spec-completeness sweep."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-bool-literals
    content: "Expr::Bool(bool) — true/false literals producing a real Type::Boolean value, not just Compare's byproduct"
    status: pending
  - id: leaf-nil-type
    content: "Type::Nil + Expr::Nil — a bare Nil value usable in a Nil-typed parameter/local and compared for equality; explicitly NOT the T? nullable-type system"
    status: pending
  - id: leaf-hash
    content: "Type::Hash(Box<Type>, Box<Type>); Expr::HashLit; Int64-keyed get/set reusing Expr::Index/Stmt::SetIndex, backed by a flat linear-scan (key, value) buffer, not real hashing"
    status: pending
  - id: leaf-array-new
    content: "Array.new(size) — a new emerald_alloc_zeroed runtime helper, zero-filled, fixed-size, no length tracking"
    status: pending
isProject: false
---

# Plan 25 — Standard Library Expansion

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) —
post-v1 tooling/language-completeness scope, same posture as plan 17:
this plan stands alone and does not modify `plan-of-plans.md` or any
other plan file.

It closes four independent gaps that earlier plans already found and
explicitly disclosed rather than solved. Each gets its own leaf with a
real, executed acceptance proof — this plan does **not** attempt to
fully close every relevant `spec/GRAMMAR.md`/`spec/TYPE_SYSTEM.md` KEEP
row; each leaf states plainly which fuller spec it is and isn't
delivering, the same discipline plans 09/11/13 already established.

## Decision log

- **`spec/RUNTIME.md` does not exist.** Verified this session — `spec/`
  contains exactly `COMPILER.md`, `GRAMMAR.md`, `SEMANTICS.md`,
  `TYPE_SYSTEM.md`. Both plan 09's own Decision log ("hashing/bucket
  layout is explicitly a `RUNTIME.md` concern") and `TYPE_SYSTEM.md` §8
  itself ("`Hash[K, V]`... no representation constraint as strict as
  `Array[T]`'s in v1... a `RUNTIME.md` concern") point at a spec file
  that was never authored. `leaf-hash` below is therefore not deviating
  from an existing spec decision when it picks a representation — it is
  filling a real, disclosed documentation gap this plan's own Decision
  log now records, not silently inventing something the spec already
  settled.
- **`{a: 1}`-style Symbol-keyed hash literal syntax is explicitly not
  implemented.** `TYPE_SYSTEM.md` §6's literal-typing table types
  `{a: 1}` as `Hash[Symbol, T]` — but this compiler has no `Symbol` type
  or `:foo` literal at all (verified: `emerald_sema::Type` has no
  `Symbol` variant, `grammar.lalrpop` has no `:foo` production).
  `leaf-hash` instead adds a general hash-rocket literal, `{k => v,
  ...}`, with `Int64` keys only — proving `Hash[K, V]`'s representation
  and get/set claims without inventing `Symbol` as an unstated
  prerequisite. A real, disclosed narrowing of `TYPE_SYSTEM.md` §6's
  literal form, not the form itself.
- **`nil` gets the narrowest possible scope, deliberately short of
  `TYPE_SYSTEM.md` §4's actual "Nil and nullable types" design.** §4 is
  explicit that `nil` is only useful once `T?` nullable types exist:
  "A plain type name... is not nil-assignable. `x: Int64 = nil` is a
  compile error... [nil is] assignable only to `T?` (nullable) forms."
  Building `T?` for real means a new type-syntax form, safe navigation
  (`&.`), and reworking every assignability check in `emerald-sema` to
  understand a type union — genuinely comparable in size to this
  compiler's entire existing type-checking surface, and not attempted
  here. `leaf-nil-type` instead proves only that `Type::Nil`/`Expr::Nil`
  exist and round-trip correctly (a `Nil`-typed parameter can be passed
  a `nil` literal and compared for equality) — deliberately trivial
  (`nil == nil` is always true), because the trivial mechanism is
  exactly what's being proven: a working `Nil` value flowing through
  parameters, locals, comparison, and codegen. `T?`, safe navigation,
  and nil-into-a-reference-type-slot assignability remain real,
  substantial, separate future work — not silently under-delivered
  against §4, explicitly deferred from it.
- **`Hash[K, V]`'s v1 representation is a flat, linear-scan `(key,
  value)` pair buffer — not a real hash table.** No bucket layout, no
  hash function, no collision resolution. `Index`/`SetIndex` on a
  `Hash` scan the buffer comparing keys with `Int64` equality (`O(n)`
  lookup). This is the same category of move plan 11 already made for
  exceptions (`setjmp`/`longjmp` standing in for true DWARF unwinding):
  the smallest real mechanism that proves the semantic claim
  (`{...}` literal construction, keyed get, keyed set) without
  committing to a hashing/collision design nothing in this repo's spec
  actually mandates (see the `RUNTIME.md` finding above). A real hash
  table is disclosed, honest follow-up work, not a silent performance
  lie — `O(n)` lookup is stated plainly, not hidden behind the name
  `Hash`.
- **`Hash`'s get/set reuse the existing `Expr::Index`/`Stmt::SetIndex`
  AST nodes** (plan 09) rather than adding parallel `HashGet`/`HashSet`
  variants — `emerald-sema`'s `infer_expr_type`/`check_stmt` already
  dispatch on the indexed receiver's resolved type; adding a `Type::
  Hash(_, _)` arm alongside the existing `Type::Array(_)` arm is a
  smaller, more consistent change than a second parallel indexing
  mechanism, and keeps `emerald-lsp`/`emerald-mcp` (plan 17) diagnostic
  rendering for indexing errors uniform across both container kinds.
- **`Array.new(size)` gets a dedicated `Expr::ArrayNew(Box<Expr>)` AST
  node, not a reuse of `Expr::New`.** `Expr::New(String, Vec<Expr>)`
  models `ClassName.new(args)` — a real user-defined class instance
  construction with class-registry dispatch behind it. `Array` is a
  reserved keyword, not a class name in `ClassInfo`'s registry (verified
  this session), and conflating "construct a builtin container" with
  "construct a class instance" in one AST node would make codegen's
  already-large `build_expr`/`build_method_call` match arms (2268 lines,
  cc up to 81 in `compile_to_object` — verified this session) special-case
  on a string comparison against `"Array"` inside class-construction
  logic that otherwise never needs to know about builtin types. A
  dedicated node keeps the two concerns separately dispatched, the same
  way `Expr::ArrayLit` already stands apart from `Expr::New`.
- **Zero-fill via a new `emerald_alloc_zeroed` runtime C function
  (`calloc`-backed), not an inline Cranelift store loop.** `runtime/
  emerald_runtime.c` already declares/exports `emerald_alloc` as a
  runtime helper codegen calls exactly like this one would (verified);
  adding a second small runtime function is far less codegen surface
  than emitting a fresh basic-block loop inline in every `Array.new`
  call site, and `calloc`'s own zero-fill guarantee is simpler to trust
  than a hand-written store loop over a runtime-known (non-constant)
  element count.
- **No length tracking, no bounds checking, no growable `.push`.**
  Exactly plan 09's own already-disclosed scope cut, unchanged here:
  `Array.new(size)`'s `size` is consumed once, at construction, to
  compute a byte count — it is not stored anywhere, so nothing after
  construction knows an array's length. A real growable/bounds-checked
  representation is a bigger, separate change to `Array[T]`'s
  fundamental layout, not a natural extension of this leaf.
- **Printing a bare `Boolean`/`Nil` value via `puts` is out of scope.**
  `puts` today only knows how to print `Int64`/`Float64`/`String`-typed
  values (verified against `build_puts`); adding "print `true`/`false`/
  `nil` as text" is a `to_s`/formatting-protocol question this plan
  doesn't need to answer. Both worked examples below route their
  boolean/nil values through an `if` condition into an `Int64` result
  before printing, proving the value round-trips through type-checking
  and codegen without also inventing boolean-to-string formatting.

## Leaf: leaf-bool-literals

### 1. Context
- Why: `emerald_sema::Type::Boolean` already exists and is already a
  legal parameter/local annotation (verified: `resolve_type` accepts
  the primitive name `"Boolean"` today, and `Expr::Compare` already
  produces a `Type::Boolean`-typed value) — but there is no literal
  syntax to write a bare `true`/`false` value. `spec/GRAMMAR.md` §1
  marks `true`/`false` KEEP.
- Target state: `Expr::Bool(bool)` in `emerald-parser`'s AST; grammar
  productions for the reserved keywords `true`/`false` (same LALR(1)
  reservation pattern as `new`/`puts`/`Array`); `emerald-sema` types
  `Expr::Bool(_)` as `Type::Boolean` unconditionally; codegen lowers it
  to a Cranelift `iconst` `0`/`1` in whatever integer type the existing
  `Type::Boolean`-typed `Expr::Compare` codegen path already produces
  (verify and match that representation exactly — no new boolean
  representation introduced).
- Composes with plan 18 (arithmetic/logical operators, already written
  — `crates/emerald-parser`'s `&&`/`||`/`!` operate on any
  `Type::Boolean`-typed operand): a bare `true`/`false` literal is just
  another such operand. This leaf does not re-touch plan 18's operator
  codegen.

### 2. Acceptance Criteria
1. This program, compiled, linked, and run, prints `1\n0\n`:
   ```ruby
   def check(flag: Boolean) -> Int64
     if flag
       return 1
     end
     return 0
   end

   puts check(true)
   puts check(false)
   ```
   Real executed proof `true`/`false` parse, type-check as `Boolean`,
   and drive a real `if` branch correctly in both directions — not just
   "compiles cleanly."
2. Regression: every prior plan's example still parses and type-checks
   identically (`true`/`false` becoming reserved keywords must not
   collide with any existing identifier used in `examples/*.em` or any
   prior plan's test fixtures — grep for `\btrue\b`/`\bfalse\b` as
   plain identifiers before landing, per the same discipline plan 07/08
   used when reserving `puts`/`new`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests),
  `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run `1\n0\n` | agent-claimed-locally |

---

## Leaf: leaf-nil-type

### 1. Context
- Why: `spec/TYPE_SYSTEM.md` §4 and `spec/GRAMMAR.md` §1 both mark
  `nil`/`Nil` KEEP, but no `Type::Nil`/`Expr::Nil` exists anywhere in
  this compiler today (verified this session).
- Target state (deliberately narrow — see Decision log): `Type::Nil` in
  `emerald-sema`; `Expr::Nil` in `emerald-parser`'s AST with a reserved
  `nil` keyword production; `Type::Nil` is a legal parameter/local
  annotation (resolved by `resolve_type` exactly like any other
  primitive); `Expr::Compare` with `CompareOp::Eq`/`Ne` accepts two
  `Type::Nil` operands (result is always `true`/`false` respectively —
  trivial, but real, type-checked, and codegen-lowered). Represented in
  codegen as a fixed Cranelift `iconst` sentinel (e.g. `I8` `0`) purely
  so it occupies a real SSA value/parameter slot — any single fixed
  representation is sufficient since a `Nil` value is never compared
  against anything but another `Nil` in this leaf's scope.

### 2. Acceptance Criteria
1. This program, compiled, linked, and run, prints `1\n`:
   ```ruby
   def check_nil(x: Nil) -> Int64
     if x == nil
       return 1
     end
     return 0
   end

   puts check_nil(nil)
   ```
   Real executed proof a `Nil`-typed parameter can be declared, passed
   a real `nil` literal, compared, and branched on.
2. `x: Int64 = nil` (or any non-`Nil` type annotation assigned a `nil`
   literal) is rejected with a diagnostic — proving this leaf does
   *not* silently implement `T?`-style nil-into-anything assignability,
   matching `TYPE_SYSTEM.md` §4's actual rule that plain (non-`T?`)
   types are never nil-assignable.
3. Regression: every prior plan's example still parses/type-checks
   identically; `nil` becoming a reserved keyword doesn't collide with
   any existing identifier (same grep discipline as leaf-bool-literals).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests),
  `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run `1\n`, incl. the AC2 rejection test | agent-claimed-locally |

---

## Leaf: leaf-hash

### 1. Context
- Why: plan 09's Decision log explicitly deferred `Hash[K, V]` in full
  ("deferred entirely... gets its own attention when a concrete program
  needs key/value lookup"). No `Type::Hash`/hash literal/hash indexing
  exists anywhere in this compiler today (verified this session).
- Target state: `Type::Hash(Box<Type>, Box<Type>)` in `emerald-sema`;
  `Expr::HashLit(Vec<(Expr, Expr)>)` in `emerald-parser`'s AST, parsed
  from `"{" (Expr "=>" Expr ",")* (Expr "=>" Expr)? "}"` (the `"=>"`
  token already exists in this grammar for `rescue Type => e` — reused,
  not newly introduced; the leading `"{"` is new in `PrimaryExpr`
  position and needs a real LALRPOP-conflict check since `"{"` already
  appears, in a different context, inside the lambda-literal production
  — verify no shift/reduce conflict arises before landing, same
  discipline every prior grammar leaf has applied). Restricted to
  `Int64` keys in this leaf (see Decision log on `Symbol`). `emerald-
  sema`'s `infer_expr_type`/`check_stmt` grow a `Type::Hash(_, _)` arm
  alongside their existing `Type::Array(_)` arm for `Expr::Index`/
  `Stmt::SetIndex` (both AST nodes reused, per Decision log — no new
  indexing syntax). Codegen represents a `Hash[Int64, V]` as a flat,
  `emerald_alloc`-backed buffer of `(Int64 key, V value)` pairs (see
  Decision log on representation); `Index`/`SetIndex` on a `Hash`-typed
  receiver linear-scan the buffer for a matching key.

### 2. Acceptance Criteria
1. This program, compiled, linked, and run, prints `20\n99\n`:
   ```ruby
   h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}
   puts h[2]
   h[2] = 99
   puts h[2]
   ```
   Real executed proof construction, keyed read, and keyed write all
   work together.
2. Indexing a `Hash` with a key it doesn't contain is rejected — decide
   and implement one concrete, disclosed behavior (a runtime error via
   a new small runtime helper, mirroring how out-of-bounds `Array`
   access is already left as undefined/unchecked per plan 09, or a
   defined sentinel) and state plainly which you chose and why; do not
   leave it silently undefined without saying so in this plan's own
   text.
3. A `{}` empty hash literal is rejected (no key/value pair to unify
   element types from) — same reasoning plan 09 already applied to `[]`.
4. A mixed-key-type or mixed-value-type literal (e.g. `{1 => 10, 2 =>
   "twenty"}`) is rejected with a diagnostic naming the mismatch.
5. Regression: every prior plan's example still parses/type-checks
   identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests),
  `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`,
  `runtime/emerald_runtime.c` (only if AC2's chosen behavior needs a
  runtime helper, e.g. a "key not found" abort function)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run `20\n99\n` and AC2-5's rejection/behavior tests | agent-claimed-locally |

---

## Leaf: leaf-array-new

### 1. Context
- Why: plan 15's Decision log explicitly found "`Array[T]` can
  currently only be constructed via a literal... there is no
  `Array.new(size)`/fill-from-empty constructor." Verified again this
  session: `grammar.lalrpop` has no `"Array" "." "new"` production at
  all (`Array` is reserved only inside the `TypeName` rule).
- Target state: a new grammar production `"Array" "." "new" "("
  <size:CallExpr> ")"` (distinct from the existing `<recv:Ident> "."
  "new" "(" args ")"` class-construction rule, since `Array` is a
  reserved keyword token, not `Ident` — verified this session) producing
  a new `Expr::ArrayNew(Box<Expr>)` AST node (see Decision log for why
  this is a dedicated node, not a reuse of `Expr::New`). `emerald-sema`
  types it as `Type::Array(elem)` where `elem` comes from the enclosing
  `Let`'s declared type annotation (same source `leaf-sema-array`,
  plan 09, already uses for indexing an array literal — no new
  inference mechanism). Codegen calls a new `emerald_alloc_zeroed(n:
  i64) -> ptr` runtime function (`runtime/emerald_runtime.c`, `calloc`-
  backed) with `size * ARRAY_ELEM_SIZE` bytes.

### 2. Acceptance Criteria
1. This program, compiled, linked, and run, prints `0\n3\n`:
   ```ruby
   arr: Array[Int64] = Array.new(5)
   puts arr[0]
   i: Int64 = 0
   while i < 5
     arr[i] = i
     i = i + 1
   end
   puts arr[3]
   ```
   Real executed proof the allocation is genuinely zero-filled (the
   first `puts`, before any write) and genuinely writable/readable
   afterward (the second `puts`, after the loop).
2. `Array.new`'s `size` argument may be a runtime value (e.g. a
   variable, not only an integer literal) — proven by the loop above
   using a computed size is not required, but the codegen path must not
   assume a compile-time-constant size; state this explicitly and test
   `Array.new(n)` where `n` is itself a `Let`-bound variable.
3. Regression: every prior plan's array example (literal construction,
   indexing) still parses/type-checks/compiles/runs identically —
   `Array.new` is additive, not a replacement for literal construction.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests),
  `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`,
  `runtime/emerald_runtime.c` (adds `emerald_alloc_zeroed`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run `0\n3\n` and the runtime-size (AC2) test | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `T?` nullable types, safe navigation (`&.`), nil-assignability into
  non-`Nil` reference-type slots — `TYPE_SYSTEM.md` §4's actual design;
  see `leaf-nil-type`'s Decision log entry.
- `Symbol` type/`:foo` literals, and therefore `{a: 1}` Symbol-keyed
  hash literal shorthand — see `leaf-hash`'s Decision log entry.
- A real `Hash[K, V]` hashing/bucket implementation (this plan ships a
  linear-scan `O(n)` pair buffer) — see `leaf-hash`'s Decision log
  entry.
- `Hash` keys other than `Int64` (no `String`/`Symbol`/`Class`-typed
  keys), `Hash` iteration, deletion, or a `.length`/`.keys`/`.values`
  method surface.
- `Array` bounds checking, runtime length tracking, and a growable
  `.push` (needs length tracking as a prerequisite this plan doesn't
  add) — same scope cut plan 09 already made, unchanged here.
- Printing `Boolean`/`Nil` values via `puts` (a `to_s`/formatting
  protocol question) — see Decision log.
- Bitwise/other operators, additional numeric widths (`Int8`..`UInt64`,
  `Float32`), `struct` types, `Symbol`, `Range[T]` — all real gaps
  `spec/TYPE_SYSTEM.md` §1/§2 documents, none touched by this plan.
