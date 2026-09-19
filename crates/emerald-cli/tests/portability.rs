//! Plan 27's runtime-bundling proof: the `emerald-cli` binary, copied
//! *alone* to an empty scratch directory with no access to this repo's
//! `runtime/emerald_runtime.c`, still compiles, links, and runs a real
//! `.em` program. Before `build.rs` embedded the compiled runtime
//! archive into the binary itself (`include_bytes!`), `link_stage`
//! looked the `.c` file up via a `CARGO_MANIFEST_DIR`-relative path at
//! *runtime* — this test is the real, executed regression guard against
//! that dev-time-only dependency ever coming back.

use std::process::Command;

#[test]
fn copied_alone_binary_links_and_runs_with_no_repo_access() {
  let scratch = std::env::temp_dir().join(format!(
    "emerald_cli_portability_{}_{:?}",
    std::process::id(),
    std::thread::current().id()
  ));
  std::fs::create_dir_all(&scratch).expect("failed to create scratch dir");

  let binary_copy = scratch.join("emerald-cli");
  std::fs::copy(env!("CARGO_BIN_EXE_emerald"), &binary_copy)
    .expect("failed to copy the emerald-cli binary into the scratch dir");
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&binary_copy).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&binary_copy, perms).unwrap();
  }

  let source_path = scratch.join("hello.em");
  std::fs::write(
    &source_path,
    "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n",
  )
  .expect("failed to write the scratch .em source");

  let output_path = scratch.join("hello_bin");

  // `current_dir(&scratch)` — the whole point: this process has no
  // relative or absolute path back to this repo's own
  // `runtime/emerald_runtime.c` available to it, only what the binary
  // itself already carries.
  let compile_status = Command::new(&binary_copy)
    .current_dir(&scratch)
    .arg(&source_path)
    .arg("-o")
    .arg(&output_path)
    .status()
    .expect("failed to run the copied-alone emerald-cli binary");
  assert!(
    compile_status.success(),
    "the copied-alone binary must still compile and link hello.em with no access to this repo"
  );

  let run = Command::new(&output_path)
    .output()
    .expect("failed to run the linked binary");
  assert!(run.status.success(), "the linked binary should exit 0");
  assert_eq!(
    String::from_utf8_lossy(&run.stdout),
    "42\n",
    "the copied-alone binary's output must match the in-repo pipeline exactly"
  );

  std::fs::remove_dir_all(&scratch).ok();
}
