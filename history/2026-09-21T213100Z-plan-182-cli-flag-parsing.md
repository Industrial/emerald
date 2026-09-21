2026-09-21T21:31:00Z

---
name: Structured CLI Flag Parsing — `clap` as a Builder-Sequence Wrapper Over `ARGV`/`ARGC`
overview: "A new `emerald-rt` module wrapping `clap` 4.x's builder API (`clap::Command`/`clap::Arg`, not the `#[derive(Parser)]` macro form, since Emerald has no macro/derive system to mirror it) behind two compiler-provided, non-user-declarable resource classes — `CliParser` (built by a sequence of `.flag`/`.option`/`.positional` calls) and `CliParseResult` (typed accessors plus `--help`/error surfacing) — that consumes plan 45's existing `ARGV: Array[String]`/`ARGC: Int64` pair directly as its input, rather than re-deriving process arguments through some parallel mechanism."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-cli-module-and-resource-classes
    content: "Create `crates/emerald-rt/src/cli.rs`. Register two compiler-provided, non-user-declarable classes — `CliParser` (one hidden field: a plan-93 opaque `u64` handle into a `HashMap<u64, clap::Command>`) and `CliParseResult` (one hidden field: a handle into a `HashMap<u64, clap::ArgMatches>`) — in `emerald-sema`'s `classes` map and `emerald-codegen`'s class registration pass, the same compiler-synthesized-class mechanism plan 92's `leaf-native-error-and-panic-raise` established for `NativeError`, extended here to two classes instead of one and to classes that *do* carry constructor arguments (`CliParser.new(name, version)`) rather than being raised into existence by the runtime alone. Every exported `emerald_rt_cli_*` function's body goes through plan 92's `emerald_rt_fn!` catch-unwind macro; `clap`'s own panics (it can panic on a malformed spec, e.g. two args registered under the same short flag) surface as a rescuable `NativeError`, not a process abort."
    status: pending
  - id: leaf-builder-sequence-flag-option-positional
    content: "`CliParser.new(name: String, version: String): CliParser` (`clap::Command::new(name).version(version)`), `.flag(long: String, short: String, help: String): Void` (`.arg(Arg::new(long).long(long).short(short.chars().next()).help(help).action(ArgAction::SetTrue))`), `.option(long: String, short: String, help: String, required: Boolean): Void` (a string-valued option, `.action(ArgAction::Set)`), `.positional(name: String, help: String, required: Boolean): Void` (`Arg::new(name).help(help).required(required)`, no `.long()`/`.short()` — clap's own convention for distinguishing a positional from a flag/option). Every builder call mutates the `clap::Command` stored behind `CliParser`'s handle in place (`HashMap::get_mut`, replacing the entry — `clap::Command`'s own builder methods consume and return `self`, so the wrapper's real implementation is `cmd = std::mem::take(slot); *slot = cmd.arg(...)`, a real, disclosed workaround for wrapping a consuming builder behind a stable handle, not a hidden inefficiency)."
    status: pending
  - id: leaf-parse-argv-argc-and-array-string-marshaling
    content: "`CliParser.parse(self, argv: Array[String], argc: Int64): CliParseResult`, backed by `emerald_rt_cli_parse(handle: u64, argv: *const *const c_char, argc: i64) -> u64` (the returned `u64` is the new `CliParseResult` handle). `Array[String]` needs an explicit `argc` companion for the identical reason `ARGV`/`ARGC` are a pair and not a single value in plan 45's own design: `Array[T]`'s real codegen representation carries no length prefix (`build_array_lit`'s own doc comment, `crates/emerald-codegen/src/lib.rs` L1657-1662, verified in plan 45's Decision log: 'no length prefix, no bounds checking') — an `emerald-rt` function receiving only a bare `Array[String]` pointer has no way to know how many `char*` slots follow it. `emerald_rt_cli_parse` therefore takes `argv`/`argc` as two separate parameters, exactly mirroring `ARGV`/`ARGC` themselves rather than inventing a length-prefixed or NUL-terminated array convention Emerald's `Array[T]` does not have. The call site in a real Emerald program is `parser.parse(ARGV, ARGC)` — plan 45's own two globals, passed straight through, unmodified, no copy, no re-derivation from `std::env::args()` on the Rust side (which would silently diverge from `ARGV`'s already-established argv[0]-skipping convention if this plan built its own argument-collection path instead of reusing plan 45's)."
    status: pending
  - id: leaf-typed-accessors-and-error-help-surfacing
    content: "`CliParseResult` accessors: `.flag(long: String): Boolean` (a `bool`, never `Option` — a flag not passed is simply `false`), `.value(long: String): Option[String]` (`Some(s)` if the option was supplied, `None` otherwise — plan 73's `Option[T]`, not plan 43's retired `T?`), `.positional_value(name: String): Option[String]`, `.help_requested(): Boolean`, `.help_text(): String` (clap's own auto-generated help, rendered via `Command::render_help()` captured at parse time, before `ArgMatches` is even produced — clap's usual flow auto-prints-and-exits on `--help`, which this wrapper explicitly disables via `.disable_help_flag(false)` plus catching `clap::error::ErrorKind::DisplayHelp` rather than letting `clap` call `std::process::exit` itself, since an FFI callee unilaterally exiting the whole host process is exactly the kind of silent, uncatchable failure plan 92's `catch_unwind`-and-raise convention exists to prevent), and `.error_message(): Option[String]` (`Some(clap's formatted error text)` on a real parse failure — a missing required option, an unrecognized flag — `None` on success). A parse failure does **not** raise a `NativeError`; it is a normal, expected outcome an Emerald program checks and handles (see Decision log), which is why it is a queryable result field, not an exception."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/cli_flag_parsing.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs`'s CI-checked table per plan 95's mandatory docs+example+test checklist. Add `#[test]`s in `emerald-rt` covering: a successful parse with a flag, an option, and a positional all supplied; a missing-required-option failure (`.error_message()` returns `Some(...)`, `.help_requested()` is `false`); a `--help`-triggered parse (`.help_requested()` is `true`, `.help_text()` is non-empty and contains the program name passed to `CliParser.new`). Run the full `AGENTS.md` gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`) plus a clean-checkout end-to-end build, matching plan 91's `leaf-example-and-full-gate` bar."
    status: pending
