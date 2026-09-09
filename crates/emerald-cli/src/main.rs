//! Emerald's command-line entry point (plan-of-plans row 06). Orchestrates
//! parse -> type-check -> codegen -> link directly here rather than in a
//! separate `emerald-driver` crate (see plan 06's Decision log: extracted
//! when a second caller needs the same pipeline).
//!
//! The pipeline is expressed as a chain of `id_effect::Effect` values
//! (plan 14, inception §15's "compiler pipeline orchestration" use
//! case) — each stage stays exactly the plain `Result`-returning
//! function it already was; only the *sequencing* between stages is
//! expressed through `Effect`/`.flat_map` instead of four repeated
//! `match { Ok/Err }` blocks. No capability DI, no async — see plan
//! 14's Decision log for why that's the right amount of the crate to
//! use here, not more.
//!
//! Plan 46 adds `new`/`build`/`run` subcommand dispatch on top of this
//! same pipeline (`manifest`/`deps`/`lockfile`/`require` modules); the
//! legacy bare `emerald <source.em> [-o <output>]` invocation (no
//! `emerald.toml` involved) keeps working unchanged for any other
//! `args[1]`.

mod deps;
mod lockfile;
mod manifest;
mod require;

use deps::DepsError;
use emerald_parser::{ParseError, Program};
use id_effect::{Effect, run_blocking};
use lockfile::Lockfile;
use manifest::{DependencySpec, Manifest, ManifestError};
use require::RequireError;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

enum CliError {
  /// Plan 26: `emerald_parser::parse_named` reports every top-level
  /// `Item` boundary's syntax error in one pass, not just the first.
  Parse(Vec<ParseError>),
  Sema(Vec<emerald_sema::Diagnostic>),
  Codegen(String),
  Link(String),
  Manifest(ManifestError),
  Deps(DepsError),
  Require(RequireError),
}

fn report_error(e: CliError) {
  match e {
    CliError::Parse(errs) => {
      for e in errs {
        eprintln!("{:?}", miette::Report::new(e));
      }
    }
    CliError::Sema(diags) => {
      for d in &diags {
        eprintln!("error: {}", d.message);
      }
    }
    CliError::Codegen(e) => eprintln!("codegen error: {e}"),
    CliError::Link(e) => eprintln!("error: {e}"),
    CliError::Manifest(e) => eprintln!("error: {e}"),
    CliError::Deps(e) => eprintln!("error: {e}"),
    CliError::Require(e) => eprintln!("error: {e}"),
  }
}

fn parse_stage(source: String, name: String) -> Effect<Program, CliError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_parser::parse_named(&source, &name).map_err(CliError::Parse)
  })
}

fn check_stage(program: Program) -> Effect<Program, CliError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_sema::check_program(&program)
      .map(|()| program)
      .map_err(CliError::Sema)
  })
}

fn codegen_stage(program: Program, obj_path: PathBuf) -> Effect<PathBuf, CliError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_codegen::compile_to_object(&program, &obj_path)
      .map(|()| obj_path)
      .map_err(CliError::Codegen)
  })
}

/// Plan 27: the compiled runtime archive's real bytes, embedded into
/// this binary at *compile* time (`build.rs` compiles `runtime/
/// emerald_runtime.c` via the `cc` crate and points
/// `EMERALD_RUNTIME_ARCHIVE` at the resulting `.a`) — not a path
/// looked up at runtime. This is what makes a shipped `emerald-cli`
/// binary, copied alone with no access to this repo's checkout, still
/// able to link a user's compiled program: the runtime archive travels
/// inside the binary itself.
static RUNTIME_ARCHIVE: &[u8] = include_bytes!(env!("EMERALD_RUNTIME_ARCHIVE"));

/// -no-pie: `emerald-codegen` emits non-PIC code (see its `host_isa`),
/// so the executable must not be a PIE either — otherwise `ld` warns
/// about (harmless but avoidable) DT_TEXTREL relocations. The object
/// file and the extracted runtime archive are both removed once
/// linking is attempted, success or failure.
fn link_stage(obj_path: PathBuf, output_path: PathBuf) -> Effect<(), CliError, ()> {
  Effect::new(move |_env: &mut ()| {
    let runtime_archive_path =
      std::env::temp_dir().join(format!("libemerald_runtime_{}.a", process::id()));
    if let Err(e) = std::fs::write(&runtime_archive_path, RUNTIME_ARCHIVE) {
      std::fs::remove_file(&obj_path).ok();
      return Err(CliError::Link(format!(
        "failed to extract the embedded runtime archive: {e}"
      )));
    }
    let link_result = Command::new("cc")
      .arg("-no-pie")
      .arg(&obj_path)
      .arg(&runtime_archive_path)
      .arg("-o")
      .arg(&output_path)
      .status();
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&runtime_archive_path).ok();
    match link_result {
      Ok(status) if status.success() => Ok(()),
      Ok(_) => Err(CliError::Link("linking failed".to_string())),
      Err(e) => Err(CliError::Link(format!("failed to invoke cc: {e}"))),
    }
  })
}

/// Splices `entry_path`'s own `require`s in place (plan 23's design,
/// resolved by `require.rs` since `emerald-driver`'s `resolve_program`
/// doesn't exist yet) instead of a plain single-file parse — this is
/// the only difference between the manifest-driven `build`/`run`
/// pipeline and the legacy single-file one.
fn require_stage(entry_path: PathBuf) -> Effect<Program, CliError, ()> {
  Effect::new(move |_env: &mut ()| require::resolve_program(&entry_path).map_err(CliError::Require))
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  match args.get(1).map(String::as_str) {
    Some("new") => cmd_new(&args),
    Some("build") => {
      cmd_build();
    }
    Some("run") => cmd_run(),
    Some("update") => {
      eprintln!(
        "error: `emerald update` is not supported yet — edit the dependency's \
         path/git/rev in emerald.toml and rebuild to re-resolve it (see plan \
         46's Decision log)."
      );
      process::exit(1);
    }
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

  let obj_path = std::env::temp_dir().join(format!("emerald_{}.o", process::id()));

  let pipeline = parse_stage(source, source_path.clone())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_stage(program, obj_path))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));

  if let Err(e) = run_blocking(pipeline, ()) {
    report_error(e);
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

/// Resolves the manifest, its dependencies, and its `require`s, then
/// runs the same check -> codegen -> link pipeline the legacy path
/// uses. Returns the path to the linked binary — never returns on
/// failure (matches `run_legacy`'s own exit-on-error shape).
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
  let obj_path = std::env::temp_dir().join(format!("emerald_{}.o", process::id()));
  let output_path = cwd.join(&manifest.package.name);
  let output_path_for_link = output_path.clone();

  let pipeline = require_stage(entry_path)
    .flat_map(check_stage)
    .flat_map(move |program| codegen_stage(program, obj_path))
    .flat_map(move |obj_path| link_stage(obj_path, output_path_for_link));

  if let Err(e) = run_blocking(pipeline, ()) {
    report_error(e);
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
