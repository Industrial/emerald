//! Plan 62 (design-by-contract) — a real CLI-level proof of the plan's
//! own two worked examples: `contracts.em` (compiles, links, and runs,
//! printing a normal result then a caught runtime `ContractViolation`)
//! and `contracts_reject.em` (a literal zero-divisor argument, rejected
//! at compile time — non-zero exit, no object file ever emitted).

use std::process::Command;

#[test]
fn contracts_worked_example_compiles_links_and_runs_via_the_real_cli() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_contracts_{}.em", std::process::id()));
  let out_path = dir.join(format!("emerald_contracts_out_{}", std::process::id()));

  std::fs::write(
    &src_path,
    "def divide(a: Int64, b: Int64) -> Int64\n  requires b != 0\n  ensures result * b <= a\n  return a / b\nend\n\ny: Int64 = 10\nz: Int64 = 2\nputs divide(y, z)\n\nx: Int64 = 3\nw: Int64 = x - 3\nbegin\n  puts divide(20, w)\nrescue ContractViolation => e\n  puts e.message\nend\n",
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
  let stdout = String::from_utf8_lossy(&run.stdout);
  let mut lines = stdout.lines();
  assert_eq!(lines.next(), Some("5"));
  let violation_line = lines.next().expect("expected a second line");
  assert!(
    violation_line.contains("divide") && violation_line.contains("b != 0"),
    "expected the requires-violation message, got: {violation_line}"
  );

  std::fs::remove_file(&src_path).ok();
  std::fs::remove_file(&out_path).ok();
}

#[test]
fn a_literal_zero_divisor_is_rejected_at_compile_time_with_no_binary_emitted() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!(
    "emerald_contracts_reject_{}.em",
    std::process::id()
  ));
  let out_path = dir.join(format!(
    "emerald_contracts_reject_out_{}",
    std::process::id()
  ));
  std::fs::remove_file(&out_path).ok();

  std::fs::write(
    &src_path,
    "def divide(a: Int64, b: Int64) -> Int64\n  requires b != 0\n  return a / b\nend\n\nputs divide(10, 0)\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    !output.status.success(),
    "compilation should fail, not silently succeed"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("contract violation provable at compile time") && stderr.contains("b = 0"),
    "stderr should name the real static-provability diagnostic: {stderr}"
  );
  assert!(
    !out_path.exists(),
    "a rejected compile must not leave a stale output binary behind"
  );

  std::fs::remove_file(&src_path).ok();
}
