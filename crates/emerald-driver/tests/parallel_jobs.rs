//! Plan 49's `leaf-parallel-codegen-and-jobs-flag`: real, executed
//! end-to-end proof that `--jobs` actually controls concurrency, not
//! just that the worked three-file example compiles and runs
//! correctly either way.
//!
//! All `EMERALD_TEST_COMPILE_DELAY_MS`/`EMERALD_TRACE_COMPILE`/
//! `EMERALD_TRACE_COMPILE_OUT`-dependent scenarios live in ONE test
//! function, run strictly sequentially inside it — these are process-
//! global environment variables, so letting two `#[test]`s that touch
//! them run concurrently in the same test binary process would be a
//! real, self-inflicted race, not a flaky-by-nature timing test.

use std::path::PathBuf;
use std::process::Command;

fn write_worked_example(dir: &std::path::Path) {
  // Adapted from the plan's own literal pseudocode (`def b_value ->
  // Int64` called as bare `b_value`) — verified this session that
  // Emerald's real grammar treats a bare identifier as a variable
  // reference, never an implicit no-arg call; `()` is required at both
  // the declaration and the call site for a zero-parameter function,
  // matching every other zero-arg example in this codebase (e.g.
  // `emerald-cli/src/require.rs`'s own `helper()`).
  std::fs::write(dir.join("b.em"), "def b_value() -> Int64\n  10\nend\n").unwrap();
  std::fs::write(dir.join("c.em"), "def c_value() -> Int64\n  20\nend\n").unwrap();
  std::fs::write(
    dir.join("main.em"),
    "require b\nrequire c\nputs b_value() + c_value()\n",
  )
  .unwrap();
}

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-driver-parallel-jobs-test-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

