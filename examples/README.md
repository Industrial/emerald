# Examples

Every `.em` file here is real, working Emerald source, compiled with
`emerald-cli`, run, and checked against its exact stdout — durably
re-verified by `crates/emerald-cli/tests/examples.rs` (`cargo test -p
emerald-cli --test examples`) on every CI run, not just by hand at
authoring time. `test_framework.em`, `property_test.em`, and
`benchmark_example.em` are the exceptions (see below) — none of the
three can run through the ordinary `emerald <file>` compile path (it
rejects a `Program` containing a `test`/`property`/`benchmark` block),
so all three are instead verified by
`crates/emerald-cli/tests/test_subcommand.rs`, via `emerald
test`/`emerald benchmark`.

```bash
cargo run -p emerald-cli -- examples/<file>.em -o /tmp/out && /tmp/out
```

## Coverage

| File | Covers | Verified output |
|---|---|---|
| `hello.em` | `def`/return type, `Call`, `puts`, `Add` | `42` |
| `control_flow.em` | `CompareOp`s, `if`/`elsif`/`else`/`unless`, `while`/`until`, `break`, `next`, `return`, `for..in` over a literal array, `case`/`when` | see test |
| `classes.em` | `class`, fields, `initialize`, `@field` read/write, `.new`, methods, `Float64` fields | `10`, `15`, `5` |
| `collections.em` | `Array[T]`/`Hash[K,V]` literals, indexed read/write, `Boolean`, `Option[String]`/`None` matched via `match` | see test |
| `closures.em` | `Proc` type, lambda literals, `.call`, `&blk`/`yield` blocks | `15`, `42`, `0`, `1`, `2` |
| `exceptions.em` | `raise`, `begin`/`rescue`/`ensure`/`retry` | `99`, `5`, `2`, `777` |
| `modules.em` | `module`, namespaced static method calls | `42` |
| `interfaces_generics.em` | `interface`, `implements`, bounded generic functions (`def max[T: Comparable]`), monomorphization | `750`, `100` |
| `symbols.em` | `:symbol` literals, `Hash[Symbol, T]` | `82`, `100`, `1`, `0` |
| `bitwise_and_assignment.em` | `& \| ^ ~ << >>`, `+= -= *= /= %=`, multiple assignment (`a, b = b, a`) | `3`,`1`,`1`,`-1`,`16`,`16`,`10`,`2`,`1` |
| `ranges.em` | `a..b` / `a...b` as a `for...in` scrutinee | `15`, `10` |
| `class_inheritance.em` | `class Dog < Animal`, inherited fields/methods, override, `read` field-accessor sugar | `5`, `3`, `103` |
| `operator_overloading.em` | `def +`/`def ==` on a class, dispatched from `a + b` / `a == b` | `4`, `6`, `0`, `1` |
| `function_signatures.em` | default parameter values, keyword-argument calls (`f(x: 1)`), splat params (`*xs: T`), tuple returns + destructuring (`a, b = f()`) | `1`,`2`,`3`,`2`,`60` |
| `strings.em` | `"...#{expr}..."` interpolation, `String` intrinsics (`.strip`/`.upcase`/`.downcase`/`.length`/`.split_count`/`.to_i`/`.to_f`) | see test |
| `nullable_safe_nav.em` | `Option[T]`/`Some`/`None`, `obj?.method` safe navigation, `match` | `1`, `0` |
| `test_framework.em` | `test "..." do ... end`, `assert_eq` — run via `emerald test examples/test_framework.em`, **not** the ordinary compile path (which rejects a `Program` containing a `test` block) | `PASS`/`FAIL`/pass-fail counts, exit 1 (one test is deliberately broken, matching the plan record's own worked example) |
| `property_test.em` | `property "..." do ... end` (plan 80) — run via `emerald test examples/property_test.em` (or the identical `emerald property` alias); parses/type-checks/compiles exactly like `test`, real, disclosed simplification: runs its body once, not across generated inputs (no input generation/shrinking exists) | `PASS`/`PASS`/pass-fail counts, exit 0 |
| `benchmark_example.em` | `benchmark "..." do ... end` (plan 80) — run via `emerald benchmark examples/benchmark_example.em`, **not** the ordinary compile path (identical rejection to `test`); measures CPU time (`clock()`) around each block via a synthetic `extern "C"` timing call | computed values (`210000000`, `1000000`) each followed by a `BENCHMARK: <description>` line and an elapsed-seconds `Float64` — the first block's elapsed time is near-instant (LLVM constant-folds it, a real, disclosed limitation matching `benchmarks/REPORT.md`'s own Headline §3), the second's is real, measured, non-instant (heap allocation defeats folding) |
| `generic_classes.em` | `class Stack[T]`, user-declared generic classes, monomorphized per instantiation (`Stack[Int64]` and `Stack[String]` in one program) | `30`, `20`, `second`, `first` — all 4 lines; the previously-noted "4th line silently dropped" bug was investigated and found not to be a compiler defect (see "Real bugs found") |
| `c_ffi.em` | `unsafe extern "C" { fn ... }`, calling real libc (`llabs`, `strlen`, `strstr`), `CString`/`String.from_cstring` | `42`, `5`, `world`, `not found` — all 4 lines; the safe-navigation bug previously noted below was fixed (see "Real bugs found") |
| `counter_actor.em` | `actor Counter`, fields, `.initialize`, ordinary methods on an actor — a shared library file, not run standalone (declares the class only, no top-level statements) | n/a — imported by `host.em`/`client.em` |
| `host.em` / `client.em` | `Counter.spawn(0)`, `.register("counter1", 9000)`, `Counter.remote("127.0.0.1:9000", "counter1")` — distributed, location-transparent actors over real TCP, across two separate OS processes | `3` (printed by the `host` process, after the `client` process's 3 real TCP `.increment` calls plus `.report`) — build with `--jobs 2` (see the bug note below: this genuinely didn't build earlier this same session, fixed via `WeakODR` linkage) |
| `packages/` | The package manager: `emerald.toml`, a path dependency (`app` depends on `mathutils`), `require`. Build with `cd examples/packages/app && emerald build`, then run `./app` (or `emerald run`) — **not** `emerald <file> -o`, which doesn't resolve `[dependencies]` | `8` |
| `parallel/` | Multi-file `require` splicing under the parallel/incremental compiler. Build with `emerald --jobs N examples/parallel/main.em -o out` for genuine level-order parallel compilation of the require-DAG (plan 49) — a bare `emerald <file> -o` (no `--jobs`) also splices `require`s correctly as of the plan 69 fix (`crates/emerald-cli/src/main.rs`'s `run_legacy` now detects and resolves `Item::Require` before compiling), just single-threaded, not in parallel | `30` |
| `module_visibility/` | Plan 76's `export`/`import` symbol-level visibility: `greeter.em`/`mathutils.em` each `export` only the functions `main.em` actually uses (`greet`, `add`) and keep a private, unexported helper each (`internal_double`) that `main.em` never references — `main.em` reaches `greeter.em` via a bare `require` (fine: it only calls the one function `greeter.em` exports) and `mathutils.em` via an explicit `import mathutils { add }`. Build with `emerald examples/module_visibility/main.em -o out` (the bare `require` is what routes this through the real multi-file resolver, same mechanism `parallel/` uses) | `Hello, Emerald!`, `7` |
| `enumerable.em` | `map`/`select`/`filter`/`reduce`/`inject`/`each_with_index`/`count`(arity-0 and predicate)/`sum`/`sort` on `Array[T]`; `map`/`reduce`/`each_with_index`/`count` on `Hash[K,V]`; the new block-attached-call syntax (`recv.method { \|params\| body }`) itself, on both a bare no-parens call and a call with explicit positional args | `0`,`1`,`2`,`3`,`4`,`10`,`2`,`15`,`3`,`15`,`1`,`10`,`60`,`2`,`3` |
| `doc_comments.em` | `##` doc comments (plan 77, design brief §36) vs. ordinary `#` line comments (plan 20) — a documented `fn`/`class`+method/`module`+method/`enum`, a `##` run before `multiply` never written (an ordinary `#` comment there instead, never captured), and a `##` run before `subtract` deliberately separated from it by a blank line (so it attaches to nothing). Run `emerald doc examples/doc_comments.em` to see only the documented declarations extracted, with their real signatures and doc text — `subtract`/`multiply`/`doubled_manhattan` are absent from the output | `42`,`7`,`42`,`7`,`14`,`36`,`500` |
| `domain_types.em` | Plan 81's `newtype` — zero-cost, nominally distinct domain types over a primitive (`Meters`/`Seconds`/`MetersPerSecond`, mirroring the Sable brief's own illustrative example): `Name.new(value)` to construct, `.value` to unwrap, `==` compares structurally between two values of the *same* newtype with no unwrap needed, `Array[Meters]` has the identical packed representation `Array[Float64]` does — see `newtype_zero_cost_benchmark.em` for the runtime-cost proof | `10.4384`, `1`, `100` |
| `newtype_zero_cost_benchmark.em` | Plan 80's `emerald benchmark` mechanism, proving plan 81's `newtype` costs nothing at runtime: sums 2,000,000 real (non-folded, allocation-forced) elements over `Array[Float64]` vs. the identical loop over `Array[Meters]` — the two timings are statistically indistinguishable (`Meters` is occasionally *faster*, pure noise), confirming `value_kind_for_type` really does resolve a newtype to its underlying primitive's own LLVM representation rather than introducing any wrapper | two near-identical CPU-time floats (exact values vary by run) |

