2026-09-22T03:00:00Z

---
name: "Iterable[T]/Iterator[T] Investigation — A Real, Pre-Existing Chained-do-Block Bug Found, Full Feature Not Attempted"
overview: "inception-3's own §4.2 recommends a real Iterable[T]/Iterator[T] compiler-recognized interface as the more foundational of its two named language-surface gaps, closing plan 74's own disclosed scope. This session investigated it directly (not from the plan text alone) and found something plan 74/87's own text does not disclose: `ChainCallExpr`'s own recursive grammar rule (`crates/emerald-parser/src/grammar.lalrpop` lines 1200-1215, plan 87), which its own doc comment claims lets a `do...end`-attached chain 'extend through as many links as written,' does NOT actually work — a second chained `do...end` block call after a first one fails to parse in every context tested (bare statement, `Let` RHS, single-line, multi-line). This is a real, reproducible, pre-existing bug, found only by running it, not assumed from the grammar source's own comments. No fix attempted this session — diagnosing and safely repairing a live LALR(1) conflict carries the same real regression risk plan 90's own general-parenless-calls investigation found and declined to attempt blind, and building the actual Iterable[T]/Iterator[T] interface is a separate, larger design question this finding does not by itself resolve."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-diagnose-chain-call-expr-conflict
    content: "Diagnose why `ChainCallExpr`'s own recursive alternative (`<recv:ChainCallExpr> \".\" <method:CallMethodName> <blk:DoBlock>`) never actually fires for a SECOND chained do...end block call, despite compiling without a LALRPOP conflict warning at build time (verified this session — `cargo build -p emerald-parser` produces no shift/reduce or reduce/reduce diagnostic for this rule). Candidates to check first: whether `ChainCallExpr` is inlined/duplicated across multiple parent nonterminals (`PrimaryExpr` line 1110, another at line 1546, a parenthesized-receiver form at 1735) in a way that causes the SECOND `do` token to be routed to a different, non-recursive reduction path than the doc comment assumes; whether `DoBlock`'s own `Stmt*` body can never correctly terminate at the first `end` when followed immediately by another `.method do`, versus greedily/incorrectly re-entering as part of the body somehow. Use LALRPOP's own conflict-diagnostic tooling (or manually enumerate the grammar states) rather than guessing from behavior alone."
    status: pending
  - id: leaf-fix-or-formally-disclose
    content: "Either fix the conflict with a real, `cargo nextest run --workspace`-verified regression test proving `nums.select do ... end.map do ... end.sort()` (or an equivalent) actually parses, type-checks, and runs correctly end to end — or, if a safe fix is not found without disproportionate risk to the rest of the grammar (plan 90's own precedent for when to stop), formally correct `ChainCallExpr`'s own doc comment and plan 87's history entry to disclose the real, narrower behavior (a `do...end`-attached call may bind ONE further non-block call, per lines 1546-1548/1735-1736, but never a second block-attached call), removing the false 'extend through as many links as written' claim."
    status: pending
  - id: leaf-scope-real-iterable-interface
    content: "Separately from the chaining bug above: scope whether a real, compiler-recognized `Iterable[T]`/`Iterator[T]` interface that `Array[T]`/`Hash[K,V]` themselves implement (not merely a user-declarable generic `interface Iterable[T]` a class may opt into, which already exists per plan 88/89's own worked example) is worth building at all, given `check_enumerable_call`'s existing hardcoded dispatch already covers `Array`/`Hash`'s own concrete method set without it. Write a real follow-up plan document (this one is an investigation record, not an implementation plan) if the answer is yes, scoping exactly which of inception-3's two named benefits (a real `for...in` over any user-defined `Iterable`, and a unified `Reader`/`Writer` pair) actually requires the retrofit versus already being achievable some other way."
    status: pending
isProject: false
---

# Plan 192 — Iterable[T]/Iterator[T] Investigation

## Correction (plan 194, 2026-09-23)