#[test]
fn worked_example_compiled_with_jobs_2_links_and_prints_30() {
  let dir = fresh_dir("worked-example");
  write_worked_example(&dir);
  let output = dir.join("main_out");
  let cache = emerald_driver::cache::QueryCache::new(dir.join(".cache"));
  emerald_driver::parallel::compile_parallel(
    &dir.join("main.em"),
    &output,
    2,
    &cache,
    &emerald_driver::cache::SilentReporter,
  )
  .unwrap();
  let run = Command::new(&output).output().unwrap();
  assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "30");
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn single_file_program_still_compiles_and_runs_identically_under_compile_parallel() {
  // Regression (leaf 3 AC4): a one-file/one-level program is the
  // degenerate case — `--jobs` defaulting to `available_parallelism()`
  // (or any value) doesn't change its output.
  let dir = fresh_dir("single-file");
  std::fs::write(
    dir.join("hello.em"),
    "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n",
  )
  .unwrap();
  let output = dir.join("hello_out");
  let cache = emerald_driver::cache::QueryCache::new(dir.join(".cache"));
  emerald_driver::parallel::compile_parallel(
    &dir.join("hello.em"),
    &output,
    4,
    &cache,
    &emerald_driver::cache::SilentReporter,
  )
  .unwrap();
  let run = Command::new(&output).output().unwrap();
  assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_actor_shared_across_require_d_files_links_and_runs_correctly_under_jobs() {
  // Regression: `examples/host.em`/`examples/client.em` (both `require
  // counter_actor`, an `actor Counter`) failed to LINK under `--jobs`
  // — every `.o` in the require-closure independently re-emits full
  // definitions for a shared class/actor's methods AND its trampoline/
  // wire-codec/method-table scaffolding (`own_function_names` only
  // scoped plain `Item::Function`s, not classes/actors), so two `.o`s
  // both defining `Counter_initialize`/`Counter_increment__trampoline`/
  // etc. hit a real `multiple definition of ...` error from `cc`/`ld`.
  // Fixed by giving all of those symbols `WeakODR` linkage (`emerald-
  // codegen/src/lib.rs`'s `declare_actor_trampolines`/`declare_wire_
  // class_codecs`/`declare_actor_wire_arg_codecs`/`build_actor_method_
  // tables`/`weak_odr_class_shaped_methods`) so the linker dedups
  // identical multi-TU definitions instead of rejecting them. This test
  // proves the link succeeds AND the actor's own message-send/mailbox
  // behavior is still correct across the file boundary — single-
  // process, local `.spawn()`, not `.remote()`/sockets (that exact
  // two-process-over-real-TCP shape was proven by hand, once, using
  // this fix, via `examples/host.em` + `examples/client.em` — not
  // automated here since it needs two real OS processes).
  let dir = fresh_dir("actor-across-require");
  std::fs::write(
    dir.join("counter.em"),
    "actor Counter\n  count: Int64\n\n  def initialize(start: Int64) -> Void\n    @count = start\n  end\n\n  def increment -> Void\n    @count = @count + 1\n  end\n\n  def report -> Void\n    puts @count\n  end\nend\n",
  )
  .unwrap();
  std::fs::write(
    dir.join("main.em"),
    "require counter\n\nc: Counter = Counter.spawn(0)\nc.increment\nc.increment\nc.increment\nc.report\n",
  )
  .unwrap();
  let output = dir.join("main_out");
  let cache = emerald_driver::cache::QueryCache::new(dir.join(".cache"));
  emerald_driver::parallel::compile_parallel(
    &dir.join("main.em"),
    &output,
    2,
    &cache,
    &emerald_driver::cache::SilentReporter,
  )
  .unwrap();
  let run = Command::new(&output).output().unwrap();
  assert!(
    run.status.success(),
    "actor-across-require binary exited non-zero: {}",
    String::from_utf8_lossy(&run.stderr)
  );
  assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "3");
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn main_referencing_a_function_b_does_not_export_is_rejected_not_silently_permissive() {
  // leaf-parallel-parse-and-typecheck AC2: level-ordering isn't merely
  // fast, it's necessary — proves this leaf didn't accidentally make
  // cross-level visibility unconditionally permissive to get
  // parallelism. `b.em` never declares `not_exported`.
  let dir = fresh_dir("cross-level-visibility");
  std::fs::write(dir.join("b.em"), "def b_value() -> Int64\n  10\nend\n").unwrap();
  std::fs::write(dir.join("main.em"), "require b\nputs not_exported()\n").unwrap();
  let cache = emerald_driver::cache::QueryCache::new(dir.join(".cache"));
  let result = emerald_driver::parallel::compile_parallel(
    &dir.join("main.em"),
    &dir.join("out"),
    2,
    &cache,
    &emerald_driver::cache::SilentReporter,
  );
  match result {
    Err(emerald_driver::DriverError::Sema(diags)) => {
      assert!(
        diags.iter().any(|d| d.message.contains("not_exported")),
        "{diags:?}"
      );
    }
    other => panic!("expected a Sema error naming `not_exported`, got {other:?}"),
  }
  std::fs::remove_dir_all(&dir).ok();
}

/// Parses `EMERALD_TRACE_COMPILE_OUT`'s dumped
/// `file\tthread_id\tstart_nanos\tend_nanos` lines.
fn read_trace(path: &std::path::Path) -> Vec<(String, String, u128, u128)> {
  std::fs::read_to_string(path)
    .unwrap()
    .lines()
    .map(|line| {
      let mut parts = line.split('\t');
      let file = parts.next().unwrap().to_string();
      let thread_id = parts.next().unwrap().to_string();
      let start: u128 = parts.next().unwrap().parse().unwrap();
      let end: u128 = parts.next().unwrap().parse().unwrap();
      (file, thread_id, start, end)
    })
    .collect()
}

#[test]
fn jobs_flag_genuinely_controls_concurrency_timed_proof() {
  let delay_ms: u128 = 200;

  // --- --jobs 2: b.em and c.em (level 0) should overlap on distinct threads ---
  let dir2 = fresh_dir("timed-jobs2");
  write_worked_example(&dir2);
  let trace_out_2 = dir2.join("trace2.tsv");
  unsafe {
    std::env::set_var("EMERALD_TEST_COMPILE_DELAY_MS", delay_ms.to_string());
    std::env::set_var("EMERALD_TRACE_COMPILE", "1");
    std::env::set_var("EMERALD_TRACE_COMPILE_OUT", &trace_out_2);
  }
  let cache2 = emerald_driver::cache::QueryCache::new(dir2.join(".cache"));
  emerald_driver::parallel::compile_parallel(
    &dir2.join("main.em"),
    &dir2.join("out"),
    2,
    &cache2,
    &emerald_driver::cache::SilentReporter,
  )
  .unwrap();
  unsafe {
    std::env::remove_var("EMERALD_TEST_COMPILE_DELAY_MS");
    std::env::remove_var("EMERALD_TRACE_COMPILE");
    std::env::remove_var("EMERALD_TRACE_COMPILE_OUT");
  }
  let records2 = read_trace(&trace_out_2);
  let level0_2: Vec<_> = records2
    .iter()
    .filter(|(f, ..)| f.ends_with("b.em") || f.ends_with("c.em"))
    .collect();
  assert_eq!(level0_2.len(), 2, "{records2:?}");
  let (_, tid_a, start_a, end_a) = level0_2[0];
  let (_, tid_b, start_b, end_b) = level0_2[1];
  assert_ne!(tid_a, tid_b, "--jobs 2 should use two distinct threads");
  let overlap = *start_a.max(start_b) < *end_a.min(end_b);
  assert!(
    overlap,
    "--jobs 2 windows should overlap: a=[{start_a},{end_a}] b=[{start_b},{end_b}]"
  );
  // Level-0's OWN wall time, derived from the trace's own start/end
  // (not a stopwatch around the whole `compile_parallel` call, which
  // also pays LLVM target-machine init, level-1's codegen, and
  // linking) — the concrete "under ~300ms vs. over ~350ms" proof the
  // plan's own AC2 describes.
  let level0_wall_2 = (*end_a.max(end_b)).saturating_sub(*start_a.min(start_b)) / 1_000_000;
  assert!(
    level0_wall_2 < 2 * delay_ms,
    "--jobs 2 level-0 wall time should be well under a serialized 2x{delay_ms}ms: {level0_wall_2}ms"
  );
  std::fs::remove_dir_all(&dir2).ok();

  // --- --jobs 1: b.em and c.em should be strictly serialized, one thread ---
  let dir1 = fresh_dir("timed-jobs1");
  write_worked_example(&dir1);
  let trace_out_1 = dir1.join("trace1.tsv");
  unsafe {
    std::env::set_var("EMERALD_TEST_COMPILE_DELAY_MS", delay_ms.to_string());
    std::env::set_var("EMERALD_TRACE_COMPILE", "1");
    std::env::set_var("EMERALD_TRACE_COMPILE_OUT", &trace_out_1);
  }
  let cache1 = emerald_driver::cache::QueryCache::new(dir1.join(".cache"));
  emerald_driver::parallel::compile_parallel(
    &dir1.join("main.em"),
    &dir1.join("out"),
    1,
    &cache1,
    &emerald_driver::cache::SilentReporter,
  )
  .unwrap();
  unsafe {
    std::env::remove_var("EMERALD_TEST_COMPILE_DELAY_MS");
    std::env::remove_var("EMERALD_TRACE_COMPILE");
    std::env::remove_var("EMERALD_TRACE_COMPILE_OUT");
  }
  let records1 = read_trace(&trace_out_1);
  let level0_1: Vec<_> = records1
    .iter()
    .filter(|(f, ..)| f.ends_with("b.em") || f.ends_with("c.em"))
    .collect();
  assert_eq!(level0_1.len(), 2, "{records1:?}");
  let (_, tid_a1, start_a1, end_a1) = level0_1[0];
  let (_, tid_b1, start_b1, end_b1) = level0_1[1];
  assert_eq!(
    tid_a1, tid_b1,
    "--jobs 1 should run both files on the same single thread"
  );
  let overlap1 = *start_a1.max(start_b1) < *end_a1.min(end_b1);
  assert!(
    !overlap1,
    "--jobs 1 windows should NOT overlap: a=[{start_a1},{end_a1}] b=[{start_b1},{end_b1}]"
  );
  let level0_wall_1 = (*end_a1.max(end_b1)).saturating_sub(*start_a1.min(start_b1)) / 1_000_000;
  assert!(
    level0_wall_1 > 2 * delay_ms - 50,
    "--jobs 1 level-0 wall time should be close to a serialized 2x{delay_ms}ms: {level0_wall_1}ms"
  );
  std::fs::remove_dir_all(&dir1).ok();
}
