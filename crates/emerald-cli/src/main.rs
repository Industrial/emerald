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

use emerald_parser::{ParseError, Program};
use id_effect::{Effect, run_blocking};
use std::path::PathBuf;
use std::process::{self, Command};

enum CliError {
  Parse(ParseError),
  Sema(Vec<emerald_sema::Diagnostic>),
  Codegen(String),
  Link(String),
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

/// Dev-time runtime location: this repo's `runtime/emerald_runtime.c`,
/// found relative to this crate's manifest dir. A real install would
/// bundle a compiled runtime object/archive instead — noted as
/// follow-up packaging work, not solved here.
///
/// -no-pie: `emerald-codegen` emits non-PIC code (see its `host_isa`),
/// so the executable must not be a PIE either — otherwise `ld` warns
/// about (harmless but avoidable) DT_TEXTREL relocations. The object
/// file is removed once linking is attempted, success or failure.
fn link_stage(obj_path: PathBuf, output_path: PathBuf) -> Effect<(), CliError, ()> {
  Effect::new(move |_env: &mut ()| {
    let runtime_path =
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/emerald_runtime.c");
    let link_result = Command::new("cc")
      .arg("-no-pie")
      .arg(&obj_path)
      .arg(&runtime_path)
      .arg("-o")
      .arg(&output_path)
      .status();
    std::fs::remove_file(&obj_path).ok();
    match link_result {
      Ok(status) if status.success() => Ok(()),
      Ok(_) => Err(CliError::Link("linking failed".to_string())),
      Err(e) => Err(CliError::Link(format!("failed to invoke cc: {e}"))),
    }
  })
}

fn main() {
  let args: Vec<String> = std::env::args().collect();
  let Some(source_path) = args.get(1) else {
    eprintln!("usage: emerald-cli <source.em> [-o <output>]");
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
    match e {
      // `ParseError` implements `miette::Diagnostic` (plan 13) — its
      // `{:?}` rendering, via miette's `fancy`-feature graphical
      // handler, is the source-snippet-and-caret display, not a bare
      // one-line message.
      CliError::Parse(e) => eprintln!("{:?}", miette::Report::new(e)),
      CliError::Sema(diags) => {
        for d in &diags {
          eprintln!("error: {}", d.message);
        }
      }
      CliError::Codegen(e) => eprintln!("codegen error: {e}"),
      CliError::Link(e) => eprintln!("error: {e}"),
    }
    process::exit(1);
  }
}
