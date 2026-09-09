//! Plan 61's `leaf-comptime-const-context-integration` AC5: a real
//! CLI-level proof that `--comptime-step-limit=N` reaches the actual
//! compile pipeline — a runaway `comptime` loop fails the whole
//! compilation with the step-ceiling diagnostic, not a hang, when the
//! flag lowers the default `1_000_000` ceiling to something a test can
//! afford to wait out.

use std::process::Command;

const SPIN_SRC: &str = "comptime def spin(n: Int64) -> Int64\n  i: Int64 = 0\n  while true\n    i = i + 1\n  end\n  return i\nend\n\nX: Int64 = comptime spin(1)\nputs X\n";

#[test]
fn comptime_step_limit_flag_fails_compilation_with_the_step_ceiling_diagnostic() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_comptime_limit_{}.em", std::process::id()));
  let out_path = dir.join(format!("emerald_comptime_limit_out_{}", std::process::id()));

  std::fs::write(&src_path, SPIN_SRC).unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .arg("--comptime-step-limit=100")
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    !output.status.success(),
    "compilation should fail, not hang or silently succeed"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("exceeded 100 steps"),
    "stderr should name the real step-ceiling diagnostic: {stderr}"
  );
  assert!(
    !out_path.exists(),
    "a failed compile must not leave a stale output binary behind"
  );

  std::fs::remove_file(&src_path).ok();
}

#[test]
fn comptime_factorial_worked_example_compiles_and_runs_via_the_real_cli() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!(
    "emerald_comptime_factorial_{}.em",
    std::process::id()
  ));
  let out_path = dir.join(format!(
    "emerald_comptime_factorial_out_{}",
    std::process::id()
  ));

  std::fs::write(
    &src_path,
    "comptime def factorial(n: Int64) -> Int64\n  if n <= 1\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nFACT10: Int64 = comptime factorial(10)\nputs FACT10\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .output()
    .expect("failed to run emerald-cli");
  assert!(
    output.status.success(),
    "emerald-cli should succeed: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  let run = Command::new(&out_path)
    .output()
    .expect("failed to run compiled binary");
  assert!(run.status.success(), "compiled binary should exit 0");
  assert_eq!(String::from_utf8_lossy(&run.stdout), "3628800\n");

  std::fs::remove_file(&src_path).ok();
  std::fs::remove_file(&out_path).ok();
}
