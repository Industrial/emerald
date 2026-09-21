2026-09-21T21:32:00Z

---
name: Layered Configuration Loading — `config` Over Files, Env, and CLI Flags
overview: "A new `emerald-rt` module wrapping the `config` crate (chosen over `figment` this session on a real, verified adoption signal — see Decision log), exposing a `ConfigBuilder` resource that merges TOML/YAML files (plans 119/120), environment variables (plan 146), and parsed CLI flags (plan 182) into one queryable, precedence-ordered configuration value, with the precedence order — CLI flags highest, then environment variables, then config file, then a program-supplied default — stated and justified explicitly rather than left arbitrary."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-scaffold-config-module-and-builder-resource
    content: "Create `crates/emerald-rt/src/config.rs`. Register a compiler-provided, non-user-declarable `ConfigBuilder` class (one hidden `u64` handle field, per plan 93, into a `HashMap<u64, config::ConfigBuilder<config::builder::DefaultState>>`) and a `ConfigValue` result class (one hidden handle into a `HashMap<u64, config::Config>`, the crate's own already-merged, already-frozen output type). Every exported `emerald_rt_config_*` function goes through plan 92's `emerald_rt_fn!` catch-unwind macro; a malformed source (invalid TOML/YAML syntax, an environment variable that can't coerce to a requested type) becomes a `NativeError` at `.build()` time, not a panic or a silent empty config."
    status: pending
  - id: leaf-source-registration-methods
    content: "`ConfigBuilder.new(): ConfigBuilder`, `.add_defaults_file(path: String): Void` (lowest precedence — `config::File::with_name(path).required(false)`, tolerating a missing defaults file rather than failing), `.add_config_file(path: String): Void` (the program's real config file — TOML via plan 119's parser or YAML via plan 120's, both already real `config::Source` implementations the `config` crate itself supports natively through its own `toml`/`yaml` feature flags, not reimplemented by this plan), `.add_env_prefix(prefix: String): Void` (`config::Environment::with_prefix(prefix).separator(\"__\")` — the crate's own documented convention for mapping `APPNAME__SECTION__KEY` env vars onto nested config keys, reused verbatim rather than inventing a different separator), and `.add_cli_overrides(result: CliParseResult, keys: Array[String], count: Int64): Void` (walks plan 182's `CliParseResult.value(key)` for each of the given `keys` and layers any `Some(v)` as the highest-precedence source via `config::Config::builder().set_override(key, v)` — see Decision log for why this is a manual bridge rather than `config` parsing `ArgMatches` itself, which it has no built-in support for). Each registration call appends one more layered source to the builder held behind the handle, in the exact order called — `config`'s own merge semantics are last-registered-wins per key, which this plan's fixed call order (`add_defaults_file` → `add_config_file` → `add_env_prefix` → `add_cli_overrides`) turns into the stated precedence order."
    status: pending
  - id: leaf-build-and-typed-getters
    content: "`ConfigBuilder.build(self): ConfigValue` (`.build()`, wrapped in `catch_unwind`, converting a `config::ConfigError` into a `NativeError` with the crate's own formatted message — e.g. which file failed to parse and why). `ConfigValue` accessors: `.get_string(key: String): Option[String]`, `.get_int(key: String): Option[Int64]`, `.get_bool(key: String): Option[Boolean]` — each a thin wrapper over `config::Config::get::<T>(key)`, converting `Err` (key absent, or present with the wrong shape) uniformly to `None` rather than distinguishing 'missing' from 'wrong type' — a real, disclosed simplification (see Decision log) chosen because `Option[T]`, not a second `Result[T,E]`-shaped error channel, is this plan's whole external contract. Dotted keys (`\"server.port\"`) work exactly as `config`'s own nested-key addressing already supports, with zero special-casing on Emerald's side."
    status: pending
  - id: leaf-example-and-gate
    content: "Add `examples/layered_config.em` (the Concrete Proof below) to `examples/`, wired into `emerald-cli/tests/examples.rs`'s CI-checked table per plan 95's mandatory checklist, run from a fixed temporary directory with a real TOML fixture file, a real set environment variable, and a real fixed `argv`, so the precedence order is demonstrated by genuine layered inputs, not asserted in prose alone. Add `#[test]`s in `emerald-rt` covering: file-only resolution, env-overriding-file, CLI-overriding-both, and a missing key producing `None` rather than a panic. Run the full `AGENTS.md` gate (`cargo nextest run --workspace`, `cargo clippy --workspace --all-targets`, `treefmt`) plus a clean-checkout end-to-end build."
    status: pending
