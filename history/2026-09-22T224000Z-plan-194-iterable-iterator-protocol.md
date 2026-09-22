2026-09-22T22:40:00Z

---
name: "Plan 194 — ChainCallExpr Second-Link Diagnosis (Fix-or-Disclose) + a Reader/Writer Interface Pair"
overview: "Follow-up implementation plan to plan 192's investigation, itself following inception-3 §4.2. Scopes two independent, separately-landable leaves rather than one under-scoped 'build Iterable[T]' leaf: (1) diagnose plan 192's real ChainCallExpr second-link parsing bug via LALRPOP state enumeration and either fix it or formally correct the grammar's own doc comments to disclose the narrower real behavior; (2) design and land a Reader[T]/Writer[T] interface pair (Go io.Reader/io.Writer, Rust std::io::Read/Write as named precedent) using the EXISTING user-declarable generic-interface mechanism (plan 41/88/89) — no dependency on retrofitting Array[T]/Hash[K,V] onto a compiler-recognized Iterable[T]. That retrofit question is deliberately NOT resolved by this plan (see Decision log) — check_enumerable_call's existing hardcoded dispatch is left exactly as-is."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-chaincallexpr-state-enumeration
    content: "Diagnose the real cause of the ChainCallExpr second-link bug plan 192 found (verified again this session, lines cited below): build emerald-parser with LALRPOP's own verbose/report mode (check `lalrpop::Configuration::process_file`'s report-target options wired into this crate's `build.rs`, or fall back to constructing a minimal isolated `.lalrpop` fragment containing only `ChainCallExpr` and its three known embedding sites — Stmt-tier line ~1110, Let-RHS-tier lines ~1546-1548, CondPrimaryExpr-tier lines ~1735-1736 — to get a small enough state table to read by hand) to find which specific state and which specific conflicting action (shift the second `do`, versus reduce the completed `ChainCallExpr \".\" method` via one of the wrapping tier's own non-block 'final call' alternatives at those same three sites) is actually chosen, and why LALRPOP's conflict reporting stays silent about it."
    status: pending
  - id: leaf-chaincallexpr-fix-or-disclose
    content: "Branch on leaf-chaincallexpr-state-enumeration's finding. If fixable without restructuring unrelated productions (mirroring plan 90's own bar for 'safe enough to attempt'): land the grammar fix plus a real regression test proving `nums.select do |x| x % 2 == 0 end.map do |x| x * 10 end.sort()` parses, type-checks, and runs correctly end to end via `cargo nextest run --workspace`. If not fixable without disproportionate risk: correct ChainCallExpr's own doc comment (grammar.lalrpop lines 1185-1199) and the mirroring comment at lines 1105-1109, removing the false 'extend through as many links as written' claim and replacing it with the real, narrower guarantee (a do-block-attached call may bind ONE further non-block call, never a second do-block-attached link) — plus a matching correction to plan 87's own history entry per this project's 'immutable history, corrected in place, wrong assumption left visible' convention (plan 54's precedent)."
    status: pending
  - id: leaf-reader-writer-interface-design
    content: "Author `interface Reader` / `interface Writer` (Emerald source, likely `stdlib/io.em` or wherever plan 41/88/89's own worked generic-interface examples live) mirroring Go's io.Reader/io.Writer and Rust's std::io::Read/Write in shape, not API surface: `Reader#read_chunk(max_bytes: Int64): Result[Array[UInt8], IoError]` (empty array signals EOF, never a sentinel error — matches Go's own documented convention, avoids Rust's more subtle 0-byte-read-is-not-always-EOF ambiguity), `Writer#write_chunk(bytes: Array[UInt8]): Result[Int64, IoError]` (returns bytes actually written, for short-write handling). A `fn copy(src: Reader, dst: Writer, chunk_size: Int64 = 65536): Result[Int64, IoError]` free function loops read_chunk/write_chunk to EOF, usable by any two conforming types."
    status: pending
  - id: leaf-retrofit-existing-streaming-handles
    content: "Mechanical, low-risk per-type follow-up (may be split into separate small plans by whoever implements, per inception-3's own §4.2 sequencing note that this retrofit 'compounds as more streaming plans ship, so it is more valuable done once, later, against a fuller set of handle types'): make `TarReader` (plan 132), `ZipReader` (plan 133), `XmlReader` streaming mode (plan 124), `GzipReader`/`ZstdReader`/`Lz4Reader`/`BrotliReader` (plans 130/131/134/135) each implement `Reader`, and any corresponding writer-shaped handles implement `Writer`. Not required for leaf-reader-writer-interface-design to land — the interface and `copy` are useful the moment any ONE conforming type exists, and this plan does not block on all five landing together."
    status: pending
isProject: false
---

# Plan 194 — ChainCallExpr Second-Link Diagnosis + a Reader/Writer Interface Pair

## Leaf 1 outcome (recorded 2026-09-23): no real grammar bug existed

