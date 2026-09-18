---
name: Generic Method Repeated-Print Bug
overview: "examples/README.md discloses a real, currently unroot-caused bug found while writing generic_classes.em: two monomorphized Stack[T] instances (Stack[Int64] and Stack[String]) each have .pop() called twice in sequence; the Int64 instance prints both results correctly, but the String instance's second puts silently drops its output — 3 lines out of an expected 4, no crash, no diagnostic. This plan is scoped as pure investigation-then-fix, deliberately kept separate from plan 66's puts/String-value-flow family rather than assumed to be the same root cause on a guess."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-reproduce-minimally
    content: "Reduce generic_classes.em's real failure to the smallest program that still reproduces it — does it require two distinct monomorphized instantiations of the same generic class in one program, or does a single Stack[String] with two sequential .pop() puts calls fail on its own? Does the bug depend on Stack[Int64] being monomorphized first in program order? Answer both before touching codegen."
    status: pending
  - id: leaf-root-cause
    content: "Once minimally reproduced, trace the actual root cause in crates/emerald-codegen/src/lib.rs's monomorphization machinery (per-instantiation method table / trampoline generation, plan 58's own generics implementation) — is a monomorphized method's return-value materialization being reused/aliased incorrectly across the second call, or is this the same puts/String-value-flow issue plan 66 targets wearing a different-looking symptom? State the finding plainly either way rather than assuming plan 66's fix incidentally closes this one."
    status: pending
  - id: leaf-fix-and-regression-test
    content: "Fix the real root cause found above and add a compile_link_run test reproducing generic_classes.em's exact original shape (two monomorphized instantiations, two sequential pops each) asserting all 4 expected lines print, not just the previously-passing 3."
    status: pending
isProject: false
---

# Plan 68 — Generic Method Repeated-Print Bug

`examples/README.md` is explicit that this was "left as a genuine open
finding rather than a guessed explanation" — this plan's job is to
close that gap honestly, which means real investigation before any fix,
not a plausible-sounding patch applied to the first reproduction found.

## Concrete proof this plan targets

The exact shape `examples/README.md` already describes, reduced to a
minimal standalone program (the investigation leaf may reduce it
further once the actual trigger condition is known):

```ruby
class Stack[T]
  items: Array[T]

  def initialize -> Void
    @items = Array.new(0)
  end

  def push(x: T) -> Void
    @items = @items + [x]
  end

  def pop -> T
    last = @items[@items.length - 1]
    @items = @items.slice(0, @items.length - 1)
    last
  end
end

ints: Stack[Int64] = Stack[Int64].new
ints.push(10)
ints.push(20)
puts ints.pop()   # 20
puts ints.pop()   # 10

strs: Stack[String] = Stack[String].new
strs.push("first")
strs.push("second")
puts strs.pop()   # "second"
puts strs.pop()   # "first" — currently silently dropped
```

Expected output, 4 lines: `20`, `10`, `second`, `first`. Today: 3 lines
— the final `first` never prints.

## Decision log

- **Why this is its own plan, not folded into plan 66.** Both plans fix
  a symptom that looks like "a `puts` silently drops output," but plan
  66's five cases are all traceable to `puts`'s own Stmt-level special
  casing not covering certain argument-expression shapes — a codegen
  dispatch problem. This bug's suspect surface is different:
  monomorphization (plan 58) generates a distinct compiled instantiation
  per concrete type argument, and the failure is specific to which
  instantiation (`Stack[String]`, the *second* one monomorphized in
  program order) and specific to the *second* call in a sequence on
  that instance. That shape — first call fine, second call on a
  specific, non-first instantiation broken — smells like generated-code
  aliasing or trampoline reuse across instantiations, not an argument-
  shape dispatch gap. Treating it as the same bug as plan 66 without
  checking would risk plan 66 shipping, this bug staying open, and the
  batch's own completion criterion (`examples/README.md`'s "Real bugs
  found" section coming back empty) declaring victory prematurely.
  `leaf-root-cause` explicitly requires stating whether the two turn
  out to be related after investigation, not assuming either way now.
- **Minimal-repro leaf comes before root-cause leaf, deliberately.**
  `examples/README.md`'s own reproduction is a full generic-stack
  example with two instantiations; whether the bug needs both
  instantiations present, or just a second sequential call on any one
  `Stack[String]`, changes where in the monomorphization pipeline the
  bug plausibly lives. Skipping straight to root-causing the original,
  larger repro risks fixing a symptom of a narrower true cause.
