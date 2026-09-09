---
name: Algebraic Data Types and Exhaustive Pattern Matching
overview: "A new top-level `enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)` declaration — a closed, non-generic sum type compiled as an LLVM tagged union — plus a variant-destructuring extension to plan 20's `case`/`when`, with sema performing real exhaustiveness checking over the enum's fixed variant set at compile time."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-parser-enum
    content: "Item::Enum(EnumDef); EnumVariant { name, fields: Vec<String> }; Stmt::Case.arms becomes Vec<(CasePattern, Vec<Stmt>)> with CasePattern::{Values, Variant}; grammar for `enum Name = V1(T1,...) | V2(...) | ...` and `when Variant(b1, b2)`"
    status: pending
  - id: leaf-sema-enums
    content: "Type::Enum(String); EnumInfo registry + global variant-name namespace; construction-site arity/type checking for Expr::Call dispatching to a variant; check_case exhaustiveness over an enum's full variant set, naming missing variants by name; arm-scoped (non-flat) pattern bindings"
    status: pending
  - id: leaf-codegen-tagged-union
    content: "EnumLayout (tag:i64 header + widest-variant payload region, mirroring Hash[K,V]'s header+payload precedent); Expr::Call construction writes the tag and payload; build_case extended with a tag-comparison icmp/brif chain and per-arm field extraction via field_ptr, bindings scoped to the arm's own block"
    status: pending
isProject: false
---

# Plan 52 — Algebraic Data Types and Exhaustive Pattern Matching

This is plan 52 of the 48–57 batch implementing "Beyond the Ceiling" in
full — ten independent sibling plans, each owning one distinct piece of
that follow-up analysis's compiler-implementation and concurrency/ADT
proposal, authored in parallel the same way the 28–35 and 36–47 batches
before it were. Like every plan in those batches, this is post-v1 scope:
it does not touch
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md) or
any other plan file. Depends on **plan 20** (`comments-and-case-when` —
this plan generalizes its real, shipped `Stmt::Case`/`CaseArm` grammar,
verified against source below, not an assumed shape), **plan 08**
(`object-model` — the class/field-layout machinery this plan's tagged
union sits alongside), and **plan 32** (`class-inheritance` — cited
throughout for the deliberate *contrast*: inheritance is this
compiler's one genuinely open, extensible type hierarchy; enums, as
built here, are the opposite by design).

Concrete proof this plan targets:

```ruby
enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)

circle: Shape = Circle(2.0)
square: Shape = Square(3.0)
rect: Shape = Rectangle(4.0, 5.0)

area: Float64 = 0.0
case circle
when Circle(r)
  area = 3.14159 * r * r
when Square(s)
  area = s * s
when Rectangle(w, h)
  area = w * h
end
puts area

case square
when Circle(r)
  area = 3.14159 * r * r
when Square(s)
  area = s * s
when Rectangle(w, h)
  area = w * h
end
puts area

case rect
when Circle(r)
  area = 3.14159 * r * r
when Square(s)
  area = s * s
when Rectangle(w, h)
  area = w * h
end
puts area
```

Expected output: three lines, the printed values of `3.14159 * 2 * 2`
(≈12.566), `3 * 3` (9), and `4 * 5` (20) — `print_f64`'s exact textual
formatting is this plan's existing, unmodified dependency; this proof
only requires each printed number be numerically correct, real,
executed proof that construction, tag storage, and per-variant field
extraction all round-trip correctly for every one of the three variants
and don't corrupt each other's memory.

A **separate, real negative proof** this plan equally targets: the same
`circle`/`square`/`rect` setup, but with the `case` over `circle`
missing its `Rectangle` arm and no `else`:

```ruby
case circle
when Circle(r)
  area = 3.14159 * r * r
when Square(s)
  area = s * s
end
```

is rejected at compile time with a diagnostic naming `Rectangle`
specifically — not a generic "non-exhaustive match" message. This is
the plan's actual payoff: a `case` over an enum that silently ignored
one variant would, in Ruby, either raise nothing (falling through) or
raise at runtime the moment that variant actually showed up; here it
never compiles at all.

## Decision log