For anything not in this table, grep `crates/emerald-parser/src/ast.rs`'s
`Expr`/`Stmt`/`Item` enums and `crates/emerald-cli/tests/examples.rs` —
those two are the actual source of truth for "does this parse" and
"does this run and print what it claims," respectively. This file is
a curated index onto them, not a substitute; the previous version of
this README drifted badly out of date with the actual grammar/codegen
(see below) precisely because it wasn't re-derived from those sources.

## Not implemented (despite being documented in `history/`)

This section previously claimed actors, the scheduler, supervision trees,
ADTs, and `Result` were "design documents only" with no trace in the AST.
That was true when originally written (after the 36-47 batch, before the
48-57 batch landed) and is **false now** — it was never refreshed and sat
stale through eight more plans' worth of commits. Corrected this session
by checking the actual, current source rather than trusting the prior
note:

- `ast.rs` defines `ActorDef`, `Supervise`, `CasePattern`, and `Result`
  `Ok`/`Err` constructors — grep for them yourself if in doubt.
- `examples/counter_actor.em` and `examples/generic_classes.em` both
  compile and run against the current build.
- The workspace test suite includes
  `supervisor_worked_example_compiled_linked_and_run_prints_the_expected_six_lines_in_order`
  and a ping-pong actor-messaging test (`emerald-codegen`), both passing,
  plus dedicated scheduler/supervisor integration tests in
  `emerald-driver/tests/`.

