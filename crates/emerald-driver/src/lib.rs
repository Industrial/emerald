//! The parse -> type-check -> codegen -> link pipeline (plan 06),
//! extracted into a library crate now that a second caller needs it —
//! `emerald-lsp`/`emerald-mcp` (plan 17) both need to check/compile
//! in-memory source that was never written to disk, something the
//! single binary crate `emerald-cli` couldn't offer on its own (plan
//! 06's own Decision log named exactly this as the extraction
//! trigger).
//!
//! Two source-text entry points: `check` (parse + sema only — what
//! live diagnostics need, the cheapest reject-without-compiling path)
//! and `compile` (the full pipeline, writing a real linked executable
//! to `output_path`). `check_program`/`compile_program` additionally
//! take an already-parsed `Program` directly — plan 46's multi-file
//! `require` splicing (`emerald-cli`'s own `require.rs`, which shipped
//! after this plan was authored) already needs to hand this driver a
//! fully-assembled in-memory `Program`, which the two source-text
//! entry points alone don't cover.

use emerald_parser::{ParseError, Program};
use id_effect::{Effect, run_blocking};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

// Plan 21's Decision log: `emerald-lsp` keeps depending only on this
// crate, never directly on `emerald-parser`/`emerald-sema` (the same
// boundary plan 17's `leaf-lsp-server` already established) — this
// re-export is what lets it name `SymbolTable`'s own type without a
// second dependency edge.
pub use emerald_sema::{ClassSymbol, FunctionSymbol, SymbolTable};

#[derive(Debug)]
pub enum DriverError {
  /// Plan 26: `emerald_parser::parse_named` reports every top-level
  /// `Item` boundary's syntax error in one pass, not just the first.
  Parse(Vec<ParseError>),
  Sema(Vec<emerald_sema::Diagnostic>),
  Codegen(String),
  Link(String),
}

fn parse_stage(source: String, name: String) -> Effect<Program, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_parser::parse_named(&source, &name).map_err(DriverError::Parse)
  })
}

fn check_stage(program: Program) -> Effect<Program, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_sema::check_program(&program)
      .map(|()| program)
      .map_err(DriverError::Sema)
  })
}

/// Plan 35's `leaf-line-table-generation`: `source_info` is `Some((source,
/// name))` only from `compile`'s own source-text entry point — `compile_
/// program`'s already-parsed-`Program` entry point has no source text to
/// derive DWARF line numbers from, so it passes `None` and gets the same
/// debug-info-free object file this crate always produced.
fn codegen_stage(
  program: Program,
  obj_path: PathBuf,
  source_info: Option<(String, String)>,
) -> Effect<PathBuf, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    let result = match &source_info {
      Some((source, name)) => {
        emerald_codegen::compile_to_object_with_debug_info(&program, &obj_path, source, name)
      }
      None => emerald_codegen::compile_to_object(&program, &obj_path),
    };
    result.map(|()| obj_path).map_err(DriverError::Codegen)
  })
}

/// Plan 27: the compiled runtime archive's real bytes, embedded at
/// *this crate's* compile time (`build.rs`) — moved here unchanged
/// from `emerald-cli`. This is what makes a shipped binary, copied
/// alone with no access to this repo's checkout, still able to link a
/// user's compiled program: the runtime archive travels inside the
/// binary itself.
static RUNTIME_ARCHIVE: &[u8] = include_bytes!(env!("EMERALD_RUNTIME_ARCHIVE"));

/// -no-pie: `emerald-codegen` emits non-PIC code (see its `host_isa`),
/// so the executable must not be a PIE either — otherwise `ld` warns
/// about (harmless but avoidable) DT_TEXTREL relocations. The object
/// file and the extracted runtime archive are both removed once
/// linking is attempted, success or failure.
fn link_stage(obj_path: PathBuf, output_path: PathBuf) -> Effect<(), DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    let runtime_archive_path =
      std::env::temp_dir().join(format!("libemerald_runtime_{}.a", process::id()));
    if let Err(e) = std::fs::write(&runtime_archive_path, RUNTIME_ARCHIVE) {
      std::fs::remove_file(&obj_path).ok();
      return Err(DriverError::Link(format!(
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
      Ok(_) => Err(DriverError::Link("linking failed".to_string())),
      Err(e) => Err(DriverError::Link(format!("failed to invoke cc: {e}"))),
    }
  })
}

/// Parses and type-checks `source` in memory — no codegen, no link,
/// no filesystem write of any kind.
pub fn check(source: &str, name: &str) -> Result<(), DriverError> {
  let pipeline = parse_stage(source.to_string(), name.to_string()).flat_map(check_stage);
  run_blocking(pipeline, ()).map(|_program| ())
}

/// The full pipeline: parse, check, codegen, link — writes a real
/// executable to `output_path`.
pub fn compile(source: &str, name: &str, output_path: &Path) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(format!("emerald_{}.o", process::id()));
  let output_path = output_path.to_path_buf();
  let source_info = Some((source.to_string(), name.to_string()));
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_stage(program, obj_path, source_info))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));
  run_blocking(pipeline, ())
}

