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

`history/`'s plan records for plans 36-57 describe a much larger
surface than what's actually wired up in the current parser/sema/
codegen. Verified this session by attempting each and reading the
relevant source, not by re-reading the plan docs:

- **Heredocs (`<<~IDENT ... IDENT`)** — plan 36's own "concrete proof"
  worked example uses one; the lexer has no heredoc token at all today
  (grep `crates/emerald-parser` for `heredoc`/`<<~`: nothing). Only
  `"...#{expr}..."` interpolation from that same plan actually landed.
- **The enumerable stdlib (`.map`/`.select`/`.sum`/`.reduce`/etc., plan
  42)** — no `Iterable` interface, no monomorphized free functions,
  anywhere in `emerald-sema`. A block literal attached to a method call
  (`arr.select { |x| ... }`) doesn't even parse; block-attachment syntax
  only works on a user `def` that declares `&blk`.
- **ADTs/pattern matching, `Result`, actors, the scheduler, supervision
  trees (plans 52-57)** — no trace in the AST (`Actor`/`Pattern`/
  `Result` don't appear in `ast.rs` at all). These plans are design
  documents only.

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
  printing the unwrapped value.

None of this is a reason not to use these features — `.strip`,
`.to_i`, splat params, keyword args, safe navigation, etc. all work
in the specific shapes these examples use them in. It's a reason to
route through this directory's actual, CI-checked examples rather than
a plan doc's worked example when unsure whether a shape is proven to
work today.
