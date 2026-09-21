2026-09-21T19:43:00Z

---
name: General Parenless Calls — Finishing GRAMMAR.md §7's Deferred KEEP Decisions
overview: "GRAMMAR.md §7 already marks 'Parenthesis-less call (`obj.method arg`)' and 'Command call (`puts x`)' as KEEP — but only two narrow slices of that intent are actually implemented today: a zero-arg dotted call (`obj.method`, no arguments) and the single hardcoded `puts` keyword. Plan 07's own Decision log explicitly deferred the general case ('a general `Ident followed-by-Expr` command-call rule and a general `Expr-as-statement` rule are genuinely LALR(1)-ambiguous... Revisit when a second builtin or user-defined command-call syntax is needed') rather than abandoning it. This plan is that revisit: can an arbitrary user-defined function or method be called with arguments and no parentheses (`greet name: \"yo\"`, not just `p.distance_from_origin` or `puts x`), and if the grammar can be made to accept it, should it become this project's canonical, `emerald format`-enforced style. Triggered by a 2026-09-21 session question about general-purpose competitiveness and idiomatic style, not by a user-facing bug report."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-disambiguation-strategy-spike
    content: "Spike two candidate strategies for resolving the real LALR(1) shift/reduce conflict recorded at `crates/emerald-parser/src/grammar.lalrpop:622-626` (an `Ident`-initial command-call production collides with an `Ident`-initial bare-`Expr`-statement production under 1-token lookahead) against LALRPOP's own conflict reporting, and pick one: (A) restrict general command-call syntax to statement-initial position only, mirroring the precedent this exact grammar already uses at `grammar.lalrpop:1010-1022` (plan 09/61's `StmtUnaryExpr` forbids leading `-`/`!` only at Stmt-initial position, while keeping both fully legal everywhere else — inside a `Let` RHS, a call argument, a binary operand); or (B) whitespace-adjacency-sensitive lexing, i.e. Ruby/Crystal's actual mechanism — teach the `logos`-based lexer (`crates/emerald-lexer`) to emit a distinct token depending on whether `(` immediately follows an identifier with no intervening space, so `foo(x)` keeps parsing as today's ordinary call and `foo x` / `foo (x)` parses as a command call. Record which was chosen and why in this plan's own Decision log before touching the grammar file, matching this project's own convention (plan 71, plan 09, plan 61 all did this)."
    status: pending
  - id: leaf-grammar-general-command-call
    content: "Implement the chosen strategy in `grammar.lalrpop`. Verify zero new shift/reduce or reduce/reduce conflicts via a clean `cargo build -p emerald-parser` (LALRPOP reports conflicts at build time, the same verification plan 71 and plan 61 both cite doing). Confirm the existing `puts`-as-keyword production and the existing zero-arg dotted-call production both still parse unchanged — this plan generalizes, it does not replace, either."
    status: pending
  - id: leaf-sema-and-codegen-generalization
    content: "Confirm no `Expr`/`Stmt` AST shape change is needed beyond call-site parsing — GRAMMAR.md:186 already calls parenthesis-less call 'purely a lexical variant' of the same static resolution, and a many-arg command call should desugar to the same `Expr::Call`/`Expr::MethodCall` node an equivalent parenthesized call produces. The one place this needs real re-checking rather than assuming: `emerald-fmt`'s own comment (`crates/emerald-fmt/src/lib.rs:1525-1533`) that a parenless zero-arg dotted call is 'a distinct AST shape' from `recv.method()`, used to reject an intrinsic-type call like `Int64.abs()` written without parens — confirm `emerald-sema` extends this same class-vs-intrinsic distinction correctly once calls can also carry arguments without parens, rather than assuming the existing zero-arg-only check generalizes for free."
    status: pending
  - id: leaf-fmt-canonical-style-decision
    content: "Decide, and record in this plan's Decision log, whether `emerald-fmt` gains enough information to treat a parenless call as canonical output (today it deliberately always re-adds parens — `lib.rs:1525-1533` — specifically because it runs directly off the AST with no sema/type pass and can't otherwise tell a legal parenless class-method call from a rejected parenless builtin-intrinsic call). This is a real architectural question, not a formatting nit: either (a) give `emerald-fmt` a type-aware pass so it can safely decide when parens are omittable, or (b) keep parens-included as the one canonical, fmt-enforced spelling and treat parenless input as legal-but-reformatted, the same way it already treats the zero-arg case today. This leaf's outcome directly determines whether example code rewritten parenless can pass `emerald format`'s own canonicalization check in CI, or whether it would be immediately reformatted back."
    status: pending
  - id: leaf-lsp-assumptions-check
    content: "Check `crates/emerald-lsp` for any completion/hover/goto-definition/semantic-token logic that assumes a call site always has an explicit `(` (e.g. a textual/regex-based heuristic rather than one reading the real parsed AST) and fix any found before the migration leaf below makes parenless calls common in-tree."
    status: pending
  - id: leaf-migrate-examples-and-spec
    content: "Once grammar + fmt land, rewrite call sites across `examples/*.em`, `benchmarks/*.em`, and `spec/*.md` to the new canonical style decided in `leaf-fmt-canonical-style-decision`. Re-run the full example/benchmark suite — `examples/` is explicitly this project's CI-checked source of truth per `README.md` ('every file there is compiled, run, and asserted against its real output on every push'), and plan 71's own record shows a migration scoped only to `examples/`/`benchmarks/`/`spec/*.md` previously missed 85+28 embedded `.em` fixtures baked into `emerald-codegen`/`emerald-driver`/`emerald-cli`/`emerald-mcp`'s own test modules — grep those crates' test modules for embedded `.em` source too, not just the top-level directories."
    status: pending
  - id: leaf-full-gate
    content: "`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`, and the full `moon run :format :check :lint :build :test :audit` pipeline (`AGENTS.md`'s own stated gate) before this is shippable. Independently re-verify beyond any single implementing agent's own self-report, matching this project's established practice (plan 71/73/74 each record an independent re-check beyond the agent's report) — at minimum, compile and run one rewritten example via the real CLI and diff its output against the pre-migration byte-for-byte baseline."
    status: pending
