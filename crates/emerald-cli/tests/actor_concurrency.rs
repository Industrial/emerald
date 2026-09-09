//! Plan 55 (scheduler and message passing), `leaf-worked-concurrency-
//! proof` — the process-level, real-binary proof this plan's own claim
//! needs: a fixed pool of OS worker threads genuinely, concurrently
//! processing actor messages, with per-actor FIFO ordering preserved.
//! Two halves, because raw concurrency is invisible in ordinary
//! sequential stdout and each half proves a different half of the
//! claim (see the plan's own "Concrete proof this plan targets"
//! section):
//!
//! (a) **Correctness under concurrency** — `PINGPONG_EXAMPLE`, a
//!     deterministic 5-round ping-pong between two actor instances,
//!     repeated 20 times under a forced `EMERALD_WORKERS=4` pool so a
//!     single-worker pool can't trivially "prove" ordering that only
//!     holds by accident of running everything on one thread. Each
//!     `hit` also logs `current_thread_id()` — best-effort,
//!     probabilistic supplementary evidence (never the primary proof)
//!     that more than one OS thread genuinely ran actor code across the
//!     20-run aggregate.
//! (b) **Real concurrent execution actually happened** — `SPINNER_
//!     EXAMPLE`'s two independent, non-communicating, equal-sized
//!     CPU-bound spins, timed once serialized (`EMERALD_WORKERS=1`,
//!     the single-spin baseline unit) and once concurrent (the default
//!     pool). Only genuine OS-thread overlap can bring the concurrent
//!     run close to the baseline rather than ~2x it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Instant;

