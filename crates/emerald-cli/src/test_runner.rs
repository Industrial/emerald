//! `emerald test <file>` (plan 47's `leaf-test-runner`) — wires
//! `emerald_driver::compile_test` (which already runs the exact same
//! parse-then-check gate the ordinary `emerald <file>` path uses
//! before ever attempting `compile_test_harness`), then runs the
//! produced binary and propagates its exit code unchanged.
//!
//! Plan 80's Decision log: also reached, unchanged, as `emerald
//! property <file>` — `main.rs` routes both subcommand names here
//! (see its own doc comment on that dispatch arm) since `property`
//! compiles through this exact same `compile_test_harness` mechanism,
//! not a separate one. `args[1]` (not a hardcoded `"test"`) is what the
//! usage message below names, so it reads correctly under either
//! invocation.

use std::path::Path;
use std::process::{self, Command};

pub fn run(args: &[String]) {
  let command = args.get(1).map(String::as_str).unwrap_or("test");
  let Some(source_path) = args.get(2) else {
    eprintln!("usage: emerald {command} <file.em>");
    process::exit(2);
  };

  let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
    eprintln!("error: cannot read `{source_path}`: {e}");
    process::exit(1);
  });

  let output_path = std::env::temp_dir().join(format!("emerald_test_bin_{}", process::id()));

  if let Err(e) = emerald_driver::compile_test(&source, source_path, &output_path) {
    crate::report_driver_error(e, Some((source_path, &source)));
    process::exit(1);
  }

  let status = run_and_stream(&output_path);
  std::fs::remove_file(&output_path).ok();
  process::exit(status.code().unwrap_or(1));
}

fn run_and_stream(output_path: &Path) -> std::process::ExitStatus {
  Command::new(output_path).status().unwrap_or_else(|e| {
    eprintln!("error: failed to run `{}`: {e}", output_path.display());
    process::exit(1);
  })
}
