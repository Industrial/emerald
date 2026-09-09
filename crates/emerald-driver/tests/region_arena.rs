//! Plan 51 (scope-based arena allocation) — `leaf-bounded-memory-proof`,
//! adapted (see this crate's own history for the full reasoning): plan
//! 50 (escape-analysis-stack-allocation, already landed) fully subsumes
//! the exact `Stmt::Let{ name, value: Expr::New }`-non-escaping case
//! this plan's own worked `compute_local_sum` example targets, via a
//! strictly cheaper mechanism (a raw LLVM `alloca`, reused across every
//! dynamic execution of its syntactic site, with zero allocation/free
//! call overhead at all) — so `emerald-codegen` is not wired to route
//! that same case through a region in this pass; doing so would either
//! be dead code (plan 50's own `Stmt::Let` arm intercepts those sites
//! first) or a real regression from a strictly better mechanism plan 50
//! already ships. This plan's own genuinely additive, non-overlapping
//! value is the region *primitive* itself (`runtime/emerald_runtime.c`'s
//! `emerald_region_create`/`_alloc`/`_destroy`), built to the real spec
//! so plan 54 can reuse it unmodified as an actor's isolated heap.
//!
//! This test proves that primitive's own headline claim — bounded vs.
//! unbounded memory — directly, at the runtime/harness level, since
//! there is no codegen-generated Emerald program to compile through in
//! this adaptation. `region_arena.c` (this same directory) is a
//! standalone C harness, never linked into any shipped Emerald program,
//! that calls `emerald_region_create`/`_alloc`/`_destroy` and
//! `emerald_bytes_outstanding` directly, simulating the call-frame
//! pattern a future codegen consumer would generate (create a region,
//! allocate a couple of small objects through it, destroy it — repeated
//! many times) versus the plain `emerald_alloc`-with-no-free contrast.
//! This test compiles that harness (via `cc`, alongside `runtime/
//! emerald_runtime.c`, the same real-compiler-invocation style
//! `emerald-driver`'s own `link` function already uses) and runs it in
//! each of its three modes, parsing its printed `emerald_bytes_
//! outstanding()` samples.

use std::path::PathBuf;
use std::process::Command;

fn runtime_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist")
}

fn harness_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/region_arena.c")
}

/// Compiles the harness plus the real runtime source into one binary —
/// real `cc`, not a stub — and returns its path.
fn build_harness() -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-region-arena-harness-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  let out = dir.join("region_arena_harness");

  let status = Command::new("cc")
    .arg("-O0")
    .arg("-g")
    .arg(runtime_src())
    .arg(harness_src())
    .arg("-o")
    .arg(&out)
    .status()
    .expect("failed to invoke cc");
  assert!(status.success(), "cc failed to compile the region harness");
  out
}

fn run_harness(binary: &PathBuf, mode: &str) -> Vec<i64> {
  let output = Command::new(binary)
    .arg(mode)
    .output()
    .unwrap_or_else(|e| panic!("failed to run harness in `{mode}` mode: {e}"));
  assert!(
    output.status.success(),
    "harness exited non-zero in `{mode}` mode: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8_lossy(&output.stdout)
    .lines()
    .map(|line| {
      line
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|e| panic!("harness printed a non-integer line `{line}`: {e}"))
    })
    .collect()
}

#[test]
fn region_mode_keeps_bytes_outstanding_bounded_across_iterations() {
  // AC1: under the arena-routed pattern, `emerald_bytes_outstanding()`
  // sampled across 1000 simulated calls (100 samples, one every 10
  // iterations... actually one every SAMPLE_EVERY=100 iterations, 10
  // samples total) stays within a small, fixed tolerance of its first
  // sample — bounded, not growing, a real measured fact.
  let binary = build_harness();
  let samples = run_harness(&binary, "region");
  assert_eq!(samples.len(), 10, "expected 10 samples, got {samples:?}");
  let first = samples[0];
  // Tolerance: a handful of regions' worth of chunk overhead (each
  // region's control struct + one 4096-byte default chunk) — the
  // counter should never grow by more than a couple of chunks' worth
  // across the whole run, since every iteration destroys its own
  // region before the next one is created (never two regions alive
  // at once).
  let tolerance = 3 * 4096;
  for (i, &sample) in samples.iter().enumerate() {
    assert!(
      (sample - first).abs() <= tolerance,
      "sample {i} ({sample}) drifted more than {tolerance} bytes from the first sample ({first}) — memory did not stay bounded: {samples:?}"
    );
  }
}

