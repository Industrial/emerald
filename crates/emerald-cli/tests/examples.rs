//! Compiles and runs every file under `examples/` (except `hello.em`,
//! already covered by `hello_em.rs`), asserting exact stdout — the
//! durable, re-checkable version of the manual verification behind
//! `examples/README.md`'s coverage table.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile_and_run(example: &str) -> String {
  let source = workspace_root().join("examples").join(example);
  let output = std::env::temp_dir().join(format!(
    "emerald_example_{}_{}",
    example.replace(['.', '/'], "_"),
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(status.success(), "emerald-cli should succeed on {example}");

  let run = Command::new(&output)
    .output()
    .expect("failed to run compiled binary");
  assert!(
    run.status.success(),
    "{example}'s compiled binary should exit 0"
  );

  std::fs::remove_file(&output).ok();
  String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn control_flow_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("control_flow.em"),
    "1\n1\n1\n1\n1\n1\n0\n1\n3\n0\n1\n2\n101\n103\n0\n1\n5\n6\n7\n200\n"
  );
}

#[test]
fn classes_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("classes.em"), "10\n15\n5\n");
}

#[test]
fn collections_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("collections.em"),
    "60\n99\n4\n10\n20\n1\n3\n4\n"
  );
}

#[test]
fn closures_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("closures.em"), "15\n42\n0\n1\n2\n");
}

#[test]
fn exceptions_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("exceptions.em"), "99\n5\n2\n777\n");
}

#[test]
fn modules_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("modules.em"), "42\n");
}

#[test]
fn interfaces_generics_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("interfaces_generics.em"), "750\n100\n");
}

#[test]
fn strings_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("strings.em"),
    "Hello, World! You are 30 years old.\nHello World\n  HELLO WORLD  \n  hello world  \n15\n6\n42\n3.5\n"
  );
}

#[test]
fn symbols_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("symbols.em"), "82\n100\n1\n0\n");
}

#[test]
fn nullable_safe_nav_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("nullable_safe_nav.em"), "1\n0\n");
}

#[test]
fn bitwise_and_assignment_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("bitwise_and_assignment.em"),
    "3\n1\n1\n-1\n16\n16\n10\n2\n1\n"
  );
}

#[test]
fn ranges_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("ranges.em"), "15\n10\n");
}

#[test]
fn class_inheritance_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("class_inheritance.em"), "5\n3\n103\n");
}

#[test]
fn operator_overloading_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("operator_overloading.em"), "4\n6\n0\n1\n");
}

#[test]
fn function_signatures_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("function_signatures.em"),
    "1\n2\n3\n2\n60\n"
  );
}

#[test]
fn enumerable_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("enumerable.em"),
    "0\n1\n2\n3\n4\n10\n2\n15\n3\n15\n1\n10\n60\n2\n3\n"
  );
}

#[test]
fn doc_comments_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("doc_comments.em"),
    "42\n7\n42\n7\n14\n36\n500\n"
  );
}

// Plan 76's `import-export-module-visibility`. A multi-file example
// (like `parallel/`/`packages/`) — `compile_and_run` only ever passes
// `examples/<example>` straight to `emerald-cli`, so a subdirectory
// entry point works unchanged; `main.em`'s own bare `require greeter`
// is what makes `run_legacy`'s pre-existing `requires_present` check
// (`main.rs`) route this through the real multi-file resolver.
#[test]
fn module_visibility_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("module_visibility/main.em"),
    "Hello, Emerald!\n7\n"
  );
}

// Regression for the plan 69/76 gap: `main.em` above always has a
// bare `require greeter` alongside its `import`, so it never actually
// exercised the case of an import-only file — `run_legacy`'s
// `requires_present` check (`main.rs`) used to only match
// `Item::Require`, silently skipping multi-file resolution for a file
// using only `import`. `import_only.em` has zero bare `require`s.
#[test]
fn module_visibility_import_only_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("module_visibility/import_only.em"),
    "7\n12\n"
  );
}

// Plan 81's `domain-types-and-units` (`newtype`).
#[test]
fn domain_types_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("domain_types.em"), "10.4384\n1\n100\n");
}

// Plans 83/84's `own`/`borrow`/`borrow var` (`spec/OWNERSHIP.md` §2/§9/§10).
#[test]
fn ownership_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("ownership.em"), "10\n11\n11\n21\n");
}

// Plan 85's `ownership-actor-ffi-integration` (`spec/OWNERSHIP.md` §7):
// `own`/`borrow` typing at an `unsafe extern "C"` boundary.
#[test]
fn ffi_ownership_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("ffi_ownership.em"), "owned copy\nworld\n");
}