**The "real, previously-undisclosed grammar bug" this document reports
below is not a real bug.** Plan 194 re-verified it directly (LALR
state-splitting experiment, immediately falsified — see plan 194's own
Decision log) and found the true cause: this document's own repro,
`nums.select do |x| x % 2 == 0 end.map do |x| x * 10 end.sort()`, uses
**untyped block parameters** (`|x|`). This language's grammar has never
supported that shape — `Param: <name:Ident> ":" <ty:TypeExpr>`
(`grammar.lalrpop`) requires an explicit type on every block parameter,
a restriction already documented in this very repo,
pre-dating this investigation, in `examples/enumerable.em`'s own header
comment ("A block's parameters need an explicit type (`|x: Int64|`, not
a bare `|x|`)"). The parse errors this session attributed to a chaining
conflict were a downstream symptom of that unrelated, already-known,
pre-existing syntax requirement, not evidence of a `ChainCallExpr`
defect. With correctly typed params
(`nums.select do |x: Int64| x % 2 == 0 end.map do |x: Int64| x * 10 end`),
the identical chain shape parses, type-checks, compiles, links, and
runs correctly on the grammar exactly as it stood before this
document was written — confirmed two ways: an existing, already-
passing test in `crates/emerald-parser/src/lib.rs`
(`plan_87_chain_of_at_least_three_do_end_calls_binds_tight_and_chains`,
which chains THREE unparenthesized do-block-attached calls) that this
investigation apparently never ran, and a new end-to-end
compile-and-run regression test plan 194 added
(`crates/emerald-cli/tests/chain_call_do_block_em.rs`). No grammar
change was needed or made. See plan 194's own record for the full
diagnosis, including the disproven cross-tier-LALR-merge hypothesis
this document's own `leaf-diagnose-chain-call-expr-conflict` todo
suggested as a starting point.

