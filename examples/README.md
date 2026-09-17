# Examples

Every `.em` file here is real, working Emerald source, compiled with
`emerald-cli`, run, and checked against its exact stdout — durably
re-verified by `crates/emerald-cli/tests/examples.rs` (`cargo test -p
emerald-cli --test examples`) on every CI run, not just by hand at
authoring time. `test_framework.em` is the one exception (see below)
and is instead verified by `crates/emerald-cli/tests/test_subcommand.rs`.

```bash
cargo run -p emerald-cli -- examples/<file>.em -o /tmp/out && /tmp/out
```

## Coverage

| File | Covers | Verified output |
|---|---|---|
| `hello.em` | `def`/return type, `Call`, `puts`, `Add` | `42` |
| `control_flow.em` | `CompareOp`s, `if`/`elsif`/`else`/`unless`, `while`/`until`, `break`, `next`, `return`, `for..in` over a literal array, `case`/`when` | see test |
| `classes.em` | `class`, fields, `initialize`, `@field` read/write, `.new`, methods, `Float64` fields | `10`, `15`, `5` |
| `collections.em` | `Array[T]`/`Hash[K,V]` literals, indexed read/write, `Boolean`, nullable `String?` compared to `nil` | see test |
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
| `nullable_safe_nav.em` | `T?` nullable types, `obj&.method` safe navigation, `||=` | `1`, `0` |
| `test_framework.em` | `test "..." do ... end`, `assert_eq` — run via `emerald test examples/test_framework.em`, **not** the ordinary compile path (which rejects a `Program` containing a `test` block) | `PASS`/`FAIL`/pass-fail counts, exit 1 (one test is deliberately broken, matching the plan record's own worked example) |
| `generic_classes.em` | `class Stack[T]`, user-declared generic classes, monomorphized per instantiation (`Stack[Int64]` and `Stack[String]` in one program) | `30`, `20`, `second` — see the bug note below; a 4th expected line is silently dropped |
| `c_ffi.em` | `unsafe extern "C" { fn ... }`, calling real libc (`llabs`, `strlen`, `strstr`), `CString`/`String.from_cstring` | `42`, `5`, `not found` — 3 lines; see the safe-navigation bug already noted below, which this example also hits |
| `counter_actor.em` | `actor Counter`, fields, `.initialize`, ordinary methods on an actor — a shared library file, not run standalone (declares the class only, no top-level statements) | n/a — imported by `host.em`/`client.em` |
| `host.em` / `client.em` | `Counter.spawn(0)`, `.register("counter1", 9000)`, `Counter.remote("127.0.0.1:9000", "counter1")` — distributed, location-transparent actors over real TCP, across two separate OS processes | **currently does not build** — see the bug note below |
| `packages/` | The package manager: `emerald.toml`, a path dependency (`app` depends on `mathutils`), `require`. Build with `cd examples/packages/app && emerald build`, then run `./app` (or `emerald run`) — **not** `emerald <file> -o`, which doesn't resolve `[dependencies]` | `8` |
| `parallel/` | Multi-file `require` splicing under the parallel/incremental compiler. Build with `emerald --jobs N examples/parallel/main.em -o out` — **not** a bare `emerald <file> -o`, which (per `crates/emerald-cli/src/main.rs`'s `run_legacy`) never splices `require`s at all without `--jobs` | `30` |

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
- **The enumerable stdlib (`.map`/`.select`/`.sum`/`.reduce`/etc., plan
  42)** — no `Iterable` interface, no monomorphized free functions,
  anywhere in `emerald-sema`. A block literal attached to a method call
  (`arr.select { |x| ... }`) doesn't even parse; block-attachment syntax
  only works on a user `def` that declares `&blk`.

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

- **`puts` cannot be called from inside a user-defined `def` function's
  body** — only at a program's top level (`crates/emerald-codegen`'s
  `build_stmt` special-cases a bare `puts` statement, but that
  special-case isn't reachable from a function body's own compilation
  path; the generic fallback then rejects `puts` as "not a compiled
  user function"). Every function in every example here returns a
  value and lets the caller `puts` it, which is why this wasn't
  visible before.
- **A top-level `String`-typed `Let` binding silently fails to print**
  — `y: String = "hello"; puts y` compiles, runs, exits 0, and prints
  nothing (no crash, no diagnostic). `puts` of a `String` *expression*
  used directly (a literal, an interpolation, or a method call result)
  works fine; only a stored-then-loaded `String` local breaks. Worked
  around by always `puts`ing the expression directly.
- **Multi-argument `String` intrinsics silently drop their `puts`
  output** — `puts phrase.slice(1, 3)` compiles and runs but prints
  nothing, while single-argument intrinsics (`.strip`, `.upcase`,
  `.to_i`, ...) print correctly. `strings.em` doesn't use `.slice`.
- **`Array[String]` indexing silently drops its `puts` output** —
  `words: Array[String] = phrase.split(" "); puts words[0]` compiles,
  runs, and prints nothing. `strings.em` doesn't index a `split()`
  result.
- **`String == String` isn't implemented in codegen** — sema accepts
  it, then codegen rejects it with "comparison operands must both be
  Int64 or both Float64." `nullable_safe_nav.em` avoids comparing
  narrowed `String` locals for equality.
- **A `String?` value populated via safe navigation through a
  function's return value, then read after an `||=`, silently drops
  its `puts` output or fails at codegen** — the exact shape plan 43's
  own worked example uses. `nullable_safe_nav.em`'s safe-navigation
  demo instead compares the result to `nil` (which works, and is
  already how `collections.em` exercises `String? == nil`) rather than
  printing the unwrapped value. `c_ffi.em` hits this same bug directly
  (its `found: String?` populated via `String.from_cstring(strstr(...))`
  then `found ||= "not found"` then `puts found`) rather than routing
  around it — left in place deliberately so this example doubles as a
  live repro, which is why its verified output is 3 lines, not 4.
- **A second consecutive `puts` of a generic method's return value on a
  monomorphized instance can silently drop its output** — found writing
  `generic_classes.em`. `ints.pop()` (a `Stack[Int64]`) prints correctly
  twice; `strs.pop()` (a separately-monomorphized `Stack[String]`)
  called the same way, back-to-back, prints only its first result, not
  its second — 3 lines out of an expected 4, no crash, no diagnostic.
  Not yet root-caused (left as a genuine open finding rather than a
  guessed explanation); left in place rather than worked around, same
  reasoning as the `c_ffi.em` case above.
- **`require`-splicing a file that declares an `actor` class produces a
  linker failure under `--jobs N`, and produces no splice at all
  without `--jobs`** — found compiling `host.em`/`client.em`, which
  `require counter_actor` (an `actor Counter` declaration). A bare
  `emerald host.em -o out` fails typecheck (`unknown type 'Counter'`,
  `undefined variable 'c'`) — the already-documented "no `--jobs`, no
  splice" gap. `emerald --jobs 2 host.em -o out` fails to *link*:
  `multiple definition of 'Counter_report__trampoline'` and the same
  for `RemoteActorError_encode`/`_decode`, every `Counter_*_encode_args`/
  `_decode_args`, and `Counter__methods` — the requiring file's
  compilation unit and the required file's own compilation unit both
  emit full, externally-linked definitions of the actor's wire-protocol
  functions, so linking both objects into one binary collides. Ordinary
  (non-actor) multi-file `require` doesn't hit this — `parallel/`'s
  plain-function example links and runs fine under `--jobs`. As a
  result, **the only examples exercising distributed, location-
  transparent actors over real TCP do not currently build, via either
  invocation path** — the scheduler/supervision runtime itself is
  proven (see above), but this specific multi-process story is not.

None of this is a reason not to use these features — `.strip`,
`.to_i`, splat params, keyword args, safe navigation, etc. all work
in the specific shapes these examples use them in. It's a reason to
route through this directory's actual, CI-checked examples rather than
a plan doc's worked example when unsure whether a shape is proven to
work today.
