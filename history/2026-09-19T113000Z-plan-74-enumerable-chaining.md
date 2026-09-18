---
name: Enumerable Chaining
overview: "Amends plan 70 in place, the same way plan 70 amended plan 42: plan 70's decision log explicitly declined arr.select { }.map { } in one expression, twice. This session's direction reopens it. Each Iterable[T] method's return type is itself required to implement Iterable, so a chained call sequence typechecks and monomorphizes the same way a single call already does — this is a typing-and-monomorphization extension of plan 70's own mechanism, not a new lazy-evaluation runtime."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-iterable-closure-under-chaining
    content: "Extend plan 70's Iterable[T] interface so that map's Array[U] result, select's Array[T] result, etc. are themselves recognized as Iterable at the type-checking stage, enabling a second .map/.select/etc. call directly on a call's result expression without an intermediate variable binding — verify this composes correctly with plan 41/58's whole-program monomorphization (a chain of N calls monomorphizes N intermediate instantiations, not a single generic pass) and does not require a lazy iterator/enumerator runtime type."
    status: pending
  - id: leaf-parser-support-for-chained-calls
    content: "Confirm (or extend) the grammar's postfix-call/method-chain production supports a block-attached call (plan 70's own block-attachment syntax) immediately followed by another .method(...) or .method { } — today's grammar may already support this structurally for non-block calls; block-attached calls specifically need checking since they're plan 70's own new addition."
    status: pending
  - id: leaf-regression-tests
    content: "A compile_link_run test chaining at least three calls in one expression (e.g. arr.select { }.map { }.sort()) and asserting the correct final value, plus a test confirming plan 70's original non-chained single-call shapes still work unchanged."
    status: pending
isProject: false
---

# Plan 74 — Enumerable Chaining

Plan 70 (written earlier this same session) states plainly: "No
chaining — inherited unchanged from plan 42's original scope... This
avoids needing a lazy-enumerator runtime type or chainable-interface
design this project has never committed to." Sable §38 wants exactly
this chaining ("Chaining should be readable... `users.filter do |user|
... end.map do |user| ... end.sort()`"), and this session's direction
reopens plan 70's decision rather than leaving it standing.

## Concrete proof this plan targets

```ruby
nums: Array[Int64] = [1, 2, 3, 4, 5]

result: Array[Int64] =
  nums
    .select { |x| x % 2 == 0 }
    .map { |x| x * 10 }
    .sort()

puts result[0]   # 20
puts result[1]   # 40
```

Under plan 70 alone, `.select { }`'s result must be bound to a variable
before `.map { }` can be called on it. This plan removes that
requirement.

## Decision log

- **Why this stays a monomorphized, eager pipeline rather than
  introducing laziness.** Plan 70's own reasoning for declining a
  lazy-enumerator runtime type (avoiding a new runtime type category
  this project has never committed to) still holds — chaining and
  laziness are separable concerns, and Sable's own examples (`§38`)
  never demonstrate or require lazy evaluation, only readable syntax
  for a sequence of eager transformations. Each call in a chain still
  produces a fully-materialized concrete value; what changes is only
  that the *next* call in the chain can be written directly against
  that value's expression instead of a named intermediate.
- **Why this is its own plan rather than an edit inside plan 70's own
  file.** Plan 70 is dated and already describes a specific, narrower,
  deliberate scope — rewriting it in place to add chaining would erase
  the historical record of what was actually decided and when,
  the same reasoning plan 70 itself gave for not silently rewriting
  plan 42. This plan supersedes plan 70's no-chaining clause
  specifically, leaving the rest of plan 70's scope (the `Iterable[T]`
  interface, block-attached-call parsing, the core method set) intact
  and unchanged.
