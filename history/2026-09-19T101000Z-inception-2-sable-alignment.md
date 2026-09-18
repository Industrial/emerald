2026-09-19T10:10:00Z

# Inception 2 — Sable Alignment

Companion to [`2026-09-08T173600Z-inception.md`](./2026-09-08T173600Z-inception.md),
not a replacement for it. The original inception document's engineering
rules (§22 — "reuse mature Rust crates", "keep compiler phases explicit",
"test every semantic rule") and its explicitly-deferred questions (§12 —
"do not design a complicated ownership/borrowing system for v1... first
determine what Ruby-like static semantics require") both still stand.
This document exists because a second design brief —
[`2026-09-19T100000Z-sable-design-brief.md`](./2026-09-19T100000Z-sable-design-brief.md),
a self-contained, deliberately-scoped specification for a
provisionally-named language "Sable" — was brought to this project with
the direction: adopt this as Emerald's north star, keeping the name
Emerald. This document records what that means concretely: what Sable
already matches, what it reverses, and — the one item large enough to
deserve its own inception-style treatment — what it reopens.

## Why a second inception document, not a plan-of-plans batch intro alone

The prior batches (36-47, 48-57, 66-70) each closed a bounded, already-
scoped gap against the *existing* design. This is different in kind: it
changes the design itself — mutability semantics, nullability's core
representation, the memory model's permanence, and the surface syntax of
every executable construct in the language. `inception.md` itself is the
only precedent for a document of this shape in this project; this one
follows its structure (stance, then rationale, then what's explicitly
declined) rather than the shorter batch-intro-paragraph convention.

## Where Sable and Emerald already agree

Checked against current source, not assumed from either document's own
claims:

| Sable principle | Emerald's status |
|---|---|
| `@` instance state | Identical since plan 08 |
| `Result[T,E]` + `?` propagation | Plan 53, shipped |
| Result (expected failure) vs. exceptions (unexpected failure) as two channels | Plan 38 + 53's existing dual-channel model — exact match, not a coincidence (Rust's own `Result`-vs-`panic!` split is the same shape) |
| Algebraic data types, exhaustive matching | Plan 52, shipped — surface syntax differs, see plan 71 |
| No macros, no runtime metaprogramming, no `method_missing`/`eval` | Day-one tenet (inception §19), never wavered |
| `test "..." do ... end` + `assert`/`assert_eq` | Plan 47, shipped |
| `#` line comments | Plan 20, shipped |
| Reuse mature Rust crates | Inception §22 engineering rule #3 — already this project's stated intent, independent of Sable |
| Safe-by-construction concurrency | Emerald's actor model (plans 54-57, 60, 65: isolated heaps, supervision, distributed placement) is **more developed** than Sable's own §32, which is explicit that its `task`/`async` model is still an open question. No action item here — Sable should learn from Emerald, not the reverse. |

This alignment is the actual argument for doing this at all: Sable is
not asking Emerald to become a different language. It's asking Emerald
to finish becoming the language its own error model, its own actor
model, and its own "no metaprogramming" tenet already pointed at.

## The four decisions made this session

1. **Ownership: a full Rust-style borrow checker, not the lighter
   escape-analysis extension.** This is the single largest commitment
   in this document — real lifetimes, real `own`/`borrow` semantics, a
   genuine borrow checker, not an automatic-inference layer over the
   existing region/arena model (plans 50/51). Scoped as its own staged
   arc (plans 82-85 below), starting with a design-only plan, because
   Sable's own §47 states plainly that even its author hasn't settled
   the syntax — only the goal. Building ahead of that design would be
   guessing at a decision Sable's own brief defers.
2. **Grammar migration: one atomic cutover, not staged.** `def`→`fn`,
   `->`→trailing colon, mandatory `do` on `if`/`while`/`for`,
   `case`/`when`→`match`/`do`, and the arrow-lambda literal's deletion
   all land together as one plan (71) rather than as a sequence of
   smaller batches. Real cost named plainly: `def`/`->`/`case`+`when`
   appear at least 47 times across current `examples/*.em` and
   `spec/*.md` alone (not counting `history/`, which is a dated record
   and stays as-is — see "What does not change" below) — every one of
   those needs updating in the same pass this plan ships, or the
   examples corpus and the compiler disagree about the language's own
   grammar for however long the gap lasts.
3. **Nullability: full replacement, not coexistence.** `Option[T]`/
   `Some`/`None`/`?.`/`??` supersede `nil`/`T?`/`&.`/`||=` outright.
   `nil`'s current representation — a fixed `i64` `0` sentinel baked
   directly into codegen (plan 25's design, referenced throughout
   plan 43's nullable-safe-navigation work) — is removed, not kept
   alongside a new type as a lower-level escape hatch.
4. **Enumerable chaining: reopened.** Plan 70 (written earlier this
   same session) explicitly declined `arr.filter{}.map{}` in one
   expression, twice — once as plan 42's original ceiling, once
   restated in plan 70's own decision log. This session's direction
   supersedes that. Plan 74 (below) amends plan 70 in place rather than
   silently duplicating it, the same convention plan 70 itself used
   when superseding plan 42.

Immutable-by-default bindings (`var` required for mutation) came up as
its own Bucket-C item in discussion but is deliberately **not** folded
into the ownership arc (82-85) despite the surface similarity to Rust's
`mut`: unlike aliasing/lifetime analysis, "reject reassignment to a
non-`var` binding" is a simple, local, immediately-shippable static
check with real standalone value — plan 72 ships it now, independent of
whether the borrow-checker design in plan 82 takes months. `mut`-style
interaction with real borrowing (does a `&mut` reference require the
underlying binding to already be `var`?) is deferred to plan 82's own
design scope, which inherits plan 72's shipped semantics as a given
rather than re-deciding them.

## What does not change

- `history/*.md` files already committed stay exactly as they are —
  they are dated records of what was true when written, the same
  standard plan 65's and the correctness batch's own corrections
  applied to `examples/README.md` rather than rewriting history.
  `.em` code samples *inside* older plan documents keep their original
  `def`/`->`/`case`/`when` spelling; only the live corpus (`examples/`,
  `spec/`, the compiler, and future history entries) speaks the new
  grammar going forward.
- Everything in "Where Sable and Emerald already agree" above needs no
  plan at all.
- The two-channel `Result`/exception error model is not touched by the
  `Option[T]` change — `Option[T]` replaces `nil`/`T?` for *absence*,
  not `Result[T,E]` for *failure*. These stay conceptually distinct,
  matching both documents' own stance.

## Plans opened by this document

Full table lives in the plan-of-plans batch this document opens
(`2026-09-08T174011Z-plan-of-plans.md`, "Batch: Sable alignment"). Plans
71-74 and 82 are authored in full alongside this document; 75-81 and
83-85 are recorded as rows only, authored "when their turn comes," per
the plan-of-plans' own stated indexing policy — most of them (doc
comments, a formatter, a linter, property/benchmark syntax, domain
types, import/export) are additive and low-risk, and re-deriving their
exact scope close to when they're actually authored costs less than
speculatively over-specifying them now against a compiler that will
have changed underneath them by then (71-74 alone touch the lexer,
grammar, sema, and codegen).