isProject: false
---

# Plan 90 — General Parenless Calls

This plan exists because a 2026-09-21 session asked two questions about
this project directly: how it compares to other general-purpose
languages, and whether a function can be called without parentheses. The
honest answer to the second question turned out to be "partially, and
deliberately not fully" — `spec/GRAMMAR.md` §7 already records the
*design intent* as `KEEP` for both parenthesis-less calls and command
calls, but the *implementation* stopped at two narrow, already-shipped
slices:

- `obj.method` — a zero-argument dotted call, no parens, legal today.
- `puts x` — legal today only because `puts` is a hardcoded grammar
  keyword (`grammar.lalrpop:622-626`), not a generalizable mechanism.

Plan 07's own Decision log named the reason precisely and left an
explicit trigger for coming back to it:

> A general `Ident followed-by-Expr` command-call rule and a general
> `Expr-as-statement` rule are genuinely LALR(1)-ambiguous when both
> start with a bare `Ident`... Reserving `puts` as a literal keyword
> token — the only builtin this compiler has — sidesteps the ambiguity
> entirely. **Revisit when a second builtin or user-defined command-call
> syntax is needed.**

This plan is that revisit. It is scoped to answering, concretely: can
`greet name: "yo"` (an arbitrary user-defined function, called with
arguments, no parens) be made to parse unambiguously, and if so, should
it become the one canonical style `emerald format` enforces everywhere
— which is what "rewrite all example code in this style" actually
requires in this codebase, not just a mechanical find-and-replace.

## Concrete proof this plan targets

```ruby
class Point
  x: Float64
  y: Float64

  fn initialize(x: Float64, y: Float64) do
    @x = x
    @y = y
  end

  fn distance_from_origin: Float64 do
    Math.sqrt @x * @x + @y * @y
  end
end

p: Point = Point.new 3.0, 4.0
puts p.distance_from_origin

fn greet(name: String, times: Int64 = 1): Int64 do
  return times
end

puts greet name: "yo"
puts greet name: "hi", times: 2
```

