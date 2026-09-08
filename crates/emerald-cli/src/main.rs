//! Emerald's command-line entry point (plan-of-plans row 06). Orchestrates
//! parse -> type-check -> codegen -> link directly here rather than in a
//! separate `emerald-driver` crate (see plan 06's Decision log: extracted
//! when a second caller needs the same pipeline).

use std::path::PathBuf;
use std::process::{self, Command};

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

  let program = match emerald_parser::parse(&source) {
    Ok(p) => p,
    Err(e) => {
      eprintln!("parse error: {e}");
      process::exit(1);
    }
  };

  if let Err(diags) = emerald_sema::check_program(&program) {
    for d in &diags {
      eprintln!("error: {}", d.message);
    }
    process::exit(1);
  }

  let obj_path = std::env::temp_dir().join(format!("emerald_{}.o", process::id()));
  if let Err(e) = emerald_codegen::compile_to_object(&program, &obj_path) {
    eprintln!("codegen error: {e}");
    process::exit(1);
  }

  // Dev-time runtime location: this repo's runtime/emerald_runtime.c,
  // found relative to this crate's manifest dir. A real install would
  // bundle a compiled runtime object/archive instead — noted as
  // follow-up packaging work, not solved here.
  let runtime_path =
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/emerald_runtime.c");

  // -no-pie: emerald-codegen emits non-PIC code (see emerald-codegen's
  // host_isa), so the executable must not be a PIE either — otherwise ld
  // warns about (harmless but avoidable) DT_TEXTREL relocations.
  let link_result = Command::new("cc")
    .arg("-no-pie")
    .arg(&obj_path)
    .arg(&runtime_path)
    .arg("-o")
    .arg(&output_path)
    .status();

  std::fs::remove_file(&obj_path).ok();

  match link_result {
    Ok(status) if status.success() => {}
    Ok(_) => {
      eprintln!("error: linking failed");
      process::exit(1);
    }
    Err(e) => {
      eprintln!("error: failed to invoke cc: {e}");
      process::exit(1);
    }
  }
}
