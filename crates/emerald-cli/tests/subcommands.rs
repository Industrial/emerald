//! Real end-to-end proof of plan 46's `new`/`build`/`run` subcommands
//! and the legacy single-file path, each invoking the actual compiled
//! `emerald` binary as a subprocess (never calling crate-internal
//! modules directly — `emerald-cli` ships no library target).

use std::path::{Path, PathBuf};
use std::process::Command;

fn emerald_bin() -> &'static str {
  env!("CARGO_BIN_EXE_emerald")
}

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-cli-subcommands-test-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

fn repo_root() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .parent()
    .unwrap()
    .parent()
    .unwrap()
    .to_path_buf()
}

#[test]
fn worked_example_builds_and_runs_across_a_real_path_dependency() {
  let root = fresh_dir("worked-example");
  let src = repo_root().join("examples").join("packages");
  fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
      let entry = entry.unwrap();
      let dest = to.join(entry.file_name());
      if entry.file_type().unwrap().is_dir() {
        copy_dir(&entry.path(), &dest);
      } else {
        std::fs::copy(entry.path(), &dest).unwrap();
      }
    }
  }
  copy_dir(&src, &root);

  let app_dir = root.join("app");
  let build_output = Command::new(emerald_bin())
    .arg("build")
    .current_dir(&app_dir)
    .output()
    .unwrap();
  assert!(
    build_output.status.success(),
    "emerald build failed: {}",
    String::from_utf8_lossy(&build_output.stderr)
  );
  assert!(app_dir.join("app").exists(), "expected ./app binary");
  assert!(app_dir.join("emerald.lock").exists());
  assert!(app_dir.join("deps").join("mathutils").exists());

  let run_direct = Command::new(app_dir.join("app")).output().unwrap();
  assert_eq!(String::from_utf8_lossy(&run_direct.stdout).trim(), "8");

  let run_output = Command::new(emerald_bin())
    .arg("run")
    .current_dir(&app_dir)
    .output()
    .unwrap();
  assert!(run_output.status.success());
  // `emerald run`'s stdout carries its own build-progress lines ahead
  // of the program's own output, unlike running the linked binary
  // directly — the program's `puts` output is always the last line.
  let run_stdout = String::from_utf8_lossy(&run_output.stdout);
  assert_eq!(run_stdout.lines().next_back(), Some("8"));

  std::fs::remove_dir_all(&root).ok();
}

#[test]
fn build_with_no_manifest_fails_clearly_not_a_panic() {
  let dir = fresh_dir("no-manifest");
  let output = Command::new(emerald_bin())
    .arg("build")
    .current_dir(&dir)
    .output()
    .unwrap();
  assert!(!output.status.success());
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(stderr.contains("emerald.toml") || stderr.to_lowercase().contains("manifest"));
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn new_scaffolds_a_package_that_run_compiles_and_executes_unmodified() {
  let root = fresh_dir("new-roundtrip");
  let new_output = Command::new(emerald_bin())
    .arg("new")
    .arg("mathutils2")
    .current_dir(&root)
    .output()
    .unwrap();
  assert!(new_output.status.success());
  let pkg_dir = root.join("mathutils2");
  assert!(pkg_dir.join("emerald.toml").exists());
  assert!(pkg_dir.join("main.em").exists());

  let run_output = Command::new(emerald_bin())
    .arg("run")
    .current_dir(&pkg_dir)
    .output()
    .unwrap();
  assert!(
    run_output.status.success(),
    "emerald run failed: {}",
    String::from_utf8_lossy(&run_output.stderr)
  );
  assert!(String::from_utf8_lossy(&run_output.stdout).contains("Hello from mathutils2!"));
  std::fs::remove_dir_all(&root).ok();
}

#[test]
fn legacy_bare_file_invocation_still_compiles_links_and_runs_unchanged() {
  let dir = fresh_dir("legacy");
  let output_path = dir.join("hello_out");
  let status = Command::new(emerald_bin())
    .arg(repo_root().join("examples").join("hello.em"))
    .arg("-o")
    .arg(&output_path)
    .status()
    .unwrap();
  assert!(status.success());
  let run = Command::new(&output_path).output().unwrap();
  assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn update_subcommand_exits_with_a_clear_not_supported_message() {
  let output = Command::new(emerald_bin()).arg("update").output().unwrap();
  assert!(!output.status.success());
  assert!(String::from_utf8_lossy(&output.stderr).contains("not supported yet"));
}