`leaf-chaincallexpr-state-enumeration` and `leaf-chaincallexpr-fix-or-
disclose` are both resolved, but not the way this document's own
Decision log anticipated. The state-enumeration leaf's own "stronger,
more specific starting hypothesis" — that LALR(1) state merging across
`ChainCallExpr`'s three embedding tiers was silently routing the parser
away from the recursive, chain-extending alternative — was tested
directly and **falsified**: splitting the shared nonterminal into three
distinctly-named, non-mergeable copies (`StmtChainCallExpr`,
`ExprChainCallExpr`, `CondChainCallExpr`, one per tier) changed nothing
— `cargo run -p emerald-cli -- <repro>` still failed with the exact
same three cascading `Unrecognized token` errors, first at the `end`
closing the first do-block. That ruled out cross-tier merging as the
cause and forced a harder look at the repro itself.

The real cause: plan 192's own repro (reused verbatim in this
document's own "Concrete proof" section below and in its `todos`) uses
**untyped block parameters** (`|x|`), which this language's grammar has
never supported — `Param: <name:Ident> ":" <ty:TypeExpr>`
(`grammar.lalrpop`) requires an explicit type on every block parameter.
This exact restriction was already documented in this repo before
either investigation, in `examples/enumerable.em`'s own header comment.
Once the repro is corrected to `nums.select do |x: Int64| x % 2 == 0
end.map do |x: Int64| x * 10 end` (typed params, everything else
identical), it parses, type-checks, compiles, links, and **runs
correctly** — on the grammar exactly as committed before this plan,
with no fix and no grammar change of any kind, reverted back out after
the falsified experiment above.

This was independently reconfirmed by an existing, already-committed,
already-passing test this investigation had apparently not run:
`crates/emerald-parser/src/lib.rs`'s
`plan_87_chain_of_at_least_three_do_end_calls_binds_tight_and_chains`,
which chains **three** unparenthesized do-block-attached calls
(`.select do |x: Int64| ... end.map do |x: Int64| ... end.count do
|x: Int64| ... end`) and asserts the resulting AST end to end —
`cargo test -p emerald-parser plan_87_chain_of_at_least_three_do_end_calls_binds_tight_and_chains`
passes on an unmodified checkout. `ChainCallExpr`'s own doc comment
("extend through as many links as written") was accurate all along.

