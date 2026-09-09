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

pub(crate) fn report_driver_error(e: DriverError) {
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
        eprintln!("error: {}", d.message);
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
      cmd_build();
    }
    Some("run") => cmd_run(),
    Some("test") => test_runner::run(&args),
    Some("repl") => repl::run(),
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
    None => repl::run(),
    _ => run_legacy(&args),
  }
}

fn run_legacy(args: &[String]) {
  let Some(source_path) = args.get(1) else {
    eprintln!("usage: emerald <source.em> [-o <output>]  |  emerald new/build/run <name>");
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

  if let Err(e) = emerald_driver::compile(&source, source_path, &output_path) {
    report_driver_error(e);
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
fn cmd_build() -> PathBuf {
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
  let program = require::resolve_program(&entry_path).unwrap_or_else(|e| {
    report_error(CliError::Require(e));
    process::exit(1);
  });

  let output_path = cwd.join(&manifest.package.name);
  if let Err(e) = emerald_driver::compile_program(program, &output_path) {
    report_driver_error(e);
    process::exit(1);
  }

  println!("    Finished build: ./{}", manifest.package.name);
  output_path
}

fn cmd_run() {
  let output_path = cmd_build();
  let status = Command::new(&output_path).status().unwrap_or_else(|e| {
    eprintln!("error: failed to run `{}`: {e}", output_path.display());
    process::exit(1);
  });
  process::exit(status.code().unwrap_or(1));
}
