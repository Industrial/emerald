//! Compiles and runs every file under `examples/` (except `hello.em`,
//! already covered by `hello_em.rs`), asserting exact stdout — the
//! durable, re-checkable version of the manual verification behind
//! `examples/README.md`'s coverage table.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile_and_run(example: &str) -> String {
  let source = workspace_root().join("examples").join(example);
  let output = std::env::temp_dir().join(format!(
    "emerald_example_{}_{}",
    example.replace(['.', '/'], "_"),
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(status.success(), "emerald-cli should succeed on {example}");

  let run = Command::new(&output)
    .output()
    .expect("failed to run compiled binary");
  assert!(
    run.status.success(),
    "{example}'s compiled binary should exit 0"
  );

  std::fs::remove_file(&output).ok();
  String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn control_flow_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("control_flow.em"),
    "1\n1\n1\n1\n1\n1\n0\n1\n3\n0\n1\n2\n"
  );
}

#[test]
fn classes_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("classes.em"), "10\n15\n5\n");
}

#[test]
fn collections_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("collections.em"), "60\n99\n4\n");
}

#[test]
fn closures_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("closures.em"), "15\n42\n");
}

#[test]
fn exceptions_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("exceptions.em"), "99\n5\n");
}

#[test]
fn modules_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("modules.em"), "42\n");
}

#[test]
fn interfaces_generics_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("interfaces_generics.em"), "750\n100\n");
}
