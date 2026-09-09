//! Plan 64 (WebAssembly (WASI) codegen target), `leaf-cli-target-flag-
//! and-linking` — real, executed CLI-process proofs of `--target`'s own
//! acceptance criteria: an unrecognized value is a real, reported error
//! (AC3), `emerald run --target wasm32-wasi` is explicitly rejected
//! (AC4), and `emerald build --target wasm32-wasi` reaches real codegen
//! rather than silently falling back to native (AC2) — this machine's
//! own `devenv shell` has no WASI toolchain configured (verified this
//! session), so the expected, disclosed outcome here is a real, named
//! "no WASI toolchain configured" link-time error, not a produced
//! `.wasm` file; `crates/emerald-driver/tests/wasm_target.rs` is where
//! the full compile-and-run proof lives, gated on `wasmtime` actually
//! being on `PATH`.

use std::path::PathBuf;
use std::process::Command;

fn scaffold_package(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!("emerald_target_flag_{tag}_{}", std::process::id()));
  std::fs::create_dir_all(&dir).expect("should create package dir");
  std::fs::write(
    dir.join("emerald.toml"),
    "[package]\nname = \"targetflag\"\nversion = \"0.1.0\"\nentry = \"main.em\"\n",
  )
  .unwrap();
  std::fs::write(dir.join("main.em"), "puts \"hello\"\n").unwrap();
  dir
}

#[test]
fn an_unrecognized_target_value_is_a_real_reported_error() {
  let dir = scaffold_package("unrecognized");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .current_dir(&dir)
    .args(["build", "--target", "bogus-target"])
    .output()
    .expect("failed to run emerald-cli");
  assert!(
    !output.status.success(),
    "an unrecognized --target value must not silently succeed"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("unrecognized --target") && stderr.contains("bogus-target"),
    "stderr should name the real, unrecognized target value: {stderr}"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_run_with_target_wasm32_wasi_is_explicitly_rejected() {
  let dir = scaffold_package("run_reject");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .current_dir(&dir)
    .args(["run", "--target", "wasm32-wasi"])
    .output()
    .expect("failed to run emerald-cli");
  assert!(
    !output.status.success(),
    "`emerald run --target wasm32-wasi` must be rejected, not silently attempted"
  );
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    stderr.contains("wasmtime") || stderr.contains("not supported"),
    "stderr should clearly explain why `run` can't launch a `.wasm` module directly: {stderr}"
  );
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn emerald_build_with_target_wasm32_wasi_reaches_real_codegen_not_a_silent_native_fallback() {
  let dir = scaffold_package("build_wasm");
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .current_dir(&dir)
    .args(["build", "--target", "wasm32-wasi"])
    .output()
    .expect("failed to run emerald-cli");
  let native_output = dir.join("targetflag");
  let wasm_output = dir.join("targetflag.wasm");
  assert!(
    !native_output.exists(),
    "a `--target wasm32-wasi` build must never silently produce a native binary instead"
  );
  if output.status.success() {
    // A real WASI toolchain happened to be configured — a genuine
    // `.wasm` module must have been produced.
    assert!(
      wasm_output.exists(),
      "a successful `--target wasm32-wasi` build must produce `<name>.wasm`"
    );
  } else {
    // This workspace's own `devenv shell` (verified this session): no
    // WASI toolchain configured — the real, disclosed, named failure
    // mode, not a crash or a wrong-target silent success.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
      stderr.contains("WASI toolchain"),
      "expected a real, named WASI-toolchain error, got: {stderr}"
    );
  }
  std::fs::remove_dir_all(&dir).ok();
}
