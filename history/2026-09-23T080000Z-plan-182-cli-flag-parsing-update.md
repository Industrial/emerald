2026-09-23T08:00:00Z

# Plan 182 — Structured CLI Flag Parsing — Update

Update record for `history/2026-09-21T213100Z-plan-182-cli-flag-
parsing.md` (the original plan text) — kept as a separate, dated file
per this batch's own append-only convention rather than editing the
original. Read that file first for the full original design (including
its own Decision log and Concrete Proof); this file records what was
actually implemented and the real, disclosed corrections the example
and the runtime module needed.

## Status: implemented, all five leaves done

```
leaf-scaffold-cli-module-and-resource-classes:      done
leaf-builder-sequence-flag-option-positional:        done
leaf-parse-argv-argc-and-array-string-marshaling:    done
leaf-typed-accessors-and-error-help-surfacing:       done
leaf-example-and-gate:                               done
```

## Implementation

`crates/emerald-rt/src/cli.rs` (new module, ~330 lines excluding
tests): `CliParser`/`CliParseResult`, two sibling plan-93 resource-
handle newtypes over `Int64`, backed by `crate::handle`'s registry
directly — the identical shape `Regex`/`Tempfile` already use, per the
plan's own Decision log. `CliParser` holds a `clap::Command`, mutated
in place via `handle_get_mut` plus a `std::mem::replace`-with-a-
placeholder swap (`clap::Command`'s own builder methods consume `self`
by value). `CliParseResult` holds this module's own private
`ParseOutcome` (`matches: Option<clap::ArgMatches>`, `help_requested:
bool`, `help_text: String`, `error_message: Option<String>`), produced
once by `.parse` via `try_get_matches_from` (never the exit()-calling
`get_matches`-family) and never mutated again. `emerald-rt/src/lib.rs`
gained 13 new `#[no_mangle] extern "C" emerald_rt_cliparser_*`/
`emerald_rt_cliparseresult_*` wrappers, each routed through
`catch_and_raise` per plan 92's convention.

