//! Emerald's command-line entry point (plan-of-plans row 06).
//! `emerald-driver` (plan 17's `leaf-driver-extraction`) now owns the
//! parse -> type-check -> codegen -> link pipeline; this crate shrinks
//! to CLI-only concerns — argument parsing, reading the source file,
//! calling into the driver, and rendering the resulting `DriverError`.
//!
//! `emerald new`/`build`/`run` subcommand dispatch (plan 46,
//! `manifest`/`deps`/`lockfile`/`require` modules) sits on top of the
//! same driver; the legacy bare `emerald <source.em> [-o <output>]`
//! invocation (no `emerald.toml` involved) keeps working unchanged for
//! any other `args[1]`.

mod bench_runner;
mod deps;
mod lockfile;
mod manifest;
mod repl;
mod require;
mod test_runner;

use deps::DepsError;
use emerald_driver::DriverError;
use lockfile::Lockfile;
use manifest::{DependencySpec, Manifest, ManifestError};
use require::RequireError;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

enum CliError {
  Manifest(ManifestError),
  Deps(DepsError),
  Require(RequireError),
}

fn report_error(e: CliError) {
  match e {
    CliError::Manifest(e) => eprintln!("error: {e}"),
    CliError::Deps(e) => eprintln!("error: {e}"),
    CliError::Require(e) => eprintln!("error: {e}"),
  }
}

/// Plan 22's `leaf-span-rendering`: a `Diagnostic`'s real byte span,
/// wrapped exactly like `emerald_parser::ParseError` (plan 13) so it
/// renders through the same miette `fancy` graphical handler — a
/// source snippet plus a caret, not a bare one-line message. `source`
/// is `Some((name, text))` only where a single coherent source string
/// actually exists for the whole checked `Program` — `run_legacy`'s
/// single-file path, not `cmd_build`'s multi-file `require`-spliced
/// one (a real, disclosed gap: a `require`-spliced program has no one
/// source string byte offsets index into, so that path keeps today's
/// plain-text rendering unchanged).
#[derive(Debug)]
struct SemaError {
  message: String,
  src: miette::NamedSource<String>,
  span: miette::SourceSpan,
}

impl std::fmt::Display for SemaError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl std::error::Error for SemaError {}

impl miette::Diagnostic for SemaError {
  fn source_code(&self) -> Option<&dyn miette::SourceCode> {
    Some(&self.src)
  }
  fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
    Some(Box::new(std::iter::once(
      miette::LabeledSpan::new_with_span(Some("here".to_string()), self.span),
    )))
  }
}