fn compile_em(source: &str, tag: &str) -> PathBuf {
  let dir = std::env::temp_dir();
  let src_path = dir.join(format!(
    "emerald_actor_concurrency_{tag}_{}.em",
    std::process::id()
  ));
  std::fs::write(&src_path, source).expect("should write source file");
  let out_path = dir.join(format!(
    "emerald_actor_concurrency_{tag}_bin_{}",
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&out_path)
    .status()
    .expect("failed to run emerald-cli");
  assert!(status.success(), "emerald-cli should succeed on {tag}.em");
  std::fs::remove_file(&src_path).ok();
  out_path
}

fn run_with_workers(bin: &Path, workers: u32) -> Output {
  Command::new(bin)
    .env("EMERALD_WORKERS", workers.to_string())
    .output()
    .expect("failed to run compiled binary")
}

/// The `hit`/`current_thread_id()` shape the plan's own AC4 names,
/// kept separate from `emerald-codegen`'s own `PINGPONG_EXAMPLE` (which
/// asserts an exact, thread-id-free 9-line output) — this crate's own
/// copy adds the thread-id column that test doesn't need.
const PINGPONG_EXAMPLE: &str = "actor PingPong\n  name: String\n  limit: Int64\n  count: Int64\n  peer: PingPong\n\n  def initialize(name: String, limit: Int64) -> Void\n    @name = name\n    @limit = limit\n    @count = 0\n  end\n\n  def set_peer(other: PingPong) -> Void\n    @peer = other\n  end\n\n  def hit -> Void\n    @count = @count + 1\n    puts \"#{@name} #{@count} #{current_thread_id()}\"\n    if @count < @limit\n      @peer.hit\n    end\n  end\nend\n\na: PingPong = PingPong.spawn(\"A\", 5)\nb: PingPong = PingPong.spawn(\"B\", 5)\na.set_peer(b)\nb.set_peer(a)\na.hit\n";

#[test]
fn pingpong_ordering_holds_across_20_repetitions_under_a_forced_multi_worker_pool() {
  let bin = compile_em(PINGPONG_EXAMPLE, "pingpong");
  let expected_order: Vec<(&str, &str)> = vec![
    ("A", "1"),
    ("B", "1"),
    ("A", "2"),
    ("B", "2"),
    ("A", "3"),
    ("B", "3"),
    ("A", "4"),
    ("B", "4"),
    ("A", "5"),
  ];

  let mut seen_thread_ids = std::collections::HashSet::new();

  for run in 0..20 {
    let output = run_with_workers(&bin, 4);
    assert!(
      output.status.success(),
      "run {run}: compiled binary should exit 0, stderr: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
      lines.len(),
      9,
      "run {run}: expected exactly 9 hops, got {lines:?}"
    );
    for (i, line) in lines.iter().enumerate() {
      let parts: Vec<&str> = line.split_whitespace().collect();
      assert_eq!(
        parts.len(),
        3,
        "run {run} line {i}: expected `name count thread_id`, got `{line}`"
      );
      assert_eq!(
        (parts[0], parts[1]),
        expected_order[i],
        "run {run} line {i}: real per-actor FIFO ordering must hold every run, regardless \
         of worker-pool size — got `{line}`, full output: {lines:?}"
      );
      seen_thread_ids.insert(parts[2].to_string());
    }
  }

  std::fs::remove_file(&bin).ok();

  // AC4: supplementary, best-effort, disclosed-probabilistic evidence
  // only (actor-to-thread assignment isn't fixed) — across the 20-run
  // aggregate, at least 2 distinct thread ids should appear somewhere.
  // Not the primary concurrency proof (that's the Spinner timing test
  // below); a failure here is logged, not a hard requirement, since a
  // pathological scheduler could in principle always pick the same
  // worker without violating any correctness property this plan makes.
  if seen_thread_ids.len() < 2 {
    eprintln!(
      "note: only {} distinct thread id(s) observed across 20 runs (best-effort evidence only, not a hard failure): {seen_thread_ids:?}",
      seen_thread_ids.len()
    );
  }
}

/// This plan's own `Spinner` worked example.
fn spinner_example(iterations: u64) -> String {
  format!(
    "actor Spinner\n  id: Int64\n  total: Int64\n\n  def initialize(id: Int64) -> Void\n    @id = id\n    @total = 0\n  end\n\n  def spin(iterations: Int64) -> Void\n    i: Int64 = 0\n    while i < iterations\n      @total = @total + i\n      i = i + 1\n    end\n    puts \"spinner #{{@id}} done\"\n  end\nend\n\ns1: Spinner = Spinner.spawn(1)\ns2: Spinner = Spinner.spawn(2)\ns1.spin({iterations})\ns2.spin({iterations})\n"
  )
}

/// A single actor, a single `spin` call — the baseline unit AC3 needs:
/// "a single `spin(200000000)` call's own measured time."
fn single_spin_example(iterations: u64) -> String {
  format!(
    "actor Spinner\n  id: Int64\n  total: Int64\n\n  def initialize(id: Int64) -> Void\n    @id = id\n    @total = 0\n  end\n\n  def spin(iterations: Int64) -> Void\n    i: Int64 = 0\n    while i < iterations\n      @total = @total + i\n      i = i + 1\n    end\n    puts \"spinner #{{@id}} done\"\n  end\nend\n\ns1: Spinner = Spinner.spawn(1)\ns1.spin({iterations})\n"
  )
}

/// Real cores available to this process — the timing assertion below is
/// only meaningful (and only satisfiable by genuine concurrency) on a
/// multi-core runner; a detected single-core one skips, disclosed as a
/// real environment-dependent limitation of a wall-clock-based proof
/// (the plan's own AC3).
fn detected_core_count() -> usize {
  std::thread::available_parallelism()
    .map(std::num::NonZero::get)
    .unwrap_or(1)
}

#[test]
fn spinner_concurrent_run_is_close_to_the_single_spin_baseline_not_double_it() {
  if detected_core_count() < 2 {
    eprintln!("skipping: detected a single-core runner — no real concurrency is possible here");
    return;
  }

  const ITERATIONS: u64 = 200_000_000;

  let single_bin = compile_em(&single_spin_example(ITERATIONS), "single_spin");
  let baseline_start = Instant::now();
  let baseline_output = run_with_workers(&single_bin, 1);
  let baseline = baseline_start.elapsed();
  assert!(
    baseline_output.status.success(),
    "single-spin baseline run should exit 0"
  );
  std::fs::remove_file(&single_bin).ok();

  let concurrent_bin = compile_em(&spinner_example(ITERATIONS), "spinner_concurrent");
  // Default pool size (no `EMERALD_WORKERS` override) — `>= 2` workers
  // on this multi-core runner, so both `spin` calls can genuinely
  // overlap.
  let concurrent_start = Instant::now();
  let concurrent_output = Command::new(&concurrent_bin)
    .output()
    .expect("failed to run compiled binary");
  let concurrent = concurrent_start.elapsed();
  assert!(
    concurrent_output.status.success(),
    "concurrent spinner run should exit 0, stderr: {}",
    String::from_utf8_lossy(&concurrent_output.stderr)
  );
  std::fs::remove_file(&concurrent_bin).ok();

  let stdout = String::from_utf8_lossy(&concurrent_output.stdout);
  let mut lines: Vec<&str> = stdout.lines().collect();
  lines.sort_unstable();
  assert_eq!(lines, vec!["spinner 1 done", "spinner 2 done"]);

  let margin = baseline.mul_f64(1.6);
  assert!(
    concurrent <= margin,
    "two spin({ITERATIONS}) calls issued back to back took {concurrent:?} — expected at most \
     1.6x the single-spin baseline ({baseline:?}, margin {margin:?}) if they genuinely ran \
     concurrently on separate OS threads; a purely serialized execution would cost close to 2x"
  );
}
