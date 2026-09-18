---
name: Grammar Unification — fn, Trailing-Colon Returns, Universal do...end
overview: "The single largest surface-syntax change in this project's history, landing as one atomic cutover per this session's decision: def becomes fn; -> as a return-type marker is deleted everywhere (function decls, interface methods, class methods, and the lambda literal) in favor of a trailing colon, exactly matching the syntax unsafe extern \"C\" fn declarations already use today (plan 59); if/while/for gain a mandatory do; case/when becomes match/do with every pattern-match arm as a block; the arrow-lambda literal (plan 10) is deleted outright in favor of a bare do |params: T| ... end block used directly as an expression; and every local binding requires an explicit type annotation, closing the one place Emerald currently allows an untyped x = expr. Touches the lexer, the LALRPOP grammar, the tree-sitter grammar, every current example, and every current spec doc — nothing about the AST's own shape needs to change, since every one of these is surface syntax over already-existing Function/Stmt/Expr::Lambda/CasePattern nodes."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-lexer-and-token-changes
    content: "Remove the -> token (crates/emerald-lexer/src/lib.rs:35-36, #[token(\"->\")] Arrow) entirely — after this plan, no grammar production references it anywhere, the same way no production will reference the old \"def\" keyword once fn replaces it. Add fn as a reserved keyword token; confirm def is fully removed as a keyword (not left as a silent alias) so `def foo` becomes a real parse error, not a deprecated-but-accepted spelling — no coexistence period, per this session's atomic-cutover decision."
    status: pending
  - id: leaf-grammar-function-and-interface-productions
    content: "Rewrite crates/emerald-parser/src/grammar.lalrpop's four -> productions to trailing-colon form: FuncDef (line 202), InterfaceDef (line 242), the class MethodDef (line 366), and the lambda literal (line 1257) — the lambda's own leading -> introducer is deleted too, not just its return-type arrow; a lambda becomes a bare do |params: T| ... end block usable directly as a PrimaryExpr, typed by its surrounding context (e.g. the fn(Int,Int): Int type annotation on the binding it's assigned to), not by its own literal syntax. Confirm this doesn't introduce a grammar ambiguity against plan 34's existing block-attached-call do...end syntax (arr.select { } already uses brace-blocks per today's grammar, but any do...end block-as-statement construct needs checking against a do...end block-as-expression now existing too) — resolve any real LALR conflict found, don't paper over it."
    status: pending
  - id: leaf-extern-fn-decision-log-note
    content: "Add a note to plan 59's own file (or a superseding comment at ExternFnDecl's grammar site, crates/emerald-parser/src/grammar.lalrpop:260-268) recording that its \"deliberately `:` rather than `->`, visually distinct from an ordinary function signature\" rationale is superseded by this plan — after unification, an extern fn declaration and an ordinary fn declaration's signature line are visually identical except for the surrounding unsafe extern \"C\" { ... } wrapper and the absent do...end body. This is an intended consequence of unification, not an oversight, and should read as one in the historical record rather than as an unexplained contradiction of plan 59's own words."
    status: pending
  - id: leaf-control-flow-and-match-grammar
    content: "Add mandatory do to if/elsif/else (plan 29) and while/until (plan 07) — a bare `if cond` `... end` without `do` becomes a parse error, not an accepted alternate form. Replace case/when (plan 20) with match/do: each pattern (including CasePattern's existing ADT-destructuring forms, plan 52) is followed by do <body> end, and a bare wildcard arm uses `_` matching Sable's own convention rather than reusing whatever case/when's own default-arm spelling was."
    status: pending
  - id: leaf-mandatory-explicit-local-types
    content: "emerald-sema rejects a Let binding with no type annotation (today's `p = Point.new(3.0, 4.0)` shape, used throughout examples/classes.em and elsewhere) with a real diagnostic, not a silent inference. Every local binding requires name: Type = expr going forward. Verify this doesn't collide with plan 31's multiple-assignment form (a, b = b, a) — decide explicitly whether multiple assignment requires both sides pre-declared with types or gets a carved-out exception, and record which was chosen."
    status: pending
  - id: leaf-tree-sitter-and-editor-tooling
    content: "Update tree-sitter-emerald/grammar.js's five -> sites (plan 24) to match the new grammar; check emerald-lsp (plan 21) and the VSCode/Cursor TextMate/tree-sitter highlighting (plan 17, the packaged .vsix from the recent packaging work) for any hardcoded def/->/case/when token assumptions in semantic-token classification, not just the parser."
    status: pending
  - id: leaf-migrate-examples-and-spec
    content: "Update every current .em file under examples/ and benchmarks/, and every current spec/*.md code sample, to the new grammar — at least 47 def/->/case+when occurrences confirmed this session across examples/*.em and spec/*.md alone (history/*.md stays untouched, per inception-2's \"what does not change\" section). Re-run crates/emerald-cli/tests/examples.rs and every benchmark after the migration — a benchmark program that silently stops compiling would corrupt benchmarks/REPORT.md's own numbers without anyone noticing until the next report is written."
    status: pending
