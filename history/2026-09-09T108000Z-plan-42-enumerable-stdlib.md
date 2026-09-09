---
name: Enumerable Standard Library
overview: "A second built-in interface, `Iterable[T]` (one required method, `each`), implemented intrinsically by `Array[T]` and `Hash[K,V]` (`Range` is excluded — corrected post-hoc against plan 37's actual Decision log, see this plan's own Decision log), plus seven generic free functions (`map`, `select`/`filter`, `reduce`/`inject`, `each_with_index`, `count`, `sum`, `sort`) monomorphized per plan 41's mechanism and surfaced via ordinary method-call syntax resolved entirely at compile time — closing the collections/stdlib gap without vtables, `method_missing`, or a mixin system."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-array-length-header
    content: "Array[T]'s representation gains a runtime `[length: Int64][elements...]` header, mirroring Hash[K,V]'s already-shipped `[count: Int64][pairs...]` layout — the missing prerequisite `each` needs to know when to stop"
    status: pending
  - id: leaf-iterable-interface
    content: "`interface Iterable[T] { def each(block: Proc[T, Void]): Void }` (plan 41's mechanism) plus an intrinsic-conformance table so Array[T]/Hash[K,V] satisfy it without a source-level `implements` clause (Range is excluded — see Decision log); new built-in `Type::Pair(K, V)` for Hash iteration"
    status: pending
  - id: leaf-block-return-value
    content: "Block literals infer a real return type from their body's trailing expression instead of plan 34's hardcoded `Void` — the minimal extension `map`/`select`/`reduce` need on top of plan 34's stated scope"
    status: pending
  - id: leaf-enumerable-functions
    content: "`map`, `select`/`filter`, `reduce`/`inject`, `each_with_index`, `count`, `sum`, `sort` as generic free functions over `Iterable[T]`/`Comparable`, monomorphized per call site, dispatched via method-call syntax"
    status: pending
isProject: false
---

# Plan 42 — Enumerable Standard Library

This is one of twelve independent sibling plans (36-47), authored in
parallel, whose combined purpose is closing Emerald's language/stdlib
surface toward its identity-preserving ceiling of roughly 45% of Ruby's
surface — accepting every gap compatible with Emerald's static,
non-reflective, non-dynamic identity and declining the rest outright.
This plan is post-v1 scope, same posture as the 17-27 and 28-35
batches before it; it does not modify
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) or
any other plan file — that table is updated separately, once, after all
twelve siblings in this batch are authored. This plan is the
highest-leverage plan in the batch for the "collections/stdlib" gap: it
depends on [plan 41](interfaces-and-generics, assumed contract below)
for the interface/monomorphization mechanism, [plan 37](ranges-and-
iteration, assumed contract below) for `Range`, and plan 09
(`2026-09-08T190129Z-plan-09-collections.md`, already written and
partially superseded by plan 25) for `Array[T]`/`Hash[K,V]`'s existing
representations. Neither plan 41 nor plan 37 exist as files in
`history/` as of this session (verified: `history/` runs from
`2026-09-08T173600Z-inception.md` through
`2026-09-09T101000Z-plan-35-debug-info.md`, nothing numbered 36+) — this
plan cites their contracts exactly as summarized by the batch's shared
brief, the normal way parallel sibling plans in this project cite each
other.

Concrete proof this plan targets (no method-chaining — see Decision
log):

```ruby
arr: Array[Int64] = [1, 2, 3, 4, 5, 6]
evens: Array[Int64] = arr.select { |x: Int64| x % 2 == 0 }
doubled: Array[Int64] = evens.map { |x: Int64| x * 2 }
total: Int64 = doubled.sum
puts total
```

Expected output: `24` — `select` materializes `[2, 4, 6]`, `map`
materializes `[4, 8, 12]`, `sum` folds them to `24`. Each step is a real
compiled/linked/run stage, not just a type-checked one; `%`/`==`/`*`
are plan 18/25's existing `Expr::Rem`/`Expr::Compare`/`Expr::Mul`, and
`x % 2 == 0` is exactly the kind of block-body-produces-a-used-`Boolean`
case that motivates this plan's own `leaf-block-return-value`.

## Decision log

