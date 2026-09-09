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

fn run_legacy(args: &[String]) {
  let Some(source_path) = args.get(1) else {
    eprintln!(
      "usage: emerald <source.em> [-o <output>] [--verbose-cache]  |  emerald new/build/run <name>"
    );
    process::exit(2);
  };

  let output_path = args
    .iter()
    .position(|a| a == "-o")
    .and_then(|i| args.get(i + 1))
    .map(PathBuf::from)
    .unwrap_or_else(|| PathBuf::from("a.out"));

  let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
    eprintln!("error: cannot read `{source_path}`: {e}");
    process::exit(1);
  });

  // Plan 48: `--verbose-cache` is strictly additive and opt-in — with
  // no such flag, this is the exact `emerald_driver::compile` call
  // every prior plan's test already proves, unchanged.
  let result = if verbose_cache_requested(args) {
    let cache = emerald_driver::cache::QueryCache::new(cache_root());
    let reporter = emerald_driver::cache::VerboseReporter;
    emerald_driver::compile_cached(&source, source_path, &output_path, &cache, &reporter)
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

  let output_path = cwd.join(&manifest.package.name);
  let result = if verbose_cache_requested(args) {
    let cache = emerald_driver::cache::QueryCache::new(cache_root());
    let reporter = emerald_driver::cache::VerboseReporter;
    let key = cache.key_for_many(&hashes.iter().map(|(_, h)| *h).collect::<Vec<_>>());
    emerald_driver::compile_program_cached(
      program,
      key,
      &manifest.package.entry,
      &output_path,
      &cache,
      &reporter,
    )
  } else {
    emerald_driver::compile_program(program, &output_path)
  };
  if let Err(e) = result {
    // No single coherent source string exists for a `require`-spliced
    // `Program` (see `report_driver_error`'s own doc comment) — plain
    // text, unchanged.
    report_driver_error(e, None);
    process::exit(1);
  }

  println!("    Finished build: ./{}", manifest.package.name);
  output_path
}

fn cmd_run(args: &[String]) {
  let output_path = cmd_build(args);
  let status = Command::new(&output_path).status().unwrap_or_else(|e| {
    eprintln!("error: failed to run `{}`: {e}", output_path.display());
    process::exit(1);
  });
  process::exit(status.code().unwrap_or(1));
}
