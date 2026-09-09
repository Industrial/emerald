//! Plan 65 (automatic actor placement), `leaf-automatic-discovery`'s own
//! AC1/AC2/AC3 — proven directly at the runtime layer, before any
//! codegen/LLVM machinery exists to generate a real actor program
//! through. Mirrors `scheduler_runtime.rs`'s own established pattern:
//! build `discovery_runtime.c` (this same directory) together with the
//! real `runtime/emerald_runtime.c` via `cc`, run the resulting binary
//! in each of its three modes with different `EMERALD_PEERS`/
//! `EMERALD_SELF` environments, and parse its printed output.

use std::path::PathBuf;
use std::process::Command;

fn runtime_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("../../runtime/emerald_runtime.c")
    .canonicalize()
    .expect("runtime/emerald_runtime.c must exist")
}

fn harness_src() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/discovery_runtime.c")
}

fn build_harness(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-discovery-runtime-harness-{}-{tag}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  let out = dir.join("discovery_runtime_harness");

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
    "cc failed to compile the discovery runtime harness"
  );
  out
}

fn run_harness(
  binary: &PathBuf,
  mode: &str,
  peers: Option<&str>,
  self_addr: Option<&str>,
) -> (i32, Vec<String>) {
  let mut cmd = Command::new(binary);
  cmd.arg(mode);
  cmd.env_remove("EMERALD_PEERS");
  cmd.env_remove("EMERALD_SELF");
  if let Some(p) = peers {
    cmd.env("EMERALD_PEERS", p);
  }
  if let Some(s) = self_addr {
    cmd.env("EMERALD_SELF", s);
  }
  let output = cmd
    .output()
    .unwrap_or_else(|e| panic!("failed to run harness in `{mode}` mode: {e}"));
  let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
    .lines()
    .map(|l| l.to_string())
    .collect();
  (output.status.code().unwrap_or(-1), lines)
}

#[test]
fn ac1_a_real_peer_list_parses_with_the_correct_self_index() {
  let binary = build_harness("basic");
  let (code, lines) = run_harness(
    &binary,
    "basic",
    Some("127.0.0.1:9001,127.0.0.1:9002,127.0.0.1:9003"),
    Some("127.0.0.1:9002"),
  );
  assert_eq!(code, 0, "harness should exit 0: {lines:?}");
  assert_eq!(lines[0], "3", "expected a 3-element peer list: {lines:?}");
  assert_eq!(
    lines[1], "1",
    "127.0.0.1:9002 is index 1 of the 3-element list: {lines:?}"
  );
  assert_eq!(
    &lines[2..5],
    &[
      "127.0.0.1:9001".to_string(),
      "127.0.0.1:9002".to_string(),
      "127.0.0.1:9003".to_string()
    ],
    "peer addrs should round-trip verbatim: {lines:?}"
  );
}

#[test]
fn ac_unset_peers_is_a_valid_single_process_configuration() {
  let binary = build_harness("basic_unset");
  let (code, lines) = run_harness(&binary, "basic", None, None);
  assert_eq!(
    code, 0,
    "an unset EMERALD_PEERS must not be an error: {lines:?}"
  );
  assert_eq!(lines[0], "0", "expected an empty peer list: {lines:?}");
  assert_eq!(lines[1], "-1", "expected self_index -1: {lines:?}");
}

#[test]
fn ac2_a_malformed_peer_entry_is_a_real_named_error() {
  let binary = build_harness("malformed");
  let (code, lines) = run_harness(
    &binary,
    "malformed",
    Some("127.0.0.1:9001,not-a-valid-entry"),
    Some("127.0.0.1:9001"),
  );
  assert_eq!(code, 0, "harness process itself should exit 0: {lines:?}");
  assert_eq!(
    lines,
    vec!["1"],
    "a malformed EMERALD_PEERS entry must be a real, reported failure: {lines:?}"
  );
}

#[test]
fn ac2_a_non_numeric_port_is_a_real_named_error() {
  let binary = build_harness("malformed_port");
  let (code, lines) = run_harness(
    &binary,
    "malformed",
    Some("127.0.0.1:abc"),
    Some("127.0.0.1:abc"),
  );
  assert_eq!(code, 0, "harness process itself should exit 0: {lines:?}");
  assert_eq!(
    lines,
    vec!["1"],
    "a non-numeric port must be a real, reported failure: {lines:?}"
  );
}

#[test]
fn ac3_a_missing_emerald_self_is_a_real_named_error() {
  let binary = build_harness("missing_self");
  let (code, lines) = run_harness(
    &binary,
    "missing_self",
    Some("127.0.0.1:9001,127.0.0.1:9002"),
    None,
  );
  assert_eq!(code, 0, "harness process itself should exit 0: {lines:?}");
  assert_eq!(
    lines,
    vec!["1"],
    "EMERALD_SELF unset while EMERALD_PEERS is set must be a real, reported failure: {lines:?}"
  );
}

#[test]
fn ac3_an_emerald_self_not_present_in_emerald_peers_is_a_real_named_error() {
  let binary = build_harness("mismatched_self");
  let (code, lines) = run_harness(
    &binary,
    "missing_self",
    Some("127.0.0.1:9001,127.0.0.1:9002"),
    Some("127.0.0.1:9999"),
  );
  assert_eq!(code, 0, "harness process itself should exit 0: {lines:?}");
  assert_eq!(
    lines,
    vec!["1"],
    "an EMERALD_SELF absent from EMERALD_PEERS must be a real, reported failure: {lines:?}"
  );
}