What's still genuinely missing, checked the same way this session:

- **Heredocs (`<<~IDENT ... IDENT`)** — plan 36's own "concrete proof"
  worked example uses one; the lexer has no heredoc token at all today
  (grep `crates/emerald-parser` for `heredoc`/`<<~`: nothing). Only
  `"...#{expr}..."` interpolation from that same plan actually landed.
- ~~**The enumerable stdlib (`.map`/`.select`/`.sum`/`.reduce`/etc.,
  plan 42)** — no `Iterable` interface, no monomorphized free
  functions, anywhere in `emerald-sema`. A block literal attached to a
  method call (`arr.select { |x| ... }`) doesn't even parse;
  block-attachment syntax only works on a user `def` that declares
  `&blk`.~~ — **Mostly fixed (plan 70).** `grammar.lalrpop` now parses
  a trailing block literal on any `Ident.method`/`.method(args)` call
  in BOTH statement-initial and expression (a `Let`'s RHS) position,
  and `emerald-parser`'s new `hoist_enumerable_blocks` pass turns it
  into exactly the pre-existing named-`Proc` call shape plan 42's own
  `check_enumerable_call`/`build_enumerable_call` already fully
  implemented for `Array[T]` — that machinery, it turns out, was
  already complete and working end-to-end; the ONLY missing piece was
  this parse-time block-attachment syntax. `map`/`reduce`/
  `each_with_index`/`count` (a real, new predicate form, not just the
  original arity-0 header read) are now also implemented for
  `Hash[K,V]`. Still genuinely not done, disclosed plainly rather than
  glossed over:
  - **No real `Iterable[T]` compiler-recognized interface** — these
    ten methods remain a hard-coded per-type (`Array`/`Hash`) dispatch
    arm in both `emerald-sema` and `emerald-codegen`, exactly as plan
    42 originally shipped it and exactly as its own doc comment already
    discloses (a real `Proc[T, U]` bracketed-annotation parsing
    prerequisite this plan did not build either).
  - **Block parameters need an explicit type** (`|x: Int64|`, not a
    bare `|x|`) — this compiler has no call-site-driven type inference
    for an unannotated parameter.
  - **`Hash[K,V]`'s `.select`/`.filter` stay Array-only** — their only
    sensible result type, `Array[Pair[K,V]]`, cannot be written in this
    language's concrete syntax at all (`grammar.lalrpop`'s `TypeName`
    rule only parses `Array[<a bare Ident>]`, never a nested compound
    element type), confirmed against the real parser, not assumed.
  - **No `for i, x in arr.each_with_index`-style iteration** — the
    plan-of-plans' own worked-example sketch for this shape; `for`
    loops in this compiler only ever accept a literal array or an
    integer range as their scrutinee, never an arbitrary expression, a
    pre-existing restriction this plan did not lift. `each_with_index`
    is still available in exactly the callback-block shape plan 42
    already designed (`arr.each_with_index do |x, i| ... end`, plan 87's
    exclusive `do...end` spelling).
  - ~~**No chaining** — `arr.select { }.map { }` in one expression is
    still unsupported (inherited, unchanged, from plan 42's own
    original scope).~~ — **Fixed (plan 87).** Braces are gone entirely
    (block-attached calls are exclusively `do...end` now), and a
    `do...end`-attached call binds tight enough, chain-locally, to be
    used as the receiver of a further `.method` call: `arr.select do
    |x| ... end.map do |x| ... end` now parses and chains through as
    many links as written.

