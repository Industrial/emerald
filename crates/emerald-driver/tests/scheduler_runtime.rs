//! Plan 55 (scheduler and message passing), `leaf-thread-safe-runtime`'s
//! own AC2/AC3/AC4 — proven directly at the runtime layer, before any
//! codegen/LLVM machinery exists to generate a real actor program
//! through. Mirrors `region_arena.rs`'s own established pattern: build
//! `scheduler_runtime.c` (this same directory) together with the real
//! `runtime/emerald_runtime.c` via `cc`, run the resulting binary in
//! each of its three modes, and parse its printed output.

use std::path::PathBuf;
use std::process::Command;

fn runtime_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist")
}

fn harness_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scheduler_runtime.c")
}

/// Compiles the harness plus the real runtime source into one binary —
/// real `cc`, not a stub — and returns its path. `-pthread` is passed
/// explicitly at both compile and link time: harmless on a glibc where
/// pthread symbols already live in `libc` itself, and load-bearing on
/// any toolchain where they don't. `mode` is folded into the output
/// path — all three `#[test]`s in this file run concurrently, as
/// distinct threads inside the SAME test-binary process, so a
/// `std::process::id()`-only path (this crate's own `region_arena.rs`
/// precedent, which only ever builds one binary per test run) would
/// have every test racing to build+execute the identical file at once
/// (a real "Text file busy" failure, hit and fixed this session).
fn build_harness(mode: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-scheduler-runtime-harness-{}-{mode}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  let out = dir.join("scheduler_runtime_harness");

  let status = Command::new("cc")
    .arg("-O0")
    .arg("-g")
    .arg("-pthread")
    .arg(runtime_src())
    .arg(harness_src())
    .arg("-o")
    .arg(&out)
    .status()
    .expect("failed to invoke cc");
  assert!(
    status.success(),
    "cc failed to compile the scheduler runtime harness"
  );
  out
}

fn run_harness(binary: &PathBuf, mode: &str, workers: u32) -> Vec<i64> {
  let output = Command::new(binary)
    .arg(mode)
    .env("EMERALD_WORKERS", workers.to_string())
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
fn ac2_fifty_messages_across_five_actors_each_run_exactly_once() {
  let binary = build_harness("pool");
  let samples = run_harness(&binary, "pool", 4);
  assert_eq!(
    samples,
    vec![50],
    "every one of the 50 enqueued messages must have run its trampoline exactly once, \
     summed across all 5 actors' own counters — drain_and_join must not return early"
  );
}

#[test]
fn ac3_two_messages_on_the_same_actor_fire_in_enqueue_order() {
  let binary = build_harness("fifo");
  let samples = run_harness(&binary, "fifo", 4);
  assert_eq!(
    samples,
    vec![1],
    "the second message's trampoline must observe the first message's effect already \
     applied — real per-actor FIFO ordering, not just 'both eventually ran'"
  );
}

/// Plan 65's `leaf-unified-fallible-send` AC3, proven directly at the
/// runtime layer — `emerald_actor_enqueue` against an already-
/// terminated actor must report failure (a nonzero return), not the
/// old hardcoded success.
#[test]
fn plan65_enqueue_against_a_terminated_actor_reports_failure() {
  let binary = build_harness("terminated");
  let samples = run_harness(&binary, "terminated", 1);
  assert_eq!(
    samples,
    vec![1],
    "enqueuing a message against an already-terminated actor must report failure, not silently succeed"
  );
}

#[test]
fn ac4_two_concurrently_running_workers_report_distinct_thread_ids() {
  let binary = build_harness("threadid");
  let samples = run_harness(&binary, "threadid", 2);
  assert_eq!(
    samples.len(),
    2,
    "expected two recorded thread ids: {samples:?}"
  );
  assert_ne!(
    samples[0], samples[1],
    "both trampolines were forced to run concurrently (each blocks on a shared barrier \
     until the other has recorded its id) — they must report two different OS thread ids: {samples:?}"
  );
}