isProject: false
---

# Plan 182 — Structured CLI Flag Parsing

Plan 45 gave every Emerald program raw access to its own invocation —
`ARGV: Array[String]` and `ARGC: Int64`, populated from the OS's real
`argv`/`argc` at the top of generated `main`, verified directly against
`crates/emerald-codegen/src/lib.rs`'s `define_main` in that plan's own
Decision log. That is deliberately the *entire* mechanism: no flag
syntax, no `--long`/`-short` convention, no `--help` generation, no
type coercion from a string argument to an `Int64` or `Boolean` value —
a program that wants any of that today parses `ARGV` by hand, one
`String` comparison at a time. This plan adds a real, structured layer
on top of that raw pair, using `clap` — the de facto standard Rust CLI
argument parser — without replacing or duplicating `ARGV`/`ARGC`
themselves. `CliParser.parse` takes `ARGV`/`ARGC` as its two arguments,
literally, the same globals plan 45 already populates; this plan adds
no second way for a program to discover its own command line.

Depends on: plan 91 (`emerald-rt` archive and link mechanism), plan 92
(`emerald_rt_<module>_<fn>` naming, `catch_unwind`-wrapped bodies, the
`NativeError`/rescuable-exception convention for a genuine native
panic), plan 93 (the opaque-`u64`-handle resource model both `CliParser`
and `CliParseResult` are built on), plan 95 (the crate-vetting policy
and docs+example+test checklist this plan's own leaves satisfy), and
plan 45 (`ARGV`/`ARGC` themselves — a hard dependency, not a soft
citation: this plan has no other source of process arguments and does
not add one).

## Concrete proof this plan targets

```ruby
parser: CliParser = CliParser.new("greet", "1.0.0")
parser.flag("verbose", "v", "print extra detail")
parser.option("name", "n", "who to greet", true)
parser.positional("suffix", "a trailing word", false)

result: CliParseResult = parser.parse(ARGV, ARGC)

if result.help_requested()
  puts result.help_text()
else
  case result.error_message()
  when Some(msg)
    puts msg
  when None
    name: String = result.value("name") ?? "world"
    puts "Hello, " + name
    if result.flag("verbose")
      puts "(verbose mode on)"
    end
    case result.positional_value("suffix")
    when Some(s)
      puts s
    when None
      puts "no suffix"
    end
  end
end

parser.close()
result.close()
```

Run three ways, each a real compiled-and-linked binary invoked with a
fixed, test-controlled `argv`, exercising a distinct path:
1. `./greet --name Ada --verbose extra` → `Hello, Ada`, then `(verbose
   mode on)`, then `extra`.
2. `./greet` (no `--name`, which is `required: true`) → clap's own
   formatted missing-required-argument error text, e.g. `error: the
   following required arguments were not provided: --name <name>`.
3. `./greet --help` → clap's own generated usage/help text, including
   the literal strings `greet`, `1.0.0`, and the three registered
   flags' help text.

## Decision log

- **No derive macros — Emerald has none, and this plan does not invent
  one to fake clap's usual ergonomics.** `clap`'s own most common,
  most-documented usage today is `#[derive(Parser)]` on a plain struct,
  generating a `Command` from field attributes at compile time. Emerald
  has no macro or attribute/derive system of any kind (verified across
  every prior plan in this repository — no `#[...]`-equivalent syntax
  exists in `grammar.lalrpop`), and adding one solely to mirror clap's
  derive ergonomics would be a vastly larger, separate language feature
  wildly disproportionate to this plan's actual job. This plan uses
  clap's other real, fully-supported, non-macro API instead — the
  builder form (`Command::new(...).arg(Arg::new(...))`) — which is
  exactly as capable, just more verbose at the call site. Stated plainly
  rather than glossed over: an Emerald `CliParser` builder sequence will
  always be more verbose than the Rust derive form it wraps; that is a
  real, permanent ergonomics gap this plan accepts rather than hides.
- **A parse failure is a queryable result, not a raised exception —
  a deliberate split from `NativeError`'s job.** Plan 92's `NativeError`
  exists for a genuine, unexpected native-code failure (a Rust panic
  caught at the FFI boundary) — the kind of thing a caller generally
  cannot have anticipated and structured control flow around in
  advance. A missing `--name` on the command line is the opposite: it
  is the single most expected, most routine outcome a CLI parser
  produces, and every real-world CLI tool's own top-level logic branches
  on it directly (print the error, print usage, exit non-zero) rather
  than wrapping the whole parse in a `begin`/`rescue`. Modeling it as an
  `Option[String]` field on `CliParseResult` (`error_message()`) keeps
  that branch ordinary, visible control flow — a `case`/`when` on
  `Option[String]`, the same pattern plan 73 already established for
  `Option[T]` generally — rather than forcing every CLI tool written in
  Emerald to `rescue` a routine, expected condition.
- **`--help` never calls `std::process::exit` on Emerald's behalf —
  clap's own default behavior is deliberately overridden.** Verified:
  clap's documented default flow for `-h`/`--help` (and `-V`/`--version`)
  is to print the generated text and call `std::process::exit(0)`
  directly from inside `Command::try_get_matches`'s error path, without
  returning control to the caller at all. An FFI callee unilaterally
  terminating the entire host process — skipping every Emerald-level
  `rescue`, every `ensure`/cleanup block, every resource this plan's own
  `CliParser`/`CliParseResult` handles would otherwise need explicit
  `.close()` calls for — is exactly the class of hazard plan 92's whole
  `catch_unwind`-at-every-boundary posture exists to rule out, applied
  here to a *deliberate* `exit()` call rather than an unwinding panic.
  This plan disables that path (`Command::disable_help_flag`-adjacent
  configuration plus intercepting `clap::error::ErrorKind::DisplayHelp`/
  `DisplayVersion` before they become a process exit) and surfaces
  `--help` as an ordinary, inspectable `CliParseResult` field instead —
  the calling Emerald program decides whether and how to actually stop,
  the same way it decides what to do with any other result.
- **`ARGV`/`ARGC` are consumed, never re-derived.** This plan's own
  `emerald_rt_cli_parse` takes `argv: *const *const c_char, argc: i64`
  as plain parameters — it does not call `std::env::args()` internally.
  Two reasons: first, plan 45's own `ARGV` already skips `argv[0]` (the
  program name) to match Ruby's own convention, verified in that plan's
  Decision log — an independent `std::env::args()` call inside
  `emerald-rt` would include the program name unless it re-implemented
  that same skip, a second place the exact same convention could
  silently drift from the first; second, and more simply, a program
  might reasonably want to parse a *sub*-slice of `ARGV` (a subcommand's
  own remaining arguments, say) in a future extension of this module —
  that only stays possible if `CliParser.parse` takes its input
  explicitly rather than reaching for global process state on the Rust
  side. `Array[String]`'s own representation (no length prefix, per
  `build_array_lit`'s doc comment at `crates/emerald-codegen/src/lib.rs`
  L1657-1662) is exactly why `argc` has to travel alongside `argv` as
  its own parameter rather than being inferred — the identical
  representation gap plan 45's own `.split_count`/`.split` pair and
  `ARGV`/`ARGC` pair already work around, not a new one this plan
  discovers.
