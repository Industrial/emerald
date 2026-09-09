//! Plan 64 (WebAssembly (WASI) codegen target), `leaf-wasm-worked-
//! proof` — this plan's own core claim, proven end to end: the SAME
//! source file (`benchmarks/sum/sum.em`, plan 15/16's own accumulator
//! loop), compiled through the SAME codegen path, produces
//! byte-for-byte identical stdout whether it's linked as a native ELF
//! binary and run directly, or linked as a `wasm32-wasi` module and run
//! under `wasmtime`. Gated on `wasmtime` actually being on `PATH` (skip
//! — not fail — when absent, matching this project's existing pattern
//! for any test that shells to an optional external tool), so `cargo
//! test --workspace` stays green on a machine without `wasmtime`
//! installed (this workspace's own `devenv shell`, verified this
//! session) while still running for real wherever it is available.

use std::process::Command;

const EXPECTED_STDOUT: &str = "49999995000000\n";

fn sum_em_source() -> String {
  let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
  std::fs::read_to_string(std::path::Path::new(&manifest_dir).join("../../benchmarks/sum/sum.em"))
    .expect("benchmarks/sum/sum.em should exist")
}

fn wasmtime_available() -> bool {
  Command::new("wasmtime")
    .arg("--version")
    .output()
    .is_ok_and(|o| o.status.success())
}

#[test]
fn sum_em_compiled_natively_and_run_directly_prints_the_expected_total() {
  let src = sum_em_source();
  let dir = std::env::temp_dir().join(format!("emerald_wasm_target_native_{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  let out_path = dir.join("sum_native");
  emerald_driver::compile(&src, "sum.em", &out_path).expect("native sum.em build should succeed");
  let output = Command::new(&out_path)
    .output()
    .expect("failed to run the compiled native binary");
  assert!(output.status.success(), "native sum.em run should exit 0");
  assert_eq!(String::from_utf8_lossy(&output.stdout), EXPECTED_STDOUT);
  std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sum_em_compiled_for_wasm32_wasi_and_run_under_wasmtime_matches_the_native_output_byte_for_byte()
{
  if !wasmtime_available() {
    eprintln!(
      "skipping: `wasmtime` not found on PATH (this workspace's own `devenv shell` has no WASI \
       toolchain/wasmtime configured, verified this session) — skip, not fail, matching this \
       project's existing pattern for any test gated on an optional external tool"
    );
    return;
  }

  let src = sum_em_source();
  let dir = std::env::temp_dir().join(format!("emerald_wasm_target_wasm_{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();

  let native_out = dir.join("sum_native");
  emerald_driver::compile(&src, "sum.em", &native_out).expect("native sum.em build should succeed");
  let native_output = Command::new(&native_out)
    .output()
    .expect("failed to run the compiled native binary");
  assert!(native_output.status.success());

  let wasm_out = dir.join("sum.wasm");
  emerald_driver::compile_with_target(
    &src,
    "sum.em",
    &wasm_out,
    emerald_driver::CodegenTarget::Wasm32Wasi,
  )
  .expect("wasm32-wasi sum.em build should succeed when wasmtime is present");
  let wasm_output = Command::new("wasmtime")
    .arg("run")
    .arg(&wasm_out)
    .output()
    .expect("failed to run the compiled wasm32-wasi module under wasmtime");
  assert!(
    wasm_output.status.success(),
    "wasmtime run should exit 0, stderr: {}",
    String::from_utf8_lossy(&wasm_output.stderr)
  );

  assert_eq!(
    String::from_utf8_lossy(&native_output.stdout),
    String::from_utf8_lossy(&wasm_output.stdout),
    "native and wasm32-wasi builds of the same source must print byte-for-byte identical stdout"
  );
  assert_eq!(
    String::from_utf8_lossy(&wasm_output.stdout),
    EXPECTED_STDOUT
  );

  std::fs::remove_dir_all(&dir).ok();
}
