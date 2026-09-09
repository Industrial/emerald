//! End-to-end test: inception §17's literal milestone-1 acceptance test,
//! executed for real. See tests/integration/README.md for why this test
//! lives here rather than in the root tests/ tree.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn compiles_and_runs_hello_em_printing_42() {
  let source = workspace_root().join("examples/hello.em");
  let output = std::env::temp_dir().join(format!("emerald_hello_{}", std::process::id()));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(status.success(), "emerald-cli should succeed on hello.em");

  let run = Command::new(&output)
    .output()
    .expect("failed to run compiled binary");
  assert!(run.status.success(), "compiled binary should exit 0");
  assert_eq!(
    String::from_utf8_lossy(&run.stdout),
    "42\n",
    "hello.em must print exactly 42"
  );

  std::fs::remove_file(&output).ok();
}

#[test]
fn rejects_type_mismatch_with_nonzero_exit_and_stderr_diagnostic() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_bad_{}.em", std::process::id()));
  std::fs::write(
    &src_path,
    "def add(a: Int64, b: String) -> Int64\n  a + b\nend\n",
  )
  .unwrap();
  let output_path = dir.join(format!("emerald_bad_out_{}", std::process::id()));

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&output_path)
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    !output.status.success(),
    "must exit non-zero on a rejected program"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("Int64") && stderr.contains("String"),
    "stderr should name both types: {stderr}"
  );
  assert!(
    !output_path.exists(),
    "must not produce an executable for a rejected program"
  );

  std::fs::remove_file(&src_path).ok();
}

#[test]
fn sema_type_mismatch_renders_a_miette_source_snippet_with_a_caret_at_b() {
  // Plan 22's own concrete-proof program (leaf-span-rendering AC1): a
  // real `miette`-rendered diagnostic — source snippet plus a caret —
  // pointing at `b` on line 2, the same rendering-quality standard
  // plan 13 already proved for parse errors, now met for a sema error
  // too.
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_sema_span_{}.em", std::process::id()));
  std::fs::write(
    &src_path,
    "def add(a: Int64, b: String) -> Int64\n  a + b\nend\n",
  )
  .unwrap();
  let output_path = dir.join(format!("emerald_sema_span_out_{}", std::process::id()));

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&output_path)
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    !output.status.success(),
    "must exit non-zero on a rejected program"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains(src_path.to_str().unwrap()),
    "diagnostic should name the source file path: {stderr}"
  );
  assert!(
    stderr.contains("  a + b"),
    "diagnostic should render the actual source line (`  a + b`): {stderr}"
  );
  assert!(
    stderr.contains("here") && (stderr.contains('┬') || stderr.contains('^')),
    "diagnostic should include a caret/underline label: {stderr}"
  );
  assert!(
    !output_path.exists(),
    "must not produce an executable for a rejected program"
  );

  std::fs::remove_file(&src_path).ok();
}

#[test]
fn parse_error_renders_a_miette_source_snippet_with_a_caret() {
  // Plan 13 AC1: not just a non-zero exit and *some* stderr text — a
  // real `miette`-rendered diagnostic containing the source line, a
  // caret/underline pointing at the offending token, and the file path.
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_parse_bad_{}.em", std::process::id()));
  std::fs::write(&src_path, "x: Int64 = +\n").unwrap();
  let output_path = dir.join(format!("emerald_parse_bad_out_{}", std::process::id()));

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&output_path)
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    !output.status.success(),
    "must exit non-zero on a parse error"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains(src_path.to_str().unwrap()),
    "diagnostic should name the source file path: {stderr}"
  );
  assert!(
    stderr.contains("x: Int64 = +"),
    "diagnostic should render the actual source line: {stderr}"
  );
  assert!(
    stderr.contains("here") && (stderr.contains('┬') || stderr.contains('^')),
    "diagnostic should include a caret/underline label: {stderr}"
  );
  assert!(
    !output_path.exists(),
    "must not produce an executable for a program that fails to parse"
  );

  std::fs::remove_file(&src_path).ok();
}
