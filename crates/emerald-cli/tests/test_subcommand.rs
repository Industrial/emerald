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
fn examples_test_framework_em_matches_the_documented_transcript() {
  // The durable, re-checkable version of examples/README.md's
  // documented `emerald test examples/test_framework.em` transcript —
  // this file is the only example that can't run through the ordinary
  // `emerald <file>` path (`Item::Test` rejects it), so it gets its
  // own verification here rather than `examples.rs`'s `compile_and_run`.
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg("examples/test_framework.em")
    .current_dir(&root)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: addition works\nexpected:\n3\nbut got:\n2\nFAIL: addition is broken on purpose: examples/test_framework.em:6\npassed:\n1\nfailed:\n1\n"
  );
  assert_eq!(output.status.code(), Some(1));
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

// Plan 80 (property-and-benchmark-test-syntax).

#[test]
fn property_block_runs_and_asserts_exactly_like_a_test_block() {
  // `property "..." do ... end` is real, disclosed syntactic sugar for
  // a single-input `test` (see `Item::Property`'s own doc comment) —
  // this proves it actually runs through `emerald test`'s real,
  // compiled harness, not just that it parses/type-checks.
  let dir = fresh_dir("property-block");
  let path = dir.join("prop_test.em");
  std::fs::write(
    &path,
    "property \"addition is commutative\" do\n  a: Int64 = 3\n  b: Int64 = 4\n  assert_eq(a + b, b + a)\nend\n\nproperty \"broken on purpose\" do\n  assert_eq(1, 2)\nend\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg("prop_test.em")
    .current_dir(&dir)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: addition is commutative\nexpected:\n1\nbut got:\n2\nFAIL: broken on purpose: prop_test.em:8\npassed:\n1\nfailed:\n1\n"
  );
  assert_eq!(output.status.code(), Some(1));

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_property_subcommand_is_a_real_alias_for_emerald_test() {
  let dir = fresh_dir("property-subcommand");
  let path = dir.join("prop_alias.em");
  std::fs::write(&path, "property \"one\" do\n  assert(true)\nend\n").unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("property")
    .arg(&path)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: one\npassed:\n1\nfailed:\n0\n"
  );
  assert_eq!(output.status.code(), Some(0));

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn examples_property_test_em_matches_the_documented_transcript() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("test")
    .arg("examples/property_test.em")
    .current_dir(&root)
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&output.stdout),
    "PASS: sorting an array preserves its element sum\nPASS: addition is commutative for a fixed pair of integers\npassed:\n2\nfailed:\n0\n"
  );
  assert_eq!(output.status.code(), Some(0));
}

#[test]
fn benchmark_block_compiles_and_reports_a_real_elapsed_time() {
  // A real, executed proof that `emerald benchmark` actually measures
  // and reports timing for a genuine workload — not just that it
  // parses/type-checks. Asserts on the STRUCTURE of the report (two
  // lines per benchmark: `BENCHMARK: <description>` then a parseable
  // `Float64` elapsed-seconds value), not an exact number, since wall/
  // CPU time is inherently machine-dependent.
  let dir = fresh_dir("benchmark-block");
  let path = dir.join("bench.em");
  std::fs::write(
    &path,
    "benchmark \"trivial addition\" do\n  x: Int64 = 1 + 1\n  puts x\nend\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("benchmark")
    .arg(&path)
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let lines: Vec<&str> = stdout.lines().collect();
  assert_eq!(lines.len(), 3, "{stdout:?}");
  assert_eq!(lines[0], "2");
  assert_eq!(lines[1], "BENCHMARK: trivial addition");
  lines[2].parse::<f64>().unwrap_or_else(|e| {
    panic!(
      "elapsed-time line `{}` should parse as a Float64: {e}",
      lines[2]
    )
  });

  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn examples_benchmark_example_em_reports_two_real_benchmarks() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("benchmark")
    .arg("examples/benchmark_example.em")
    .current_dir(&root)
    .output()
    .unwrap();

  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let lines: Vec<&str> = stdout.lines().collect();
  assert_eq!(lines.len(), 6, "{stdout:?}");
  assert_eq!(lines[0], "210000000");
  assert_eq!(
    lines[1],
    "BENCHMARK: traversing a 20-element array one million times"
  );
  lines[2].parse::<f64>().unwrap();
  assert_eq!(lines[3], "1000000");
  assert_eq!(
    lines[4],
    "BENCHMARK: allocating and summing a 20-element array one million times"
  );
  lines[5].parse::<f64>().unwrap();
}

#[test]
fn compiling_a_benchmark_block_via_the_ordinary_path_is_a_described_rejection() {
  let dir = fresh_dir("benchmark-ordinary-path-rejection");
  let path = dir.join("bench.em");
  std::fs::write(&path, "benchmark \"x\" do\n  puts 1\nend\n").unwrap();
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
    stderr.contains("only valid under `emerald test`/`emerald benchmark`"),
    "{stderr}"
  );
  assert!(!out_bin.exists());

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