- **`CliParser` and `CliParseResult` are two separate handles, not one
  reused across build-then-parse.** `clap::Command::get_matches`-family
  methods consume `self` by value in idiomatic Rust clap usage; this
  wrapper's `HashMap::get_mut`-and-`mem::take` workaround (see
  `leaf-builder-sequence-flag-option-positional`) already pays the cost
  of treating a consuming builder as if it were in-place-mutable once,
  for the `.flag`/`.option`/`.positional` sequence — `.parse()` itself
  is not read as consuming the `CliParser` handle a second time (an
  Emerald program may reasonably want to call `.parse()` again against
  the same spec with a different `argv` slice, e.g. in a test harness
  driving the same `CliParser` against several fixed fixtures), so
  `parse` clones the internal `clap::Command` for that one call rather
  than moving out of the handle table's slot. This is a real, small,
  disclosed cost (`clap::Command` clones are not free, though they are
  cheap relative to actually parsing) accepted for the sake of a
  `CliParser` a caller can reuse.
- **`clap`'s real current shape, verified this session against
  docs.rs.** `clap` 4.6.7 (published 14 September 2026, actively
  maintained by the `clap-rs`/`rust-cli` teams) is the workspace's
  target version; its builder types (`Command`, `Arg`, `ArgAction`) are
  part of the non-derive, always-available core API — no `derive`
  feature flag needs enabling for this plan's wrapper to compile,
  keeping `emerald-rt`'s own dependency footprint on `clap` to exactly
  the subset this plan actually uses.
- **Out of scope.** Subcommands (`clap`'s own `Command::subcommand`
  nesting) — a real, common clap feature, but this plan's builder
  sequence has no way yet to express "register a nested parser," and
  bolting that on doubles the surface this plan's own worked proof would
  need to cover; a future plan can add it once this one's flat
  flag/option/positional model is proven. Value type coercion beyond
  `String` (clap's own `value_parser!(i64)`-style typed values) — every
  option value this plan returns is a plain `String`; an Emerald program
  wanting an `Int64` calls `.to_i` itself (plan 45's own existing
  intrinsic), rather than this plan inventing a second, FFI-level type
  system parallel to Emerald's own. Shell-completion generation
  (`clap_complete`) — a separate crate, a separate vetting decision, not
  bundled here. Environment-variable-backed defaults for a flag (clap's
  own `Arg::env` — a real, useful feature, but one this plan defers
  specifically so it doesn't quietly duplicate whatever precedence order
  plan 183's layered configuration loading defines; a CLI flag with a
  silent environment-variable fallback baked into this module, decided
  independently of plan 183's own env-var-precedence design, is exactly
  the kind of "one-off decision instead of one coherent shape" plan 92's
  own overview warns against for the whole `emerald-rt` archive).
