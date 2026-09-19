---
name: Block-Attached Call — do...end as an Alternative to Braces
overview: "Plan 70 shipped block-attached enumerable calls using brace syntax only (arr.select { |x| ... }). This session's do...end-unification direction (plan 71) makes do...end the primary way executable code is written in Emerald; the explicit decision on braces specifically was to keep Ruby's classic dual convention (braces for short/single-expression blocks, do...end available for longer/multi-line ones) rather than remove braces outright. That decision requires a real, small grammar addition this plan makes: arr.select do |x| ... end must become a second, equally valid spelling for the exact same block-attached-call sites plan 70 already built, not a replacement for the brace form."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-do-end-block-attachment-grammar
    content: "Add a do...end alternative to plan 70's existing brace-only block-attached-call productions in crates/emerald-parser/src/grammar.lalrpop (both the bare .method { blk } and .method(args) { blk } forms, and their StmtPrimaryExpr/PrimaryExpr counterparts) — same semantics, same post-parse hoist-to-named-Proc rewrite plan 70 already built (hoist_enumerable_blocks in crates/emerald-parser/src/lib.rs), purely an alternate surface delimiter. Must be attempted only after plan 71's grammar.lalrpop rewrite has landed and is stable — both touch the same file, and plan 71 was already in flight when this plan was authored specifically to avoid a concurrent-edit collision."
    status: pending
  - id: leaf-precedence-check
    content: "Confirm (or fix, if not already handled by plan 71's own do...end-as-expression work) that .method do |x| ... end binds correctly when the call itself is an argument to something else, or is followed by further chaining (plan 74, once unblocked) — Ruby's own do...end deliberately binds looser than braces at exactly this kind of call site, and Emerald should decide explicitly whether to replicate that precedence difference or make the two delimiters fully interchangeable with no precedence distinction, documenting whichever is chosen."
    status: pending
  - id: leaf-examples-and-regression-tests
    content: "Add at least one example exercising the do...end form for the same methods examples/enumerable.em already covers with braces (map/select/reduce/etc.), verifying identical output to the brace form; regression test asserting both delimiters produce the same compiled behavior for an identical block body."
    status: pending
isProject: false
---

# Plan 86 — Block-Attached Call: do...end as an Alternative to Braces

This plan exists because of a real, concrete inconsistency surfaced mid-
session: plan 70 (block-attached enumerable calls) shipped brace-only
syntax, while this session's do...end-unification direction (plan 71,
citing Sable §44/§50's "do...end is not merely syntax... the central
abstraction") makes do...end the default way executable code is written
everywhere else in the language. The explicit decision made when this was
raised: keep both delimiters, Ruby-style — braces for short blocks,
do...end available for longer ones — rather than remove braces outright.
That decision is not free; it requires this plan's grammar addition,
not just a documentation note.

## Concrete proof this plan targets

Both forms compiling and running identically:

```ruby
nums: Array[Int64] = [1, 2, 3, 4, 5]

doubled_braces: Array[Int64] = nums.map { |x| x * 2 }

doubled_do_end: Array[Int64] = nums.map do |x|
  x * 2
end

puts doubled_braces[0]   # 2
puts doubled_do_end[0]   # 2
```

Today, only the first form parses; the second is a real parse error.

## Decision log

- **Why this waits on plan 71.** Both plans touch
  `crates/emerald-parser/src/grammar.lalrpop`. Plan 71 was already
  executing (an atomic grammar cutover across the whole file) when this
  gap was found; dispatching a second concurrent grammar edit against
  the same file would risk exactly the kind of collision this session
  has deliberately avoided throughout (see the correctness batch's own
  file-ownership discipline). This plan is authored now, precisely so
  it's ready to execute the moment plan 71 lands and the file is stable
  again, rather than being designed from scratch afterward.
- **Additive, not a replacement — restated because it's easy to
  over-correct given how strongly this session leaned into do...end.**
  The explicit decision was dual-delimiter, matching real Ruby
  convention, not do...end-exclusivity. This plan's own concrete proof
  requires both forms to keep working, so a future review can catch a
  well-intentioned but wrong "just replace braces" implementation.
- **Precedence is named as an open question, not assumed.** Ruby's own
  `do...end` binds looser than `{ }` specifically to avoid ambiguity at
  certain call sites (famously, `method a, b do ... end` vs `method a, b
  { ... }` can attach to different things). Emerald's grammar is not
  Ruby's, and whether that exact distinction needs replicating here is
  a real decision this plan must make and record, not inherit silently
  by assumption.
