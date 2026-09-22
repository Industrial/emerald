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

// Plan 168's own structured-logging output is deliberately on stderr,
// structurally separate from `puts`'s own stdout — a sibling of
// `compile_and_run` asserting against stderr instead, rather than
// changing that function's own stdout-only contract for everything
// else in this table.
fn compile_and_run_stderr(example: &str) -> String {
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
  String::from_utf8_lossy(&run.stderr).into_owned()
}

#[test]
fn rust_native_runtime_proof_em_prints_expected_sequence() {
  // Plan 91: FNV-1a-32 of "hello"/"hello world"/"" widened to Int64 —
  // hand-computed and independently verified against a from-scratch
  // Python re-implementation of the algorithm before being hard-coded
  // here, not copied from this crate's own output.
  // Plan 92 extends the same example: `.fnv1a_hash_checked`'s Ok path
  // on "hello" (the identical FNV-1a value a second time), its Err
  // path on "" (`input must not be empty`), and a real panic from
  // `.fnv1a_hash_panic_for_test` caught via `rescue NativeError => e`
  // (`e.message` — the exact literal the panic! call in
  // `crates/emerald-rt/src/lib.rs` uses).
  assert_eq!(
    compile_and_run("rust_native_runtime_proof.em"),
    "1335831723\n3582672807\n2166136261\n1335831723\ninput must not be empty\nnative panic in fnv1a_hash_panic_for_test\n"
  );
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
  assert_eq!(compile_and_run("closures.em"), "15\n42\n0\n1\n2\n42\n");
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
// The trailing `42` is this session's own "find all bugs" sweep fix
// (2026-09-21): a top-level `borrow var Int64` parameter can now
// actually be reassigned, reaching the real pointer/writeback codegen
// mechanism plan 84 already built.
#[test]
fn ownership_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("ownership.em"), "10\n11\n10\n21\n42\n");
}

// Plan 85's `ownership-actor-ffi-integration` (`spec/OWNERSHIP.md` §7):
// `own`/`borrow` typing at an `unsafe extern "C"` boundary.
#[test]
fn ffi_ownership_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("ffi_ownership.em"), "owned copy\nworld\n");
}

// Plan 93's resource handle & lifetime model (`crates/emerald-rt`'s
// handle registry): two real bumps of the same counter handle (`1`,
// `2`), a use-after-close raising a real `NativeError` caught via
// `rescue` (its message names the closed handle), and a redundant
// second `.close()` proving double-close is a genuine no-op rather
// than merely untested.
#[test]
fn resource_handle_lifetime_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("resource_handle_lifetime_proof.em"),
    "1\n2\nuse of closed counter handle\nstill running\n"
  );
}

// Plan 118 (JSON): Json.parse/JsonValue.get/.to_s against a real,
// nested document — the Ok path (a found key, a missing key, an
// array's own .count, and a full to_s round-trip) and a second,
// negative Err proof (invalid JSON reaching Err with a real,
// non-empty serde_json parser error, never a crash or empty string).
#[test]
fn json_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("json_demo.em"),
    "Ada\nmissing\n2\n{\"name\":\"Ada\",\"age\":36.0,\"active\":true,\"tags\":[\"math\",\"cs\"]}\nkey must be a string at line 1 column 2\n"
  );
}

// Plan 168 (Structured Logging): three JSON lines on stderr — a
// plain event, an event with two structured fields, a warning — and
// the fourth, DEBUG-level line never appears at all, the real proof
// that Log.configure("info", ...)'s level floor is doing its job.
#[test]
fn structured_logging_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run_stderr("structured_logging_proof.em"),
    "{\"level\":\"INFO\",\"message\":\"service starting\"}\n{\"level\":\"INFO\",\"message\":\"user signed in\",\"fields\":{\"user_id\":\"42\",\"plan\":\"pro\"}}\n{\"level\":\"WARN\",\"message\":\"cache miss\"}\n"
  );
}

// `derive Serializable` (this session's `derive-serializable` leaf):
// a real class's synthesized `to_json_value` round-tripped through
// `.to_s`. Field order is alphabetical (`active`, `age`, `gpa`,
// `name`) — the same determinism convention `derive Comparable`'s
// own field-name sort already established, reused here rather than
// declaration order. `age` (`Int64`) widens to `36.0` through the
// new `.to_f` conversion this leaf adds; `gpa` (already `Float64`)
// does not.
#[test]
fn derive_serializable_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("derive_serializable.em"),
    "{\"active\":true,\"age\":36.0,\"gpa\":3.9,\"name\":\"Ada\"}\n"
  );
}

// Plan 123 (Base64 & Hex Encoding, revised `String`-only scope — see
// `examples/base64_hex_encoding.em`'s own header comment): RFC 4648's
// own worked example ("hello world"), round-tripped through standard
// base64, url-safe base64 (unpadded by default), hex, and a real
// negative proof (malformed base64 reaching Err with a real,
// non-empty library error message).
#[test]
fn base64_hex_encoding_em_prints_expected_sequence() {
  let output = compile_and_run("base64_hex_encoding.em");
  let mut lines = output.lines();
  assert_eq!(lines.next(), Some("aGVsbG8gd29ybGQ="));
  assert_eq!(lines.next(), Some("hello world"));
  assert_eq!(lines.next(), Some("aGVsbG8gd29ybGQ"));
  assert_eq!(lines.next(), Some("68656c6c6f20776f726c64"));
  assert_eq!(lines.next(), Some("hello world"));
  let last = lines.next().expect("expected a fifth, negative-proof line");
  assert!(!last.is_empty() && last != "unexpected ok", "{last:?}");
  assert_eq!(lines.next(), None);
}

// Plan 122 (Regular Expressions): `Regex.compile`/`.is_match`/`.find`/
// `.captures`/`.replace_all` against a date pattern, matching the
// original plan's own predicted values exactly (adapted only for two
// real, disclosed syntax corrections — see `examples/regex_dates.em`'s
// own header comment).
#[test]
fn regex_dates_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("regex_dates.em"),
    "true\n2026-09-21\n2026\n21/09/2026 and 08/01/2026\n"
  );
}
