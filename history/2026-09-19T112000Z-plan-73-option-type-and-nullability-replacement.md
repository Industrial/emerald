---
name: Option[T] and Nullability Replacement
overview: "Replaces nil/T?/&./||= (plans 25, 43) outright with a real Option[T] ADT (Some(T)/None), built on plan 52's existing enum/pattern-matching mechanism, plus ?./?? operators. Removes nil's current i64-zero sentinel from codegen entirely rather than keeping it as a lower-level escape hatch alongside the new type — this session's explicit decision."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-option-as-builtin-enum
    content: "Define Option[T] using plan 52's ADT mechanism, extended to a generic (single-type-parameter, monomorphized per plan 41/58's existing strategy) enum with two variants, Some(T) and None — the first generic enum this project ships; plan 52's own scope was explicitly non-generic, so this is real new ground for the ADT mechanism, not a mechanical reuse."
    status: pending
  - id: leaf-remove-nil-sentinel-and-t-question-mark
    content: "Remove the nil i64-zero sentinel (runtime/emerald_runtime.c and crates/emerald-codegen's Nil handling, plan 25) and the T? nullable-type sigil (plan 43) from the type system and codegen entirely — every T? in current source becomes Option[T]; every nil literal becomes None; every bare .method on a narrowed non-nil value stays as today's static narrowing, now narrowing Option[T]'s Some(v) pattern-match arm instead of a nil-check."
    status: pending
  - id: leaf-safe-nav-and-coalesce-operators
    content: "Implement ?. and ?? as sugar over Option[T] pattern matching (?. short-circuits to None if the receiver is None, otherwise calls through to Some's inner value; ?? extracts a default when the left side is None) — no new runtime representation, purely a desugaring in sema/codegen onto the same match machinery leaf-option-as-builtin-enum ships."
    status: pending
  - id: leaf-migrate-examples-and-regression-tests
    content: "Migrate nullable_safe_nav.em and every other T?/nil/&./||= usage across examples/ and spec/ to Option[T]/Some/None/?./??; add regression tests for Some/None construction, pattern-match exhaustiveness on Option[T] specifically, and both operators."
    status: pending
isProject: false
---

# Plan 73 — Option[T] and Nullability Replacement

Sable §19: "There is no `null`... `Option[T]`... prevents an entire
class of null-reference errors." Emerald's current nullable-type design
(plan 43, built on plan 25's `nil` sentinel) is a different, lower-level
mechanism — a fixed pointer-nullness check, not a real sum type. This
plan replaces it outright, per this session's explicit decision to not
keep both.

## Concrete proof this plan targets

```ruby
fn find(items: Array[Int64], target: Int64): Option[Int64] do
  for i, x in items.each_with_index do
    if x == target do
      return Some(i)
    end
  end
  None
end

result: Option[Int64] = find([1, 2, 3], 2)

match result do
  Some(idx) do
    puts idx
  end
  None do
    puts -1
  end
end

name: String = result?.to_s ?? "not found"
```

`T?`, `nil`, `&.`, and `||=` do not appear anywhere in this program —
today's grammar has no `Option`/`Some`/`None` and no `?`/`??` operators
at all.

## Decision log

- **Full replacement, not coexistence — restated from inception-2
  because it's the highest-blast-radius part of this plan.** `nil` is
  not a library value today; it is a fixed sentinel `emerald-codegen`
  bakes into every nullable-pointer comparison (plan 43's `build_is_
  null` codegen, cited directly in this session's earlier correctness-
  batch research). Removing it is a real codegen deletion, not an
  additive change sitting next to the old mechanism.
- **`Option[T]` is this project's first *generic* enum.** Plan 52's ADT
  work was explicitly scoped non-generic ("Closed, non-generic `enum`
  sum types" per its own plan-of-plans row). Making `Option[T]` generic
  means extending the ADT mechanism to accept a type parameter and
  monomorphize per concrete `T`, the same strategy plan 41/58 already
  use for generic functions/classes — this is real new compiler work,
  not a trivial instantiation of existing machinery.
- **Relationship to `Result[T,E]` — explicitly unchanged.** `Option[T]`
  models *absence*; `Result[T,E]` (plan 53) models *expected failure*.
  Sable keeps these conceptually distinct and so does this plan — no
  merging, no `Result[T, ()]`-as-`Option[T]` trick.
