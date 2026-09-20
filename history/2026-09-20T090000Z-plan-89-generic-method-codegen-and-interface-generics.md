---
name: Generic Method Codegen, Interface Generics, and Indirect Proc Calls
overview: "Plan 88 gave Emerald a real structured type system (nested generics, a written Proc[Args, Ret] form, multi-bound conjunction, sema-level generic-method type-checking) but stopped short of a working Iterable[T], for three real, disclosed reasons found only by actually trying to compile and run the plan's own worked example: (1) InterfaceDef has no type-parameter clause at all and is capped at one method, so `interface Iterable[T]` cannot even parse; (2) a generic method's body is compiled exactly once with its type parameter folded to a raw pointer — real callers crash the LLVM verifier, closed in plan 88 only by a stopgap codegen-level rejection, not a fix; (3) codegen's `.call` dispatch only resolves a receiver that is a literal top-level Let-bound lambda Ident — calling a Proc through any parameter, field, or method-local fails outright. This plan closes all three, which together are what plan 74 (enumerable chaining) actually needs to become real."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-interface-generics-and-multi-method
    content: "Extend InterfaceDef's grammar to accept a TypeParamClause (interface Iterable[T] ... end) and more than one method declaration — today's grammar caps an interface at exactly one method with no type-parameter clause at all, verified directly against the current grammar.lalrpop, not assumed. Extend emerald-sema's interface-implementation checking (the machinery plan 41 already built for a single, non-generic method) to verify a class's implements clause against a generic interface's full method set, substituting the interface's own type parameter with whatever concrete type the implementing class binds it to."
    status: pending
  - id: leaf-generic-method-codegen
    content: "Replace plan 88's stopgap codegen-level rejection (crates/emerald-codegen/src/lib.rs's generic_class_methods gate, added specifically to avoid a crash while this real work was pending) with actual per-call-site monomorphization for class/interface generic methods, reusing the existing top-level generic-function monomorphization pipeline's strategy (a distinct compiled function per concrete type-parameter binding, mangled name, no vtables) rather than inventing a separate mechanism. Remove the stopgap diagnostic once real codegen exists for the cases it was gating -- do not leave both a working path and a dead rejection path for the same shape."
    status: pending
  - id: leaf-indirect-proc-calls
    content: "Extend codegen's `.call` dispatch beyond its current sole recognized shape (a literal top-level Let-bound lambda Ident, resolved via ctx.lambda_func_ids) to a real indirect call through any Proc-typed value -- a function parameter, a method parameter, a field, or a local computed from an expression. This is the mechanism a generic map[U](f: Proc[T, U]) method's body actually needs to call its own f parameter, and is required independently of generic-method monomorphization itself (a NON-generic method taking a Proc parameter and calling it already hits this same gap today)."
    status: pending
  - id: leaf-regression-tests
    content: "Reproduce plan 88's own original worked example for real this time: a full interface Iterable[T] with a generic map[U](f: Proc[T, U]): Array[U] method, implemented by a concrete class, called end-to-end through the real CLI with an actual lambda argument, asserting correct output -- not a substitute test standing in for the real shape. Add a dedicated indirect-Proc-call test independent of generics (a plain, non-generic method taking and calling a Proc parameter) to isolate that capability's own correctness from the generics work layered on top of it."
    status: pending
isProject: false
---

# Plan 89 — Generic Method Codegen, Interface Generics, and Indirect Proc Calls

Plan 88 is real, working, and independently verified — but it closed
one layer of the "why can't we build `Iterable[T]`" question and
uncovered two more sitting directly underneath it, found only by
actually trying to compile the plan's own concrete-proof example
end-to-end rather than stopping once its own explicitly-listed leaves
were done. This plan is those two additional layers, plus the codegen
half of the third (generic methods) that plan 88 deliberately deferred
rather than rushing.

## Concrete proof this plan targets

Plan 88's own original worked example, this time actually compiling,
linking, and running via the real CLI, not substituted for a narrower
analog:

```ruby
interface Iterable[T]
  fn map[U](f: Proc[T, U]): Array[U]
end

class Numbers
  implements Iterable[Int64]

  values: Array[Int64]

  fn initialize(values: Array[Int64]): Void do
    @values = values
  end

  fn map[U](f: Proc[T, U]): Array[U] do
    result: Array[U] = Array.new(0)
    for x in @values
      result = result + [f.call(x)]
    end
    result
  end
end

n: Numbers = Numbers.new([1, 2, 3])
doubled: Array[Int64] = n.map(do |x: Int64|: Int64 x * 2 end)
puts doubled[0]   # 2
puts doubled[2]   # 6
```

Today: `interface Iterable[T]` is a parse error (no type-parameter
clause on `InterfaceDef`); even rewritten around that, `map`'s own
`f.call(x)` inside its body fails codegen (`.call` only resolves a
top-level `Let`-bound lambda, not a parameter); even rewritten around
*that*, the generic method itself hits plan 88's own stopgap rejection.
All three must be fixed for this program to run.

## Decision log

- **Why the stopgap from plan 88 gets removed here, not kept
  permanently.** Plan 88 chose a clean compile-time rejection over a
  compiler crash — the right call at the time, given real monomorphized
  codegen for generic methods is a genuinely separate, larger
  undertaking than "structured type expressions." That reasoning holds
  only until this plan actually builds the real codegen; once it does,
  leaving the old stopgap in place alongside a working path would mean
  two conflicting behaviors for the same input, which is its own kind
  of bug.
- **Why indirect Proc calls are named as their own leaf, not folded
  silently into generic-method codegen.** The gap is independent of
  generics — a plain, non-generic method taking a `Proc` parameter and
  calling it already fails today, checked directly. Fixing it only as
  a side effect of generic-method work would leave that simpler,
  narrower case's own regression test missing, and would make it easy
  to mistake "generics are the reason this doesn't work" for the real,
  separate cause.
- **Why interface generics were not caught by plan 88's own review.**
  Plan 88's own todo list never mentioned `InterfaceDef` at all — its
  worked example assumed interface generics already existed, an
  assumption never checked against the grammar before being written
  into the plan. Naming this plainly rather than treating it as a minor
  addendum: it's a real gap in how thoroughly that plan's own concrete
  proof was verified before being committed to, and part of why this
  plan exists as an explicit correction rather than a quiet follow-on.
- **Relationship to plan 74.** This plan does not itself build
  `Iterable[T]`/enumerable chaining — it makes `Iterable[T]` buildable.
  Plan 74's own dependency on plan 88 (recorded in the plan-of-plans)
  should be read as depending on this plan too; a future editing pass
  should update that row directly rather than leave the dependency
  implicit.