- **Calling convention: method-call syntax (`arr.map { ... }`),
  resolved entirely at compile time to a generic free function
  monomorphized for the receiver's statically known concrete type — no
  new dispatch mechanism.** `arr.select { ... }` and `select(arr) { ... }`
  compile to the exact same thing; method-call syntax is chosen because
  it gets the Ruby-idiomatic *feel* of `.map`/`.select` chains, while the
  underlying mechanism is identical to every other method call this
  compiler already performs: verified this session against
  `crates/emerald-codegen/src/lib.rs`'s `build_method_call` (lines
  1562-1655) — a call's target function is already resolved from the
  receiver's *statically declared* type (via `local_classes: HashMap<
  String, String>`, itself populated once, at `Let`/parameter-binding
  time, from the declared type annotation) with zero runtime type
  inspection of the receiver value itself, the same static-resolution
  discipline this project has used since plan 08's `Point` methods.
  This plan adds a second lookup path alongside the existing
  `local_classes` one (see `leaf-enumerable-functions`), not a v-table,
  not a runtime `is_a?` check.
- **Each of the seven functions is implemented once, generically,
  against `Iterable[T]`/`Comparable`, and monomorphized per concrete
  instantiation — reusing plan 41's monomorphization wholesale, not a
  second parallel specialization mechanism.** Concretely: this plan's
  `map`/`select`/etc. are ordinary generic functions in plan 41's sense
  (`def map[T, R](items: Iterable[T], block: Proc[T, R]): Array[R]`-
  shaped), and the compiled symbol per call site follows the exact
  naming/emission pattern `build_method_call` already uses for
  per-class method compilation (`"{class_name}_{method}"`, verified at
  line 1622 — one compiled function per defining class, keyed by name
  string) generalized to one compiled function per `(function, concrete
  type argument)` pair actually instantiated in the program — e.g.
  `sort` over `Array[Int64]` and `sort` over `Array[Float64]` in the same
  program are two distinct compiled functions, exactly as `Dog_speak`
  and `Cat_speak` already are today for ordinary class methods.
- **`Array[T]`'s current representation has no runtime length — this is
  a real, independently re-confirmed gap this plan must close before
  `Iterable[T]` can mean anything for `Array[T]`.** Verified three times
  over, independently, by three different already-written plans: plan
  09's Decision log ("no runtime length, no bounds checking... a raw
  pointer to a contiguous buffer"), plan 25's Decision log on
  `Array.new(size)` ("No length tracking... `size` is consumed once, at
  construction... it is not stored anywhere"), and plan 30's Decision
  log ("blocked on an `Array[T]` representation change (length
  tracking) that neither this plan nor plan 25 delivers"). Re-verified
  directly against the current `build_array_lit` in
  `crates/emerald-codegen/src/lib.rs` (lines 1663-1696) this session:
  it `emerald_alloc`s exactly `elements.len() * 8` bytes and stores each
  element sequentially — no header, no count, nothing else in the
  buffer. `leaf-array-length-header` is this plan's own answer to the
  gap plan 30 predicted a future plan would need to close: `Array[T]`
  gains a `[length: Int64][elem0][elem1]...]` layout, allocating `8 +
  n*8` bytes instead of `n*8`. This is **not a novel scheme invented for
  this plan** — it is the identical layout `Hash[K, V]` already ships
  with today (`build_hash_lit`'s doc comment, verified at line 1459:
  "allocates `8 + n*16` bytes... an `i64` pair-count header"; `build_
  hash_lookup`, verified at lines 1714-1815, already reads that count
  via `build_load(i64_ty, base_ptr, "hashcount")` at offset 0 to bound
  its linear scan). `Hash[K, V]` needs **no representation change at
  all** for this plan — its count header already exists and already
  gives `each` exactly what it needs.
- **`Hash[K, V]` iteration yields a new built-in `Type::Pair(Box<Type>,
  Box<Type>)`, not a user-declarable generic class.** Plan 41's own
  summary contract (assumed per this batch's brief) covers generic
  *functions* and single-class `implements` — it does not mention
  generic *classes* (`class Foo[T]`), and this plan does not invent that
  capability on plan 41's behalf. Instead, `Pair[K, V]` is added the
  same way `Array[T]`/`Hash[K, V]` themselves already exist: a hand-
  rolled, hard-coded compound `Type` variant (verified this session
  against `crates/emerald-sema/src/lib.rs` lines 12-38 — `Type::Array`
  and `Type::Hash` are already exactly this: non-generic-mechanism,
  compiler-native enum variants with their own `resolve_type` string-
  parsing, lines 100-117), with exactly two accessors, `.key`/`.value`,
  and a fixed 16-byte `[key: 8][value: 8]` layout — deliberately the
  same per-pair byte layout `Hash[K, V]`'s own buffer already uses
  (verified at `build_hash_lookup`'s `pair_off`/`key_off`/`value_off`
  arithmetic, lines 1763-1814), so `Hash[K,V].each` can construct a
  `Pair` by copying 16 contiguous bytes straight out of its own backing
  buffer rather than inventing a second layout convention.
- **Built-in primitive types (`Int64`, `Float64`, `String`) are treated
  as intrinsically satisfying `Comparable` — they cannot write
  `implements Comparable` (no `class` declaration exists for them at
  all).** Verified this session: `ClassInfo`'s registry (`emerald-sema`
  lines 65-78) is populated exclusively from `Item::Class`/`Item::
  Module`; `Int64`/`Float64`/`String` are `resolve_type`'s hard-coded
  primitive-name branches (lines 85-89), never `ClassInfo`-registered.
  This plan asks plan 41's generic-bound checker for one small,
  disclosed additional hook beyond its stated class-`implements`
  contract: a fixed table of intrinsic conformances for built-in types
  (`Array[Elem]: Iterable[Elem]`, `Hash[K,V]: Iterable[Pair[K,V]]`,
  `Range: Iterable[Int64]`, `Int64`/`Float64`/`String`: `Comparable`),
  consulted whenever a bound is checked against a receiver whose type
  isn't `ClassInfo`-registered. `sort`'s generic body then branches, per
  monomorphized instantiation, between a native `Expr::Compare`
  (`Int64`/`Float64`/`String`) and a genuine `<=>`-method call (any
  user class that wrote `implements Comparable`) — a real, disclosed
  per-instantiation codegen fork, not a runtime type test (the fork is
  baked into which specialized function body gets emitted, decided at
  monomorphization time from the concrete type argument, same as every
  other monomorphized fork).
- **`sum`'s generic bound is narrower than the other six functions':
  `T` must be `Int64` or `Float64`, not `Comparable`.** Emerald has no
  operator-overloading mechanism at all for user-defined classes today
  (verified: no AST/sema path dispatches `+`/`Expr::Add` to a class-
  defined method; every numeric binary op resolves directly against
  `Int64`/`Float64` operands via `build_numeric_binop`). Extending `sum`
  to arbitrary user types would require inventing operator-overload
  dispatch from scratch — a real, separate, and non-trivial future
  gap this plan does not fold in un-announced; `sum` is therefore two
  concrete monomorphizations (`Int64`, `Float64`), not a `T: Comparable`-
  bounded generic like the other six.
- **Blocks passed to these functions reuse plan 34's block/`&blk`/
  call-site-specialization machinery verbatim for *how* a block
  attaches to a call — but plan 34, as written, hardcodes every block
  literal's inferred return type to `Void`.** Re-verified this session
  against plan 34's own Decision log ("Block literals have no explicit
  return-type annotation and are always treated as `Void`-returning...
  A block whose result *is* used... is real, disclosed future work — it
  needs either an explicit return-type annotation on the block literal
  or genuine call-site inference, neither of which this plan builds.")
  and against `Expr::Lambda`'s shape in `crates/emerald-parser/src/
  ast.rs` (`params`, `return_type: String`, `body`) — the block-literal
  construction site (per plan 34) builds this with `return_type:
  "Void".into()` unconditionally. `map`/`select`/`reduce` all need a
  block whose value is genuinely used and typed. `leaf-block-return-
  value` is this plan's disclosed, minimal extension on top of plan 34's
  stated scope: infer the block's `return_type` from its body's
  trailing expression statement (reusing `infer_expr_type` on that
  final `Stmt::Expr`, the same implicit-return reasoning `check_
  implicit_return` — verified present in `emerald-sema`'s function map —
  already applies to ordinary function bodies) instead of hardcoding
  `Void`; a block whose last statement isn't a bare expression still
  infers `Void`, exactly as today. `Type::Proc(Vec<Type>, Box<Type>)`
  itself needs **zero** new machinery — it already carries an arbitrary
  `Box<Type>` return slot (used today by plan 10's explicit `->(x) ->
  R { ... }` lambda values); only the block-literal's own hardcoded
  `Void` construction changes.
- **`Proc[...]`'s bracketed generic annotation form (`Proc[T, Void]`,
  as this plan's own prompt already writes it for `Iterable[T].each`)
  is inherited from plan 41, not built here — but this plan generalizes
  it to N parameters for `reduce`.** Verified this session: `emerald-
  sema::resolve_type`'s current `"Proc"` case (line 124) accepts only
  the bare keyword, producing an empty-signature placeholder — there is
  no bracketed `Proc[...]` annotation parsing in the compiler at all
  today. This plan assumes plan 41 adds that parsing (its own contract
  requires it, independent of this plan, since `Iterable[T]`'s `each`
  signature already needs it). This plan's own disclosed addition on
  top of that: `reduce`'s block takes two parameters (`{ |acc: R, elem:
  T| ... }`), so its annotation is `Proc[R, T, R]` — the natural
  generalization of a bracket list as "all parameter types, then the
  return type last," not a second, competing annotation syntax.
- **No method-chaining on the result of one call feeding into the
  next — this plan requires two sequential named-local statements
  instead (`evens = arr.select {...}`, then `evens.map {...}`), and
  this plan's own worked example above is written exactly that way.**
  This is a real, currently-enforced restriction, not a stylistic
  choice: verified this session against `build_method_call` (line
  1573-1577) — `let Expr::Ident(recv_name) = recv else { return
  Err("codegen: method calls are only supported on a plain
  local-variable receiver".to_string()) }` — and against `build_index`'s
  parallel restriction (plan 09's Decision log: "Indexing a non-`Ident`
  array expression is unsupported in codegen"). `arr.select{...}.map
  {...}` would place a `MethodCall` as `.map`'s receiver, which is not
  an `Expr::Ident` and is therefore rejected by this exact, pre-existing
  guard. Generalizing every method-call and index receiver to an
  arbitrary expression (evaluate it into an anonymous temporary rather
  than looking it up by name in `vars`/`local_classes`/`local_array_
  elem_types`) is a real, legitimate, and non-trivial future codegen
  generalization that touches every method-call and index site in the
  compiler, not something specific to `Iterable`; this plan declines it
  and keeps the existing named-local-receiver restriction consistent
  across the whole language, rather than carving out a special
  chaining exception just for these seven functions.
- **User-defined classes that `implements Iterable[T]` with their own
  `each` get all seven generic functions for free too — but via the
  exact same static generic-function resolution as `Array[T]`, never
  via Ruby's `include Enumerable` mixin mechanism.** This is the
  concrete difference between this plan's design and Ruby's: Ruby's
  `include Enumerable` reflectively derives ~40 methods from one `each`
  at runtime (a form of dynamism this project's identity constraints
  forbid outright — no mixins, no method_missing-shaped derivation).
  Here, `map`/`select`/etc. are seven separately-defined, ordinary
  generic functions (per plan 41's mechanism) whose only requirement is
  "the argument's type satisfies `Iterable[T]`" — checked statically,
  monomorphized per call site, exactly like calling any other generic
  function on any other conforming type. A user class gets the same
  seven functions a built-in container gets, through the same
  mechanism, with zero magic and zero new dispatch machinery — this is
  the concrete cash-out of Decision log's first bullet ("Ruby-idiomatic
  *feel*... without the Ruby *mechanism*").
- **Declined: lazy/deferred enumerators (`Enumerator`, `.lazy` chains).**
  Every one of these seven functions is eager — `map`/`select` allocate
  and fully populate a real `Array[R]` immediately, `reduce`/`sum`/
  `count` fully consume their `Iterable[T]` immediately. A lazy
  `Enumerator` needs first-class suspended-computation values (an
  iterator/generator object with mutable internal state resumed across
  calls) — a real, disclosed simplification consistent with this
  compiler's total absence of any coroutine/generator mechanism
  anywhere else in the language; nothing this plan needs (or any prior
  plan built) provides a resumable-computation primitive to build
  `.lazy` on top of.
- **Declined: `sort!`/`map!`/`select!` (in-place mutating variants).**
  `sort!` alone could plausibly work today (fixed-size in-place swaps
  over `Array[T]`'s existing buffer), but `map!`/`select!` cannot —
  `select!` changes an array's length, and `Array[T]` has no growable/
  resizable representation even after this plan's `leaf-array-length-
  header` (that leaf adds a *stored* length, not a *resizable* one).
  Declining all three uniformly, rather than shipping `sort!` alone,
  keeps this surface's contract consistent ("every one of these
  functions returns a fresh value") instead of one silent, easily-
  missed exception.
- **Out of scope, with real reasoning, not just "future work": `zip`,
  `group_by`, `flat_map`, `partition`, `take`/`drop`, `min_by`/`max_by`,
  `any?`/`all?`/`none?`, `find`/`detect`, `each_slice`, `tally`, `uniq`.**
  Each of Ruby's ~40 real `Enumerable` methods is a legitimate future
  increment under this exact same mechanism (define one more generic
  function against `Iterable[T]`/`Comparable`) — but each also needs its
  own real, separate design decision this plan does not make: `zip`
  needs multi-`Iterable` arity and a tuple-return type this compiler
  doesn't have; `partition`/`group_by` need a two-array or `Hash`-typed
  return inferred from the block's own result type; `min_by`/`max_by`
  need a *second* type parameter (the sort key) distinct from `T`. The
  seven chosen here are the smallest set that proves every distinct
  shape of the mechanism end-to-end: a pure producer (`each` itself), a
  boolean-predicate filter (`select`), a value transformer (`map`), a
  fold/accumulator (`reduce`, `sum`), an indexed variant (`each_with_
  index`), and an ordering consumer (`sort`, `Comparable`). Every
  remaining method is additive, not architectural, future work.

## Leaf: leaf-array-length-header

### 1. Context
- Why: `Iterable[T].each` cannot be implemented for `Array[T]` at all
  without a runtime element count — see Decision log's `Array[T]`
  bullet. This is a genuine prerequisite leaf every other leaf in this
  plan depends on.
- Current state (re-verified this session): `build_array_lit` (`crates/
  emerald-codegen/src/lib.rs`, lines 1663-1696) allocates exactly
  `elements.len() * 8` bytes via `emerald_alloc` and stores each element
  at `i * 8`; `build_index`/`build_set_index` (lines 1822-1995) address
  elements directly from the raw pointer at `index * 8` with no offset;
  `Array.new(size)` (per plan 25, `Expr::ArrayNew`, handled inside `build_
  stmt`'s `Let` arm — verified `build_expr`'s own `Expr::ArrayNew` arm at
  lines 1449-1455 explicitly defers to that caller) zero-fills via
  `emerald_alloc_zeroed(size * 8)`, storing nothing about `size` itself.
- Target state: allocate `8 + elements.len() * 8` bytes; store `elements.
  len()` as an `i64` at offset 0; every element shifts to `8 + i * 8`.
  `build_index`/`build_set_index` add a fixed `+8` byte offset before
  their existing address arithmetic. `Array.new(size)`'s codegen arm
  allocates `8 + size * 8` bytes (`size` is runtime-known, not always a
  compile-time constant — the multiply/add already has to happen at
  runtime, not just a constant-folded literal length as in
  `build_array_lit`), zero-fills only the element region (`ptr + 8`
  onward, `size * 8` bytes) via `emerald_alloc_zeroed`, and stores
  `size` itself at offset 0 after allocation. This is a pure
  representation change: `emerald-sema`'s `Type::Array(Box<Type>)` is
  untouched (mirroring `Type::Hash`, whose own count header is already
  fully opaque to sema).

### 2. Acceptance Criteria
1. Regression: plan 09/25's existing `Array[Int64]` example (allocate
   `[10, 20, 30]`, sum via loop+index, `arr[1] = 99`, read back),
   compiled, linked, and run, still prints `60\n99\n` — identical
   observable behavior despite the underlying byte-layout change,
   proving the header is invisible to every existing indexed-read/write
   call site.
2. A real new proof the header actually exists and is correctly
   populated: a minimal internal length-read (the same mechanism
   `leaf-enumerable-functions`'s `count` will later reuse), compiled,
   linked, and run against `arr: Array[Int64] = [10, 20, 30]`, prints
   `3`.
3. `Array.new(5)`, compiled, linked, and run, followed by the same
   length-read, prints `5` — proving the header is populated correctly
   on the zero-fill construction path too, not only the literal path.
4. Negative/regression-safety: the two allocation sites (`build_array_
   lit`, `Array.new`'s codegen arm) and the two consumption sites
   (`build_index`, `build_set_index`) must agree on layout by
   construction — this leaf's own quality gate (full workspace test
   suite) is the real proof; a layout mismatch between producer and
   consumer sites would manifest as AC1's regression check failing
   (wrong values read back), not a distinct new error path.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs` (`build_array_lit`,
  `build_index`, `build_set_index`, the `Expr::ArrayNew` handling inside
  `build_stmt`'s `Let` arm).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen` | clean | agent-claimed-locally |
| Test (regression) | `cargo test -p emerald-codegen` | all pass, incl. unchanged `60\n99\n` output | agent-claimed-locally |
| Workspace (real length-read proofs) | `cargo test --workspace` | all pass, incl. `3` and `5` length proofs | agent-claimed-locally |

---

## Leaf: leaf-iterable-interface

### 1. Context
- Why: no container in this compiler can be iterated generically today
  — `for...in` (plan 30) only accepts a literal array at the grammar
  level, and there is no `each` method on anything. This leaf defines
  the interface and wires the three built-in conformances plan 41's
  class-only `implements` mechanism cannot reach on its own (see
  Decision log).
- Target state: `interface Iterable[T] { def each(block: Proc[T,
  Void]): Void }`, written using plan 41's own interface syntax
  verbatim. `emerald-sema` gains: (a) a small intrinsic-conformance
  table consulted by plan 41's bound-checker whenever a bound is
  checked against `Type::Array(_)`, `Type::Hash(_, _)`, or `Type::Range`
  (plan 37) instead of a `ClassInfo`-registered class; (b) a new built-
  in `Type::Pair(Box<Type>, Box<Type>)` with exactly two accessors,
  `.key`/`.value` (reusing plan 33's field-access-sugar dispatch
  mechanism if its shape fits, or a small dedicated accessor path
  otherwise — decide and document whichever this leaf actually lands,
  same discipline every prior leaf uses). `emerald-codegen` gains
  `each`'s three concrete bodies: `Array[Elem]` loops `0..length` (the
  header from `leaf-array-length-header`) calling `block(arr[i])`;
  `Hash[K,V]` loops `0..count` (the header that already exists, per
  `build_hash_lookup`) constructing a fresh 16-byte `Pair` per slot by
  copying straight out of the hash's own buffer (see Decision log) and
  calling `block(pair)`; `Range` loops its two `Int64` endpoints (per
  plan 37's contract) calling `block(i)`.

### 2. Acceptance Criteria
1. `arr: Array[Int64] = [7, 8, 9]` then `arr.each { |x: Int64| puts x }`,
   compiled, linked, and run, prints `7\n8\n9\n`.
2. `h: Hash[Int64, Int64] = {1 => 10, 2 => 20}` then `h.each { |p: Pair
   [Int64, Int64]| puts p.key }` (a second statement printing `p.value`
   is a separate proof point), compiled, linked, and run, prints `1\n2\n`
   for keys and `10\n20\n` for values, in the hash's own existing
   storage order (no sorting guarantee — stated plainly, matching plan
   25's own `O(n)`-and-disclosed precedent).
3. `r: Range = 1..3` (plan 37) then `r.each { |i: Int64| puts i }`,
   compiled, linked, and run, prints `1\n2\n3\n`.
4. Calling `.each` on a value whose type does not conform to `Iterable
   [T]` (e.g., an `Int64` local) is rejected at sema time with a
   diagnostic, not a codegen panic.
5. Regression: `leaf-array-length-header`'s own AC1-AC3 proofs still
   pass unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (`Type::Pair`, the
  intrinsic-conformance table, `each`'s block-type checking), `crates/
  emerald-codegen/src/lib.rs` (three `each` codegen bodies, `Pair`
  construction/accessors).
- **Create:** none — `Pair[K, V]` is not source-constructible directly
  (it only ever originates from `Hash[K,V].each`), so no new literal
  grammar is needed in `crates/emerald-parser`.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. AC4's rejection | agent-claimed-locally |
| Workspace (real Array/Hash/Range each proofs) | `cargo test --workspace` | all pass, incl. AC1-AC3's exact stdout | agent-claimed-locally |

---

## Leaf: leaf-block-return-value

### 1. Context
- Why: `map`/`select`/`reduce` all need a block literal whose value is
  genuinely used and typed; plan 34, as written and already implemented
  (per its own Decision log, re-verified this session), hardcodes every
  block literal's `return_type` to `Void`. See Decision log for the
  full citation.
- Current state: wherever plan 34's grammar builds a block literal's
  `Expr::Lambda { params, return_type, body }` (`crates/emerald-parser`,
  per plan 34's `leaf-ast-blocks`), `return_type` is constructed as the
  literal string `"Void"` unconditionally.
- Target state: `return_type` is instead inferred at sema time from
  `body`'s trailing statement — `Void` if the last statement isn't a
  bare `Stmt::Expr`, otherwise that expression's `infer_expr_type`
  result (reusing the same trailing-expression reasoning `check_
  implicit_return` already applies to ordinary function bodies, per
  `crates/emerald-sema/src/lib.rs`). No new grammar or `Expr::Lambda`
  shape — the AST field itself (`return_type: String`) is untouched;
  only what value flows into it, and when, changes.

### 2. Acceptance Criteria
1. `{ |x: Int64| x * 2 }`, used in a position expecting `Proc[Int64,
   Int64]`, infers `return_type == "Int64"` (a direct sema-level
   assertion, mirroring plan 34's own AC-style parsed/typed checks).
2. A standalone minimal proof independent of `leaf-enumerable-
   functions`'s full `map`: a hand-written two-call program that invokes
   a block literal directly (reusing plan 10's `.call` mechanism against
   a block literal bound to a `Let`, if that shape is reachable, or the
   smallest available real invocation path) and uses its returned
   value, compiled, linked, and run, prints the transformed value —
   proving the inferred, non-`Void` return type is real and usable, not
   just accepted by the type checker.
3. Regression: plan 34's own `repeat`/`yield` example (a genuinely
   `Void`-returning block), compiled, linked, and run, still prints
   `0\n1\n2\n` unchanged.
4. A block literal whose inferred return type disagrees with its call
   site's expected `Proc[..., R]` (e.g., a block ending in a `Boolean`
   expression passed where `each`'s `Proc[T, Void]` is expected but the
   value is then wrongly used) is rejected with a type-mismatch
   diagnostic, not silently coerced or discarded.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` and/or `grammar.
  lalrpop` (wherever plan 34 hardcodes `"Void"` for a block literal),
  `crates/emerald-sema/src/lib.rs` (`infer_lambda_type` or the block-
  literal-specific call path it shares with ordinary lambdas).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-parser -p emerald-sema` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema` | all pass, incl. AC1/AC4 | agent-claimed-locally |
| Workspace (regression + real run) | `cargo test --workspace` | all pass, incl. AC2/AC3's exact stdout | agent-claimed-locally |

---

## Leaf: leaf-enumerable-functions

### 1. Context
- Why: with `Iterable[T]` defined and blocks able to return a used
  value, this leaf is where the seven actual functions (`map`, `select`/
  `filter`, `reduce`/`inject`, `each_with_index`, `count`, `sum`, `sort`)
  and their method-call-syntax dispatch get built.
- Current state: `build_method_call` (`crates/emerald-codegen/src/
  lib.rs`, lines 1562-1655) resolves a call's target function through
  exactly two paths — a module-method lookup (`ctx.module_names`) and a
  `local_classes`-keyed per-class-method lookup (`ctx.method_owners` /
  `"{class}_{method}"`) — with no path at all for a receiver whose type
  is a built-in container (`Array`/`Hash`/`Range`) or for a generic
  function's monomorphized specialization.
- Target state: `build_method_call` gains a third resolution path,
  consulted when the receiver's statically known type (from `local_
  array_elem_types`, a new equivalent side-table for `Hash`/`Range`
  locals, or `local_classes` for a user class implementing `Iterable`/
  `Comparable`) and `method` name together identify one of the seven
  blessed generic functions: resolve to that `(function, concrete type
  argument)` pair's monomorphized compiled symbol (see Decision log's
  naming-convention bullet), falling back to the existing `local_
  classes` per-class-method path first so a class's own hand-written
  method of the same name (if any) still wins — generic functions never
  shadow a class's own methods. `emerald-sema` gains the seven generic
  function signatures (against `Iterable[T]`/`Comparable`/the narrower
  `sum` bound — see Decision log) and reuses plan 41's own bound-
  checking/monomorphization-instantiation bookkeeping to decide, per
  call site, which concrete specialization must exist. `sort`'s
  generic body materializes into a fresh `Array[T]` (via `each` once to
  collect, per Decision log's eager-materialization stance) and runs a
  plain insertion sort over it using per-instantiation comparison
  codegen (native `Expr::Compare` for `Int64`/`Float64`/`String`, a real
  `<=>` method call for a `Comparable`-implementing class) — `O(n²)`,
  stated plainly, the same disclosed-simplicity move plan 25 already
  made for `Hash`'s `O(n)` lookup.

### 2. Acceptance Criteria
1. This plan's own full worked example (`select` then `map` then `sum`
   over `Array[Int64]`, two sequential named-local statements, no
   chaining), compiled, linked, and run, prints `24`.
2. `each_with_index` over `arr: Array[Int64] = [10, 20, 30]` (`arr.
   each_with_index { |x: Int64, i: Int64| puts i; puts x }`, or the
   two-value-block shape this leaf actually lands — document whichever
   is chosen), compiled, linked, and run, prints `0\n10\n1\n20\n2\n30\n`.
3. `sort` over `arr: Array[Int64] = [3, 1, 2]`, compiled, linked, and
   run (iterating the sorted result via the now-real `each`), prints
   `1\n2\n3\n`.
4. `count` over `h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}`
   (`h.count`), compiled, linked, and run, prints `3`.
5. Negative: `sum` invoked where `T` is neither `Int64` nor `Float64`
   (e.g. an `Array[String]`, if `Array[String]` is otherwise
   constructible in this compiler — else the smallest available non-
   numeric `Iterable[T]`) is rejected at sema time with a diagnostic
   naming the unsupported element type, not a codegen panic.
6. Negative: `arr.select { ... }.map { ... }` (an attempted chain,
   receiver of `.map` is a `MethodCall`, not an `Expr::Ident`) is
   rejected — the exact diagnostic `build_method_call` already produces
   today for a non-local-variable receiver, unchanged by this leaf —
   proving the declined-chaining decision is genuinely enforced, not
   silently mis-compiled into reading garbage.
7. Regression: every prior plan's example (all of `hello.em` through
   plan 35's debug-info fixture) still compiles, links, and runs
   identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs` (seven generic function
  signatures, bound checking, monomorphization-instantiation
  bookkeeping reused from plan 41), `crates/emerald-codegen/src/lib.rs`
  (`build_method_call`'s new third resolution path; the seven functions'
  compiled bodies; per-instantiation `sort`/`Comparable` codegen fork).
- **Create:** none anticipated in `crates/emerald-parser` — method-call
  syntax for these seven names is ordinary `Expr::MethodCall`, already
  fully general.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema -p emerald-codegen` | all pass, incl. AC5/AC6's rejections | agent-claimed-locally |
| Workspace (real run proofs) | `cargo test --workspace` | all pass, incl. `24`, the `each_with_index`/`sort`/`count` outputs, and full regression | agent-claimed-locally |
| Lint | `cargo clippy --workspace --all-targets` | clean | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