**Take the "Not implemented" label in this section literally and
narrowly** — it means "checked directly against source/build this
session, confirmed absent." Do not infer that anything not listed here
is therefore implemented; large parts of this file (and of `history/`)
have not been re-verified this way. If you're relying on a specific
feature working, check it the way this session did — compile a real
`.em` file against the current build — rather than trusting either this
list or a plan record's worked example.

## Real bugs found while writing these examples

Writing `strings.em`, `function_signatures.em`, and
`nullable_safe_nav.em` surfaced compiler bugs that the corresponding
plan records' own "concrete proof" examples claim work but don't, as
tested against the current build. Every example in this directory is
written to route around them; the workaround is noted so a future fix
doesn't need to rediscover the constraint from scratch:

- ~~**`puts` cannot be called from inside a user-defined `def` function's
  body**~~ — **Fixed; was never actually a codegen gap.** Plan 66's
  root-cause investigation found this, and the four bullets below it,
  do not reproduce against the current build at all: `build_puts` is
  fully generic over `ValKind` regardless of the AST shape that
  produced the `String` value, and every statement position (including
  inside a `def` body) reaches the same `build_stmt` `puts`
  special-case. The real defect was a nondeterministic runtime race —
  the (since-removed) actor worker-pool thread spawn/join that used to
  wrap every compiled program's `main` unconditionally, racing against
  glibc's block-buffered stdout — incidentally fixed by `b0d8bc1`
  ("perf(codegen): skip actor worker-pool startup/shutdown for programs
  with no actors"), which predates this correction. Regression test:
  `plan_66_puts_inside_a_def_body_prints_deterministically` (runs the
  compiled binary 20x per test, asserting byte-identical output every
  time — insurance against the *race* regressing, not just the output).
- ~~**A top-level `String`-typed `Let` binding silently fails to
  print**~~ — **Fixed; same race as above, not a codegen gap.**
  `y: String = "hello"; puts y` prints correctly and deterministically
  on the current build. Regression test:
  `plan_66_a_stored_top_level_string_let_binding_prints_deterministically`.
- ~~**Multi-argument `String` intrinsics silently drop their `puts`
  output**~~ — **Fixed; same race as above, not a codegen gap.**
  `puts phrase.slice(1, 3)` prints `ell` correctly and deterministically.
  Regression test:
  `plan_66_puts_of_a_multi_arg_string_intrinsic_result_prints_deterministically`.
- ~~**`Array[String]` indexing silently drops its `puts` output**~~ —
  **Fixed; same race as above, not a codegen gap.**
  `words: Array[String] = phrase.split(" "); puts words[0]` prints
  `hello` correctly and deterministically. Regression test:
  `plan_66_puts_of_an_array_string_index_read_prints_deterministically`.
- **`String == String` isn't implemented in codegen** — sema accepts
  it, then codegen rejects it with "comparison operands must both be
  Int64 or both Float64." `nullable_safe_nav.em` avoids comparing
  narrowed `String` locals for equality. (Not this plan's scope — see
  plan 67.)
- ~~**A `String?` value populated via safe navigation through a
  function's return value, then read after an `||=`, silently drops
  its `puts` output or fails at codegen**~~ — **Fixed; same race as
  above, not a codegen gap.** The exact shape plan 43's own worked
  example uses, and the exact shape `c_ffi.em` hits directly (its
  `found: Option[String]` populated via
  `String.from_cstring(strstr(...))` then `puts found ?? "not found"`),
  both print correctly and deterministically on the current build —
  `c_ffi.em`'s verified output is the full 4 lines (`42`, `5`, `world`,
  `not found`), not 3 as previously noted here. Regression tests:
  `plan_66_a_string_optional_via_safe_nav_and_coalesce_prints_deterministically`
  and `plan_66_the_c_ffi_string_optional_safe_nav_shape_prints_deterministically`
  (the latter runs the exact `c_ffi.em` FFI example 20x end-to-end).
  Post plan-73 migration, `nullable_safe_nav.em`'s own safe-navigation
  demo now `match`es the `Option[String]` result instead of comparing
  to a removed `nil` sentinel — same underlying proof, updated syntax.