Every parenless call above except `p.distance_from_origin` and the
outer `puts` is a real parse error under today's grammar — `Point.new
3.0, 4.0`, `Math.sqrt @x * @x + @y * @y`, and `greet name: "yo"` (as an
argument to another call, not a bare statement) all currently require
explicit parens. All of it must parse, typecheck, and run identically
to the parenthesized form once this plan ships, and — pending the
`leaf-fmt-canonical-style-decision` leaf's outcome — either stay
parenless or be canonically reformatted, consistently, under `emerald
format`.

## Decision log

- **This is finishing a deferred decision, not reopening a closed one.**
  `GRAMMAR.md` §7 has said `KEEP` for both rows since plan 01's spec
  foundations; plan 07 shipped only the minimum slice its own milestone
  needed (locals/`if`/`while`/comparisons) and explicitly deferred the
  general mechanism rather than declining it. Framing this as a reversal
  would misdescribe the project's own record.
- **The blocker is a real, load-bearing grammar conflict, not an
  oversight.** `grammar.lalrpop:622-626` is unambiguous about why: two
  productions sharing an `Ident`-initial prefix can't be resolved with
  LALR(1)'s one token of lookahead without more structure. Any fix has
  to add that structure, not paper over the conflict — LALRPOP will
  refuse to build on a real unresolved conflict, which is itself the
  verification method every prior plan in this history used
  (`leaf-grammar-general-command-call` above cites the same check plan
  71 and plan 61 already relied on).
- **A precedent for resolving exactly this class of conflict already
  exists in this grammar, without discarding the feature.**
  `grammar.lalrpop:1010-1022` (plan 09, extended by plan 61) forbids
  leading unary `-`/`!` only at statement-initial position — because
  that position's FIRST/FOLLOW sets collide with the binary-operator
  continuation of the *previous* statement — while keeping unary
  `-`/`!` fully legal everywhere else (a `Let` RHS, a call argument, a
  binary operand). This plan's `leaf-disambiguation-strategy-spike`
  should weigh that same statement-position restriction against real
  whitespace-adjacency lexing (Ruby's and Crystal's actual mechanism)
  before choosing — they are not equivalent: statement-position-only
  keeps a nested parenless call illegal as another call's argument
  (`foo(bar x)` would still need `bar`'s parens even after this plan),
  while whitespace-adjacency is fully general but is a bigger change to
  `crates/emerald-lexer` and reintroduces its own version of the exact
  `foo -bar` unary/binary ambiguity plan 09/61 sidestepped once already
  — for command-call *arguments* specifically this time, not statement
  continuation.
- **`emerald-fmt` already made a deliberate, disclosed choice that
  directly contradicts "write all example code parenless" as stated.**
  `crates/emerald-fmt/src/lib.rs:1525-1533` always re-adds parens to a
  zero-arg dotted call today, on the record, because the formatter runs
  off the bare AST with no type information and can't otherwise tell a
  legal parenless class-method call from a rejected parenless
  builtin-intrinsic call. Rewriting examples parenless without resolving
  this leaf first would either (a) get silently reformatted back to
  parens by the same `emerald format` gate this project's own `AGENTS.md`
  lists as a required check, or (b) require accepting "legal but
  non-canonical" as a permanent state, which this plan should decide
  explicitly rather than let fall out of the fmt implementation by
  default.
- **Out of scope.** Splat call arguments (`method(*args)`) remain
  `UNDECIDED` per `GRAMMAR.md` §7 — unrelated to this plan, not
  resolved by it. `send`/`public_send`/`method(:name)` stay `REMOVE`
  (reflection surface, inception §20) — irrelevant to a purely lexical
  call-site change. This plan does not touch block-argument syntax
  (`method { |x| ... }` / `method do |x| ... end`, already `KEEP` and
  shipped since plan 34) beyond whatever the chosen disambiguation
  strategy requires to keep parsing unchanged.

## Not yet decided (blocking EXECUTE)

1. Statement-position-only restriction vs. whitespace-adjacency lexing —
   `leaf-disambiguation-strategy-spike`'s actual output, not assumed
   here.
2. Whether `emerald-fmt` becomes type-aware or parens-included stays the
   one canonical spelling — `leaf-fmt-canonical-style-decision`'s actual
   output. This second question gates whether "rewrite all example code
   parenless" is even the right final instruction to give
   `leaf-migrate-examples-and-spec`, or whether the canonical target
   turns out to be "parens omitted only where truly unambiguous, parens
   kept everywhere else" — a real possible outcome this plan must not
   prejudge before the spike leaf runs.