- **Runtime representation: a tagged union — an LLVM-level `[tag:
  i64][payload: 8 * max_fields bytes]` struct, generalizing the exact
  "hand-rolled compound type with hardcoded layout" pattern
  `Array[T]`/`Hash[K,V]`/`Pair[K,V]` already established, not a novel
  scheme.** Verified this session against `crates/emerald-codegen/src/
  lib.rs`: `build_hash_lit` (L1642–1692) allocates `8 + pairs.len() *
  16` bytes — an 8-byte `i64` count header followed by a fixed-stride
  payload region — and `field_ptr` (L1719–1731) does every field access
  in this codebase via a byte-offset `i8` in-bounds GEP off a base
  pointer, never a typed struct GEP. Plan 42's own Decision log (this
  batch's sibling for `Iterable`/stdlib) already generalizes this once,
  to `Pair[K,V]`: "a hand-rolled, hard-coded compound `Type` variant...
  with... a fixed 16-byte `[key: 8][value: 8]` layout — deliberately the
  same per-pair byte layout `Hash[K,V]`'s own buffer already uses." This
  plan's `EnumLayout` is the same idea generalized one more step: a
  header field (the tag, replacing `Hash`'s count) followed by a
  byte region sized to the *widest* variant's payload (replacing a
  fixed per-element/per-pair stride), because unlike a `Hash`'s
  uniform pairs, an enum's variants can carry different numbers of
  fields. Per-variant field access is a `field_ptr` byte-offset GEP at
  `8 + i * 8` for field `i`, typed per that variant's own declared
  field kind at the access site — exactly `load_field`/`field_ptr`'s
  existing mechanism (`ClassLayout`'s own `FieldInfo{offset, kind}`),
  reused rather than reinvented.
- **Enums are a CLOSED set of variants, fixed at declaration — the
  deliberate opposite of class inheritance's open, extensible
  hierarchy, and this contrast is the point, not an oversight.** Plan
  32's `ClassDef` can always gain a new subclass from another file via
  `class Dog < Animal`, extending the hierarchy after the fact — that is
  exactly what makes plan 32's dispatch story (open resolution, no
  compile-time-known "complete" set of subclasses) incompatible with
  exhaustiveness checking in principle: there is no such thing as
  "every subclass of Animal" checkable at compile time in an open
  system without whole-program closure assumptions this compiler
  doesn't make anywhere else. An `EnumDef`'s variant list, by contrast,
  is the *entire* type, in one place, forever — there is no grammar
  path (no `enum Shape += Triangle(...)`, no cross-file reopening) by
  which a second file adds a variant to an already-declared enum, and
  this plan deliberately does not let `EnumDef` reuse any part of
  `ClassDef`'s grammar shape that might imply otherwise (no `<`
  superclass-style clause, no method bodies). This closedness is
  exactly what makes `check_case`'s exhaustiveness check *decidable* at
  all — the reason this plan exists as a byte-for-byte contrast to
  plan 32, not a variation on it.
