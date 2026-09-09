//! Plan 57 (supervision trees) — proves `runtime/emerald_runtime.c`'s
//! real crash-isolation and supervisor API directly, before any
//! codegen/LLVM machinery exists to generate a real actor program
//! through it. Mirrors `scheduler_runtime.rs`'s own established
//! pattern: build `supervisor_runtime.c` (this same directory)
//! together with the real `runtime/emerald_runtime.c` via `cc`, run
//! the resulting binary in each of its three modes, and parse its
//! printed output.

use std::path::PathBuf;
use std::process::Command;

fn runtime_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist")
}

fn harness_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/supervisor_runtime.c")
}

fn build_harness(mode: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-supervisor-runtime-harness-{}-{mode}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  let out = dir.join("supervisor_runtime_harness");

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
    "cc failed to compile the supervisor runtime harness"
  );
  out
}

fn run_harness(binary: &PathBuf, mode: &str, workers: Option<u32>) -> std::process::Output {
  let mut cmd = Command::new(binary);
  cmd.arg(mode);
  if let Some(w) = workers {
    cmd.env("EMERALD_WORKERS", w.to_string());
  }
  cmd
    .output()
    .unwrap_or_else(|e| panic!("failed to run harness in `{mode}` mode: {e}"))
}

#[test]
fn ac1_an_unsupervised_actors_crash_does_not_kill_the_process() {
  let binary = build_harness("crash_isolation");
  let output = run_harness(&binary, "crash_isolation", None);
  std::fs::remove_dir_all(binary.parent().unwrap()).ok();
  assert!(
    output.status.success(),
    "the process must exit 0, not crash or call exit(1) via the old h==NULL uncaught path: {output:?}"
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(
    stdout.contains("survivor ok"),
    "a sibling actor must keep working after an unrelated actor's crash:\n{stdout}"
  );
  assert!(
    stdout.contains("process survived"),
    "reaching this line at all is half the proof — the crashing actor's own raise must not \
     have reached the old process-fatal path:\n{stdout}"
  );
}

#[test]
fn ac2_two_concurrently_crashing_actors_each_report_their_own_correct_tag() {
  // Forces both actors onto distinct worker threads via EMERALD_WORKERS=2
  // and a barrier inside the harness itself — proving the thread-local
  // handler-stack conversion (plan 55's own prerequisite) genuinely
  // isolates the two stacks rather than one actor's longjmp occasionally
  // landing in the other's frame.
  let binary = build_harness("concurrent_crash");
  let output = run_harness(&binary, "concurrent_crash", Some(2));
  std::fs::remove_dir_all(binary.parent().unwrap()).ok();
  assert!(output.status.success(), "{output:?}");
  let stdout = String::from_utf8_lossy(&output.stdout);
  let lines: Vec<&str> = stdout.lines().collect();
  assert_eq!(
    lines,
    vec!["101", "202"],
    "each actor's own exception tag must be attributed to the correct actor, not swapped or \
     corrupted by a shared (non-thread-local) handler stack:\n{stdout}"
  );
}

#[test]
fn one_for_one_supervisor_worked_example() {
  // Forced EMERALD_WORKERS=1: cross-ACTOR ordering (as opposed to the
  // per-actor FIFO ordering this scheduler always guarantees — plan
  // 55's own established, disclosed non-guarantee) is not otherwise
  // deterministic, so this test controls for it directly rather than
  // asserting an ordering the scheduler was never designed to promise
  // across two independent actors.
  let binary = build_harness("supervisor");
  let output = run_harness(&binary, "supervisor", Some(1));
  std::fs::remove_dir_all(binary.parent().unwrap()).ok();
  assert!(
    output.status.success(),
    "supervisor worked example should exit 0: {output:?}"
  );
  let stdout = String::from_utf8_lossy(&output.stdout);
  let lines: Vec<&str> = stdout.lines().collect();

  // Deterministic prefix: `Worker`'s own mailbox is drained strictly
  // FIFO (plan 55's own guarantee), and crash-handling — including
  // this restart's own "restarting Worker" print — runs synchronously,
  // inline, on the SAME worker thread processing that 3rd message,
  // before it moves on to anything else. What is NOT deterministic,
  // even under `EMERALD_WORKERS=1`, is exactly WHEN the single worker
  // thread interleaves `Logger`'s own two independently-enqueued sends
  // relative to `Worker`'s three — this scheduler only ever promises
  // PER-ACTOR FIFO order, never a fixed CROSS-actor interleaving (the
  // same disclosed non-guarantee plan 55's own `PingPong`/`Spinner`
  // examples already established) — so the two `Logger` sends can land
  // before, between, or after `Worker`'s own messages instead of
  // exactly matching the plan's own literal source-order narrative.
  assert_eq!(
    &lines[0..3],
    &["1", "2", "restarting Worker"],
    "Worker's own FIFO order, and the crash's restart happening synchronously right after \
     Worker's 3rd message, are both real guarantees:\n{stdout}"
  );
  assert_eq!(
    lines.last(),
    Some(&"dead send did not crash"),
    "AC4: the pre-crash reference's later send must be silently dropped, not a crash, and \
     drain_and_join must return normally after it:\n{stdout}"
  );
  let middle = &lines[3..lines.len() - 1];
  let mut sorted_middle = middle.to_vec();
  sorted_middle.sort_unstable();
  assert_eq!(
    sorted_middle,
    vec!["1", "log", "log"],
    "AC2 (one_for_one: Logger's own two sends both print plain \"log\", it never restarts \
     and never notices Worker's crash) and AC3 (state-reset: the post-restart send prints 1, \
     not 4 — proving the fresh instance's own @count started from the captured original \
     argument, not the crashed instance's count==3) together, order-independent per this \
     scheduler's own real cross-actor guarantees:\n{stdout}"
  );
}