inception-3 (`history/2026-09-21T195000Z-inception-3-stdlib-supremacy.md`
§4.2) names `Iterable[T]`/`Iterator[T]` as the more foundational of two
real language-surface gaps this batch should close, ahead of plan 118
and `derive Serializable`, citing plan 74's own disclosed record that
"`Iterable[T]` doesn't exist as a real, compiler-recognized interface
(blocked on `Proc[Args...,Ret]` bracketed-type parsing at the time)."
This session picked up that recommendation and investigated the actual
current state directly, rather than implementing against inception-3's
own text alone — the standing discipline every plan in this batch has
followed since plan 91 ("found only by running it, not assumed from
the plan text").

## What was actually found, verified directly

1. **The bracketed-type-parsing blocker plan 74 cited is stale.**
   `Proc[Args..., Ret]` bracketed-type parsing exists today (plan 89's
   own worked example, verified: `interface Iterable[T]\n  fn map[U](f:
   Proc[T, U]): Array[U]\nend` parses, type-checks, and a user class
   implementing it compiles and runs correctly — `crates/emerald-sema/
   src/lib.rs` and `crates/emerald-codegen/src/lib.rs` both have a real
   `ITERABLE_WORKED_EXAMPLE` test constant proving this end to end).
   So the specific reason plan 74 gave for not building a real
   interface no longer holds.
2. **But `Iterable[T]` is a user-declarable generic interface (plan
   41/88/89's own mechanism), never something `Array[T]`/`Hash[K,V]`
   themselves implement.** Every built-in collection method (`.map`,
   `.select`, `.each`, `.sum`, ...) is dispatched through
   `check_enumerable_call` (`crates/emerald-sema/src/lib.rs`, cc=65) —
   a large, hardcoded, per-method match, not real interface conformance.
   A user cannot write `fn process[T](items: Iterable[T])` and pass an
   `Array[Int64]` to it today; `Array`/`Hash` and a user's own
   `interface Iterable[T]` declaration are two unrelated mechanisms
   that happen to share a name in one worked example.
3. ~~**A real, previously-undisclosed bug: chained `do...end`-attached
   calls do not actually work, despite `ChainCallExpr`'s own doc
   comment (plan 87, `grammar.lalrpop` lines 1185-1215) claiming they
   do.**~~ — **Not a real bug (see the Correction at the top of this
   document).** The repro below used untyped block params (`|x|`),
   already-invalid syntax unrelated to `ChainCallExpr`. Verified
   directly, in every context tried:
   ```
   nums.select do |x| x % 2 == 0 end.map do |x| x * 10 end.sort()
   ```
   fails to parse — `Unrecognized token 'do' found` at the SECOND
   `.map do`, regardless of whether the expression is a bare statement,
   a `Let`'s right-hand side, or written on one line versus several.
   `ChainCallExpr`'s own recursive alternative
   (`<recv:ChainCallExpr> "." <method> <blk:DoBlock>`) is present in
   the grammar source and its own comment states plainly that a chain
   should "extend through as many links as written," but this is not
   what actually happens — `cargo build -p emerald-parser` produces no
   LALRPOP conflict warning for this rule, so whatever silently
   prevents the recursive case from firing is not flagged by the
   generator's own diagnostics; it needs real grammar-state
   enumeration to diagnose, not guessing from surface behavior.
   `.select do...end.sort()` (exactly ONE do-block, followed by exactly
   one further NON-block call) does appear to be what the OTHER,
   separate grammar rules (lines 1546-1548, 1735-1736 — "one more,
   final, non-block-attached call") actually support; a SECOND
   do-block-attached call chained after the first is what fails.

## Decision log

- **Why no fix was attempted this session.** Live LALR(1) grammar
  surgery on a rule that already compiles cleanly (no conflict warning
  to work from) but silently misbehaves at runtime is exactly the kind
  of "looks safe, isn't" grammar work plan 90's own general-parenless-
  calls investigation found and explicitly declined to attempt without
  a full, deliberate scoping pass first — that investigation's own
  concrete finding was that a seemingly small grammar extension
  cascaded into unresolved conflicts across unrelated productions.
  Diagnosing and repairing `ChainCallExpr` deserves the same care, not
  a rushed fix layered on top of an already-long session's worth of
  other changes to this same crate area (grammar.lalrpop was untouched
  by any of plans 91-95, so this would be the first touch to it this
  session, and the riskiest one to rush).
- **Why this is filed as an investigation record, not an implementation
  plan, and why it is numbered 192 rather than slotted into the
  91-191 batch.** It does not implement anything — it corrects the
  record (the stale bracketed-generics blocker, the real chaining bug)
  and hands off two distinct, separately-scoped follow-ups
  (`leaf-fix-or-formally-disclose`, `leaf-scope-real-iterable-
  interface`) rather than merging them into one large, under-scoped
  "build Iterable[T]" leaf the way inception-3's own high-level
  recommendation reads. This mirrors plan 90's own precedent exactly:
  investigate first, document the real finding precisely, implement
  later against a properly scoped plan — not this session, given the
  chaining bug alone needs its own dedicated diagnosis pass.
- **Whether a real compiler-recognized `Iterable[T]` (retrofitted onto
  `Array`/`Hash`) is even worth its own real cost is left open,
  deliberately.** `check_enumerable_call`'s hardcoded dispatch already
  gives every Emerald program `.map`/`.select`/`.each`/etc. on
  `Array`/`Hash` with no interface indirection at all — retrofitting
  those two built-in types onto the SAME generic-interface dispatch
  machinery real user classes use would be a substantial, structural
  compiler change (a monomorphization/dispatch unification, not a
  additive leaf), and inception-3's own two cited benefits (a real
  `for...in` over any `Iterable`, a unified `Reader`/`Writer` pair)
  may or may not actually require it — `leaf-scope-real-iterable-
  interface` hands that scoping question to whichever future plan
  picks this up, rather than this investigation asserting an answer
  it has not verified.
- **Out of scope.** No grammar change of any kind (see above — the
  actual fix, if one exists, is deferred). No new `Iterable[T]`
  compiler-recognized interface (scoping only, not design or
  implementation). No change to `check_enumerable_call`'s existing
  hardcoded dispatch. No retraction of plan 74/87's own history
  entries — this document corrects the record going forward, per this
  project's own "immutable history, corrected in place with the wrong
  assumption left visible" convention (plan 54's own precedent, cited
  directly in plan 93's Decision log this same session), not by
  editing those two files.

## Not yet decided (blocking EXECUTE of either follow-up)

1. Whether the `ChainCallExpr` bug is a genuine LALR(1) grammar
   limitation (unfixable without a larger rewrite, mirroring plan 90's
   own conclusion for parenless calls) or a fixable, narrower mistake
   in how `ChainCallExpr` is wired into its parent nonterminals —
   `leaf-diagnose-chain-call-expr-conflict` must answer this with real
   grammar-state evidence before `leaf-fix-or-formally-disclose` can
   choose between "fix it" and "correct the doc comment instead."
2. Whether `leaf-scope-real-iterable-interface`'s answer is "yes, build
   it" or "no, `check_enumerable_call`'s existing hardcoded dispatch is
   sufficient and a real interface retrofit is not worth its structural
   cost" — genuinely undecided, left to whichever future plan picks
   this up with time to weigh it properly rather than asserted here
   under this session's own time pressure.