- **Pattern syntax extends plan 20's real, verified `when` production —
  not a new `case`/`in` keyword pair.** Verified this session against
  the shipped `grammar.lalrpop`: `Stmt`'s `"case" <scrutinee:Expr>
  <arms:CaseArm+> <else_body:ElseClause?> "end"` and `CaseArm: "when"
  <values:CaseValues> <body:Stmt*>` (where `CaseValues` is itself
  restricted to a right-recursive list of bare `Num` literals — not a
  general `Expr` list, to avoid a real, already-documented FIRST-set
  conflict with `Stmt*`). This plan adds one alternative `CaseArm`
  production, `"when" <variant:Ident> "(" <bindings:BindingNames> ")"
  <body:Stmt*>`, distinguished from the existing `CaseValues` form by
  its very first token after `"when"` (`Num` vs. `Ident` — no
  overlap), producing `ast::Stmt::Case`'s arms as `(CasePattern,
  Vec<Stmt>)` instead of the old bare `(Vec<Expr>, Vec<Stmt>)`, where
  `CasePattern` is `Values(Vec<Expr>)` (plan 20's original shape,
  wrapped, byte-for-byte unchanged behavior) or `Variant { name:
  String, bindings: Vec<String> }` (this plan's addition). `case`
  itself remains a `Stmt`, not an `Expr` — plan 20's own Decision log
  already established `if`/`case` are statement-only in this AST; this
  plan doesn't relitigate that.
- **Variant construction (`Circle(2.0)`) needs no new grammar
  production at all — it reuses `Expr::Call` because the grammar
  cannot tell the two apart at parse time, and that's the correct,
  forced design, not an economy shortcut.** Verified this session:
  `StmtPrimaryExpr`'s `<name:Ident> "(" <mut args:Args> ")" <blk:
  BlockLiteral?> => Expr::Call(name, args)` (L444–447) is the exact
  same production an ordinary function call already uses; `PrimaryExpr`
  (the non-`Stmt` mirror) has the identical shape. There is no token
  that could distinguish "call a function" from "construct a variant"
  at the point the parser sees `Circle(2.0)` — that distinction is a
  sema-time fact (which registry the name `Circle` resolves against),
  exactly the same cascade `resolve_type` already uses to disambiguate
  a bare type name across primitives / `classes.get` / `Array[...]` /
  `Hash[...]` (L90–134). Sema's construction-site check therefore
  looks `name` up in a new variant-owner registry *before* falling
  through to the existing function-signature lookup; the two are kept
  disjoint by rejecting, at registration time, any variant name that
  collides with a declared function, class, or another enum's variant
  — the same flat, single-namespace discipline `resolve_type` already
  enforces between class names and module names (`other if classes.get
  (other).is_some_and(|c| !c.is_module)`).
- **Exhaustiveness checking is the actual payoff feature, and it is a
  real, disclosed departure from what `case`/`when` has ever checked
  in this compiler before.** Plan 20's Int64 value-match `case` never
  required (and structurally could never require) covering "every
  possible Int64" — that's an unbounded domain. An enum's variant set
  is the first *bounded, fully-known-at-compile-time* domain a `case`
  scrutinee has ever had in this language. `check_case`'s enum branch
  collects every `CasePattern::Variant` name actually matched across
  the `case`'s arms, diffs that set against the enum's full declared
  variant list, and — if any are missing and there is no trailing
  `else` — rejects the program with a diagnostic naming the missing
  variant(s) by name (e.g. "`case` over `Shape` does not cover variant
  `Rectangle`"). This is the concrete mechanism that turns what would
  be a Ruby `NoMethodError`/silently-wrong-branch runtime bug into a
  compile error naming the exact gap — stated here as one of this
  plan's real motivations, not an incidental nicety.
- **Pattern bindings are scoped to their own arm's body only — a real,
  disclosed departure from this compiler's standing flat-scoping
  convention, forced by what a closed match actually means.** Verified
  this session: `emerald-sema`'s `check_begin` (L1287–1298) states its
  own convention outright — "Flat scoping, same as everything else in
  this compiler (plan 07's Decision log) — each typed clause's `var`
  joins the same environment... not a fresh scope" — and every other
  binding this compiler has ever introduced (`Let`, `for`-loop
  variables, `rescue`'s `var`) lives in that one flat, function-wide
  `env`/`vars` map for the rest of the function, never popped. A
  pattern binding is different in kind: `Circle(r)`'s `r` is only
  type-safe, and only meaningful, inside the one arm it was destructured
  for — a later, unrelated arm (or a statement after the `case` ends)
  referencing `r` is referencing a value that was never actually
  produced on that control-flow path. This plan's sema therefore
  type-checks each `Variant` arm's body against a *cloned* extension of
  `env` (bindings inserted into the clone, the clone discarded once
  that arm's body is checked — the outer `env` is never mutated by a
  pattern binding), and codegen mirrors this by inserting each binding
  into `vars` only for the duration of building that arm's LLVM block,
  removing (or restoring the prior) entry immediately after — so a
  stale pointer to a value that doesn't exist outside that basic block
  never lingers for a later arm or statement to accidentally read. This
  is the first genuinely block-scoped binding construct in this
  compiler; the departure is named here specifically so it isn't
  mistaken for an inconsistency later.
- **Generic enums are explicitly out of scope — already decided, cited
  here rather than relitigated.** Per this batch's already-locked
  design decision: plan 41 (`interfaces-and-generics`) declined generic
  *types* altogether ("Type parameters are legal on top-level functions
  only — never on classes/structs... a generic class needs a
  *storage-layout* strategy per instantiation... materially larger than
  monomorphizing one function body"), offering only generic
  *functions*. This plan's `enum Shape = ...` is therefore always
  concrete-payload-typed — `enum Option[T] = Some(T) | None` cannot be
  written, full stop, and this plan does not attempt a workaround. Plan
  53 (`result-type-and-error-propagation`, a separate sibling in this
  same batch) needs a *generic-shaped* `Ok`/`Err` value and gets it by
  making `Result[T, E]` a third hardcoded compiler-native compound type
  — the same non-generic-mechanism pattern `Array[T]`/`Hash[K,V]`/
  `Pair[K,V]` already are — not by routing through this plan's
  user-facing `enum` syntax at all. This plan builds no part of
  `Result`; it is named here only so this plan's own scope boundary is
  legible against it.
- **An enum is pure data — no methods, no `implements`, no behavior
  beyond its fields.** `EnumDef`'s grammar shape has no method-body
  repetition at all (unlike `ClassDef`'s trailing `FuncDef*`) — it is
  not possible to write `enum Shape = ... def area -> Float64 ... end`
  in this plan's grammar. A full algebraic-data-type-plus-typeclass
  system (Haskell's own `data`/`class`/instance-method dispatch, the
  naming precedent this batch already leans on elsewhere) would let an
  enum satisfy an interface and dispatch behavior per-variant; this
  plan does not build that — an enum here is exactly as inert as a
  `struct` (plan 08's non-inheriting record shape), just with a tag and
  a choice of payload shapes instead of one fixed one. Real, disclosed
  future work, not attempted here.
- **Nested/deep pattern matching is out of scope: a variant's payload
  field *may* itself be another enum's (or a class's) value — codegen
  and the type system don't forbid that, since it's just another
  8-byte `Ptr`-kind slot — but a single pattern arm cannot destructure
  *into* it.** `when Outer(Circle(r))` (matching one pattern nested
  inside another) is not a shape this plan's grammar produces at all —
  `BindingNames` is a flat list of plain identifiers, never itself a
  nested pattern. A program that needs to inspect a nested enum value
  writes a second, separate `case` over the binding a first arm already
  produced. Real, disclosed future work.
- **No guard clauses on pattern arms** (`when Circle(r) if r > 0`) —
  `CaseArm`'s grammar ends at `<body:Stmt*>` immediately after the
  binding list closes; there is no room for (and this plan adds no)
  conditional-expression slot between the pattern and the body. A body
  that needs to branch on the bound value's own contents writes an
  ordinary `if` as its first statement instead.
- **Every variant must declare at least one payload field — no bare,
  nullary tag variants (`enum Signal = Red | Yellow | Green`) in this
  plan.** `EnumVariant`'s grammar requires the `"(" TypeNameList ")"`
  suffix unconditionally; this sidesteps a real, otherwise-necessary
  second `CaseArm` alternative for a parenthesis-free pattern (`when
  Red` with no binding list), which would need its own FIRST-set
  disambiguation against `CaseValues`' `Ident`-vs-`Num` split changing
  shape mid-design for no proof value this plan's own worked example
  needs. Real, disclosed future work — the tagged-union layout this
  plan builds trivially supports a zero-field variant (its payload
  region is simply never read for that tag), so extending to nullary
  variants later is additive, not a redesign.
- **A single `when` pattern arm matches exactly one variant — no
  `when Circle(r), Square(s)`-style multi-variant arm**, unlike plan
  20's own `Values` arms (which do allow `when 2, 3`). A multi-variant
  pattern arm would need one body type-checked against two structurally
  different binding sets simultaneously (`r: Float64` from one
  variant, `s: Float64` from another, potentially of different
  arities/types for a less coincidentally-uniform enum) — a real,
  separate generalization this plan's own worked example doesn't need
  and doesn't build.
- **Tooling follow-up, not built here:** plan 21 (`lsp-symbols-and-
  navigation`)'s symbol table will need `enum`/variant names added as
  a new top-level symbol kind for go-to-definition/completion to see
  them at all — a real, disclosed gap in plan 21's existing scope, left
  for whoever next touches `emerald-sema`'s symbol-table surface. Plan
  24 (`tree-sitter-grammar`)'s `grammar.js` will need new
  `enum_declaration`/`variant_pattern` node types added to stay
  grounded in `grammar.lalrpop`'s real terminals (its own stated
  design constraint) once this plan's two new productions exist — also
  not built here, since neither plan's own file is touched by this
  batch's "author, don't implement each other's plans" discipline.

## Leaf: leaf-ast-parser-enum

### 1. Context
- Why: no ADT/pattern-matching syntax exists anywhere in this compiler
  — `Stmt::Case` (plan 20) only ever value-matches `Int64`, and
  `ClassDef` (plan 08/32) has no closed-variant-set concept at all.
- Target state: `EnumVariant { name: String, fields: Vec<String> }`
  (payload field types as raw `TypeName` strings — the exact same
  string-based type-annotation convention `Param.ty` already uses,
  deferring resolution to sema, same as every other type annotation in
  this AST); `EnumDef { name: String, variants: Vec<EnumVariant> }`;
  new `Item::Enum(EnumDef)`. `Stmt::Case.arms` changes from `Vec<(Vec
  <Expr>, Vec<Stmt>)>` to `Vec<(CasePattern, Vec<Stmt>)>`, with
  `CasePattern { Values(Vec<Expr>), Variant { name: String, bindings:
  Vec<String> } }`. Grammar: `EnumDef: "enum" <name:Ident> "="
  <variants:EnumVariants>` (no trailing `end` — a one-line declaration,
  the same "no block needed" shape `Stmt::Let` already has, matching
  this plan's own worked example's literal syntax); `EnumVariants` a
  `"|"`-separated list (mandatory first element, right-recursive on
  `"|"`); `EnumVariant: <name:Ident> "(" <fields:TypeNameList> ")"`;
  `TypeNameList` a left-recursive, trailing-comma list of `TypeName`
  (safe left recursion — always closed by a known `")"`, the same
  reasoning `Params`/`Args`/`HashPairs` already rely on). `CaseArm`
  gains `"when" <variant:Ident> "(" <bindings:BindingNames> ")"
  <body:Stmt*> => (CasePattern::Variant { name: variant, bindings },
  body)` alongside the existing `CaseValues` alternative (now wrapped
  as `CasePattern::Values`), disambiguated from it by the very next
  token after `"when"` (`Num` vs. `Ident`). No new production is added
  for variant construction — see Decision log.

### 2. Acceptance Criteria
1. `enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64,
   Float64)` parses to `Item::Enum(EnumDef { name: "Shape", variants:
   [EnumVariant{"Circle", ["Float64"]}, EnumVariant{"Square",
   ["Float64"]}, EnumVariant{"Rectangle", ["Float64", "Float64"]}] })`.
2. This plan's worked `case circle when Circle(r) ... when Square(s)
   ... when Rectangle(w, h) ... end` parses `arms` as three
   `(CasePattern::Variant{...}, body)` tuples with `bindings` in
   source order (`["r"]`, `["s"]`, `["w", "h"]`).
3. Plan 20's own `CASE_EXAMPLE` (Int64 value-match, `when 1`, `when 2,
   3`, `else`) still parses to an AST equal to before, modulo the
   mechanical `(Vec<Expr>, ...)` → `(CasePattern::Values(Vec<Expr>),
   ...)` wrapper — a real diff-the-AST regression check, not just
   "still compiles."
4. Regression: every prior plan's worked example still parses
   identically; `enum`/`when <Ident>(...)` are new reserved-shape
   productions but introduce no new reserved keyword beyond `enum`
   itself (`when` was already reserved by plan 20).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean — re-verify the `EnumVariants` `"|"`-separator and the new `CaseArm` alternative introduce no shift/reduce conflict against real LALRPOP output, not just the Decision log's reasoning | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new enum/pattern-arm parse tests and the plan-20 AST-equality regression check | agent-claimed-locally |

---

## Leaf: leaf-sema-enums

### 1. Context
- Why: `emerald-sema`'s `Type`/`resolve_type`/`ClassInfo` has no notion
  of a closed variant set; `check_case` only ever validates an
  `Int64` scrutinee against `Int64` `when` values (plan 20); reusing
  this compiler's standing flat-scoping convention verbatim for pattern
  bindings would let a binding from one arm leak into another (or past
  the `case` entirely) — see Decision log.
- Target state: new `Type::Enum(String)`; a new `EnumInfo { variants:
  Vec<(String, Vec<Type>)> }` registry built up front from every
  `Item::Enum`, resolving each field's raw `TypeName` string through
  the existing `resolve_type` (a payload field may itself be `Class
  (_)` or `Enum(_)` — legal data, just not something a pattern can
  destructure into, per the Decision log); a flat `variant_owner:
  HashMap<String, String>` (variant name → owning enum name) built at
  the same pass, rejecting at registration time any variant name that
  collides with a declared function, class, module, or another enum's
  variant. `resolve_type` gains an `Enum` arm alongside its existing
  `Class`/`Array`/`Hash` cascade, with an enum name colliding with an
  existing class/module name rejected the same way. `infer_expr_type`'s
  `Expr::Call` handling gains a check against `variant_owner` *before*
  its existing function-signature lookup: a hit checks `args.len()`
  against the variant's field count (arity diagnostic naming
  expected/actual) and each argument's inferred type against the
  variant's declared field type positionally (mirroring `check_args`'s
  existing per-position diagnostic style), producing `Type::Enum
  (owner_name)`. `check_case` gains a real branch on `scrutinee_ty`:
  `Int64` keeps today's exact behavior (a `Variant` pattern here is
  now a real, new diagnostic: "case over an Int64 scrutinee cannot use
  a variant pattern"); `Enum(name)` requires every arm's pattern to be
  `Variant` (a `Values` pattern here is the symmetric diagnostic),
  checks each pattern's variant name is a real member of `name`'s
  `EnumInfo` (unknown-variant diagnostic), checks each pattern's
  `bindings.len()` against that variant's field count (arity
  diagnostic), checks no variant is matched by more than one arm
  (duplicate-arm diagnostic), then type-checks each arm's body against
  a **cloned** extension of `env` (bindings inserted into the clone
  only — see Decision log) before finally diffing the set of matched
  variant names against `name`'s full variant list: any variant absent
  from that set, with no trailing `else_body`, is rejected by name.
  Any other scrutinee type keeps the existing "must be Int64" rejection,
  its message widened to say "Int64 or an enum type."

### 2. Acceptance Criteria
1. This plan's full worked example (three `case` statements, one per
   `Shape` value) type-checks `Ok(())`.
2. The negative proof — `case circle` with only `Circle`/`Square` arms,
   no `Rectangle`, no `else` — is rejected with a diagnostic naming
   `Rectangle` specifically, not a generic "non-exhaustive" message.
3. A `case` covering all three real variants plus one extra, unknown
   pattern (`when Triangle(a, b, c)`) is rejected naming `Triangle` as
   an unknown variant of `Shape`.
4. A `case` matching the same variant twice (`when Circle(r) ... when
   Circle(r2) ...`) is rejected as a duplicate arm.
5. An arity-mismatched pattern (`when Rectangle(w)`, `Rectangle`
   declared with two fields) is rejected naming expected vs. actual
   binding count.
6. A type-mismatched construction (`Circle("not a float")`, `Circle`
   expects `Float64`) is rejected the same way any other call's
   mismatched argument already is.
7. A pattern binding referenced outside its own arm — inside a
   different arm's body, in the `else` body, or in a statement after
   the `case` ends — is rejected as an undefined-variable diagnostic:
   the concrete, executable proof the scoping departure in the
   Decision log is real and enforced, not merely documented.
8. Two declarations claiming the same name (two enums each declaring a
   variant `Empty`, or an enum variant colliding with an existing
   function or class name) are rejected at registration time, before
   any `case`/construction is even checked.
9. Regression: plan 20's existing Int64 `case`/`when` sema tests, and
   every prior plan's sema suite, pass unmodified.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 9 cases above | agent-claimed-locally |

---

## Leaf: leaf-codegen-tagged-union

### 1. Context
- Why: nothing compiles a discriminated union, a variant construction
  call, or a tag-comparison pattern match to machine code; `build_case`
  today only ever emits an `icmp`/`brif` chain over a raw `Int64`
  scrutinee value itself (plan 20), never over a loaded header field of
  a heap value the way `Hash[K,V]`'s count header already is.
- Target state: `EnumLayout { variant_tags: HashMap<String, u64>,
  variant_fields: HashMap<String, Vec<ValKind>>, size: u64 }`,
  built by `build_enum_layout` independently re-deriving each `EnumDef`
  from the raw `Program`/`Item::Enum` list — the same "no shared
  sema→codegen structure" architecture `build_class_layout` already
  established (plan 32's Decision log) — with `size = 8 + 8 *
  max_variant_field_count` and each variant assigned a discriminant
  equal to its declaration-order index (`Circle = 0`, `Square = 1`,
  `Rectangle = 2`). The existing `local_classes: HashMap<String,
  String>` side-table (already repurposed once, for `Hash[K, V]`
  compound-type strings — see `build_index`'s own comment) carries an
  enum-typed local's enum name the same way. `build_expr`'s `Expr::
  Call` arm gains a check against the enum-layout table *before* its
  existing function-symbol lookup: a hit allocates `layout.size` bytes
  via `ctx.alloc` (the same allocator `build_hash_lit`/`build_array_lit`
  already call), stores the tag at offset `0` via `field_ptr(ptr, 0)` +
  `build_store` (mirroring `build_hash_lit`'s own count-header store),
  then stores each argument at `field_ptr(ptr, 8 + i * 8)` for `i` in
  `0..args.len()`, returning the pointer as `ValKind::Ptr`. `build_case`
  gains a branch: when the scrutinee's resolved type is a known enum
  (not `Int64`), it loads the tag via `build_load` at offset `0`
  (mirroring `build_hash_lookup`'s existing `hashcount` header read),
  then for each `Variant` arm emits the same `icmp eq`/conditional-
  branch chain shape the existing `Values` path already emits — now
  comparing the loaded tag against `layout.variant_tags[name]` — and,
  before building that arm's body, binds each pattern name in `vars`
  via `field_ptr(ptr, 8 + i * 8)` + a typed load (mirroring
  `load_field`), removing (or restoring) that `vars` entry immediately
  after the arm's block finishes building — the codegen half of the
  scoping departure in the Decision log. `Values` arms over an `Int64`
  scrutinee keep today's exact, unmodified code path.

### 2. Acceptance Criteria
1. This plan's worked example — three `case` statements over `Circle`/
   `Square`/`Rectangle` values — compiled, linked, and run, prints the
   three correct computed areas: real, executed proof the tagged-union
   layout, construction, tag comparison, and per-variant field
   extraction are all correct and don't corrupt each other's memory.
2. The `Rectangle(w, h)` two-field case specifically proves multi-field
   extraction at distinct byte offsets — `w` and `h` (and the tag) must
   not alias one another; a deliberate proof this leaf's own test
   doesn't just exercise the single-field `Circle`/`Square` path.
3. A pattern binding's `vars` entry does not survive past its own
   arm's block — verified directly (not just inferred from the sema
   leaf's own AC 7) by compiling a case whose two arms bind different
   names and confirming codegen for the second arm never resolves the
   first arm's binding.
4. An unsupported/malformed shape (e.g., an `Expr::Call` name that
   resolves in neither the enum-layout table nor the function-symbol
   table, which sema's own registration-time collision check should
   already have prevented from reaching codegen at all) returns a
   descriptive `Err`, not a panic — same defensive standard as every
   prior codegen leaf in this crate.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run output for all three `Shape` variants | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **Generic enums** (`enum Option[T] = Some(T) | None`) — locked out of
  scope by this batch's own already-made design decision (plan 41
  declines generic types); plan 53's `Result[T, E]` gets its generic
  shape as a fourth hardcoded compiler-native compound type instead,
  not via this plan's `enum` mechanism. See Decision log.
- **Nested/deep pattern matching** (`when Outer(Circle(r))`) — a
  variant payload may itself be an enum value, but no single pattern
  arm destructures into it; a second, separate `case` is required. See
  Decision log.
- **Guard clauses** (`when Circle(r) if r > 0`) — no conditional slot
  exists between a pattern and its body. See Decision log.
- **Enum methods / `implements` / behavior beyond plain data** — an
  enum in this plan is pure data, structurally incapable of declaring
  methods or implementing an interface; a full ADT-plus-typeclass
  system (in the spirit of the Haskell naming precedent this batch
  already leans on elsewhere) is real, disclosed future work. See
  Decision log.
- **Nullary (bare-tag) variants** (`enum Signal = Red | Yellow |
  Green`) — every variant in this plan requires at least one payload
  field; the tagged-union layout trivially extends to a zero-field
  variant later, but this plan doesn't add the grammar for it. See
  Decision log.
- **Multi-variant single arms** (`when Circle(r), Square(s)`) — unlike
  plan 20's `Values` arms, a pattern arm matches exactly one variant.
  See Decision log.
- **`case`/`if` as expressions returning a value** — plan 20 already
  established `case`/`if` are statement-only in this AST; this plan
  doesn't revisit that.
- **`case`/`in` destructuring against `deconstruct`/`deconstruct_keys`
  protocols** — plan 20 already declined this as its own separate,
  still-undecided `spec/GRAMMAR.md` row; this plan's variant patterns
  are a different, narrower mechanism (closed enum tags, not an
  open protocol), not an implementation of that row.
- **LSP symbol-table and tree-sitter-grammar coverage for `enum`/
  variant-pattern syntax** — real, disclosed follow-up work for plans
  21 and 24 respectively, not built in this plan. See Decision log.