`emerald-sema`: `CliParser`/`CliParseResult` registered as compiler-
synthesized `Int64` newtypes in the `classes` map (after `Tempfile`/
`Tempdir`); `CliParser.new(name, version)` carved out of the ordinary
single-argument newtype constructor in the `Expr::New` arm (the same
way `LogFields.new()`'s zero-argument case already is); every instance
method on both classes carved out of the ordinary newtype `.value`-
only restriction, the same mechanism `Regex`'s own nine methods
already establish.

`emerald-codegen`: `CliParser`/`CliParseResult` added to both newtype
registries (`NEWTYPE_UNDERLYING` and the separate `newtypes.insert`
calls near `compile_to_object_impl`) — the exact gap this session's
own task briefing warned about, verified directly rather than assumed.
13 new `Ctx` struct fields/`module.add_function` declarations/`Ctx`
literal entries; a new `Expr::New` arm for `CliParser.new`; a new
`build_method_call` carve-out for both classes, including `.parse`'s
own `Array[String]`-header-skipping `argv` marshaling (`field_ptr(...,
8)`), identical to `Process.run`'s own established pattern, and an
explicit `i1`->`i64` zero-extend for `.option`/`.positional`'s own
`required: Boolean` argument (`Deque[Boolean]#push_front`'s own
precedent for widening on the way IN, not just narrowing on the way
out).

`crates/emerald-rt/Cargo.toml`/`DEPENDENCIES.md`: `clap = { version =
"4.6", features = ["string"] }`.

`examples/cli_flag_parsing.em` + a new
`cli_flag_parsing_em_prints_expected_sequence` test in
`crates/emerald-cli/tests/examples.rs`, using a new
`compile_and_run_with_args` helper (compiles once per call, runs the
resulting binary with caller-supplied `argv` — `compile_and_run` always
runs with zero extra arguments, which cannot exercise
`CliParser.parse(ARGV, ARGC)` against more than one fixed command
line). The test invokes the one compiled binary three separate times,
proving all three paths the plan's own Concrete Proof names: a full
successful parse, a missing-required-option error, and `--help`.

## Real, disclosed corrections against the plan's own literal Concrete
## Proof text, found only by actually compiling and running it

1. **`case X when Some(v) ... when None ... end` is not this
   grammar's real `Option[T]` pattern-matching syntax.** Plan 73's own
   actual construct (`examples/nullable_safe_nav.em`'s own precedent)
   is `match X do Some(v) do ... end None do ... end end`, used
   throughout `cli_flag_parsing.em` in place of every `case`/`when` in
   the plan's own literal text.

2. **Every `if` needs this grammar's real `do` keyword immediately
   after its condition** (`examples/control_flow.em`'s own precedent)
   — the plan's own literal `if result.help_requested()` (no `do`)
   does not parse.

3. **`match`'s own scrutinee must be a plain `Let`-bound local, never
   a direct method-call expression.** `emerald-codegen`'s own
   `build_case` states this outright in its own doc comment: an
   enum-typed scrutinee is detected via `local_classes`, which "only a
   plain `Expr::Ident` scrutinee can carry a known static type" for —
   "the same receiver restriction `build_method_call` already imposes
   elsewhere in this backend." `match result.error_message() do ...
   end` (the plan's own literal shape, direct on the method call)
   fails with `codegen: 'case' scrutinee must be Int64`; every `match`
   in the example first `Let`-binds its own `Option[String]` to a
   local instead, mirroring `nullable_safe_nav.em`'s own
   `m1: Option[String] = g1?.shout` before `match m1 do ... end`.

4. **`??`'s own left operand has the identical restriction** — a real
   compiler diagnostic states it outright: "`??`'s left operand must
   be a plain local variable or a `?.` chain." `result.value("name")
   ?? "world"` (the plan's own literal shape) is rejected the same
   way; the example `Let`-binds `name_val: Option[String] = result.
   value("name")` first, then matches `name_val ?? "world"` against a
   plain local.

5. **`clap` 4.6.7's `Str`/`Id` types do not implement `From<String>`
   (only `From<&'static str>`/`From<&String>`) unless the crate's own
   `string` feature is enabled** — `clap_builder::builder::str::Str`'s
   own doc comment states this outright ("To support dynamic values
   (i.e. `String`), enable the `string` feature"). Every `.flag`/
   `.option`/`.positional` call passes a runtime-constructed `String`
   (an Emerald program's own literal argument, never a `&'static
   str`), which does not compile against clap's default feature set at
   all. Fixed by adding `features = ["string"]` to the `clap`
   dependency in `Cargo.toml` — not by working around it with
   `.as_str()` borrows, since `Str`/`Id` have no generic
   `From<&'a str>` impl for a non-`'static` lifetime either.

6. **clap 4's own default help template omits the `{name} {version}`
   banner line entirely** — a real v4 behavior change from v2/v3's own
   template, verified directly against `clap_builder`'s own vendored
   source in this workspace's exact pinned version
   (`clap_builder-4.6.7/src/builder/...`). The plan's own Concrete
   Proof requires the literal string `"1.0.0"` to appear in `--help`'s
   own output (not just `--version`'s) — `cliparser_new` reintroduces
   that banner line via an explicit `.help_template(...)` override
   (`"{before-help}{name} {version}\n{about-with-newline}\n{usage-
   heading} {usage}\n\n{all-args}{after-help}"`) so the plan's own
   worked proof holds as literally stated, rather than weakening the
   test to drop the version assertion.

## Gate

- `cargo nextest run -p emerald-rt -p emerald-sema -p emerald-codegen`
  — 773/773 passed.
- `cargo nextest run -p emerald-cli --test examples` — 66/66 passed,
  including `cli_flag_parsing_em_prints_expected_sequence`.
- `cargo clippy -p emerald-rt -p emerald-sema -p emerald-codegen -p
  emerald-cli --all-targets` — clean (no errors; only pre-existing
  warnings in unrelated modules, e.g. `missing_safety_doc` on
  `emerald_rt_gzip_*`).
- `cargo build --workspace` — clean.
- `cargo nextest run --workspace -j 4` — every test passed except the
  one pre-existing, environment-specific failure this session's own
  task briefing named in advance and explicitly excused from a fix:
  `emerald-driver::cache::tests::
  corrupting_the_cached_object_file_forces_a_real_recompile_not_an_
  error` (a cache-corruption/forced-recompile test, orthogonal to this
  plan's own surface — not touched by any file this plan changes).