pub(crate) fn report_driver_error(e: DriverError, source: Option<(&str, &str)>) {
  match e {
    // `ParseError` implements `miette::Diagnostic` (plan 13) — its
    // `{:?}` rendering, via miette's `fancy`-feature graphical
    // handler, is the source-snippet-and-caret display, not a bare
    // one-line message. Plan 26: one report per recovered error, not
    // just the first — still exits non-zero once, after printing all
    // of them.
    DriverError::Parse(errs) => {
      for e in errs {
        eprintln!("{:?}", miette::Report::new(e));
      }
    }
    DriverError::Sema(diags) => {
      for d in &diags {
        match source {
          Some((name, text)) => {
            let report = SemaError {
              message: d.message.clone(),
              src: miette::NamedSource::new(name, text.to_string()),
              span: (d.span.0, d.span.1.saturating_sub(d.span.0).max(1)).into(),
            };
            eprintln!("{:?}", miette::Report::new(report));
          }
          None => eprintln!("error: {}", d.message),
        }
      }
    }
    DriverError::Codegen(e) => eprintln!("codegen error: {e}"),
    DriverError::Link(e) => eprintln!("error: {e}"),
    // Plan 49: a require cycle, a missing/unreadable required file, or
    // a construct `--jobs`-parallel multi-file codegen doesn't support
    // yet — see `emerald_driver::require_graph::unsupported_construct`.
    DriverError::Require(e) => eprintln!("error: {e}"),
  }
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  match args.get(1).map(String::as_str) {
    Some("new") => cmd_new(&args),
    Some("build") => {
      cmd_build(&args);
    }
    Some("run") => cmd_run(&args),
    Some("test") => test_runner::run(&args),
    // Plan 80's Decision log: `property "..." do ... end` compiles and
    // runs through the exact same `test_runner`/`compile_test_harness`
    // mechanism `test` blocks already use (see `Item::Property`'s own
    // doc comment) — a real, disclosed simplification (a `property`
    // block runs its body once, not across many generated inputs), not
    // a separate pipeline. `emerald property <file>.em` is offered
    // alongside `emerald test <file>.em` purely for naming ergonomics
    // on a file that only declares `property` blocks; both subcommands
    // are byte-for-byte the same code path.
    Some("property") => test_runner::run(&args),
    Some("benchmark") => bench_runner::run(&args),
    Some("repl") => repl::run(&args),
    Some("update") => {
      eprintln!(
        "error: `emerald update` is not supported yet — edit the dependency's \
         path/git/rev in emerald.toml and rebuild to re-resolve it (see plan \
         46's Decision log)."
      );
      process::exit(1);
    }
    // Plan 47's Decision log: zero args (a bare `emerald`) enters the
    // REPL too — `run_legacy` below already requires a real
    // `args.get(1)` source-file argument, so this arm must come first.
    None => repl::run(&args),
    _ => run_legacy(&args),
  }
}

/// Plan 48's `leaf-verbose-flag-and-consumer-wiring`: `.emerald/cache/`
/// is a sibling of plan 46's own `.emerald/deps/` convention — reusing
/// the established `.emerald/` namespace rather than inventing a
/// second one. Relative to `cwd` for every cache-aware call site
/// (`run_legacy`'s single-file mode included), so two invocations from
/// the same directory share the same cache.
fn cache_root() -> PathBuf {
  std::env::current_dir()
    .unwrap_or_else(|_| PathBuf::from("."))
    .join(".emerald")
    .join("cache")
}

fn verbose_cache_requested(args: &[String]) -> bool {
  args.iter().any(|a| a == "--verbose-cache")
}

/// Plan 50's `leaf-escape-instrumentation-and-report`: a strictly
/// additive/opt-in flag exactly like `--verbose-cache` above — with no
/// such flag, `run_legacy`'s existing branches are completely
/// unchanged.
fn escape_report_requested(args: &[String]) -> bool {
  args.iter().any(|a| a == "--emit=escape-report")
}

/// Plan 49's `leaf-parallel-codegen-and-jobs-flag`: `--jobs N` — a new,
/// strictly additive/opt-in flag exactly like `--verbose-cache` above.
/// `None` (no flag) means every pre-plan-49 code path here is
/// completely unchanged; `Some(n)` (n.max(1)) routes through
/// `emerald_driver::parallel::compile_parallel` instead.
fn jobs_requested(args: &[String]) -> Option<usize> {
  args
    .iter()
    .position(|a| a == "--jobs")
    .and_then(|i| args.get(i + 1))
    .and_then(|v| v.parse::<usize>().ok())
    .map(|n| n.max(1))
}

/// Plan 61's Decision log: `--comptime-step-limit=N` — a real, strictly
/// additive/opt-in flag exactly like `--verbose-cache`/`--jobs` above.
/// `=`-joined (not space-separated like `--jobs N`), matching the task
/// brief's own literal spelling; `None` (no flag) means `emerald_
/// driver::compile`'s existing `1_000_000`-step default is unchanged.
fn comptime_step_limit_requested(args: &[String]) -> Option<u64> {
  args.iter().find_map(|a| {
    a.strip_prefix("--comptime-step-limit=")
      .and_then(|v| v.parse::<u64>().ok())
  })
}

