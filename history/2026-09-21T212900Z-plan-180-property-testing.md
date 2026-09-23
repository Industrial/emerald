2026-09-21T21:29:00Z

---
name: Property-Based Testing Generators
overview: "`property \"...\" (a: Int64, b: Int64) do ... end` — real generator-driven, shrinking-on-failure property testing, backing the `emerald property` subcommand this session's own investigation found already real, shipped, and working (plan 80) but deliberately simplified: `Item::Property { description, body }` — verified this session against the live `grammar.lalrpop` production and `main.rs`'s dispatch table — is byte-for-byte the same shape as `Item::Test` and runs its body exactly once, with no generated inputs and no shrinking at all, a fact its own code comment discloses plainly (\"a property block runs its body once, not across many generated inputs — a real, disclosed simplification, not a separate pipeline\"). This plan is the first real backing implementation the existing simplified command was always waiting for: `proptest` 1.11 (verified this session: 17M downloads/month, 12,859 dependent crates, vs. `quickcheck`'s 4M downloads/month, 2,024 dependents) drives a Rust-side generate/run/shrink loop that calls back into one compiled Emerald function per generated case — introducing no new concurrency model at all, since `proptest`'s own `TestRunner` is sequential by default, unlike plans 177-179's genuine actor-model tensions."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-cargo-dependency
    content: "Add `proptest = \"1.11\"` to `crates/emerald-rt/Cargo.toml`. Run plan 95's crate-vetting checklist against it explicitly, citing the verified comparison against `quickcheck` in this plan's own Decision log."
    status: pending
  - id: leaf-property-params-grammar
    content: "Extend `grammar.lalrpop`'s real, current `\"property\" <description:StringLitTok> \"do\" <body:StmtList> \"end\" => Item::Property { description, body }` production to `\"property\" <description:StringLitTok> <params:(\"(\" <Params> \")\")?> \"do\" <body:StmtList> \"end\" => Item::Property { description, params: params.unwrap_or_default(), body }` — an optional, backward-compatible parenthesized parameter list reusing the existing `Params` production verbatim (the same rule `FuncDef` already uses). A zero-param `property \"...\" do ... end` block (plan 80's exact existing shape) still parses to an empty `params: Vec::new()` and, per `leaf-legacy-zero-param-compat` below, keeps running exactly once, unchanged."
    status: pending
  - id: leaf-legacy-zero-param-compat
    content: "`emerald_driver::compile_test`'s real, current `codegen_test_stage`/`compile_test_harness` path (verified: `Result<usize, DriverError>`, shared byte-for-byte between `emerald test` and `emerald property` per `main.rs`'s dispatch table) is left completely unchanged for every `Item::Test` and every zero-param `Item::Property` — both keep the existing \"compile and run the body once\" behavior. A new `emerald_codegen::compile_property_harness` sibling function handles only `Item::Property` entries whose `params` is non-empty, routed from a new `emerald_driver::compile_property` (mirroring `compile_test`/`compile_benchmark`'s own real shape) that `main.rs`'s `\"property\"` dispatch arm calls instead of `test_runner::run`'s current `compile_test` call, once any parameterized property block is present in the file."
    status: pending
  - id: leaf-guarded-property-case-function
    content: "For each parameterized `Item::Property`, codegen emits an ordinary function (reusing `FuncDef`-body lowering wholesale, ValKind-typed parameters matching `params`) wrapped in a synthesized `begin ... rescue Exception => e ... end` (reusing plan 11's real `push_handler`/`rescue` mechanism, not a new exception primitive) whose body is the property's real `StmtList`, returning a fixed two-word out-shape: `0i64` (case passed) or `1i64` plus an allocated `String` holding the caught exception's message (case failed) — never allowing a raised exception's `longjmp` to escape the function's own frame (see Decision log for why this guard is mandatory, not optional, given this function is called back into from Rust)."
    status: pending
  - id: leaf-proptest-driver
    content: "Add `emerald_rt_proptest_run_i64_i64(case_fn: extern \"C\" fn(i64, i64) -> PropertyCaseResult, ...) -> *mut c_void`-shaped exports to `crates/emerald-rt/src/lib.rs` per supported parameter-type-signature arity (Int64/Float64/String/Boolean only, per plan 59's real `Type` enum finding — no narrower ints exist to generate), each building `proptest`'s own `Strategy`/`TestRunner` for the declared parameter types, running up to `proptest`'s own default case count (256), calling `case_fn` once per generated case, and — on the first failure — driving `proptest`'s real `ValueTree::simplify()` shrink loop to a minimal failing input before returning a `Result`-shaped pointer (plan 92's `emerald_rt_result_ok`/`emerald_rt_result_err` helpers) describing pass/fail, case count, and (on failure) the minimal shrunk arguments plus the captured exception message. `emerald_codegen::compile_property_harness` synthesizes the driver `main` that calls the correct arity-specific export, per property block, and formats the pass/fail report per `leaf-cli-report-format` below."
    status: pending
  - id: leaf-cli-report-format
    content: "`emerald property <file>` prints, per property block: `PASS: <description> (<n> cases)` on success, or `FAIL: <description>: minimal input <params>: <exception message>` on a shrunk failure, reusing plan 47's real `emerald test`/exit-code-1-on-any-failure convention (verified: `test_runner::run` propagates the compiled binary's real exit code unchanged). Add `examples/property_shrink_proof.em` (this plan's Concrete Proof below) wired into `emerald-cli/tests/examples.rs`'s CI-checked table, and run the full `AGENTS.md` gate."
    status: pending