isProject: false
---

# Plan 183 — Layered Configuration Loading

A real program's configuration rarely comes from one place: a checked-in
defaults file, a deployment-specific config file, environment variables
set by the surrounding process manager, and a handful of flags a human
typed on the command line for this one invocation. Plans 119/120 (TOML/
YAML parsing), 146 (environment variables), and 182 (this batch's own
structured CLI parsing) each give Emerald a way to read *one* of those
sources. None of them, alone or combined by hand, gives a program one
merged view with a defined precedence — a caller today would have to
write the override logic itself, once per program, inconsistently. This
plan is that merge layer: one `ConfigBuilder`, fed each source in a
fixed, load-bearing call order, producing one `ConfigValue` a program
queries by key without caring which layer actually supplied the answer.

Depends on: plan 91 (`emerald-rt` archive), plan 92 (`catch_unwind`
convention, `NativeError`), plan 93 (opaque-handle resource model for
`ConfigBuilder`/`ConfigValue`), plan 95 (crate-vetting policy and
checklist), plan 119/120 (the real TOML/YAML `config::Source`
implementations this plan's file layer delegates to), plan 146
(environment-variable access — cited for scope boundary, not code reuse:
`config::Environment` reads `std::env` directly on the Rust side, so
this plan does not call through plan 146's own accessor functions, see
Decision log), and plan 182 (`CliParseResult`, this plan's highest-
precedence source).

## Concrete proof this plan targets

Given a fixture file `app.toml` in the working directory:

```toml
[server]
port = 8080
host = "0.0.0.0"
```

and run with `APP__SERVER__PORT=9090` set in the environment and
invoked as `./app --port 7070`:

```ruby
parser: CliParser = CliParser.new("app", "1.0.0")
parser.option("port", "p", "server port override", false)
cli_result: CliParseResult = parser.parse(ARGV, ARGC)

builder: ConfigBuilder = ConfigBuilder.new()
builder.add_config_file("app")
builder.add_env_prefix("APP")
override_keys: Array[String] = ["server.port"]
builder.add_cli_overrides(cli_result, override_keys, 1)

cfg: ConfigValue = builder.build()

case cfg.get_string("server.port")
when Some(port)
  puts port
when None
  puts "no port configured"
end

case cfg.get_string("server.host")
when Some(host)
  puts host
when None
  puts "no host configured"
end
```

Expected output: `7070` (the CLI flag wins over both the `9090`
environment variable and the `8080` file value), then `0.0.0.0` (no
env var or CLI flag touches `server.host`, so the file value survives
unshadowed) — a real, three-layer precedence proof, not a two-source
toy case that could pass by accident.

## Decision log

- **`config`, not `figment` — a real, verified-this-session maintenance
  signal, not a coin flip.** Both are real, widely used layered-config
  crates for Rust with broadly overlapping feature sets (file + env +
  defaults, nested keys, multiple format support). Checked directly
  against crates.io this session: the `config` crate's most recent
  publish is dated **21 September 2026** — the day before this plan was
  written — while `figment`'s most recent publish is dated **17 May
  2024**, over two years stale by comparison. For a foundational,
  permanent `emerald-rt` module every future config-consuming program
  depends on, an actively-published crate with a maintained release
  cadence is the safer long-term bet over one that, whatever its past
  design merits, has gone quiet. This is the real call plan 95's
  crate-vetting policy asks every domain plan to make explicitly rather
  than defaulting to whichever crate is more familiar — the comparison
  is decided on freshness evidence pulled this session, not on prior
  general reputation.
- **Precedence order, stated explicitly: CLI flag > environment variable
  > config file > built-in/defaults file.** This is the same order
  nearly every mature CLI tool (`git`, `cargo`, `kubectl`, `docker`)
  converges on independently, for a consistent reason: each layer
  represents a progressively more specific, more temporary intent. A
  defaults file is the program's own checked-in baseline, true for
  every invocation until someone changes the file. A config file is a
  deployment's semi-permanent choice, true until someone redeploys. An
  environment variable is the surrounding process's choice for this
  *session* — often injected by an orchestrator per container, per
  shell. A CLI flag is one human's explicit choice for *this one
  invocation*, typed by hand, right now — the most specific, most
  recent, most deliberate signal available, and therefore the one that
  should win when two sources disagree. This plan's fixed builder call
  order (`add_defaults_file` → `add_config_file` → `add_env_prefix` →
  `add_cli_overrides`) encodes that reasoning directly into the
  registration sequence, relying on `config`'s own documented
  last-registered-wins merge semantics rather than this plan inventing
  a separate priority-number system alongside it.
- **CLI overrides are bridged manually, not natively — `config` has no
  built-in `clap`/`ArgMatches` source.** Verified: the `config` crate's
  own `Source` trait has real, built-in implementations for files
  (`config::File`) and environment variables (`config::Environment`),
  but none for a CLI parser's output — that integration is conventionally
  left to the application, in every real-world `config`-crate usage
  this session's research turned up. `leaf-source-registration-methods`'s
  `add_cli_overrides` is this plan's own bridge: it does not hand
  `config` a `clap::ArgMatches` to parse a second time; it reads plan
  182's *already-parsed* `CliParseResult` values directly and calls
  `set_override` once per supplied key. This also means plan 182's own
  parse step runs exactly once per program — this plan is strictly a
  consumer of its output, never a second parser of `ARGV`.
- **Environment variables are read by `config::Environment` directly,
  not proxied through plan 146's own accessor functions.** Plan 146
  gives an Emerald *program* a way to read one named environment
  variable (`Env.get("SOME_VAR")`-shaped, exact surface is that plan's
  own decision, not re-derived here). This plan's `.add_env_prefix`
  does not call through that surface — `config::Environment`'s own Rust
  implementation reads `std::env::vars()` directly, bulk-scanning for
  everything matching the given prefix, which plan 146's presumed
  single-variable-lookup shape has no way to do efficiently or at all.
  The two modules therefore both ultimately read the same underlying
  process environment, through two independent, non-overlapping code
  paths — stated explicitly here so a future reader does not assume
  this plan is layered on top of plan 146's own exported functions.
- **A missing key and a wrong-shape key both collapse to `None` — a
  real, disclosed loss of information, not an oversight.** `config`'s
  own `Config::get::<T>` returns a `Result<T, ConfigError>`, and
  `ConfigError` distinguishes a genuinely absent key from one present
  with a value that fails to deserialize as the requested `T` (e.g.
  `server.port = "not-a-number"` requested via `.get_int`). This plan's
  `Option[T]`-only accessor surface cannot represent that distinction —
  a caller sees `None` either way. The alternative (`Result[T, String]`
  accessors carrying `config`'s own error text) was considered and
  declined for this plan's v1: it would make every single config read
  a `case`/`when Ok`/`Err` site for a class of error (a config file with
  a malformed value) that is realistically caught once, at startup, by
  a program that validates its whole config shape up front — not
  something worth taxing every individual `.get_string` call site for.
  A validated, typed config *schema* (closer to `config`'s own `serde`
  deserialization-into-a-struct support) is a natural, larger follow-up
  this plan explicitly does not attempt.
- **File format parsing is delegated to plans 119/120, not
  reimplemented.** `config::File::with_name(path)` auto-detects format
  by trying each of the crate's enabled format features in turn (or by
  a file's own extension when one is given); this plan enables exactly
  the `toml` and `yaml` `config`-crate feature flags, so a config file
  is parsed by the same underlying `toml`/`serde_yaml`-family crates
  plans 119/120 already vetted for Emerald's own direct TOML/YAML
  reading — `config`'s file layer is a thin orchestration wrapper around
  parsers this project already trusts, not a third, independently
  vetted parsing implementation.
- **Out of scope.** Live config reload (watching a config file for
  changes and re-merging without a process restart) — `config` has no
  built-in file-watching of its own, and bolting one on (via a
  filesystem-notification crate) is a substantial, separate feature
  this plan's static, build-once-and-query `ConfigValue` does not
  attempt. Deserializing a whole config into a user-defined Emerald
  class in one call (`config`'s own `serde`-struct-deserialization path)
  — blocked structurally, not by choice: Emerald has no `serde`-
  equivalent derive mechanism (the same limitation plan 182 names for
  clap's derive form), so every value this plan exposes is read one
  typed key at a time through `.get_string`/`.get_int`/`.get_bool`, not
  materialized into a whole struct at once. Secrets/keyring-backed
  config sources (`figment-keyring`-style) — a real, separate crate
  and trust boundary, not folded into this plan's file/env/CLI scope.
