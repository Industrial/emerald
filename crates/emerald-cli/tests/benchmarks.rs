//! Plan 15's real, measured comparison of Emerald vs Rust/C/C++ on the
//! benchmark programs under `benchmarks/`. Every number here comes from
//! actually compiling and running each program during this test —
//! nothing is simulated or hand-typed (see plan 15's Decision log for
//! why only 2 of inception §21's 9 suggested benchmarks are
//! expressible in this compiler today, why Ruby isn't compared, and why
//! memory usage isn't measured).
//!
//! `#[ignore]`d — a real multi-language, multi-benchmark comparison
//! takes real wall-clock time and isn't part of the fast correctness
//! suite `cargo test --workspace` runs by default. Run explicitly:
//!
//! ```text
//! cargo test --release -p emerald-cli -- --ignored --nocapture benchmarks
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Measurement {
  language: &'static str,
  compile_time: Duration,
  run_time: Duration,
  binary_size: u64,
}

/// `backend` is `"cranelift"` or `"llvm"` (plan 16's bake-off — see
/// `emerald-codegen-llvm`'s module doc for why the LLVM backend only
/// covers `sum`/`array_traversal`'s AST shape, not the full language).
fn compile_emerald(backend: &str, src: &Path, out: &Path) -> Duration {
  let start = Instant::now();
  let status = Command::new(env!("CARGO_BIN_EXE_emerald-cli"))
    .arg(src)
    .arg("-o")
    .arg(out)
    .arg(format!("--backend={backend}"))
    .status()
    .expect("failed to run emerald-cli");
  let elapsed = start.elapsed();
  assert!(
    status.success(),
    "emerald-cli --backend={backend} should succeed compiling {src:?}"
  );
  elapsed
}

fn compile_rustc(src: &Path, out: &Path) -> Duration {
  let start = Instant::now();
  let status = Command::new("rustc")
    .arg("-O")
    .arg(src)
    .arg("-o")
    .arg(out)
    .status()
    .expect("failed to run rustc");
  let elapsed = start.elapsed();
  assert!(status.success(), "rustc should succeed compiling {src:?}");
  elapsed
}

fn compile_cc(compiler: &str, src: &Path, out: &Path) -> Duration {
  let start = Instant::now();
  let status = Command::new(compiler)
    .arg("-O2")
    .arg(src)
    .arg("-o")
    .arg(out)
    .status()
    .unwrap_or_else(|e| panic!("failed to run {compiler}: {e}"));
  let elapsed = start.elapsed();
  assert!(
    status.success(),
    "{compiler} should succeed compiling {src:?}"
  );
  elapsed
}

/// Runs the compiled binary, asserting its stdout matches `expected`
/// before returning the elapsed time — a timing number attached to an
/// incorrect program isn't meaningful (plan 15 AC2).
fn run_and_check(bin: &Path, expected: &str) -> Duration {
  let start = Instant::now();
  let output = Command::new(bin)
    .output()
    .expect("failed to run compiled binary");
  let elapsed = start.elapsed();
  assert!(output.status.success(), "{bin:?} should exit 0");
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert_eq!(
    stdout.trim(),
    expected,
    "{bin:?} produced unexpected output"
  );
  elapsed
}

#[allow(clippy::too_many_arguments)]
fn measure_one(
  name: &str,
  language: &'static str,
  src_ext: &str,
  compile: impl FnOnce(&Path, &Path) -> Duration,
  expected: &str,
  bench_dir: &Path,
) -> Measurement {
  let src = bench_dir.join(format!("{name}.{src_ext}"));
  let out = std::env::temp_dir().join(format!(
    "emerald_bench_{name}_{language}_{}",
    std::process::id()
  ));
  let compile_time = compile(&src, &out);
  let binary_size = std::fs::metadata(&out)
    .unwrap_or_else(|e| panic!("compiled binary {out:?} should exist: {e}"))
    .len();
  let run_time = run_and_check(&out, expected);
  std::fs::remove_file(&out).ok();
  Measurement {
    language,
    compile_time,
    run_time,
    binary_size,
  }
}

fn format_duration(d: Duration) -> String {
  format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn benchmark(name: &str, expected: &str) -> Vec<Measurement> {
  let bench_dir = workspace_root().join("benchmarks").join(name);
  vec![
    measure_one(
      name,
      "Emerald (Cranelift)",
      "em",
      |s, o| compile_emerald("cranelift", s, o),
      expected,
      &bench_dir,
    ),
    measure_one(
      name,
      "Emerald (LLVM)",
      "em",
      |s, o| compile_emerald("llvm", s, o),
      expected,
      &bench_dir,
    ),
    measure_one(name, "Rust", "rs", compile_rustc, expected, &bench_dir),
    measure_one(
      name,
      "C",
      "c",
      |s, o| compile_cc("cc", s, o),
      expected,
      &bench_dir,
    ),
    measure_one(
      name,
      "C++",
      "cpp",
      |s, o| compile_cc("g++", s, o),
      expected,
      &bench_dir,
    ),
  ]
}

#[test]
#[ignore]
fn benchmarks() {
  let benches: [(&str, &str); 2] = [("sum", "49999995000000"), ("array_traversal", "210000000")];

  let mut report = String::new();
  report.push_str("# Emerald Benchmark Report\n\n");
  report.push_str(
    "Plan 15 (`benchmarking`) established this harness; plan 16 \
     (`codegen-backend-bakeoff`), measured 2026-09-08, added a second, \
     tuned Emerald backend (LLVM via `inkwell`) alongside a real \
     `opt_level=speed` fix to the original Cranelift backend, and \
     re-measured everything for real by compiling and running each \
     program in `benchmarks/` during this test. Ruby is excluded (not \
     installed in this environment); memory usage is not measured. See \
     `.cursor/plans/benchmarking.plan.md` and \
     `.cursor/plans/codegen-backend-bakeoff.plan.md` for the full \
     Decision logs, including why only 2 of inception §21's 9 suggested \
     benchmarks are expressible in this compiler today, and why the LLVM \
     backend is scoped to exactly the AST shape those 2 programs use.\n\n\
     **These specific numbers are a snapshot from one run on one \
     machine** — they will vary on different hardware/load; the durable \
     artifacts are the benchmark source programs and this runner, not \
     these exact figures.\n\n\
     Every row's binary is asserted to produce the exact expected output \
     before its timing is recorded, so a wrong-but-fast program can't \
     appear here.\n\n",
  );

  for (name, expected) in benches {
    report.push_str(&format!("## {name}\n\n"));
    report.push_str("| Language | Compile time | Run time | Binary size |\n");
    report.push_str("|---|---|---|---|\n");
    let measurements = benchmark(name, expected);
    for m in &measurements {
      report.push_str(&format!(
        "| {} | {} | {} | {} bytes |\n",
        m.language,
        format_duration(m.compile_time),
        format_duration(m.run_time),
        m.binary_size
      ));
    }
    report.push('\n');
  }

  println!("{report}");
  std::fs::write(workspace_root().join("benchmarks/REPORT.md"), &report)
    .expect("should write benchmarks/REPORT.md");
}