isProject: false
---

# Plan 180 — Property-Based Testing Generators

This session's own investigation (mandatory per this plan's brief, not
skipped) found `emerald property`/`emerald benchmark` are real, shipped,
working CLI subcommands, not stubs — verified directly against live
source, not assumed from the task brief's own hedge. `main.rs`'s real
dispatch table (read this session): `Some("property") =>
test_runner::run(&args)`, with its own code comment stating plainly
*why* — "`property \"...\" do ... end` compiles and runs through the
exact same `test_runner`/`compile_test_harness` mechanism `test` blocks
already use... a real, disclosed simplification (a `property` block runs
its body once, not across many generated inputs), not a separate
pipeline." `grammar.lalrpop`'s real, current production confirms the
AST shape has no room for generator parameters at all:
`Item::Property { description, body }`, structurally identical to
`Item::Test { description, body }`. This plan is exactly the "real
backing implementation" that simplified version was always waiting for
— it does not invent the subcommand or its name, it fills in the
mechanism plan 80 explicitly deferred.

## Concrete proof this plan targets

```ruby
property "addition is commutative" (a: Int64, b: Int64) do
  assert_eq(a + b, b + a)
end

property "subtraction finds a real bug" (a: Int64, b: Int64) do
  assert_eq(a - b, b - a)
end
```

Expected `emerald property` output:

```
PASS: addition is commutative (256 cases)
FAIL: subtraction finds a real bug: minimal input a=0, b=1: expected 1 but got -1
```

exit code `1`. The first property holds for every generated `(a, b)`
pair, so all 256 of `proptest`'s own default case count pass. The second
genuinely fails whenever `a != b` (`a - b == b - a` only when `a == b`);
`proptest`'s documented default integer strategy shrinks toward zero, so
the minimal counterexample it converges on for a two-`i64`-argument
failing property is deterministically `a=0, b=1` (or a value `proptest`
itself treats as equally minimal under its own shrink ordering) — the
genuinely checkable claim here is that shrinking runs at all and
converges on values *smaller* than whatever large random pair first
triggered the failure, not the exact digits of one specific counter-
example, which this plan's own Rust-side `#[test]` (below) asserts
precisely against `proptest`'s real, observed behavior rather than
guessed in advance.

## Decision log

- **`proptest` 1.11 over `quickcheck`, verified and compared this
  session, not assumed.** `lib.rs`'s real crate pages: `proptest`
  `1.11.0` (24 Mar 2026), 16,988,937 downloads/month, used in 12,859
  dependent crates (10,480 directly); `quickcheck` `1.1.0` (10 Feb
  2026), 4,007,747 downloads/month, used in 2,024 dependent crates —
  roughly a 4x adoption gap in `proptest`'s favor on every axis
  checked. `proptest`'s own listing describes itself as "Hypothesis-like
  property-based testing and shrinking" — shrinking is a first-class,
  advertised design goal (a real `ValueTree`/`Strategy` trait pair built
  specifically to support it), not a bolt-on; `quickcheck`'s own
  shrinking (real, and also present) is comparatively less
  compositional. Both are real, maintained options — this plan picks
  `proptest` on adoption plus shrinking-as-a-first-class-citizen, not
  because `quickcheck` is unsound.
