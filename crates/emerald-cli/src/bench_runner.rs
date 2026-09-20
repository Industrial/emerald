//! `emerald benchmark <file>` (plan 80's `leaf-benchmark-runner`) —
//! `test_runner`'s own timing-report sibling, byte-for-byte the same
//! shape: wires `emerald_driver::compile_benchmark` (which runs the
//! exact same parse-then-check gate `compile_test`/the ordinary
//! `emerald <file>` path use, before ever attempting `compile_
//! benchmark_harness`), then runs the produced binary and propagates
//! its exit code unchanged. The compiled binary itself prints each
//! `benchmark "..." do ... end` block's own `BENCHMARK: <description>`
//! line followed by its measured CPU-time elapsed seconds — this
//! runner adds no reporting of its own beyond streaming that output.

use std::path::Path;
use std::process::{self, Command};

pub fn run(args: &[String]) {
  let Some(source_path) = args.get(2) else {
    eprintln!("usage: emerald benchmark <file.em>");
    process::exit(2);
  };

  let source = std::fs::read_to_string(source_path).unwrap_or_else(|e| {
    eprintln!("error: cannot read `{source_path}`: {e}");
    process::exit(1);
  });

  let output_path = std::env::temp_dir().join(format!("emerald_benchmark_bin_{}", process::id()));

  if let Err(e) = emerald_driver::compile_benchmark(&source, source_path, &output_path) {
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