- ~~**A second consecutive `puts` of a generic method's return value on
  a monomorphized instance can silently drop its output**~~ —
  **Investigated (plan 68) and not a compiler defect.** `strs.pop()`'s
  second call (a `Stack[String]`, monomorphized separately from the
  `Stack[Int64]` instance also in this program) prints correctly and
  deterministically on the current build: 50 fresh compile-link-run
  cycles of this exact program, each executed and its stdout captured
  entirely in-process (`std::process::Command`, no external capture
  tool in the loop), produced the correct 4 lines every single time —
  see `plan_68_generic_stack_second_pop_on_a_monomorphized_string_instance_prints_deterministically`.
  Disassembly of the compiled binary (`Stack$String_pop`,
  `Stack$String_push`, `main`, `emerald_print_str`) is also textbook-
  correct: two distinct `emerald_print_str` calls, fed the two real
  return values of two real `Stack$String_pop` calls, the same shape as
  the `Int64` instance's own two calls right above them. The original
  one-off observation (and this plan's own prior "reproduced once in
  roughly 250 runs" finding) is best explained by an output-capture
  artifact in the tooling used to observe repeated runs, not a real
  race in the compiled program: piping the identical freshly-linked
  binary's repeated stdout through this environment's `ctx_shell` MCP
  tool *without* its `raw: true` verbatim-capture escape reproduced a
  3-line-instead-of-4 truncation matching the disclosed symptom at some
  repeat counts and not others against the exact same binary and
  command — a tool-side parameter with no causal path into the compiled
  program's own execution. `raw: true` on that same tool, plain shell
  redirection, and this file's own `compile_link_run`/
  `compile_link_run_n_times` in-process capture all showed the correct
  4 lines, every time, on every attempt.
- **`require`-splicing a file that declares an `actor` class produced a
  linker failure under `--jobs N`, and produces no splice at all
  without `--jobs`** — found compiling `host.em`/`client.em`, which
  `require counter_actor` (an `actor Counter` declaration). **Fixed,
  same session, for the `--jobs` half.** A bare `emerald host.em -o out`
  still fails typecheck (`unknown type 'Counter'`, `undefined variable
  'c'`) — the "no `--jobs`, no splice" gap is unrelated and still real.
  But `emerald --jobs 2 host.em -o out` used to fail to *link*:
  `multiple definition of 'Counter_report__trampoline'` and the same
  for `RemoteActorError_encode`/`_decode`, every `Counter_*_encode_args`/
  `_decode_args`, and `Counter__methods` — the requiring file's
  compilation unit and the required file's own compilation unit both
  emitted full, externally-linked definitions of the actor's wire-protocol
  functions (and, found in the same pass, ordinary class/actor/module
  method bodies too), so linking both objects into one binary collided.
  Fixed via `WeakODR` linkage (`crates/emerald-codegen/src/lib.rs`'s
  `declare_actor_trampolines`/`declare_wire_class_codecs`/`declare_actor_
  wire_arg_codecs`/`build_actor_method_tables`/`weak_odr_class_shaped_
  methods`), verified end-to-end as two real, separate OS processes:
  `host` listens on `:9000`, `client` sends three real TCP `.increment`
  calls plus `.report`, `host` prints `3`. Regression test:
  `an_actor_shared_across_require_d_files_links_and_runs_correctly_under_jobs`
  in `crates/emerald-driver/tests/parallel_jobs.rs`.

None of this is a reason not to use these features — `.strip`,
`.to_i`, splat params, keyword args, safe navigation, etc. all work
in the specific shapes these examples use them in. It's a reason to
route through this directory's actual, CI-checked examples rather than
a plan doc's worked example when unsure whether a shape is proven to
work today.
