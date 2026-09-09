//! Plan 50's `leaf-escape-instrumentation-and-report` AC4: a real
//! CLI-level proof that `--emit=escape-report` prints a correct
//! stack-vs-heap count for a program containing both this plan's own
//! worked functions.

use std::process::Command;

#[test]
fn emit_escape_report_prints_the_real_stack_and_heap_counts() {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_escape_report_{}.em", std::process::id()));
  let out_path = dir.join(format!("emerald_escape_report_out_{}", std::process::id()));

  // `distance_squared`'s own `p` is provably non-escaping (stack); the
  // real `puts` call is what `escape_stats_on_distance_squared_is_one_
  // stack_zero_heap` (emerald-codegen's own test) already proves in
  // isolation — this only adds `make_point`'s own `p`, which escapes
  // by returning directly (heap), for a real 1-stack/1-heap program.
  std::fs::write(
    &src_path,
    "class Point\n  x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\ndef distance_squared(x: Int64, y: Int64) -> Int64\n  p: Point = Point.new(x, y)\n  x * x + y * y\nend\n\ndef make_point(x: Int64, y: Int64) -> Point\n  p: Point = Point.new(x, y)\n  p\nend\n\nputs distance_squared(3, 4)\nq: Point = make_point(1, 2)\n",
  )
  .unwrap();

  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .arg("--emit=escape-report")
    .output()
    .expect("failed to run emerald-cli");

  assert!(
    output.status.success(),
    "emerald-cli should succeed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("1 instances stack-allocated, 1 heap-allocated"),
    "stderr should name both real counts: {stderr}"
  );

  // The linked binary itself must still work identically — the report
  // is printed *before* proceeding with linking as normal, never
  // instead of it.
  let run = Command::new(&out_path)
    .output()
    .expect("failed to run compiled binary");
  assert!(run.status.success(), "compiled binary should exit 0");
  assert_eq!(String::from_utf8_lossy(&run.stdout), "25\n");

  std::fs::remove_file(&src_path).ok();
  std::fs::remove_file(&out_path).ok();
}