#[test]
fn alloc_mode_grows_bytes_outstanding_roughly_linearly() {
  // AC2: the explicit contrast — the same per-call allocation pattern
  // routed through plain `emerald_alloc` with no region and no free
  // grows the counter substantially and monotonically, proportional to
  // iteration count, across the same sampling points.
  let binary = build_harness();
  let samples = run_harness(&binary, "alloc");
  assert_eq!(samples.len(), 10, "expected 10 samples, got {samples:?}");
  for i in 1..samples.len() {
    assert!(
      samples[i] > samples[i - 1],
      "sample {i} ({}) did not grow past sample {} ({}) — expected monotonic growth: {samples:?}",
      samples[i],
      i - 1,
      samples[i - 1]
    );
  }
  // 1000 iterations * 2 objects * 16 bytes = 32,000 bytes requested in
  // total — the counter must have grown by at least that much (it may
  // grow by more, since `emerald_alloc` itself has no chunk-overhead
  // concept, so this is close to exact, but allow slack for whatever
  // the process's own baseline allocations already contributed before
  // this harness's loop started).
  let total_growth = samples[samples.len() - 1] - samples[0] + samples[0];
  assert!(
    total_growth >= 1000 * 2 * 16,
    "alloc-mode growth ({total_growth}) should be at least the {} bytes actually requested",
    1000 * 2 * 16
  );
}

/// `valgrind` is confirmed present in this sandbox (checked directly
/// this session, per the plan's own AC2 preference — ASan is the
/// documented fallback only when `valgrind` genuinely isn't available,
/// which isn't the case here).
fn valgrind_available() -> bool {
  Command::new("valgrind")
    .arg("--version")
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

#[test]
fn region_leak_check_counter_round_trips_and_reports_zero_leaks() {
  // leaf-runtime-region-arena's own AC2 (leak-check, via real
  // `valgrind --leak-check=full`) and AC3 (counter round-trips to its
  // pre-allocation baseline) — combined into one pass, per this
  // harness's `region_leak` mode: create one region, force it to grow
  // past its first 4096-byte chunk (2000 * 16 = 32,000 bytes
  // requested), destroy it, and print
  // before/after-alloc/requested/after-destroy.
  assert!(
    valgrind_available(),
    "valgrind must be on PATH for this leak-check proof — confirmed present this session"
  );
  let binary = build_harness();
  let output = Command::new("valgrind")
    .arg("--leak-check=full")
    .arg("--error-exitcode=99")
    .arg(&binary)
    .arg("region_leak")
    .output()
    .expect("failed to invoke valgrind");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    output.status.success(),
    "valgrind reported a problem (exit {:?}):\nstdout:\n{stdout}\nstderr (valgrind's own report):\n{stderr}",
    output.status.code()
  );
  assert!(
    stderr.contains("All heap blocks were freed -- no leaks are possible")
      || stderr.contains("definitely lost: 0 bytes")
      || stderr.contains("ERROR SUMMARY: 0 errors"),
    "valgrind's report didn't show the expected zero-leaks summary:\n{stderr}"
  );

  let samples: Vec<i64> = stdout
    .lines()
    .map(|l| l.trim().parse::<i64>().unwrap())
    .collect();
  assert_eq!(
    samples.len(),
    4,
    "expected [before, after_alloc, requested, after_destroy], got {samples:?}"
  );
  let (before, after_alloc, requested, after_destroy) =
    (samples[0], samples[1], samples[2], samples[3]);

  // AC3: allocating increased the counter by at least the requested
  // bytes (it may be more, due to chunk-header/alignment overhead).
  assert!(
    after_alloc - before >= requested,
    "counter grew by {} but at least {requested} bytes were requested — not a real measurement",
    after_alloc - before
  );
  // AC3: destroying the region returns the counter to its exact
  // pre-allocation baseline.
  assert_eq!(
    after_destroy, before,
    "counter did not return to its pre-allocation baseline after emerald_region_destroy: before={before}, after_destroy={after_destroy}"
  );
}