/// Plan 64's `leaf-cli-target-flag-and-linking`: `--target wasm32-wasi`
/// (or `--target=wasm32-wasi`) — a real, strictly additive/opt-in flag
/// exactly like `--verbose-cache`/`--jobs`/`--comptime-step-limit=N`
/// above. `Ok(None)` (no flag) means `CodegenTarget::Native`, every
/// pre-plan-64 invocation's unchanged behavior; `Err(value)` is a real,
/// reported CLI error for anything other than the two supported
/// spellings (AC3) — never a silent fallback to native.
fn target_requested(args: &[String]) -> Result<Option<emerald_driver::CodegenTarget>, String> {
  let raw = args
    .iter()
    .position(|a| a == "--target")
    .and_then(|i| args.get(i + 1).map(String::as_str))
    .or_else(|| args.iter().find_map(|a| a.strip_prefix("--target=")));
  match raw {
    None => Ok(None),
    Some("wasm32-wasi") => Ok(Some(emerald_driver::CodegenTarget::Wasm32Wasi)),
    Some(other) => Err(format!(
      "unrecognized --target value `{other}` — supported targets: wasm32-wasi (native is the \
       default, no flag needed)"
    )),
  }
}

/// Plan 49: the source path is the first positional (non-flag) argument
/// — `emerald --jobs 2 main.em -o main` (the plan's own worked-example
/// invocation) puts a flag *before* the source path, so this can no
/// longer just be `args[1]` unconditionally, the way it was before
/// `--jobs` existed. Skips `-o`/`--jobs`'s own consumed value too, not
/// just the flag token itself.
fn find_source_path(args: &[String]) -> Option<&String> {
  let mut i = 1;
  while i < args.len() {
    match args[i].as_str() {
      "-o" | "--jobs" | "--target" => i += 2,
      "--verbose-cache" | "--emit=escape-report" => i += 1,
      a if a.starts_with("--comptime-step-limit=") || a.starts_with("--target=") => i += 1,
      _ => return Some(&args[i]),
    }
  }
  None
}

