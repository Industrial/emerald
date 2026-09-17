# Emerald — SPEC.md

**Status:** written retroactively at 65 plans in, as the top-level document
inception §4 called for and that was never written. Everything below is a
synthesis of decisions already made and shipped elsewhere (inception,
`GRAMMAR.md`/`TYPE_SYSTEM.md`/`SEMANTICS.md`/`COMPILER.md`/`RUNTIME.md`,
and 65 plan records) — this file introduces no new decisions of its own. It
exists so a reader can get the whole shape of the language from one place
instead of reconstructing it from 65 Decision logs.

## What Emerald is

Emerald is Ruby's surface syntax — classes, blocks, symbols, string
interpolation, exceptions, modules — recompiled onto a fully static type
system, ahead-of-time compiled to native code (or WebAssembly) via LLVM.
It is not a faster Ruby interpreter and does not preserve Ruby's dynamic
runtime. See inception §1 for the founding statement of this; nothing
since has revised it.

## What Emerald deliberately is not

No `eval`, `method_missing`, `send`, monkey-patching, open classes, runtime
reflection, or duck typing. No mixins in Ruby's sense (modules exist as
namespaces; see `SEMANTICS.md`). No garbage collector and no
ownership/borrow checker (`RUNTIME.md` §1) — memory is either
process-lifetime (`emerald_alloc`, never freed) or scope-bounded
(arenas/regions, bulk-freed). These are not gaps to be filled later; they
are inception §2's own scope reduction, restated in `SEMANTICS.md`'s
per-topic decisions and unchanged across all 65 plans since.

## The two pillars

1. **A statically-checkable Ruby.** Every variable, parameter, field, and
   return type is known at compile time (inception §3). Generics are
   whole-program monomorphized, not erased or boxed (plan 41/58).
   Interfaces are nominal, not structural. Pattern matching and algebraic
   data types (plan 52) and a `Result[T,E]` error channel (plan 53) sit
   alongside — not instead of — exceptions (plan 38), as two deliberately
   separate error-handling tools for two different failure categories
   (programmer error vs. expected/recoverable failure).
2. **An Erlang/Pony-inspired concurrency model**, bolted onto the same
   static, natively-compiled foundation: isolated-heap actors (plan 54),
   an N-actors-over-fixed-threads scheduler (plan 55), one compile-time
   linear-use message-safety check (plan 56), `one_for_one` supervision
   trees (plan 57), and location-transparent actors over real TCP with
   automatic consistent-hash cluster placement (plans 60, 65). Full detail
   in `RUNTIME.md` §§2-3, including what's still unproven (the distributed
   examples don't currently build — see `examples/README.md`).

No other language found in this session's landscape research combines
both pillars — see the field-audit artifact from this session for the
sourced comparison against Crystal, Elixir, Gleam, Pony, and Akka.

## Where the language actually stands, measured

- 43.7k lines of Rust across 8 crates; 757 tests passing, 0 clippy
  warnings (this session's own build/test run, not a carried-forward
  claim — re-run `cargo nextest run --workspace` to check it yourself).
- Roughly 10-15% of standard Ruby's language-and-stdlib surface by a
  prior internal estimate (`history/2026-09-08T174011Z-plan-of-plans.md`),
  by design — the 36-47 batch's own stated ceiling, not a target of 100%.
- Two rigorously-measured benchmark programs exist as of this session
  (see `benchmarks/REPORT.md`); most of the concurrency/distribution
  surface above is proven by unit/integration test, not by an end-to-end
  user-facing example, and one specific gap (multi-process distributed
  actors) is proven *not* to work yet, disclosed rather than hidden.

## Reading order for a new contributor

1. This file, for the shape.
2. `GRAMMAR.md` and `TYPE_SYSTEM.md`, for what's legal to write.
3. `SEMANTICS.md`, for what it means once written.
4. `RUNTIME.md`, for what the compiled binary actually does.
5. `COMPILER.md`, for how source becomes that binary.
6. `examples/README.md`, for what's actually proven to work today, as
   opposed to what's merely documented — the two are not the same set,
   and that file is where the difference is tracked honestly.