- **The investigation, restated precisely: `property` is real, not a
  stub, and this plan is filling a named, disclosed gap, not inventing
  new surface from nothing.** See the framing above — `Item::Property`'s
  actual, current shape (`description: String, body: Vec<Stmt>`, no
  `params` field at all) and `main.rs`'s actual dispatch (`property`
  and `test` are, today, the literal same function call) were both
  verified directly this session, not inferred. This plan's own
  `leaf-property-params-grammar` extends that exact real shape rather
  than replacing it; `leaf-legacy-zero-param-compat` guarantees every
  existing zero-param `property` block anywhere in this project's own
  examples/tests keeps compiling and behaving identically.
- **No new concurrency model — `proptest`'s `TestRunner` is sequential
  by default, a real, disclosed contrast with plans 177-179.** Unlike
  `rayon`/`crossbeam`/`crossbeam-channel`, `proptest`'s standard
  generate-run-shrink loop runs one case at a time, on the one calling
  thread, by design (parallel case execution is not `proptest`'s
  default mode and this plan does not opt into it). The compiled
  Emerald property-case function is therefore called back into
  synchronously, from the exact same OS thread that invoked
  `emerald_rt_proptest_run_*` in the first place — no actor-isolation
  question arises at all, since no second thread is ever involved.
  This plan states this contrast explicitly so a reader comparing it to
  177/178/179 doesn't have to re-derive why no Decision-log bullet here
  resolves an actor-model tension: there isn't one.
- **A raised Emerald exception must never `longjmp` past the Rust
  stack frame that called back into it — the mirror image of plan 91/
  92's own Rust-panic-across-C-frames hazard, now in the other
  direction, and just as real.** Plan 11's exception mechanism is
  `setjmp`/`longjmp`-based; when `emerald_rt_proptest_run_*` calls the
  compiled property-case function as a plain C function pointer, that
  call frame is a genuine Rust stack frame sitting between the
  `longjmp`'s target handler (somewhere back in the *original*, Emerald-
  compiled `main`) and the point where a `raise`/failed `assert_eq`
  would fire *inside* the callback. A `longjmp` unwinding through that
  Rust frame is undefined behavior for the identical reason plan 91's
  Decision log gives for the reverse case (Rust panics unwinding past
  C/`setjmp` frames): neither language's stack-unwinding mechanism
  understands the other's frames. This plan's resolution is exactly
  symmetric to plan 92's: `leaf-guarded-property-case-function` wraps
  every property-case function's body in its own local `rescue`
  handler (plan 11's real, existing mechanism — no new exception
  primitive), so no `raise` ever propagates past that function's own
  frame; a caught failure is converted to a plain, C-ABI-safe `i64`
  pass/fail code plus an allocated message pointer *before* returning
  to the Rust caller, never via a second `longjmp` reaching further out.
- **Shrinking surfaces a genuinely minimal counterexample by reusing
  `proptest`'s own `ValueTree::simplify()` loop verbatim — this plan
  does not hand-roll a shrink algorithm.** On the first case-function
  failure, the Rust-side driver captures the failing generated
  `ValueTree`, then repeatedly calls its real `simplify()`/`complicate()`
  methods (the library's own binary-search-like shrink strategy per
  type — documented, real behavior, not this plan's own invention),
  re-running the guarded property-case call against each candidate,
  keeping the smallest value that still reproduces the failure until
  `proptest`'s own convergence criteria are met (left at the library's
  default iteration cap). The Emerald-facing report prints only the
  final, converged minimal input — the intermediate shrink steps are
  Rust-internal, never surfaced to Emerald source or stdout.