fn run_legacy(args: &[String]) {
  let Some(source_path) = find_source_path(args) else {
    eprintln!(
      "usage: emerald <source.em> [-o <output>] [--verbose-cache] [--jobs N] [--comptime-step-limit=N]  |  emerald new/build/run <name>"
    );
    process::exit(2);
  };

  let output_path = args
    .iter()
    .position(|a| a == "-o")
    .and_then(|i| args.get(i + 1))
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("a.out"));

  // Plan 64's `leaf-cli-target-flag-and-linking`: `--target` is
  // validated before any other flag is even inspected (AC3 — an
  // unrecognized value is a real, reported error regardless of what
  // else was passed), and, when it names `wasm32-wasi`, handled as its
  // own complete compile+link+exit cycle — real, disclosed scope
  // narrowing, the same posture `--emit=escape-report` immediately
  // below already takes toward `--jobs`/`--verbose-cache`.
  let target = target_requested(args).unwrap_or_else(|msg| {
    eprintln!("error: {msg}");
    process::exit(2);
  });
  if let Some(target) = target {
    let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
      eprintln!("error: cannot read `{source_path}`: {e}");
      process::exit(1);
    });
    if let Err(e) = emerald_driver::compile_with_target(&source, source_path, &output_path, target)
    {
      report_driver_error(e, Some((source_path, &source)));
      process::exit(1);
    }
    return;
  }

  // Plan 50: `--emit=escape-report` — strictly additive/opt-in,
  // checked ahead of `--jobs`/`--verbose-cache` and handled as its own
  // complete compile+report+exit cycle (not composed with either —
  // real, disclosed scope narrowing; neither plan 48 nor plan 49
  // anticipated this flag).
  if escape_report_requested(args) {
    let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
      eprintln!("error: cannot read `{source_path}`: {e}");
      process::exit(1);
    });
    match emerald_driver::compile_with_escape_report(&source, source_path, &output_path) {
      Ok(stats) => {
        eprintln!(
          "{} instances stack-allocated, {} heap-allocated",
          stats.stack_allocated, stats.heap_allocated
        );
      }
      Err(e) => {
        report_driver_error(e, Some((source_path, &source)));
        process::exit(1);
      }
    }
    return;
  }

  // Plan 49: `--jobs N` builds and levels `source_path`'s own require
  // graph directly (never `require::resolve_program`'s single-flattened
  // `Program` — see `emerald_driver::parallel`'s own doc comment on
  // why that step is exactly what per-file parallel work needs to
  // avoid) — real end-to-end proof for the plan's own worked example:
  // `emerald-cli --jobs 2 examples/parallel/main.em -o main`.
  if let Some(jobs) = jobs_requested(args) {
    let cache = emerald_driver::cache::QueryCache::new(cache_root());
    let result = if verbose_cache_requested(args) {
      emerald_driver::parallel::compile_parallel(
        Path::new(source_path),
        &output_path,
        jobs,
        &cache,
        &emerald_driver::cache::VerboseReporter,
      )
    } else {
      emerald_driver::parallel::compile_parallel(
        Path::new(source_path),
        &output_path,
        jobs,
        &cache,
        &emerald_driver::cache::SilentReporter,
      )
    };
    if let Err(e) = result {
      // A require-graph program has no single coherent source string
      // (same reasoning `cmd_build`'s own `None` already uses below).
      report_driver_error(e, None);
      process::exit(1);
    }
    return;
  }

  let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
    eprintln!("error: cannot read `{source_path}`: {e}");
    process::exit(1);
  });

  // Plan 69: a bare `emerald <file>.em -o out` (no `--jobs`) used to
  // hand `source` straight to `compile`/`compile_cached`/
  // `compile_with_comptime_step_limit` without ever looking at its
  // `require`s — `emerald_sema` treats a `require` item as a no-op
  // (`ast.rs`'s own doc comment on `Item::Require`), so every symbol
  // the required file was supposed to bring into scope (a class, an
  // actor, a plain function) came back "unknown type"/"undefined
  // variable"/"undefined function" at typecheck instead, even though
  // `require.rs`'s own splicer (plan 23) has always spliced correctly
  // once actually invoked — `cmd_build`'s `emerald build` path (which
  // needs an `emerald.toml`) was the only caller ever reaching it.
  // Detected by a cheap up-front parse (reusing `emerald_parser`
  // directly, the same dependency `require.rs` already has) rather
  // than a textual scan, so a `require` appearing only inside a
  // string/comment can't false-positive; a parse failure here is left
  // alone and falls through to the unchanged `compile`/`compile_cached`/
  // `compile_with_comptime_step_limit` call below, which reports it
  // with the exact same rich span-based rendering as always.
  let requires_present = emerald_parser::parse_named(&source, source_path)
    .map(|program| {
      program
        .items
        .iter()
        .any(|item| matches!(item, emerald_parser::Item::Require(_)))
    })
    .unwrap_or(false);

  if requires_present {
    let (program, hashes) = require::resolve_program_with_hashes(Path::new(source_path))
      .unwrap_or_else(|e| {
        report_error(CliError::Require(e));
        process::exit(1);
      });
    // No single coherent source string exists for a `require`-spliced
    // `Program` (`report_driver_error`'s own doc comment has the full
    // reasoning, `cmd_build` already relies on it below in the same
    // way) — `--comptime-step-limit` has no `_program`-taking
    // counterpart to route through here, the same real, disclosed
    // narrowing `--jobs`'s own early return above already accepts for
    // that flag.
    let result = if verbose_cache_requested(args) {
      let cache = emerald_driver::cache::QueryCache::new(cache_root());
      let reporter = emerald_driver::cache::VerboseReporter;
      let key = cache.key_for_many(&hashes.iter().map(|(_, h)| *h).collect::<Vec<_>>());
      emerald_driver::compile_program_cached(
        program,
        key,
        source_path,
        &output_path,
        &cache,
        &reporter,
      )
    } else {
      emerald_driver::compile_program(program, &output_path)
    };
    if let Err(e) = result {
      report_driver_error(e, None);
      process::exit(1);
    }
    return;
  }

  // Plan 48: `--verbose-cache` is strictly additive and opt-in — with
  // no such flag, this is the exact `emerald_driver::compile` call
  // every prior plan's test already proves, unchanged.
  //
  // Plan 61's Decision log: `--comptime-step-limit=N` is real, disclosed
  // narrow scope — wired only into this plain (uncached) path, not
  // `--verbose-cache`'s own cached one (threading it through `QueryCache`
  // is `leaf-comptime-query-cache-integration`'s own job, not this
  // leaf's).
  let result = if verbose_cache_requested(args) {
    let cache = emerald_driver::cache::QueryCache::new(cache_root());
    let reporter = emerald_driver::cache::VerboseReporter;
    emerald_driver::compile_cached(&source, source_path, &output_path, &cache, &reporter)
  } else if let Some(limit) = comptime_step_limit_requested(args) {
    emerald_driver::compile_with_comptime_step_limit(&source, source_path, &output_path, limit)
  } else {
    emerald_driver::compile(&source, source_path, &output_path)
  };

  if let Err(e) = result {
    report_driver_error(e, Some((source_path, &source)));
    process::exit(1);
  }
}

