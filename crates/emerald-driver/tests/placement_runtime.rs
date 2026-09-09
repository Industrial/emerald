//! Plan 65 (automatic actor placement), `leaf-virtual-actor-placement`'s
//! own consistent-hash proof — proven directly at the runtime layer,
//! before any codegen/LLVM machinery exists to generate a real actor
//! program through. Mirrors `discovery_runtime.rs`'s own established
//! pattern.

use std::path::PathBuf;
use std::process::Command;

fn runtime_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist")
}

fn harness_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/placement_runtime.c")
}

/// `tag` disambiguates the output path per test — all `#[test]`s in
/// this file run concurrently, as distinct threads inside the SAME
/// test-binary process (`scheduler_runtime.rs`'s own established
/// precedent/doc comment for the identical "Text file busy" race,
/// found and fixed here the same way).
fn build_harness(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-placement-runtime-harness-{}-{tag}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  let out = dir.join("placement_runtime_harness");

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
    "cc failed to compile the placement runtime harness"
  );
  out
}

fn run_harness(binary: &PathBuf, args: &[&str]) -> Vec<i32> {
  let output = Command::new(binary)
    .args(args)
    .output()
    .unwrap_or_else(|e| panic!("failed to run harness with {args:?}: {e}"));
  assert!(
    output.status.success(),
    "harness exited non-zero with {args:?}: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  String::from_utf8_lossy(&output.stdout)
    .lines()
    .map(|l| {
      l.trim()
        .parse::<i32>()
        .unwrap_or_else(|e| panic!("non-integer line `{l}`: {e}"))
    })
    .collect()
}

#[test]
fn owner_index_is_within_the_live_peer_range() {
  let binary = build_harness("owner");
  let owner = run_harness(&binary, &["owner", "shard-7"]);
  assert_eq!(owner.len(), 1);
  assert!(
    (0..3).contains(&owner[0]),
    "owner index must name one of the 3 real peers: {owner:?}"
  );
}

#[test]
fn ac3_adding_a_never_contacted_dead_peer_does_not_change_ownership() {
  let binary = build_harness("stability");
  let result = run_harness(&binary, &["stability"]);
  assert_eq!(
    result.len(),
    2,
    "expected two owner indices (3-peer, then 4-peer-with-one-dead): {result:?}"
  );
  assert_eq!(
    result[0], result[1],
    "adding a peer that's always marked dead must not change who owns an existing key: {result:?}"
  );
}

#[test]
fn the_ring_computation_is_deterministic_across_calls_and_processes() {
  let binary = build_harness("deterministic");
  let within_process = run_harness(&binary, &["deterministic", "shard-7"]);
  assert_eq!(within_process.len(), 2);
  assert_eq!(
    within_process[0], within_process[1],
    "two calls in the same process must agree: {within_process:?}"
  );

  // A second, independent process invocation (the real "every process
  // computes the identical ring independently" claim, not just "the
  // same process remembers its own answer").
  let across_processes = run_harness(&binary, &["owner", "shard-7"]);
  assert_eq!(
    within_process[0], across_processes[0],
    "a second, independent process must compute the identical owner"
  );
}