- **Type coverage in v1: `Int64`, `Float64`, `String`, `Boolean` only —
  matching plan 59's own verified `Type` enum finding, not an arbitrary
  cut.** Plan 59's Decision log (verified, re-cited here): the real,
  implemented `Type` enum has no fixed-width integer narrower than
  `Int64` and no `Float32` — so there is nothing narrower for a
  generator to target. `String` generation uses a constrained `proptest`
  strategy (excluding embedded NUL bytes via `.prop_filter`, not
  `proptest`'s own default arbitrary-Unicode string strategy unmodified)
  so every generated case round-trips safely through Emerald's
  `char*`-based `String` representation (plan 59's own finding).
  Composite types — `Array`, `Hash`, class instances — have no
  generator in this plan; declaring a property parameter of one of
  those types is a real, new sema error this plan adds, not a silent
  fallback.
- **FFI/ABI: cite plan 92's naming/`catch_unwind` conventions for the
  outbound direction; the inbound-callback guard above is genuinely
  new surface, disclosed as such.** `emerald_rt_proptest_run_*` is an
  ordinary `emerald_rt_fn!`-wrapped export exactly like every other
  function in this batch — a Rust panic *inside proptest's own driver
  code* is caught and converted to `NativeError` completely normally.
  The callback-guard mechanism above is a second, distinct safety
  property this plan adds on top of that, needed only because this
  plan (uniquely among the six in this batch) calls *back into*
  compiled Emerald code from inside a native function's own body.
- **Out of scope.** No composite-type (`Array`/`Hash`/class-instance)
  generators. No user-composable `Strategy` combinators exposed to
  Emerald source (`Gen.map`/`Gen.zip`-style — v1 ships a fixed,
  built-in generator per primitive type only, chosen internally by the
  declared parameter type, with no Emerald-visible generator-
  construction API at all). No parallel/concurrent case execution
  (matches `proptest`'s own sequential default, see above). No
  `quickcheck` dual-backend support. No change to `emerald benchmark`'s
  own, separate, already-real timing-harness mechanism (`compile_
  benchmark_harness`, verified present and unrelated to this plan).

## Update (2026-09-23)

All five todos are done. Per this project's append-only convention for
finished plans, the original todos list above is left unedited; this
section records what actually shipped and where it genuinely diverges
from the plan's own text above.

- **`leaf-cargo-dependency`** — `proptest = "1.11"` added to
  `crates/emerald-rt/Cargo.toml`, ledger row in
  `crates/emerald-rt/DEPENDENCIES.md` (plan 95's vetting checklist:
  real crate, real adoption numbers, no open RustSec advisory).
- **`leaf-property-params-grammar`** — `Item::Property` gained a real
  `params: Vec<Param>` field; `grammar.lalrpop`'s production grew the
  documented optional `("(" <Params> ")")?` clause, defaulting to
  `Vec::new()` — every existing zero-param `property "..." do ... end`
  block still parses identically. `emerald-fmt` reprints the new
  `(a: Type, b: Type)` clause only when non-empty.
  `emerald-sema`'s `is_property_generatable_type` rejects any declared
  parameter type outside `Int64`/`Float64`/`String`/`Boolean` with a
  real diagnostic, exactly as the Decision log specifies.
- **`leaf-legacy-zero-param-compat`** — real, but shaped differently
  than this todo's own literal text describes. Rather than a second,
  parallel `emerald_codegen::compile_property_harness` function plus
  driver-level routing between it and `compile_test_harness`, the
  shipped implementation extends `compile_test_harness` itself (a
  `HarnessCase::Simple`/`HarnessCase::Property` split inside the one
  function) — every `Item::Test` and zero-param `Item::Property` keeps
  the exact, byte-identical `begin...rescue AssertionError`/`PASS:`/
  `FAIL:` handling it always had (verified: the pre-existing
  `emerald_property_subcommand_is_a_real_alias_for_emerald_test` test
  in `emerald-cli/tests/test_subcommand.rs` still passes unmodified),
  and a non-empty-`params` `Item::Property` gets the new
  generator-driven treatment in the same pass. This means
  `emerald_driver::compile_test`/`compile_property` do not need to
  diverge at all — `main.rs`'s existing `"property" =>
  test_runner::run(&args)` dispatch (unchanged) already reaches the
  new mechanism with zero CLI/driver-layer routing code, a real,
  disclosed simplification found preferable to hand-rolling a
  pre-parse peek to decide which of two near-duplicate harness
  builders to call.
- **`leaf-guarded-property-case-function`** — each parameterized
  property's body compiles to an ordinary `__emerald_property_case_N`
  function; the harness's own synthesized `while` loop wraps *each
  call site* in a `begin ... rescue AssertionError => e ... end`
  (reusing plan 11's real mechanism, not a new one) and reports
  pass/fail plus `e.message` straight to the native session via
  `emerald_rt_proptest_report` — never letting a raised exception
  reach any Rust frame at all.
- **`leaf-proptest-driver`** — real, but architecturally different
  from this todo's own literal "Rust holds a raw callback function
  pointer and drives the loop" text, disclosed in
  `crates/emerald-rt/src/proptest_support.rs`'s own module doc: no
  existing mechanism in this codebase lets native code call back into
  already-compiled Emerald code outside `Http.serve`'s one hand-built,
  call-site-specific LLVM trampoline (plan 101) — building a second,
  general one from scratch was judged too large and too risky to get
  right blind for this plan's own scope. Instead, the COMPILED
  Emerald harness itself drives the generate/run/shrink loop via
  ordinary sequential native calls (`emerald_rt_proptest_begin`/
  `_current_i64`/`_current_f64`/`_current_string`/`_current_bool`/
  `_report`/`_failed`/`_case_count`/`_fail_message`) — the same
  "compiled Emerald calls into `emerald-rt`" direction every other
  domain plan in this crate already uses, never the reverse. Real,
  proptest-backed generation and shrinking: each parameter gets its
  own independent `proptest::strategy::BoxedStrategy`, shrunk
  coordinate-wise via `ValueTree::simplify()`/`complicate()` (the
  library's own real API, not a hand-rolled shrink algorithm), up to
  proptest's own default 256 cases per property.
- **`leaf-cli-report-format`** — `PASS: <description> (<n> cases)` /
  `FAIL: <description>: minimal input <params>: <message>`, exit code
  1 on any failure (the harness's pre-existing `failed > 0 → raise
  AssertionError` tail, unchanged). `examples/property_shrink_proof.em`
  added, wired into `emerald-cli/tests/examples.rs`'s CI-checked table
  (`property_shrink_proof_em_shrinks_a_real_failure_to_a_minimal_
  counterexample`) via a new `compile_and_run_property_subcommand`
  helper (the ordinary `emerald <file> -o <out>` path this table's
  other helpers use can never compile a file containing a `property`
  block at all — plan 47/80's own, unchanged restriction). One real,
  disclosed difference from this plan's own Concrete Proof text,
  found by actually running the proof rather than trusting it:
  `assert_eq`'s `AssertionError#message` is its raise site's *source
  location* (`desugar_assert_eq`'s own pre-existing codegen, verified
  against already-passing tests predating this plan), never "expected
  1 but got -1" — those values are eagerly `puts` on their own
  separate lines instead, matching `emerald test`'s own already-shipped
  behavior exactly, not new noise this plan introduces. The exact
  shrunk counterexample is `a=0`, `b=1` or `b=-1` (`proptest`'s
  `TestRunner` seeds its RNG afresh, unseeded, each run) — both
  genuinely minimal, matching the Decision log's own "or a value
  `proptest` itself treats as equally minimal" caveat; this plan's own
  new `emerald-rt` unit tests
  (`proptest_support::tests::a_property_that_always_fails_shrinks_to_
  a_real_minimal_counterexample`) assert against this real, observed
  behavior rather than a guessed exact value, per the plan's own
  instruction.

Gate: `cargo build --workspace`, `cargo clippy --workspace --all-targets`
(no new warnings), `treefmt` (clean), and `cargo nextest run` across
every touched crate (`emerald-parser`+`emerald-sema`+`emerald-fmt`:
540/540; `emerald-codegen`: 203/203; `emerald-rt`: 245/245;
`emerald-cli`: 180/180; `emerald-driver`: 53/54, the one failure being
the pre-existing, environment-specific
`cache::tests::corrupting_the_cached_object_file_forces_a_real_
recompile_not_an_error` flake this project's own tooling notes
disclose, confirmed unrelated by isolated re-run) all pass.