/// Plan 47's `leaf-test-runner`: parses + checks `source` exactly like
/// `compile` does, then hands the result to
/// `emerald_codegen::compile_test_harness` (not `compile_to_object`)
/// and links the result — a thin passthrough, mirroring how `compile`
/// itself already wraps `compile_to_object`. Returns the number of
/// `test` blocks found.
pub fn compile_test(source: &str, name: &str, output_path: &Path) -> Result<usize, DriverError> {
  let obj_path = std::env::temp_dir().join(format!("emerald_test_{}.o", process::id()));
  let output_path = output_path.to_path_buf();
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_test_stage(program, obj_path))
    .flat_map(move |(obj_path, count)| {
      let output_path = output_path.clone();
      link_stage(obj_path, output_path).flat_map(move |()| {
        Effect::new(move |_env: &mut ()| -> Result<usize, DriverError> { Ok(count) })
      })
    });
  run_blocking(pipeline, ())
}

fn codegen_test_stage(
  program: Program,
  obj_path: PathBuf,
) -> Effect<(PathBuf, usize), DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_codegen::compile_test_harness(&program, &obj_path)
      .map(|count| (obj_path.clone(), count))
      .map_err(DriverError::Codegen)
  })
}

/// Plan 21's `leaf-symbol-table`: parses `source` and returns
/// `emerald_sema::collect_symbols`'s best-effort table — sitting
/// alongside `check`/`compile`, reusing the existing `DriverError::
/// Parse` variant for a parse failure rather than inventing a new
/// error path. `emerald-lsp` reaches this instead of depending on
/// `emerald-parser`/`emerald-sema` directly (plan 17's own boundary).
pub fn symbols(source: &str, name: &str) -> Result<emerald_sema::SymbolTable, DriverError> {
  let program = emerald_parser::parse_named(source, name).map_err(DriverError::Parse)?;
  Ok(emerald_sema::collect_symbols(&program))
}

/// Type-checks an already-parsed `Program` directly.
pub fn check_program(program: &Program) -> Result<(), DriverError> {
  emerald_sema::check_program(program).map_err(DriverError::Sema)
}

/// Compiles and links an already-parsed `Program` directly.
pub fn compile_program(program: Program, output_path: &Path) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(format!("emerald_{}.o", process::id()));
  let output_path = output_path.to_path_buf();
  let pipeline = check_stage(program)
    .flat_map(move |program| codegen_stage(program, obj_path, None))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));
  run_blocking(pipeline, ())
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_SRC: &str =
    "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn check_accepts_hello_em_in_memory_with_no_filesystem_write() {
    assert!(check(HELLO_SRC, "hello.em").is_ok());
  }

  #[test]
  fn check_rejects_a_sema_error() {
    let source = "x: Int64 = \"nope\"\n";
    assert!(matches!(check(source, "bad.em"), Err(DriverError::Sema(_))));
  }

  #[test]
  fn check_rejects_a_parse_error() {
    let source = "def add(a: Int64\n";
    assert!(matches!(
      check(source, "bad.em"),
      Err(DriverError::Parse(_))
    ));
  }

  #[test]
  fn compile_links_a_real_binary_that_runs_and_prints_42() {
    let dir = std::env::temp_dir().join(format!("emerald-driver-compile-test-{}", process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("hello_out");
    compile(HELLO_SRC, "hello.em", &output).unwrap();
    let run = Command::new(&output).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn compile_program_compiles_an_already_parsed_program() {
    let dir = std::env::temp_dir().join(format!(
      "emerald-driver-compile-program-test-{}",
      process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("hello_out");
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    assert!(check_program(&program).is_ok());
    compile_program(program, &output).unwrap();
    let run = Command::new(&output).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    std::fs::remove_dir_all(&dir).ok();
  }

  // Plan 21 (LSP symbols and navigation).

  #[test]
  fn symbols_returns_a_table_for_hello_em_in_memory_with_no_filesystem_write() {
    let table = symbols(HELLO_SRC, "hello.em").expect("should collect symbols");
    let add = table.functions.get("add").expect("`add` should be present");
    assert_eq!(add.params.len(), 2);
  }

  #[test]
  fn symbols_returns_a_table_for_classes_em() {
    let src = "class Counter\n  value: Int64\n\n  def initialize(start: Int64) -> Void\n    @value = start\n  end\nend\n";
    let table = symbols(src, "classes.em").expect("should collect symbols");
    assert!(table.classes.contains_key("Counter"));
  }

  #[test]
  fn symbols_rejects_an_unparseable_buffer() {
    let source = "def add(a: Int64\n";
    assert!(matches!(
      symbols(source, "bad.em"),
      Err(DriverError::Parse(_))
    ));
  }
}
