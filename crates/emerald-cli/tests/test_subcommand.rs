//! Real end-to-end proof of plan 47's `leaf-test-runner` acceptance
//! criteria — spawns the actual compiled `emerald` binary, never
//! calling `emerald_driver`/`emerald_codegen` directly.

use std::path::PathBuf;
use std::process::Command;

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-cli-test-subcommand-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

const MATH_TEST_SRC: &str = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n\ntest \"addition is broken on purpose\" do\n  assert_eq(3, 1 + 1)\nend\n";

#[test]
fn math_test_worked_example_matches_the_transcript_and_exits_1() {
  let dir = fresh_dir("worked-example");
  let path = dir.join("math_test.em");
  std::fs::write(&path, MATH_TEST_SRC).unwrap();

  // `current_dir` + a bare relative filename, matching a real `cd
  // <dir> && emerald test math_test.em` invocation — `emerald_driver::
  // compile_test` names its diagnostics/`assert` locations after
  // whatever `args[2]` literally was (the same convention the ordinary
  // compile path already uses), so a full absolute temp-dir path here
  // would leak into (and break) the exact-match assertion below.
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg("math_test.em")
    .current_dir(&dir)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: addition works\nexpected:\n3\nbut got:\n2\nFAIL: addition is broken on purpose: math_test.em:6\npassed:\n1\nfailed:\n1\n"
  );
  assert_eq!(output.status.code(), Some(1));

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_where_every_test_passes_exits_0() {
  let dir = fresh_dir("all-green");
  let path = dir.join("all_green_test.em");
  std::fs::write(
    &path,
    "test \"one\" do\n  assert_eq(1, 1)\nend\n\ntest \"two\" do\n  assert(true)\nend\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg(&path)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: one\nPASS: two\npassed:\n2\nfailed:\n0\n"
  );
  assert_eq!(output.status.code(), Some(0));

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_type_error_is_rejected_before_ever_attempting_the_test_harness() {
  let dir = fresh_dir("type-error");
  let path = dir.join("bad_test.em");
  std::fs::write(&path, "test \"bad\" do\n  assert_eq(\"s\", 1)\nend\n").unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg(&path)
    .output()
    .unwrap();

  assert!(!output.status.success());
  assert!(String::from_utf8_lossy(&output.stdout).is_empty());
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("String") && stderr.contains("Int64"),
    "{stderr}"
  );

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn compiling_the_same_file_via_the_ordinary_path_is_a_described_rejection() {
  let dir = fresh_dir("ordinary-path-rejection");
  let path = dir.join("math_test.em");
  std::fs::write(&path, MATH_TEST_SRC).unwrap();
  let out_bin = dir.join("out");

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&path)
    .arg("-o")
    .arg(&out_bin)
    .output()
    .unwrap();

  assert!(!output.status.success());
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("only valid under `emerald test`"),
    "{stderr}"
  );
  assert!(!out_bin.exists());

  std::fs::remove_dir_all(&dir).ok();
}
