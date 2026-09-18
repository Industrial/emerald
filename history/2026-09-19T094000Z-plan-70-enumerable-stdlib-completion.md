---
name: Enumerable Stdlib Completion — Row 42, Finished For Real
overview: "Plan 42's own commit (49f9da8) shipped the array-length-header change and real Array/Hash indexed get/set, but examples/README.md's audit confirms no Iterable[T] interface exists in emerald-sema and a block literal attached to a method call (arr.select { |x| ... }) does not parse — block-attachment syntax only works on a user def that declares &blk (plan 34). This plan is row 42's real, complete scope: Iterable[T], block-attached-call parsing, and monomorphized map/select/reduce/each_with_index/count/sum/sort on Array[T]/Hash[K,V]. No chaining (a.map{}.select{} in one expression) — same ceiling plan 42 originally set."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-block-attached-call-parsing
    content: "Extend the grammar/parser so a block literal can attach directly to a method call on any expression receiver (arr.select { |x| x > 0 }), not only to a bare def call with a declared &blk parameter (plan 34's existing scope). Verify this doesn't regress plan 34's own existing yield/&blk mechanism — both call shapes should coexist."
    status: pending
  - id: leaf-iterable-interface-and-monomorphization
    content: "Define Iterable[T] as a compiler-recognized interface implemented by Array[T] and Hash[K,V] (K,V pairs, per plan 42's original Pair[K,V] precedent), following plan 41's whole-program monomorphization strategy — no vtables, no runtime type tags, consistent with every other generic mechanism this project has shipped (plans 41, 58)."
    status: pending
  - id: leaf-core-enumerable-methods
    content: "Implement map, select, reduce, each_with_index, count, sum, and sort on Array[T] (and the K,V-appropriate subset on Hash[K,V]) as monomorphized, non-chaining calls — each call's result is a concrete, fully-realized value (a new Array[U] for map, an Int64/Float64 for sum, etc.), not a lazy iterator or a chainable enumerator object."
    status: pending
  - id: leaf-examples-and-regression-tests
    content: "A new examples/enumerable.em (or extend an existing collections example) exercising every method above, CI-checked the same way every other example is (crates/emerald-cli/tests/examples.rs); update examples/README.md's coverage table to add this file and remove the corresponding 'Not implemented' bullets about the enumerable stdlib and block-attached parsing."
    status: pending
isProject: false
---

# Plan 70 — Enumerable Stdlib Completion

This is not a new feature — it is plan 42's own original scope,
finished. The plan-of-plans table's row 42 was corrected this session
from `planned` to `in-progress` specifically because its shipping commit
(`49f9da8`) delivered real, useful groundwork (the array-length header
every `Array[T]` now uses, real indexed get/set) but never delivered the
headline capability the row was named for. This plan closes that gap
under its own number rather than silently amending plan 42's history.

## Concrete proof this plan targets

```ruby
nums: Array[Int64] = [1, 2, 3, 4, 5]

doubled: Array[Int64] = nums.map { |x| x * 2 }
evens: Array[Int64] = nums.select { |x| x % 2 == 0 }
total: Int64 = nums.reduce(0) { |acc, x| acc + x }
c: Int64 = nums.count { |x| x > 2 }
s: Int64 = nums.sum
sorted: Array[Int64] = [3, 1, 2].sort

for i, x in nums.each_with_index
  puts i
  puts x
end

puts doubled[4]   # 10
puts evens.length # 2
puts total        # 15
puts c            # 3
puts s            # 15
puts sorted[0]    # 1
```

None of this parses or typechecks today — `arr.select { |x| ... }` does
not parse at all, per `examples/README.md`.

## Decision log

- **Why this is scoped as a full replacement of plan 42's remaining
  work, not a small patch.** The missing piece isn't one method — it's
  the entire block-attached-call grammar production plus the interface
  abstraction every one of these methods needs to be monomorphized
  against. Without `leaf-block-attached-call-parsing`, none of the
  other leaves have a call site to attach to; without
  `leaf-iterable-interface-and-monomorphization`, `map`/`select`/etc.
  would each need one-off, non-generic implementations per concrete
  element type rather than a real `Iterable[T]` abstraction — exactly
  the vtable-free, whole-program-monomorphized style plan 41 already
  established and this project has stuck to since.
- **No chaining — inherited unchanged from plan 42's original scope,
  restated because it's easy to assume otherwise once real `map`/
  `select` exist.** `arr.select { }.map { }` in one expression is not
  supported; each enumerable call's result is a concrete, already-
  materialized value that must be bound to a variable before the next
  call. This avoids needing a lazy-enumerator runtime type or
  chainable-interface design this project has never committed to.
- **`Range` stays excluded, per the existing correction already on
  record.** The plan-of-plans batch intro for 36-47 already documents
  that plan 37 declined a first-class `Range` value; this plan does not
  reopen that — `Iterable[T]` is implemented by `Array[T]`/`Hash[K,V]`
  only, matching plan 42's own already-corrected scope.
- **Relationship to plan 66.** Plan 66 fixes `puts`/String-value-flow
  bugs; several of this plan's worked-example lines (`puts doubled[4]`,
  an `Array[Int64]` index read) are exactly the shape plan 66 targets
  for `Array[String]`. This plan assumes plan 66 ships first — its own
  concrete proof should be re-verified against `Int64` element types
  specifically if plan 66 lands only a partial fix, since `examples/
  README.md`'s original bug report was about `Array[String]` indexing,
  not `Array[Int64]`.
