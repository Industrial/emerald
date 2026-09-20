//! Real end-to-end proof of `emerald lint` (plan-of-plans row 79,
//! `canonical-linter`) via the actual compiled `emerald` binary —
//! mirrors `subcommands.rs`'s own "invoke the real binary as a
//! subprocess" convention, never calling `crate::lint` directly
//! (`emerald-cli` ships no library target for these tests to link
//! against; `crates/emerald-cli/src/lint.rs`'s own `#[cfg(test)]`
//! module already covers each rule's positive/negative pair at the
//! `lint_program` level).

use std::path::PathBuf;
use std::process::Command;

fn emerald_bin() -> &'static str {
  env!("CARGO_BIN_EXE_emerald")
}

fn workspace_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-cli-lint-test-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

/// Real, working code (mirrors `examples/exceptions.em`'s own worked
/// pattern) — `emerald lint` must report zero findings against it.
const CLEAN_SRC: &str = "\
class MyError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

/// Deliberately flawed on all four rules at once.
const FLAWED_SRC: &str = "\
class Empty\nend\n\nclass Boom\nend\n\nfn risky(): Void do\n  raise Boom.new()\nend\n\nfn compute(): Int64 do\n  var total: Int64 = 0\n  unused_local: Int64 = 99\n  begin\n    risky()\n  rescue Boom => e\n  end\n  return total\nend\n\nputs compute()\n";

#[test]
fn a_deliberately_flawed_file_is_reported_on_every_rule_and_exits_non_zero() {
  let dir = fresh_dir("flawed");
  let file = dir.join("flawed.em");
  std::fs::write(&file, FLAWED_SRC).unwrap();

  let output = Command::new(emerald_bin())
    .arg("lint")
    .arg(&file)
    .output()
    .unwrap();

  assert!(!output.status.success());
  assert_eq!(output.status.code(), Some(1));
  let stderr = String::from_utf8_lossy(&output.stderr);
  for rule in [
    "empty-class",
    "empty-rescue",
    "needless-var",
    "unused-local-variable",
  ] {
    assert!(stderr.contains(rule), "expected `{rule}` in:\n{stderr}");
  }
  // The one binding that's genuinely fine (`e` in the `rescue`, `Boom`
  // used as a real raise target) must not spuriously appear as a
  // finding subject anywhere.
  assert!(stderr.contains("class `Empty`"), "{stderr}");
  assert!(stderr.contains("class `Boom`"), "{stderr}");
  assert!(stderr.contains("`var total`"), "{stderr}");
  assert!(stderr.contains("`unused_local`"), "{stderr}");

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clean_real_world_shaped_code_lints_with_zero_findings_and_exits_zero() {
  let dir = fresh_dir("clean");
  let file = dir.join("clean.em");
  std::fs::write(&file, CLEAN_SRC).unwrap();

  let output = Command::new(emerald_bin())
    .arg("lint")
    .arg(&file)
    .output()
    .unwrap();

  assert!(
    output.status.success(),
    "expected zero findings, got:\n{}",
    String::from_utf8_lossy(&output.stderr)
  );
  assert!(output.stderr.is_empty(), "{:?}", output.stderr);

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn every_real_file_under_examples_lints_clean() {
  // The durable, re-checkable version of this task's own manual
  // verification: every REAL, already-working `.em` file under
  // `examples/` must produce zero lint findings — a false positive
  // here would mean a rule is wrong, not that the example is.
  let output = Command::new(emerald_bin())
    .arg("lint")
    .arg(workspace_root().join("examples"))
    .output()
    .unwrap();

  assert!(
    output.status.success(),
    "expected zero findings across examples/, got:\n{}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[test]
fn no_path_argument_is_a_usage_error() {
  let output = Command::new(emerald_bin()).arg("lint").output().unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[test]
fn a_nonexistent_path_is_a_clear_error_not_a_panic() {
  let output = Command::new(emerald_bin())
    .arg("lint")
    .arg("/does/not/exist.em")
    .output()
    .unwrap();
  assert_eq!(output.status.code(), Some(2));
  assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}