fn cmd_new(args: &[String]) {
  let Some(name) = args.get(2) else {
    eprintln!("usage: emerald new <name>");
    process::exit(2);
  };
  let dir = PathBuf::from(name);
  if let Err(e) = std::fs::create_dir_all(&dir) {
    eprintln!("error: cannot create `{}`: {e}", dir.display());
    process::exit(1);
  }
  let manifest_toml =
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"main.em\"\n");
  let main_em = format!("puts \"Hello from {name}!\"\n");
  if let Err(e) = std::fs::write(dir.join("emerald.toml"), manifest_toml) {
    eprintln!(
      "error: cannot write `{}`: {e}",
      dir.join("emerald.toml").display()
    );
    process::exit(1);
  }
  if let Err(e) = std::fs::write(dir.join("main.em"), main_em) {
    eprintln!(
      "error: cannot write `{}`: {e}",
      dir.join("main.em").display()
    );
    process::exit(1);
  }
  println!("     Created package `{name}` at ./{name}");
}

fn load_manifest_or_exit(dir: &Path) -> Manifest {
  match Manifest::load(dir) {
    Ok(m) => m,
    Err(e) => {
      report_error(CliError::Manifest(e));
      eprintln!(
        "hint: `emerald build`/`emerald run` require an `emerald.toml` \
         in the current directory (see `emerald new`)."
      );
      process::exit(1);
    }
  }
}