**What this plan actually lands for leaf 1**, given both original
branches (fix, or narrow the doc comment) turned out to not apply:
a real end-to-end regression test,
`crates/emerald-cli/tests/chain_call_do_block_em.rs`, proving the
exact two-link shape from this document's own "Concrete proof" section
(corrected to typed params) compiles and runs, printing the selected-
then-doubled values (`20`, `40`, `60`, one per line, via `.count` then
`.each`, since `Array[T]` has no `.to_s`/`.join` this session found —
a separate, real, pre-existing gap, out of this plan's own scope to
close). `crates/emerald-parser/src/grammar.lalrpop` is untouched by
this plan — `git diff` against the commit before this session shows no
changes to it. Plan 192's own history entry has its matching false
claim corrected in place (see that document's own "Correction"
section) per this project's "immutable history, corrected in place"
convention (plan 54's precedent) — plan 87's history entry needed no
correction, since its own claim was accurate the whole time.

Picks up exactly where plan 192 (`2026-09-22T030000Z-plan-192-iterable-chaining-investigation.md`)
left off: an investigation record, not an implementation plan, that
found a real, previously-undisclosed grammar bug and left two distinct
follow-ups unscoped. This plan scopes both, makes the retrofit
decision plan 192 explicitly declined to make, and narrows to what is
actually landable without first resolving the harder, structural
Array[T]/Hash[K,V] question.

## Concrete proof this plan targets

**(1) The bug this plan diagnoses and fixes-or-discloses** — re-verified
directly against `crates/emerald-parser/src/grammar.lalrpop` this
session, not assumed from plan 192's own text alone:

```ruby
nums: Array[Int64] = [1, 2, 3, 4, 5, 6]
result = nums.select do |x| x % 2 == 0 end.map do |x| x * 10 end
puts result.to_s
```

Currently fails to parse (`Unrecognized token 'do' found`, at the
second `.map do`) in every context tried: bare statement, `Let` RHS,
single-line, multi-line. Expected output once fixed: `[20, 40, 60]`.
If leaf-chaincallexpr-fix-or-disclose lands the "disclose" branch
instead, the concrete proof becomes a negative test: the above snippet
must produce a real, actionable parse error (not a silent
misparse), and the corrected doc comment must state the one-more-
non-block-call-only guarantee explicitly, matching what
`nums.select do |x| x % 2 == 0 end.length` (currently working, per
grammar.lalrpop lines 1546-1548's own non-block "final call"
alternative) actually supports today.

**(2) The Reader/Writer interface**, once `leaf-reader-writer-interface-design`
lands (a plain user-declarable generic interface — no dependency on
(1) or on any compiler-recognized-by-Array/Hash mechanism):

```ruby
class FixedChunkReader
  implements Reader

  fn initialize(data: Array[UInt8]): Void do
    @data = data
    @pos = 0
  end

  fn read_chunk(max_bytes: Int64): Result[Array[UInt8], IoError] do
    # ... slice @data[@pos, @pos + max_bytes], advance @pos, empty Array at EOF
  end
end

src: FixedChunkReader = FixedChunkReader.new(some_bytes)
dst: SomeWriter = SomeWriter.new()
bytes_copied: Int64 = copy(src, dst)?
```

## Decision log

- **Re-verified plan 192's grammar citation directly, and found it is
  incomplete in a way that matters for diagnosis.** Plan 192 names one
  embedding site (`ChainCallExpr`'s own recursive alternative, lines
  ~1200-1215) plus "lines 1546-1548/1735-1736" as the 'one more, final,
  non-block-attached call' rule. Reading all three sites directly this
  session (Stmt-tier ~1103-1110, Let-RHS/Expr-tier ~1530-1548,
  CondPrimaryExpr-tier ~1720-1736) shows `ChainCallExpr` is embedded at
  **three** separate grammar tiers, and at least two of those tiers
  (Let-RHS/Expr-tier and CondPrimaryExpr-tier) each carry their own
  *additional*, sibling `<recv:ChainCallExpr> "." <method> <r:@R>`
  alternative with **no trailing DoBlock** — i.e., at the exact point
  where the parser has just reduced `ChainCallExpr` and sees a `.`
  followed eventually by a second `do`, there are competing productions
  at multiple tiers that all want to consume the same `ChainCallExpr "."
  method` prefix, only one of which (the recursive alternative inside
  `ChainCallExpr` itself) expects the following `do` to start a new
  block. This is a stronger, more specific starting hypothesis for
  `leaf-chaincallexpr-state-enumeration` than plan 192 left: check
  first whether LALR(1) state merging across these three embedding
  sites is what silently routes the parser toward a reduce path that
  never re-enters `ChainCallExpr`'s own recursive rule, rather than
  assuming the recursive rule itself is unreachable in isolation (it is
  not — read alone, `ChainCallExpr := ChainCallExpr "." method DoBlock`
  is ordinary, unproblematic left recursion).
- **Decided NOT to scope the Array[T]/Hash[K,V] Iterable[T] retrofit
  into this plan, resolving what plan 192 explicitly left open.**
  `check_enumerable_call` (`crates/emerald-sema/src/lib.rs`, cc≈65)
  already gives every Emerald program `.map`/`.select`/`.each`/etc. on
  `Array`/`Hash` today, with no interface indirection. Retrofitting
  those two built-in types onto the same generic-interface dispatch
  machinery real user classes use is a monomorphization/dispatch-
  unification change to the compiler's own type-checking core, not an
  additive leaf — real, but unbounded in this plan's own scope without
  a dedicated design pass this document does not attempt to fake by
  hand-waving a leaf for it. Rather than re-open the question plan 192
  left pending, this plan makes the call: **not now, not in this
  plan** — inception-3's own two cited benefits of a real Iterable[T]
  (a real `for...in` over any user-defined `Iterable`, and a unified
  Reader/Writer pair) turn out, on inspection, to only need the SECOND
  one, and the Reader/Writer interface does not require Array/Hash to
  participate in it at all (see next point). If a future plan later
  finds a real, load-bearing need for `for x in some_user_iterable`
  syntax specifically, that is the point to reopen the retrofit
  question — not before.
- **The Reader/Writer interface needs none of the above, and that is
  the real reason this plan can land it independently.** `TarReader`,
  `ZipReader`, `GzipReader`, and the rest are compiler/runtime-defined
  *handle types*, not `Array`/`Hash` — they can `implements Reader`
  today using the exact same user-declarable generic-interface
  mechanism plan 88/89's own `ITERABLE_WORKED_EXAMPLE` already proves
  works end to end (parses, type-checks, compiles, runs). No grammar
  change, no `check_enumerable_call` change, no dependency on leaf 1 or
  leaf 2 above. This is why `leaf-retrofit-existing-streaming-handles`
  is filed as a separate, non-blocking mechanical follow-up rather than
  folded into this plan's own required scope — landing the interface
  itself unblocks each streaming-handle retrofit independently, exactly
  as inception-3 §4.2 itself recommends ("retrofitted... mechanical,
  low-risk per-plan follow-ups... rather than redesigned into each of
  them today").
- **`read_chunk` returns an empty `Array[UInt8]` at EOF, never a
  sentinel error.** Chosen to match Go's `io.Reader` documented
  contract exactly (a 0-length read is a valid, non-error outcome, and
  callers must check the returned length, not treat `err != nil` as the
  only EOF signal) rather than Rust's `std::io::Read::read` (which
  overloads `Ok(0)` for both EOF and "no data available right now,
  try again" in the non-blocking case — a distinction Emerald's own
  fully-synchronous native-runtime handles, per plan 91-93's own
  blocking-FFI-call model, do not need and should not import).
- **Out of scope.** No change to `check_enumerable_call`. No new
  `for...in`-over-`Iterable` grammar. No retrofit of any specific
  streaming-handle type (left to `leaf-retrofit-existing-streaming-
  handles`, explicitly non-blocking). No async/non-blocking Reader
  variant — this batch's own native runtime is synchronous throughout
  (plan 92's own disclosed scope), and `Reader`/`Writer` inherit that
  the same way every other `emerald-rt`-backed interface in this batch
  does.