isProject: false
---

# Plan 71 — Grammar Unification: fn, Trailing-Colon Returns, Universal do...end

This plan exists because the Sable design brief
(`2026-09-19T100000Z-sable-design-brief.md`) rejects `->` and `def`
outright (§9, §46), mandates `do...end` as the one executable-block
construct across every language feature (§44, §50), and — checked
against `crates/emerald-parser/src/grammar.lalrpop` this session — the
target syntax for return types already exists in this exact codebase
today, just scoped to one construct: `unsafe extern "C" { fn
llabs(x: Int64): Int64 }` (plan 59) already uses a trailing colon, not
an arrow. This plan generalizes what plan 59 already proved compiles
and parses correctly to every function-shaped construct in the language.

## Concrete proof this plan targets

The `examples/classes.em`-equivalent program, fully rewritten in the new
grammar, exercising every changed construct in one file:

```ruby
class Point
  x: Float64
  y: Float64

  fn initialize(x: Float64, y: Float64) do
    @x = x
    @y = y
  end

  fn distance_from_origin: Float64 do
    Math.sqrt(@x * @x + @y * @y)
  end
end

p: Point = Point.new(3.0, 4.0)
d: Float64 = p.distance_from_origin

if d > 4.0 do
  puts "far"
else
  puts "near"
end

square: fn(Int64): Int64 = do |x: Int64|
  x * x
end
puts square.call(5)

shape: Shape = Circle.new(2.0)
match shape do
  Circle(radius) do
    puts radius
  end
  _ do
    puts 0
  end
end
```

Every line above is a real parse/typecheck/codegen error under today's
grammar (`def`, `->`, bare untyped `p = ...`, `case`/`when`, the old
`->(x: Int64) -> Int64 { ... }` lambda literal). All of it must compile
and run correctly once this plan ships, printing `5` (distance,
truncated for the example — `4.0` from a 3-4-5 triangle would be `5.0`
here), `far`, `25`, `2.0`.

## Decision log

- **Why one plan, not several, despite the size.** This session
  explicitly chose atomic cutover over staged migration (inception-2's
  decision #2) specifically because `def`/`->`/`case`+`when` are not
  independent features — they're all read by the same parser off the
  same source files, and a compiler that accepts old syntax in some
  constructs and new syntax in others is a worse intermediate state
  than a bigger single change, for the length of time between "commit
  A changes the grammar" and "commit A also fixes every example that
  broke." No coexistence period is intended; `def foo` is a parse error
  from the moment this plan ships, not a deprecated form.
- **The lambda literal is deleted, not arrow-shortened.** The original
  open question from earlier this session — "what happens to the
  lambda's leading `->`" — is resolved by Sable's own model, not by
  either option floated earlier: Sable has no separate lambda syntax at
  all (§10, §11). A function value is a `do |params: T| ... end` block
  used as an ordinary expression, typed by context. This is a real
  grammar change beyond a cosmetic rename — plan 10's `Expr::Lambda`
  AST node's *surface* syntax changes, but the AST node itself does not
  need to, since it already represents "parameters + body," which is
  exactly what a bare block literal is.
- **`ExternFnDecl`'s "visually distinct" rationale is superseded, named
  explicitly.** Plan 59 chose `:` specifically *because* it looked
  different from `def`'s `->`. After this plan, that difference is
  gone — an extern signature line and an ordinary `fn` signature line
  are the same shape. This is recorded as an intended consequence, not
  quietly overwritten: the `unsafe extern "C" { ... }` wrapper remains
  the real, structural (not merely visual) marker plan 59's own grammar
  already made mandatory at the parser level.
- **Real risk, named rather than hidden: a bare `do...end` block as a
  `PrimaryExpr` may collide with existing `do...end`/brace-block usage
  elsewhere in the grammar** (plan 34's block-attached calls, and
  whatever `if`/`while`'s own newly-mandatory `do` introduces). LALR(1)
  grammars are sensitive to exactly this kind of shared-prefix
  ambiguity; `leaf-grammar-function-and-interface-productions` requires
  checking for and resolving a real conflict here, not assuming none
  exists because the change looks small on paper.
- **Out of scope.** This plan does not touch `Option[T]`/nullability
  (plan 73), does not add `var`/mutability enforcement (plan 72), and
  does not touch enumerable chaining (plan 74) — each is a semantic
  change with its own decision log; bundling them into a syntax-only
  migration would make a syntax regression indistinguishable from a
  semantics regression if something breaks.