/// Resolves the manifest, its dependencies, and its `require`s
/// (`require.rs` — the multi-file splicing `emerald-driver` doesn't do
/// itself, see its own doc comment), then hands the assembled
/// `Program` to `emerald_driver::compile_program`. Returns the path to
/// the linked binary — never returns on failure (matches
/// `run_legacy`'s own exit-on-error shape).
fn cmd_build(args: &[String]) -> PathBuf {
  let cwd = std::env::current_dir().unwrap_or_else(|e| {
    eprintln!("error: cannot read current directory: {e}");
    process::exit(1);
  });
  let manifest = load_manifest_or_exit(&cwd);

  for (name, spec) in &manifest.dependencies {
    let desc = match spec {
      DependencySpec::Path { path } => format!("path {path}"),
      DependencySpec::Git { git, rev } => format!("git {git} rev {rev}"),
    };
    println!("   Resolving {name} ({desc})");
  }
  let existing_lock = Lockfile::load(&cwd);
  if let Err(e) = deps::resolve_or_reuse(&cwd, &manifest, existing_lock.as_ref()) {
    report_error(CliError::Deps(e));
    process::exit(1);
  }

  println!(
    "   Compiling {} v{} ({})",
    manifest.package.name, manifest.package.version, manifest.package.entry
  );

  let entry_path = cwd.join(&manifest.package.entry);
  // Plan 48: `resolve_program_with_hashes` is `resolve_program`'s own
  // superset (unconditionally used — a pure function with no side
  // effects, so calling it costs nothing when `--verbose-cache` is
  // absent); only the cache-aware branch below actually reaches for
  // the per-file hashes it also returns.
  let (program, hashes) = require::resolve_program_with_hashes(&entry_path).unwrap_or_else(|e| {
    report_error(CliError::Require(e));
    process::exit(1);
  });

  // Plan 64's `leaf-cli-target-flag-and-linking`: `--target` validated
  // the same way `run_legacy`'s own copy is (AC3) — an unrecognized
  // value is a real, reported error, never a silent fallback to
  // native. `Wasm32Wasi`'s own output path gains a `.wasm` extension
  // (AC2 — `wasmtime run app.wasm` vs. this plan's own `./app` worked
  // proof), the conventional extension for a WASM module.
  let target = target_requested(args).unwrap_or_else(|msg| {
    eprintln!("error: {msg}");
    process::exit(2);
  });
  let output_path = match target {
    Some(emerald_driver::CodegenTarget::Wasm32Wasi) => {
      cwd.join(format!("{}.wasm", manifest.package.name))
    }
    _ => cwd.join(&manifest.package.name),
  };
  let result = match (target, verbose_cache_requested(args)) {
    (Some(target), _) => emerald_driver::compile_program_with_libs_and_target(
      program,
      &output_path,
      &manifest.ffi.link,
      target,
    ),
    (None, true) => {
      let cache = emerald_driver::cache::QueryCache::new(cache_root());
      let reporter = emerald_driver::cache::VerboseReporter;
      let key = cache.key_for_many(&hashes.iter().map(|(_, h)| *h).collect::<Vec<_>>());
      emerald_driver::compile_program_cached_with_libs(
        program,
        key,
        &manifest.package.entry,
        &output_path,
        &cache,
        &reporter,
        &manifest.ffi.link,
      )
    }
    (None, false) => {
      emerald_driver::compile_program_with_libs(program, &output_path, &manifest.ffi.link)
    }
  };
  if let Err(e) = result {
    // No single coherent source string exists for a `require`-spliced
    // `Program` (see `report_driver_error`'s own doc comment) — plain
    // text, unchanged.
    report_driver_error(e, None);
    process::exit(1);
  }

  println!(
    "    Finished build: ./{}",
    output_path
      .file_name()
      .map(|f| f.to_string_lossy().to_string())
      .unwrap_or_else(|| manifest.package.name.clone())
  );
  output_path
}

fn cmd_run(args: &[String]) {
  // Plan 64's `leaf-cli-target-flag-and-linking` AC4: a `.wasm` module
  // is not a directly-executable native binary — `Command::new` below
  // would otherwise fail with a confusing native-launcher error (e.g.
  // "Exec format error"), not a clear, named rejection.
  if let Ok(Some(emerald_driver::CodegenTarget::Wasm32Wasi)) = target_requested(args) {
    eprintln!(
      "error: `emerald run --target wasm32-wasi` is not supported — a compiled `.wasm` module \
       needs a WASI host (e.g. `wasmtime run`) to execute, not a direct native launch. Use \
       `emerald build --target wasm32-wasi` and run the result under `wasmtime` instead."
    );
    process::exit(2);
  }
  let output_path = cmd_build(args);
  let status = Command::new(&output_path).status().unwrap_or_else(|e| {
    eprintln!("error: failed to run `{}`: {e}", output_path.display());
    process::exit(1);
  });
  process::exit(status.code().unwrap_or(1));
}
