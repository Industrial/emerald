---
name: Immutable-by-Default Bindings
overview: "Reverses plan 31's shipped bare-local reassignment: a plain name: Type = expr binding becomes immutable, and reassigning it is a compile error. Mutation requires var name: Type = expr. This is a standalone, immediately-shippable static check — it does not require the ownership/borrow-checker design (plan 82), which is scoped separately and inherits this plan's semantics as a given rather than re-deciding them."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-var-keyword-and-grammar
    content: "Add var as a reserved keyword marking a binding mutable at declaration (crates/emerald-parser/src/grammar.lalrpop's Let/variable-declaration production); a binding declared without var is immutable by default. Fields (class instance state, @x) are a separate question — decide explicitly whether class fields default to mutable (matching today's shipped @x = value reassignment in every example) or also require an explicit mutable marker, and record which was chosen rather than leaving it implicit."
    status: pending
  - id: leaf-sema-reassignment-check
    content: "emerald-sema tracks, per local binding, whether it was declared with var; a later Stmt::Assign (or plan 31's compound-assignment forms, += etc.) targeting a non-var binding is a real diagnostic (\"cannot reassign immutable binding `x`, declared without `var`\"), not a silent accept. Loop induction variables (for i in ...) and function parameters need an explicit ruling: can a parameter be reassigned inside its own function body without var, or does this rule apply there too?"
    status: pending
  - id: leaf-migrate-examples
    content: "Every current example/spec sample that reassigns a bare local (grep for plan 31's own multiple-assignment and compound-assignment examples specifically) gets var added or is rewritten to avoid the reassignment, whichever reflects the actual intent of that example."
    status: pending
  - id: leaf-regression-tests
    content: "A compile_link_run test proving both directions: a var binding reassigns successfully; a non-var binding's reassignment is rejected at compile time with a real diagnostic, not accepted or silently miscompiled."
    status: pending
isProject: false
---

# Plan 72 — Immutable-by-Default Bindings

Sable §5: "Bindings are immutable by default... mutable bindings use
`var`... This makes mutation visually apparent." Emerald's plan 31
shipped the opposite: any bare local can be reassigned freely, Ruby-
style. This plan reverses that.

## Concrete proof this plan targets

```ruby
name: String = "Alice"
name = "Bob"          # compile error: cannot reassign immutable binding

var count: Int64 = 0
count = count + 1      # legal
puts count             # 1
```

Today, both reassignments compile and run identically. After this
plan, the first is a real compile-time diagnostic; the second is
unchanged.

## Decision log

- **Why this doesn't wait for the ownership/borrow-checker design
  (plan 82).** Rust's `mut` and Sable's `var` look similar, but
  "reject reassignment to a binding not marked mutable" needs no
  aliasing or lifetime analysis — it's a local property of one binding,
  trackable with the same kind of symbol-table bookkeeping plan 21's
  `emerald-sema` symbol table already does. Plan 82 inherits this
  plan's shipped semantics as a given when it designs how `&mut`-style
  borrowing interacts with mutability, rather than the two being
  designed together and this plan waiting on the much larger one.
- **Fields are called out as an open ruling, not silently decided.**
  Every existing example (`classes.em`, `class_inheritance.em`, the
  actor examples) reassigns `@field` inside `initialize` and ordinary
  methods with no `var`-equivalent marker today. Sable's own brief
  doesn't address instance-field mutability explicitly (its examples
  only ever assign `@field` once, in `initialize`). Silently making
  fields exempt from this rule would be an unstated, easy-to-miss
  exception; `leaf-var-keyword-and-grammar` requires a real, recorded
  decision instead.
