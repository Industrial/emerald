---
name: puts and String Value-Flow Codegen — Fix the Silent-Failure Family
overview: "Five distinct, already-disclosed bugs in examples/README.md's \"Real bugs found\" section share a common shape: a `puts` of some String-producing expression compiles, runs, exits 0, and prints nothing (or drops output partway through a sequence) — no diagnostic, no crash. `puts` is special-cased in codegen at the `Stmt::Expr` level (a single match arm gated on `name == \"puts\" && args.len() == 1`, crates/emerald-codegen/src/lib.rs:10119) rather than treated as an ordinary call into a String-materializing expression path usable from any codegen context. This plan root-causes whether the five symptoms share that one structural cause or are several unrelated bugs wearing the same visible symptom, then fixes what it finds, then regression-tests every one of the five shapes individually so a future silent regression fails loudly instead of shipping quietly again."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-root-cause-investigation
    content: "Trace each of the five symptom sites through emerald-codegen and emerald-sema to determine whether they share one structural cause (puts's Stmt-level special-casing at lib.rs:10119 not being reachable/composable from every context a String value can flow through) or are independent bugs: (1) puts inside a def body, (2) a stored top-level String Let binding read back into puts, (3) puts of a multi-arg String intrinsic's result (e.g. .slice(1,3)), (4) puts of an Array[String] index read, (5) puts of a String? populated via safe-nav through a function return then read after ||=. Do not assume the answer going in — examples/README.md explicitly leaves this uninvestigated; report findings honestly even if they turn out to be five unrelated bugs rather than one."
    status: pending
  - id: leaf-fix-puts-codegen
    content: "Based on the investigation, make puts (and any other String-consuming context it shares machinery with) operate uniformly on any codegen-produced String value rather than only the specific AST shapes the current Stmt::Expr arm and its neighboring hardcoded call sites (lib.rs's \"not a compiled user function\" fallback at lines 8182/8224) happen to recognize. A String value that reaches puts through a local variable, an intrinsic call, an array index, or a safe-navigation chain must print exactly like a String literal or a direct method-call result already does today."
    status: pending
  - id: leaf-regression-tests
    content: "One dedicated compile_link_run assertion per previously-broken shape (five total, mirroring the five bullets above), added to crates/emerald-cli/tests/examples.rs or emerald-codegen's own test module wherever the existing precedent lives; then update examples/README.md's \"Real bugs found\" section to remove each fixed bullet (or mark it fixed with the regression test's name, following plan 65's own precedent of striking through fixed findings rather than deleting them) so the file keeps reflecting current, checked truth rather than accumulating stale bug reports the way the plan-of-plans table did."
    status: pending
isProject: false
---

# Plan 66 — puts and String Value-Flow Codegen

This plan exists because `examples/README.md`'s own audit — checked
against the current build, not a plan record's worked example — found
five separate cases where `puts`-ing a `String` value silently prints
nothing (or drops a later line in a sequence) instead of erroring or
printing correctly. This is a worse failure mode than a compile error:
a user with no test harness watching stdout has no signal anything went
wrong. Fixing this gates every other post-v1 plan by this session's own
decision (recorded in the plan-of-plans' "correctness before extension"
batch intro) — a compiler that silently drops output cannot be trusted
enough to be worth extending further.

## Concrete proof this plan targets

A single `.em` program, compiled and run, that exercises all five
previously-broken shapes in sequence and prints the correct line for
each — not five separate tiny programs, so a fix that only patches one
call site rather than the shared root cause is caught immediately by
the others still failing:

```ruby
def greet(name: String) -> String
  puts "hello from inside a def"   # (1) previously silent
  name
end

y: String = "hello"
puts y                              # (2) previously silent

phrase: String = "hello world"
puts phrase.slice(1, 3)             # (3) previously silent — "ell"

words: Array[String] = phrase.split(" ")
puts words[0]                       # (4) previously silent — "hello"

found: String? = nil
found ||= "not found"
puts found                          # (5) previously silent — "not found"

puts greet("world")                 # confirms (1)'s def-body puts also ran
```

Expected output, all six lines, in order:
```
hello from inside a def
hello
ell
hello
not found
world
```

Today, per `examples/README.md`, every one of the marked lines prints
nothing at all — the program still exits 0.

## Decision log

- **Why these five are grouped into one plan rather than five.** All
  five are `puts` failing to produce output for a `String` value that
  did not arrive at the call site as a single, direct literal-or-method-
  call expression. `puts`'s only special-casing lives in one place:
  `crates/emerald-codegen/src/lib.rs:10119`'s `Stmt::Expr` match arm,
  gated on `name == "puts" && args.len() == 1`, calling `build_puts`.
  Anything that reaches `puts` through a different AST shape, or from a
  different statement-compilation entry point than the one this arm is
  matched inside, risks falling through to the generic `Expr::Call`
  path instead — which the codebase's own test
  (`unsupported_top_level_shape_errors_not_panics`, lib.rs:15611-15625)
  documents as erroring with `"codegen: unsupported call to `{name}`
  (not a compiled user function)"` (lib.rs:8182, 8224) precisely because
  `"puts"` was never registered as an ordinary compiled function. Five
  symptoms, one plausible shared shape: **this is a hypothesis to
  verify, not a confirmed root cause** — `leaf-root-cause-investigation`
  exists specifically because grouping these prematurely and patching
  only the first reproduction found would leave the other four
  unfixed while looking closed. It is genuinely possible this is not
  one bug — whether a `def` body's statement list is even compiled
  through `build_stmt`'s match arms at all, or through some separate
  function-body path that bypasses them entirely, is exactly what
  `leaf-root-cause-investigation` should answer, not something this
  plan asserts up front.
- **Why "fix the root cause" instead of "add four more special-cased
  arms."** Adding a sixth, seventh, eighth hardcoded match arm each time
  a new AST shape reaches `puts` is exactly the pattern that produced
  this bug family in the first place — plan 45's original `puts`
  implementation covered the shapes its own worked examples used, and
  every shape outside that set silently fell through. The fix this plan
  wants is structural: whatever produces a `String`-typed LLVM value
  during codegen (a local load, an intrinsic call, an array index, a
  safe-navigation unwrap) should be usable as `puts`'s argument through
  one uniform materialize-then-print path, not through per-shape
  enumeration.
- **Out of scope.** This plan does not add new String methods, does not
  touch `String == String` (plan 67's job — a distinct, already-
  root-caused gap, not part of this investigation), and does not touch
  the monomorphized-generic double-print bug (plan 68 — explicitly
  disclosed there as a *different*, still-unroot-caused symptom rather
  than folded in here on a guess that it's the same family).
