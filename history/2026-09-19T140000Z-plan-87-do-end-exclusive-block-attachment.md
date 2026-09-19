---
name: do...end Exclusive — Braces Removed as Block Syntax
overview: "Supersedes plan 86 in place, the same way plan 70 superseded plan 42: the decision when plan 86 was authored was to keep Ruby's dual brace/do...end convention; the decision now is that do...end must be the one and only way to express a block in Emerald, no exception. This plan removes plan 34's `{ |params| body }` and plan 70's `.method { blk }`/`.method(args) { blk }` entirely, replacing both with `do |params| ... end` exclusively, and resolves the two real ambiguities that removal surfaces: a while/if condition whose own expression takes an attached block (resolved via mandatory parens) and do...end's traditionally loose binding breaking plan 74's chaining ambition (resolved via a scoped, chain-local tight-binding rule). Must execute strictly after plan 71's in-flight grammar.lalrpop rewrite lands — both touch the same file."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-remove-brace-block-literal
    content: "Delete plan 34's `{ |params| body }` block-literal production and plan 70's `.method { blk }`/`.method(args) { blk }` block-attached-call productions from crates/emerald-parser/src/grammar.lalrpop, in both `StmtPrimaryExpr` and `PrimaryExpr` positions. Replace each with the equivalent `do |params| ... end` form, reusing the exact same desugaring (`Expr::Lambda` construction, plan 70's `hoist_enumerable_blocks` post-parse pass) — this is a delimiter swap, not a semantic change. `HashLit`'s own bare `{ key: value }` syntax is unaffected and untouched."
    status: pending
  - id: leaf-condition-position-parens
    content: "A while/if condition expression that itself contains a block-attached call must be wrapped in explicit parens to be legal — e.g. `while (arr.each do |x| ... end) do ... end`, never a bare `while arr.each do |x| ... end do ... end`. Implement this as a real grammar-level restriction (the condition-expression nonterminal used by `if`/`while` does not itself admit an unparenthesized block-attached call as its top-level form) with a real, specific parse diagnostic for the bare/disallowed case — not merely documentation advising against writing it, and not a silent misparse."
    status: pending
  - id: leaf-chain-position-tight-binding
    content: "A do...end block immediately followed by a `.method` call binds to the call it is attached to (tight, chain-local), not loosely outward to an enclosing statement — this is a scoped precedence rule, not a full adoption of Ruby's global do...end-is-always-loose convention, since Emerald has only one delimiter now and needs at least this much tightness for plan 74's chaining to be expressible at all once Iterable[T] unblocks it. Document precisely how far \"chain-local\" extends (e.g. does it apply transitively through three or more chained calls, and does it change if the chain is itself wrapped in parens) rather than leaving the boundary implicit."
    status: pending
  - id: leaf-migrate-existing-usages
    content: "Migrate every existing brace-block usage to the new exclusive do...end form: plan 34's own worked example, and examples/enumerable.em plus any other example exercising plan 70's block-attached calls. Re-verify plan 70's own disclosed narrowings (no arbitrary expression receiver, only `Ident`; `Hash[K,V]`'s `.select`/`.filter` never shipped) still hold unchanged — this plan does not attempt to lift them, only to change the delimiter of what already exists."
    status: pending
  - id: leaf-regression-tests
    content: "Cover both directions: successful do...end attachment in statement position, expression position, and a chained sequence (three calls minimum); and the disallowed case — a bare, unparenthesized block-attached call used as a while/if condition — asserting a real compile error, not a hang, a silent misparse, or an unrelated error message."
    status: pending
isProject: false
---

# Plan 87 — do...end Exclusive: Braces Removed as Block Syntax

Plan 86's own file is not rewritten by this plan — it stands as the
historical record of the decision actually made when the brace-vs-
do...end question was first raised (keep both, Ruby-style). That
decision was superseded before plan 86 was ever executed: `do...end`
must be the one and only way to express a block in Emerald, full stop.
This plan is that decision, made concrete.

## Concrete proof this plan targets

```ruby
nums: Array[Int64] = [1, 2, 3, 4, 5]

doubled: Array[Int64] = nums.map do |x|
  x * 2
end

# Chaining: the do...end block attaches to .select, not loosely outward.
evens_doubled: Array[Int64] = nums
  .select do |x|
    x % 2 == 0
  end
  .map do |x|
    x * 10
  end

# A block-attached call used as a while condition MUST be parenthesized.
while (nums.select do |x| x > 100 end).length > 0 do
  puts "unreachable for this data"
end
```

Today, `{ |x| ... }` is the only working spelling for any of this; a bare
`do |x| ... end` attached to a call is a parse error. After this plan,
the reverse is true — `{ |x| ... }` is a parse error, `do...end` is the
only legal form, chaining works, and the condition-position case above
is legal only because of its explicit parens; the unparenthesized form
is a real, specific compile error.

## Decision log

- **Braces existed because of a real, already-documented fight with
  Hash literals — this plan's removal of them is a genuine
  simplification, not just a stylistic swap.** Plan 34's own decision
  log states plainly that attaching an optional block to a call created
  "a real, build-verified LALR(1) conflict... a genuine shift/reduce
  ambiguity on `{`" against `HashLit`'s own `{ key: value }` syntax —
  the reason block-attachment was ever restricted to statement-initial
  position in the first place. Removing `{ }` as block syntax removes
  that conflict outright, since there is no competing `do`-spelled Hash
  literal to collide with.
- **A new ambiguity appears in its place, and this plan resolves it
  structurally rather than by convention.** With `do` now mandatory for
  `if`/`while`/`match`/every function body (plan 71) *and* the only
  block-attachment delimiter, `while some_call() do ... end` is
  genuinely ambiguous — Ruby's own real answer to this exact problem is
  giving `{ }` and `do...end` different binding precedence specifically
  so they never collide in this position; that escape hatch does not
  exist once there is only one delimiter. The decision made here is to
  require explicit parens around any condition expression that itself
  carries a block-attached call, resolving the ambiguity at the grammar
  level (a real, enforced restriction with its own diagnostic) rather
  than leaving it to programmer discipline or documentation.
- **Chaining needs a real, scoped precedence rule, not full Ruby
  looseness.** Braces bound tightly in Ruby specifically so a block-
  returning call could sit embedded inside a larger expression or a
  chain without floating loose. Making `do...end` fully loose
  everywhere (Ruby's actual global rule) would break plan 74's whole
  premise before that plan even starts. The rule adopted here is
  narrower and more mechanical than Ruby's: tight binding applies only
  when a `do...end`-attached call is immediately followed by another
  `.method` call — a chain-local rule, not a general re-litigation of
  where `do...end` binds everywhere in the grammar.
- **Sequencing, restated because it matters for execution, not just
  planning.** This plan touches the exact same file plan 71 is
  rewriting right now. It is authored and ready the moment that lands,
  the same posture plan 86 was authored under — but must not begin
  executing until plan 71's own grammar changes are committed and
  stable, for the same collision reasons already established this
  session.
