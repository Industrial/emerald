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

// Plan 182's own sibling to `compile_and_run` above: compiles `example`
// exactly once, then runs the resulting binary with the real, caller-
// supplied `args` as its own real `argv` — the only way to exercise
// `CliParser.parse(ARGV, ARGC)` against more than one fixed command
// line, since `compile_and_run` always invokes the compiled binary
// with zero extra arguments.
fn compile_and_run_with_args(example: &str, args: &[&str]) -> String {
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
    .args(args)
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

// Plan 194's leaf 1: a real end-to-end proof that `ChainCallExpr`
// do-block chaining (`recv.method do ... end.method do ... end`, no
// parens) already works with typed block params — plan 192's own
// "real grammar bug" finding was a misdiagnosis of unrelated,
// pre-existing, already-documented untyped-block-param syntax (see
// `examples/chain_call_do_block.em`'s own header comment and
// `history/2026-09-22T224000Z-plan-194-iterable-iterator-protocol.md`).
// `[1..6].select(even).map(*10)` = `[2,4,6] -> [20,40,60]`, printed as
// a count then one value per line (`Array[T]` has no `.to_s`/`.join`).
#[test]
fn chain_call_do_block_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("chain_call_do_block.em"), "3\n20\n40\n60\n");
}

// Plan 194's leaf 2: a real, end-to-end `Reader`/`Writer` interface
// pair (Go's `io.Reader`/`io.Writer` as named precedent), built
// entirely with the existing plain-interface (plan 41) and single-
// type-parameter generic-function (plan 41/88/89) mechanisms — see
// `examples/reader_writer_copy.em`'s own header comment for the five
// real, disclosed adaptations this session found necessary (no
// `UInt8`/`Bytes` type, no plain interface-typed parameter, no
// multiple type parameters, no direct method/index access on an
// instance variable, generic-function default parameters not honored).
// `copy(src, dst, 2)` on "Hello" (`[72,101,108,108,111]`) forces three
// real `read_chunk`/`write_chunk` round trips (2+2+1 bytes); a second
// call with a one-chunk-covers-everything size proves the same `copy`
// works for both shapes.
#[test]
fn reader_writer_copy_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("reader_writer_copy.em"),
    "5\n72\n101\n108\n108\n111\n5\n"
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
// array's own .count, and a full to_s round-trip) and two negative
// `Err` proofs (a real syntax error and a real truncation, each
// landing on the correct `JsonError` variant, never a crash or empty
// string). Plan 195 (Typed Domain Errors) retrofit: `Json.parse` now
// returns `Result[JsonValue, JsonError]`, not `Result[JsonValue,
// String]` — see `examples/json_demo.em`'s own updated header comment
// for the disclosed breaking change.
#[test]
fn json_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("json_demo.em"),
    "Ada\nmissing\n2\n{\"name\":\"Ada\",\"age\":36.0,\"active\":true,\"tags\":[\"math\",\"cs\"]}\ntrailing comma at line 1 column 8\nunexpected end of input\n"
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
// own header comment), plus a real negative `Err` proof. Plan 195
// (Typed Domain Errors) retrofit: `Regex.compile` now returns
// `Result[Regex, RegexError]`, not `Result[Regex, String]` — see
// `examples/regex_dates.em`'s own updated header comment for the
// disclosed breaking change.
#[test]
fn regex_dates_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("regex_dates.em"),
    "true\n2026-09-21\n2026\n21/09/2026 and 08/01/2026\nregex parse error:\n    (unclosed\n    ^\nerror: unclosed group\n"
  );
}

// Plan 146 (Environment Variables): `Env.set`/`.get`/`.keys_count`/
// `.remove` round-tripping a real process environment variable — not
// an in-process shadow table, a genuine OS-level mutation. Adapted
// only for two real, disclosed syntax corrections (see `examples/
// environment_variables_proof.em`'s own header comment).
#[test]
fn environment_variables_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("environment_variables_proof.em"),
    "hello\ntrue\nunset\n"
  );
}

// Plan 164 (Portable Math Functions): `Math.sqrt`/`.pow`/`.sin`/`.cos`
// plus the new `Float64#is_nan` prerequisite this plan's own text
// calls for. `sqrt(2.0)`/`sin`+`cos` identity verified against Rust's
// own std float math in `crates/emerald-rt/src/math.rs`'s own tests;
// this test pins the real, actually-observed `puts`/interpolation
// output rather than assuming the plan's own prose (which predicted
// a bare `1024`, not accounting for this compiler's own Float64
// printing convention).
#[test]
fn libm_proof_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("libm_proof.em"), "1.41421\n1024\ntrue\n1\n");
}

// Plan 162 (Human-Readable Duration/Time Formatting): `Duration.
// humanize`/`.parse_human`, `Timestamp.to_rfc3339`/`.parse_rfc3339`.
// `humanize(266400)` and the RFC 3339 rendering are the crate's own
// real output, pinned here (not hand-typed), matching this plan's
// own `crates/emerald-rt/src/humantime.rs` `#[test]`s.
#[test]
fn humantime_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("humantime_proof.em"),
    "3days 2h\n266400\nexpected number at 0\n2026-05-28T20:26:40Z\n1780000000\n"
  );
}

// Plan 152 (System Information): `System.*` read-only CPU/memory/
// disk/process introspection — every assertion is relative/derived
// (`> 0`, `>=`), true on any real machine this runs on, per this
// plan's own Decision log.
#[test]
fn system_info_proof_em_prints_expected_sequence() {
  let output = compile_and_run("system_info_proof.em");
  let mut lines = output.lines();
  assert_eq!(lines.next(), Some("true"), "cpu_count > 0");
  assert_eq!(lines.next(), Some("true"), "total_memory >= used_memory");
  assert_eq!(lines.next(), Some("true"), "used_memory >= 0");
  assert_eq!(lines.next(), Some("true"), "disk_names_count >= 0");
  assert_eq!(lines.next(), Some("true"), "process_ids_count > 0");
  let last = lines
    .next()
    .expect("expected a sixth line (process_name result)");
  assert!(
    last == "true" || last == "unknown",
    "unexpected process_name line: {last:?}"
  );
  assert_eq!(lines.next(), None);
}

// Plan 109 (Cryptographic Hashing): Sha256/Sha3_256/Blake3/Md5
// one-shot digests plus Sha256Hasher's incremental handle, each value
// a real, independently-checkable test vector (see this plan's own
// history doc for each one's primary source).
#[test]
fn crypto_hashing_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("crypto_hashing_proof.em"),
    "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9\n\
     3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532\n\
     af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262\n\
     5eb63bbbe01eeed093cb22bb8f5acdc3\n\
     b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9\n"
  );
}

// Plan 110 (Symmetric AEAD Encryption): a correctness-PROPERTY proof,
// not a fixed-ciphertext assertion — the auto-nonce path is
// intentionally non-deterministic run-to-run (this plan's own
// Decision log). What's checked: decrypting with the right key
// recovers the exact original plaintext, and decrypting with a
// different, unrelated key fails closed every time.
#[test]
fn crypto_aead_proof_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("crypto_aead_proof.em"), "true\ntrue\n");
}

// Plan 111 (Asymmetric Cryptography and Digital Signatures): another
// correctness-property proof — key generation is randomized by
// design, so there is no fixed signature/shared-secret to assert.
#[test]
fn crypto_asymmetric_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("crypto_asymmetric_proof.em"),
    "true\ntrue\ntrue\n"
  );
}

// Plan 184 (TOTP/HOTP Two-Factor Authentication): `Totp.new`,
// `Totp#generate_current`/`#check_current`/`#provisioning_uri` — see
// `totp_2fa.em`'s own header comment for the two real, disclosed
// deviations from this plan's own literally-quoted Concrete Proof
// (`Result[Totp, TotpError]` in place of a bare `Totp`; the plan's own
// literal secret doubled to clear `totp_rs`'s real 128-bit minimum).
// `code` (line 1) is genuinely time-dependent — checked for shape
// (six ASCII digits) here rather than asserted as a fixed literal,
// exactly as this plan's own Concrete Proof describes; `valid`/
// `invalid`/`uri` (lines 2-4) are fully deterministic given the fixed
// issuer/account/secret this example uses and are asserted exactly.
#[test]
fn totp_2fa_em_prints_expected_sequence() {
  let output = compile_and_run("totp_2fa.em");
  let mut lines = output.lines();
  let code = lines.next().expect("line 1: code");
  assert_eq!(code.len(), 6, "code should be six digits, got {code:?}");
  assert!(
    code.chars().all(|c| c.is_ascii_digit()),
    "code should be all ASCII digits, got {code:?}"
  );
  let rest: Vec<&str> = lines.collect();
  assert_eq!(
    rest,
    vec![
      "true",
      "false",
      "otpauth://totp/Emerald%20Corp:alice%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Emerald%20Corp",
    ]
  );
}

// Plan 117 (Constant-Time Comparison): `SecureCompare.eq` — the
// return-value correctness is what's checkable at this level (the
// constant-time property is `subtle`'s own, verified upstream).
#[test]
fn secure_compare_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("secure_compare_proof.em"),
    "true\nfalse\nfalse\n"
  );
}

// Plan 112 (Password Hashing): `Password.hash`'s own PHC-string
// output is salted and non-deterministic by design — hashing the same
// plaintext twice produces two different strings — so only round-trip
// correctness is checkable here, the same honest constraint plan
// 111's/113's own randomized-output proofs above already establish.
#[test]
fn password_hashing_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("password_hashing_proof.em"),
    "true\nfalse\ntrue\n"
  );
}

// Plan 113 (Cryptographically Secure Random Number Generation):
// `token`'s own value is randomized by design — only its length
// (`32` for 16 hex-encoded bytes) is checked.
#[test]
fn random_csprng_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("random_csprng_proof.em"),
    "32\ntrue\ntrue\n"
  );
}

// Plan 98 (URL Parsing): `plain.port` (`443`, no explicit port at
// all) is the real proof of `#port`'s scheme-default fallback.
#[test]
fn url_parsing_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("url_parsing.em"),
    "https\nexample.com\n443\n/search\nq=emerald\nresults\n443\n/api\nid=42\n"
  );
}

// Plan 96 (Raw TCP/UDP Sockets): a real, deterministic, single-process
// bind->connect->accept->read/write-both-ways->close proof — no actor
// spawn, no second process. Works because TCP's own three-way
// handshake completes as soon as `connect()` is called against a
// listener that has already `bind`+`listen`ed (the kernel queues the
// connection in the accept backlog regardless of whether the
// application has called `accept()` yet).
#[test]
fn raw_tcp_sockets_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("raw_tcp_sockets.em"), "ping\npong\n");
}

// Plan 97 (DNS Resolution): network-dependent, real DNS lookups against
// Cloudflare's own stable `one.one.one.one` vanity hostname — the
// second call routes the query through DNS-over-TLS to that same
// service, a real proof the DoT configuration path actually changes
// transport, not an inert flag. Per the plan's own Concrete Proof, the
// exact resolved address is not asserted verbatim (randomized by
// upstream DNS load-balancing/IPv4-vs-IPv6 ordering) — only that each
// call succeeds and returns a well-formed address, checked here via a
// real `Regex` match rather than an unchecked `puts` of the raw value.
#[test]
fn dns_resolution_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("dns_resolution.em"), "true\ntrue\n");
}

// Plan 100 (HTTP Client): network-dependent, real GET requests against
// `httpbin.org`'s own deterministic `/status/<code>` endpoint. All
// three lines go through the `Ok` arm — a completed 404/500 response
// is a successful transport outcome, not a `Result` failure, this
// plan's own central, deliberate divergence from `ureq`'s own default.
#[test]
fn http_client_proof_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("http_client_proof.em"), "200\n404\n500\n");
}

// Plan 99 (TLS): network-dependent, a real client handshake against
// IANA's own reserved, stable-by-design `example.com`. Per the plan's
// own Concrete Proof, the exact remainder of the 64-byte read is not
// asserted verbatim -- only that it begins with a well-formed HTTP
// status line, since `example.com`'s own response headers are not a
// contract this proof depends on beyond "the TLS handshake succeeded
// and an HTTP response started arriving." Real, disclosed correction
// found writing this example: `\r` is not a recognized escape in this
// lexer's string-literal grammar (`emerald-parser`'s own `decode_
// string_lit` admits only `\"`/`\n`, the same constraint plan 122's
// own update already found) -- the request uses bare `\n` line endings
// instead of `\r\n`, which `example.com`'s own edge server accepts.
#[test]
fn tls_client_em_prints_a_well_formed_status_line() {
  let output = compile_and_run("tls_client.em");
  assert!(
    output.starts_with("HTTP/1."),
    "expected a well-formed HTTP status line, got {output:?}"
  );
}

// Plan 115 (Key Derivation Functions): RFC 5869 Test Case 1's own
// published HKDF vector, and the `pbkdf2` crate's own published
// doctest vector — both quoted directly, not invented (see this
// plan's own Decision log). Real, disclosed correction found writing
// this example: the plan's own Concrete Proof passes a literal
// `"salt"` (raw ASCII) to `Kdf.pbkdf2`, contradicting its own Decision
// log's stated hex-in/hex-out convention for both functions — this
// example passes `"73616c74"` (hex for `"salt"`) instead, matching
// the Decision log's own stated design and still producing the exact
// same cited expected output.
#[test]
fn key_derivation_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("key_derivation_proof.em"),
    "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865\n669cfe52482116fda1aa2cbe409b2f56c8e45637\n"
  );
}

// Plan 119 (TOML): a nested TOML table round-trips through `toml`'s
// own parser into the same `JsonValue` enum plan 118 already defines
// — `.get` works identically regardless of which parser produced the
// tree. Second, real negative proof: invalid TOML reaches `Err` with
// the `toml` crate's own real parse-error text, never a crash. Real,
// disclosed finding: unlike `serde_json::Error`'s single-line
// `Display`, `toml` 1.x's own parse-error text is a real, multi-line,
// pretty-printed diagnostic (a source snippet plus a `|`/`^` caret
// line) — this assertion checks the first line and that real,
// non-empty diagnostic text follows, not an exact single-line shape.
// Plan 121 (CSV): the three classic CSV edge cases — a quoted field
// containing the delimiter, a quoted field containing an embedded
// newline, and doubled-quote escaping — all handled by the `csv`
// crate's own real, standard-compliant parser, none hand-rolled here.
#[test]
fn csv_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("csv_demo.em"),
    "role\nengineer, lead\nwrote \"the\nfirst\" program\nengineer, lead\n"
  );
}

#[test]
fn toml_demo_em_prints_expected_sequence() {
  let output = compile_and_run("toml_demo.em");
  let mut lines = output.lines();
  assert_eq!(lines.next(), Some("demo"));
  let rest: String = lines.collect::<Vec<_>>().join("\n");
  assert!(
    !rest.trim().is_empty(),
    "expected real, non-empty TOML parse-error diagnostic text after the first line, got {rest:?}"
  );
}

// Plan 120 (YAML): `Yaml.parse`/`JsonValue.to_yaml` against a real
// block-style document — the Ok path (a found string key, a found
// array's own .count, and a full .to_yaml round-trip beginning with
// `saphyr::YamlEmitter`'s own `---` document-start marker) and two
// negative `Err` proofs (a real YAML indentation error landing on
// `YamlError::Syntax`, and an empty document stream landing on
// `YamlError::EmptyDocument`), both typed per plan 195's convention
// from this plan's own first EXECUTE (unlike `Toml.parse` above,
// which predates it).
#[test]
fn yaml_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("yaml_demo.em"),
    "Ada\n2\n---\nname: Ada\nlanguages:\n  - math\n  - cs\nactive: true\nsimple key expected at byte 14 line 4 column 1\nempty document\n"
  );
}

// Plan 125 (Binary Serialization: bincode/msgpack): `Bincode.encode`/
// `.decode`, `MessagePack.encode`/`.decode`, both round-tripping the
// same `JsonValue` tree — see `examples/binary_serialization.em`'s own
// header comment for the full account of the real, disclosed
// adaptations this required (no `==` on `JsonValue`, `puts` rejects
// `Boolean`, and — the one gap found only by actually running this
// example, not by reading either crate's docs — a new `Bytes#length`
// method this plan itself had to add, since no existing `Bytes`
// method exposed a byte count before it). Expected sequence: `true`
// (the `Bincode` round trip, compared via `.to_s` rather than `==`),
// `51` (the real, measured MessagePack wire size for this exact
// value), `true` (the `MessagePack` round trip), `other error` (a
// real `String`'s own bytes are not a valid `Bincode` encoding of
// anything — a genuine `BincodeError::Other`, not `UnexpectedEnd`,
// confirmed by actually running this, not assumed), `56` (the real,
// measured `Bincode` wire size for the same value — larger than
// MessagePack's here, the reverse of this plan's own text's general
// expectation, because `bincode.rs`'s own `BincodeValue` workaround
// reintroduces a per-node tag `bincode`'s native encoding otherwise
// omits; see that module's own doc comment for the full account).
#[test]
fn binary_serialization_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("binary_serialization.em"),
    "true\n51\ntrue\nother error\n56\n"
  );
}

// Plan 189 (CBOR Binary Format): `Cbor.encode`/`.decode` (`ciborium`),
// reusing plan 118's own `JsonValue` as the shared dynamic-value
// representation — a value built via `Json.parse`, encoded to CBOR,
// decoded back, and compared equal to the original via `.to_s` (see
// `examples/cbor_roundtrip.em`'s own header comment for the full
// account of the disclosed adaptations this required). Expected
// sequence: `true` (a non-empty CBOR byte buffer was produced), the
// decoded value's own `.to_s` text (JSON numbers always widen to
// `Float64` — `1`/`2`/`3` print back as `1.0`/`2.0`/`3.0`), then
// `true` (the round trip matches the original exactly).
#[test]
fn cbor_roundtrip_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("cbor_roundtrip.em"),
    "true\n{\"name\":\"Ada\",\"scores\":[1.0,2.0,3.0],\"active\":true}\ntrue\n"
  );
}

// Plan 124 (XML): `Xml.parse`/`.reader_from_string` proving tree mode
// and streaming mode agree on the same document's root element —
// `library` printed twice (tree mode's own root tag, then the
// streaming reader's very first real event), with each `<book>`'s
// `title` attribute read in between via `Hash[String,String].each`/
// `Pair[String,String]#key`/`#value` (plan 124's own real, disclosed
// finding: `Hash[K,V]` has no `.get` method at all — plan 25's history
// doc already establishes `[]`/`Expr::Index` as the only access path —
// and `[]` indexing a `Hash[String,V]` by a `String` key is itself
// unimplemented in codegen, a real crash found and disclosed by
// running exactly this. `.each`'s own inline block, in turn, only
// compiles as a bare top-level statement, never nested inside `if`/
// `while`/`match` — a second real, disclosed limitation, broader than
// plan 121's own already-disclosed "nested inside `Ok(v)`" version of
// the same gap — so both attribute maps are hoisted to top-level
// `var`s and read after every enclosing `match` has closed).
#[test]
fn xml_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("xml_demo.em"),
    "library\nDune\n1984\nlibrary\n"
  );
}

// Plan 153 (Character Set / Encoding Conversion): `Encoding.encode`
// transcodes "café" to real windows-1252 bytes (`63 61 66 e9` — the
// trailing `e9` is not valid standalone UTF-8), `Encoding.decode`
// transcodes it back to "café" exactly. `Encoding.decode_strict`
// proves the asymmetry: asking whether those same bytes are already
// valid UTF-8 (label `"utf-8"`) is `None` (`e9` alone is malformed
// UTF-8), while asking under the correct `"windows-1252"` label
// round-trips successfully — `??` (`Expr::Coalesce`) substitutes the
// fallback text only on the genuine `None` case.
#[test]
fn charset_encoding_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("charset_encoding_proof.em"),
    "café\nnot valid utf-8\ncafé\n"
  );
}

// Plan 154 (Unicode Normalization & Segmentation): `composed` is
// "café" with a single precomposed U+00E9 (4 codepoints, 5 UTF-8
// bytes); `decomposed` is "cafe" + a literal COMBINING ACUTE ACCENT
// U+0301 (5 codepoints, 6 UTF-8 bytes) — both render identically and
// both have `.grapheme_count == 4` (the combining accent attaches to
// the preceding "e" as one extended grapheme cluster, UAX #29). Only
// `decomposed.nfc`'s canonical composition collapses `.length` to `5`,
// matching `composed` byte-for-byte.
#[test]
fn unicode_normalization_segmentation_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("unicode_normalization_segmentation_proof.em"),
    "5\n6\n4\n5\n4\n4\n5\n"
  );
}

// Plan 196 (Class-Level Static Methods): `Point.origin()` and `Point.
// midpoint(a, b)` are both real `static fn` methods, dispatched with no
// implicit `self` receiver — `Point.origin` constructs a `Point`
// internally via the already-existing `.new` dispatch; `Point.midpoint`
// reads its two `Point` arguments' fields via ordinary (non-static)
// `read x: Int64`/`read y: Int64` accessor calls (plan 33's sugar).
// Neither static method touches `@x`/`@y`/`self` directly.
#[test]
fn static_methods_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("static_methods.em"), "0\n0\n5\n7\n");
}

// Plan 193 (`Set[T]`, `Deque[T]`, `PriorityQueue[T]`) — inception-3
// §4.3's own "missing collection primitives" gap. All three route
// through `emerald-rt`'s plan-93 resource-handle registry, backed
// directly by Rust's own `std::collections` (`HashSet`, `VecDeque`,
// `BinaryHeap`) — see this plan's own Decision log
// (`history/2026-09-22T223900Z-plan-193-set-deque-priority-queue.md`)
// for the full account, including the two real, disclosed grammar
// corrections found while writing this example (explicit `.new()`,
// `puts` needing string interpolation for a `Boolean`).
#[test]
fn set_deque_priority_queue_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("set_deque_priority_queue.em"),
    "2\ntrue\nz\n9\n"
  );
}

// Plan 163 (Arbitrary-Precision Integers & Decimals): `BigInt`
// (`num-bigint`, a `crate::handle`-registry opaque `Int64` handle) and
// `Decimal` (`rust_decimal`, a packed two-`Int64`-field class) — the
// worked proof that `Int64` (25! overflows it) and `Float64` (`0.1 +
// 0.2 != 0.3` in IEEE 754 double precision) both genuinely need a
// replacement, for two different reasons, with two different
// representations. Plan 195 (Typed Domain Errors) applied at this
// plan's own EXECUTE time — `BigInt.from_s`/`Decimal.from_s`/`.div`
// return `Result[T, BigIntError]`/`Result[T, DecimalError]`, never a
// bare `Result[T, String]` — see `examples/bignum_decimal_proof.em`'s
// own header comment.
#[test]
fn bignum_decimal_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("bignum_decimal_proof.em"),
    "2432902008176640000\n15511210043330985984000000\ninvalid digit string for BigInt: \"not a number\"\n42\n440\n0.3\n0.1\n0.02\ndivision by zero\n0.3\nInvalid decimal: unknown character\n"
  );
}

// Plan 160 (Date/Time & Timezones): `DateTime` (a UTC instant, packed
// two-`Int64`-field class) and `ZonedDateTime` (the same two fields
// plus a `tz_name: String` IANA identifier), backed by `jiff` — RFC
// 3339 parsing/formatting, a real unrecognized-timezone `Err`, and a
// DST-aware `.plus_days` crossing 2026's real US spring-forward
// transition (2026-03-08, `America/New_York`). Plan 195 (Typed Domain
// Errors) applied at this plan's own EXECUTE time — `.parse_rfc3339`/
// `.in_tz` return `Result[T, DateTimeError]`, never a bare `Result[T,
// String]` — see `examples/datetime_timezones_proof.em`'s own header
// comment.
#[test]
fn datetime_timezones_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("datetime_timezones_proof.em"),
    "2026-06-15T12:00:00Z\n2026-06-15T12:00:00Z\nfailed to find time zone `Nonexistent/Zone` in time zone database\n-3600\n"
  );
}

// Plan 130 (Gzip/Deflate/Zlib Compression): `Gzip`/`Deflate`/`Zlib`
// `.compress`/`.decompress` round-tripping a real, repetitive input
// through all three of gzip's/zlib's/raw-deflate's actual wire
// formats — see `examples/gzip_roundtrip.em`'s own header comment for
// the two real, disclosed adaptations from the plan's own literal
// Concrete Proof text (no string-literal method receivers, no chained
// call off a non-block-attached `MethodCall` result, and comparing
// each `Bytes` value's own `.to_hex()` `String` rather than a
// `Bytes.length`/`Bytes == Bytes` surface that does not exist).
#[test]
fn gzip_roundtrip_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("gzip_roundtrip.em"),
    "true\ntrue\ntrue\ntrue\n"
  );
}

// Plan 131 (Zstandard Compression): `Zstd.compress`/`.decompress`
// round-tripping a real, repetitive input at both a low (3, the
// library default) and a real, higher (19) compression level — see
// `examples/zstd_roundtrip.em`'s own header comment for the real,
// disclosed adaptations from the plan's own literal Concrete Proof
// text, reused verbatim from plan 130's own `gzip_roundtrip.em`.
#[test]
fn zstd_roundtrip_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("zstd_roundtrip.em"),
    "true\ntrue\ntrue\ntrue\n"
  );
}

// Plan 144 (Extended Filesystem Operations): `Dir.entries`/`.entries_
// count`/`.walk`/`.walk_count` (non-recursive `std::fs::read_dir` +
// recursive, symlink-cycle-safe `walkdir` traversal) and `Path.exists`/
// `.is_file`/`.is_dir`/`.is_symlink`/`.metadata`/`.unix_mode`/`.set_
// unix_mode`/`.symlink`/`.read_link` plus `FileMetadata`'s own five
// accessors. Three real, disclosed findings from actually running this
// plan's own worked example — see `examples/extended_filesystem_
// proof.em`'s own header comment for the full account: (1) the plan's
// own Concrete Proof writes into `plan144_demo/nested/b.txt` before
// anything ever creates `plan144_demo/nested/` (`File.write` never
// creates missing parent directories, and no plan has ever given
// Emerald source a directory-creation primitive) — this test creates
// that fixture tree itself, one level up from this plan's own
// `emerald-rt` Rust-level `tempfile::tempdir()` fixtures; (2) `Path.
// metadata`/`.read_link` return `Result[T, PathError]` (plan 195),
// superseding the plan's own original, now-dead `T?`/`||=` nullable
// syntax (removed by plan 73); (3) `puts` does not accept a `Boolean`
// argument (`emerald-sema`'s own real `Int64`/`Float64`/`String`-only
// intrinsic), so every `Boolean` this example prints goes through an
// `if ... do puts "true" else puts "false" end`, `examples/control_
// flow.em`'s own precedent.
#[test]
fn extended_filesystem_proof_em_prints_expected_sequence() {
  let fixture_root = std::env::current_dir()
    .expect("current dir")
    .join("plan144_demo");
  std::fs::create_dir_all(fixture_root.join("nested")).expect("create plan144_demo fixture tree");

  assert_eq!(
    compile_and_run("extended_filesystem_proof.em"),
    "2\n3\n5\nfalse\nplan144_demo/a.txt\ntrue\nfalse\ntrue\ntrue\ntrue\ntrue\nnot found\n"
  );

  std::fs::remove_dir_all(&fixture_root).ok();
}

// Plan 150 (Path Globbing): `Glob.glob`/`.glob_count`, wrapping the
// `glob` crate for real shell-style `*`/`?`/`[...]`/`**` pattern
// matching against the real filesystem. Two real, disclosed findings
// from actually running this plan's own worked example — see
// `examples/path_globbing_proof.em`'s own header comment for the full
// account: (1) exactly `extended_filesystem_proof.em`'s own finding
// (1) for plan 144 — `File.write` never creates missing parent
// directories, so this test creates the `plan150_demo/nested/`
// fixture tree itself before running the example; (2) the plan's own
// literal method name, `Glob.match`/`.match_count`, is unparseable
// (`match` is grammar-reserved) — renamed to `Glob.glob`/`.glob_count`
// instead, mirroring the `glob` crate's own `glob::glob` function.
#[test]
fn path_globbing_proof_em_prints_expected_sequence() {
  let fixture_root = std::env::current_dir()
    .expect("current dir")
    .join("plan150_demo");
  std::fs::create_dir_all(fixture_root.join("nested")).expect("create plan150_demo fixture tree");

  assert_eq!(
    compile_and_run("path_globbing_proof.em"),
    "2\nplan150_demo/a.txt\nplan150_demo/b.txt\n1\nplan150_demo/nested/d.rs\n"
  );

  std::fs::remove_dir_all(&fixture_root).ok();
}

// Plan 132 (Tar Archives): `Tar.create`/`.extract`, `TarReader.open`/
// `.next_entry`/`.entry_size`/`.read_entry_data`/`.close`, wrapping
// the pure-Rust `tar` crate — a real create-then-list-then-extract
// round trip against two real files, `.next_entry()` genuinely
// terminating past the last entry, and `Tar.extract` recreating the
// exact original bytes — see `examples/tar_roundtrip.em`'s own header
// comment for the four real, disclosed adaptations from the plan's
// own literal Concrete Proof text.
#[test]
fn tar_roundtrip_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("tar_roundtrip.em"),
    "tar_demo_a.txt\n68656c6c6f\ntar_demo_b.txt\n776f726c642c2061206c6f6e676572207365636f6e642066696c65\nend\ntrue\n"
  );
}

// Plan 133 (Zip Archives): `Zip.create`/`.extract`, `ZipReader.open`/
// `.entry_count`/`.entry_name`/`.entry_size`/`.read_entry_data`/
// `.close`, wrapping the pure-Rust-only `zip` crate (`deflate-flate2-
// zlib-rs` feature) — a real create-then-list-then-extract round trip
// against two real files, `entry_count()` proving the central
// directory was written and parsed correctly, and `Zip.extract`
// recreating the exact original bytes — see `examples/zip_roundtrip.
// em`'s own header comment for the real, disclosed adaptations from
// the plan's own literal Concrete Proof text.
#[test]
fn zip_roundtrip_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("zip_roundtrip.em"),
"2\nzip_demo_a.txt\n68656c6c6f\nzip_demo_b.txt\n776f726c642c2061206c6f6e676572207365636f6e642066696c65207468617420636f6d707265737365732077656c6c2077656c6c2077656c6c2077656c6c\ntrue\n"
  );
}

// Plan 145 (Process Spawning & Control): `Process.run(cmd, args, argc,
// stdin_data): ProcessResult`, wrapping plain `std::process::Command`
// (no third-party crate) — a zero/nonzero exit code travels back as a
// plain field, never an error; only a genuine spawn failure (`ENOENT`)
// raises a catchable `NativeError`. See `examples/process_spawning_
// proof.em`'s own header comment for two real, disclosed deviations
// from this plan's own literal Concrete Proof text (`puts`'s real
// `Boolean`-rejecting signature; `rescue`'s real class-naming
// requirement).
#[test]
fn process_spawning_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("process_spawning_proof.em"),
    "hello from emerald\n\n0\ntrue\n1\nfalse\nspawn failed\n"
  );
}

// Plan 147 (Temporary Files & Directories): `Tempfile.create`/`.path`/
// `.close`, `Tempdir.create`/`.path`/`.close`, wrapping the pure-Rust
// `tempfile` crate — `Tempfile`'s own path really exists on disk while
// open (proven by composing plan 45's `File.write`/`.read` and plan
// 144's `Path.exists` against it) and is really gone after `.close()`;
// `Tempdir`'s own nested contents are all gone too, not just its
// top-level entry. See `examples/temp_files_proof.em`'s own header
// comment for the one real, disclosed deviation from this plan's own
// literal Concrete Proof text.
#[test]
fn temp_files_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("temp_files_proof.em"),
    "temporary contents\ntrue\nfalse\ntrue\nfalse\n"
  );
}

// Plan 137 (SQLite): `Sqlite.open_memory`/`.execute_direct`/`.prepare`/
// `.bind_string`/`.bind_int64`/`.execute`/`.query`/`.step`/
// `.column_int64`/`.column_string`/`.close`, wrapping `rusqlite`
// (`features = ["bundled"]`, this batch's one deliberate C-binding
// exception) — an in-memory database created, a row inserted through
// a bound parameter, and read back through a prepared, bound
// `SELECT`, matching this plan's own Concrete Proof verbatim (see
// `examples/sqlite_todo.em`).
#[test]
fn sqlite_todo_em_prints_expected_sequence() {
  assert_eq!(compile_and_run("sqlite_todo.em"), "1\nwrite plan 137\n");
}

// Plan 182 (Structured CLI Flag Parsing): `CliParser.new`/`.flag`/
// `.option`/`.positional`/`.parse`/`.close`, `CliParseResult#flag`/
// `#value`/`#positional_value`/`#help_requested`/`#help_text`/
// `#error_message`/`#close`, wrapping `clap`'s non-derive builder API
// and consuming plan 45's own `ARGV`/`ARGC` directly. One compiled
// binary (`examples/cli_flag_parsing.em`), invoked three separate
// times with three different real `argv`s via `compile_and_run_with_
// args`, exercising the three distinct paths this plan's own Concrete
// Proof names — a full successful parse, a missing-required-option
// error, and `--help`. The error/help paths assert `.contains(...)`
// rather than an exact string, since clap's own formatted error/help
// text is not this plan's own contract to pin byte-for-byte; the
// success path is fully deterministic and asserted exactly, matching
// every other example test in this file. See `examples/cli_flag_
// parsing.em`'s own header comment for the real, disclosed deviations
// from this plan's own literal Concrete Proof text — plus the one
// noted just below instead, `--help`'s own default missing version
// banner, worked around in `cli.rs`'s own `cliparser_new` via an
// explicit `.help_template` override so the proof's "1.0.0"
// requirement still holds.
#[test]
fn cli_flag_parsing_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run_with_args(
      "cli_flag_parsing.em",
      &["--name", "Ada", "--verbose", "extra"]
    ),
    "Hello, Ada\n(verbose mode on)\nextra\n"
  );

  let missing_required = compile_and_run_with_args("cli_flag_parsing.em", &[]);
  assert!(
    missing_required.contains("--name"),
    "missing required --name should be named in the error message, got: {missing_required:?}"
  );

  // A real, disclosed correction found only by actually running this:
  // clap 4's own DEFAULT help template (`clap_builder::output::
  // help_template::DEFAULT_TEMPLATE`) omits the `{name} {version}`
  // banner line entirely — a real v4 behavior change from v2/v3's own
  // template, verified directly against `clap_builder`'s own source
  // in this workspace's exact pinned version. `cli.rs`'s own
  // `cliparser_new` works around this with an explicit `.help_
  // template` override reintroducing that banner line, so this
  // plan's own Concrete Proof requirement (the literal string
  // "1.0.0" present in `--help`'s own output, not just `--version`'s)
  // does hold, asserted below.
  let help = compile_and_run_with_args("cli_flag_parsing.em", &["--help"]);
  assert!(
    help.contains("greet"),
    "help text should contain the program name, got: {help:?}"
  );
  assert!(
    help.contains("1.0.0"),
    "help text should contain the version, got: {help:?}"
  );
  assert!(
    help.contains("verbose"),
    "help text should contain the verbose flag, got: {help:?}"
  );
  assert!(
    help.contains("name"),
    "help text should contain the name option, got: {help:?}"
  );
  assert!(
    help.contains("suffix"),
    "help text should contain the suffix positional, got: {help:?}"
  );
}

// Plan 183 (Layered Configuration Loading): `ConfigBuilder`/
// `ConfigValue`, wrapping the `config` crate — one compiled binary
// (`examples/layered_config.em`), run from a fixed temporary
// directory containing a real `app.toml` fixture, with a real
// `APP__SERVER__PORT` environment variable set and a real `--port
// 7070` argv, so this plan's own precedence order (CLI flag >
// environment variable > config file) is demonstrated by genuine
// layered inputs, not asserted in prose alone — the identical
// standard `compile_and_run_with_args` already sets for plan 182's
// own `cli_flag_parsing.em`, extended here with a real fixture file
// and a real environment variable neither of that function's own
// simpler siblings supports.
#[test]
fn layered_config_em_prints_expected_sequence() {
  let source = workspace_root().join("examples").join("layered_config.em");
  let output = std::env::temp_dir().join(format!(
    "emerald_example_layered_config_em_{}",
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(
    status.success(),
    "emerald-cli should succeed on layered_config.em"
  );

  let fixture_dir = std::env::temp_dir().join(format!(
    "emerald_example_layered_config_fixture_{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&fixture_dir).expect("failed to create fixture dir");
  std::fs::write(
    fixture_dir.join("app.toml"),
    "[server]\nport = 8080\nhost = \"0.0.0.0\"\n",
  )
  .expect("failed to write app.toml fixture");

  let run = Command::new(&output)
    .args(["--port", "7070"])
    .env("APP__SERVER__PORT", "9090")
    .current_dir(&fixture_dir)
    .output()
    .expect("failed to run compiled binary");
  assert!(
    run.status.success(),
    "layered_config.em's compiled binary should exit 0"
  );

  std::fs::remove_file(&output).ok();
  std::fs::remove_dir_all(&fixture_dir).ok();

  assert_eq!(
    String::from_utf8_lossy(&run.stdout).into_owned(),
    "7070\n0.0.0.0\n"
  );
}

// Plan 191 (Progress Bars & Terminal Formatting): `ProgressBar.new`/
// `.increment`/`.finish` drive a real 100-step bar to completion (no
// visible assertion on its own redraw output — that lands on stderr,
// per `indicatif`'s own default draw target, and is silenced entirely
// under this harness's own piped, non-terminal stderr regardless —
// see `progress.rs`'s own module doc); `Console.styled("done",
// "green")` and `Console.is_terminal` are the byte-for-byte-asserted
// half of this proof. Since this test harness's own `Command::output`
// captures stdout via a real OS pipe (never a pty), `Console.
// is_terminal` is genuinely `false` here — not a mocked value — so
// `.styled`'s own real auto-detection (`console::colors_enabled()`)
// returns the plain, escape-code-free "done" text, per this plan's
// own Decision log naming this gap explicitly: the colored/escape-
// code path is real, shipped code, just not the path this exact
// assertion exercises.
#[test]
fn progress_and_formatting_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("progress_and_formatting_proof.em"),
    "done\nfalse\n"
  );
}

// Plan 180's own sibling to `compile_and_run` above: a `test`/
// `property`/`benchmark` block is never reachable via the ordinary
// `emerald <file>` compile path this table's other helpers all use
// (`Item::Property`'s own doc comment) — this runs `emerald property
// <file>` directly instead (`test_runner::run`'s own real subcommand),
// returning `(stdout, exit code)` rather than asserting `status.
// success()` the way every other example in this table does, since
// `property_shrink_proof.em` below deliberately contains one failing
// property (proving shrinking actually ran) and is expected to exit
// non-zero.
fn compile_and_run_property_subcommand(example: &str) -> (String, i32) {
  let source = workspace_root().join("examples").join(example);
  let output = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("property")
    .arg(&source)
    .output()
    .expect("failed to run `emerald property`");
  (
    String::from_utf8_lossy(&output.stdout).into_owned(),
    output.status.code().unwrap_or(-1),
  )
}

// Plan 180 (Property-Based Testing Generators) — `property "..."
// (a: Int64, b: Int64) do ... end`'s real, generator-driven,
// shrinking-on-failure Concrete Proof: `addition is commutative` holds
// for every one of `proptest`'s own default 256 generated `(a, b)`
// pairs; `subtraction finds a real bug` genuinely fails whenever
// `a != b`, and `proptest`'s real `ValueTree::simplify()` shrinks the
// counterexample toward zero. Two real, disclosed differences from
// the plan's own illustrative Concrete Proof text, both found by
// actually running this proof repeatedly rather than trusting it after
// one observed run:
//
// 1. The exact shrunk pair is `a=0` and `b` equal to `1` OR `-1` —
//    both are genuinely minimal (`proptest`'s own `TestRunner` seeds
//    its RNG afresh, unseeded, on every run, so which of the two
//    equally-small counterexamples it lands on varies run to run) —
//    matching the plan's own Decision log caveat ("`a=0, b=1` (or a
//    value `proptest` itself treats as equally minimal under its own
//    shrink ordering)") precisely, so this only asserts `a=0` plus
//    `b`'s own magnitude, never a hardcoded sign.
// 2. The FAIL line's own trailing message is the failing
//    `assert_eq`'s real, verified `AssertionError#message` — its
//    *source location* (`desugar_assert_eq`'s own pre-existing,
//    already-tested codegen shape — see `test_subcommand.rs`'s own
//    `examples_property_test_em_matches_the_documented_transcript`),
//    not "expected 1 but got -1" (those values are eagerly `puts` on
//    their own separate lines instead, the same pre-existing
//    behavior, not new noise this plan introduces).
#[test]
fn property_shrink_proof_em_shrinks_a_real_failure_to_a_minimal_counterexample() {
  let (stdout, code) = compile_and_run_property_subcommand("property_shrink_proof.em");
  assert_eq!(
    code, 1,
    "one property is deliberately broken (proving shrinking ran); full stdout:\n{stdout}"
  );
  assert!(
    stdout.contains("PASS: addition is commutative (256 cases)"),
    "missing the passing property's own PASS line; full stdout:\n{stdout}"
  );
  let prefix = "FAIL: subtraction finds a real bug: minimal input a=0, b=";
  let after_prefix = stdout
    .split(prefix)
    .nth(1)
    .unwrap_or_else(|| panic!("missing the shrunk-to-minimal FAIL line; full stdout:\n{stdout}"));
  let b_str = after_prefix
    .split(|c: char| c == ',' || c == ':')
    .next()
    .unwrap_or("");
  let b: i64 = b_str
    .parse()
    .unwrap_or_else(|e| panic!("FAIL line's own `b=` value `{b_str}` didn't parse as i64: {e}"));
  assert!(
    b == 1 || b == -1,
    "shrinking should have converged to b=1 or b=-1, found b={b}; full stdout:\n{stdout}"
  );
}

// Plan 102 (WebSocket): `Http.serve` (plan 101) never returns, so
// unlike every other example in this table `websocket_proof.em`'s own
// compiled process never exits on its own — this test spawns it in
// the BACKGROUND, drives a real `tungstenite::connect` client directly
// from Rust against it (the reverse role from every other proof in
// this file, since the *Emerald* side here is the server, per this
// plan's own Concrete Proof), and kills the process once the exchange
// completes, rather than using `compile_and_run`'s own wait-for-exit-
// then-assert-stdout shape at all — this file's own stdout is never
// asserted against.
#[test]
fn websocket_proof_em_round_trips_a_real_echo_over_a_real_socket() {
  let source = workspace_root().join("examples").join("websocket_proof.em");
  let output = std::env::temp_dir().join(format!(
    "emerald_example_websocket_proof_em_{}",
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(
    status.success(),
    "emerald-cli should succeed on websocket_proof.em"
  );

  let mut child = Command::new(&output)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("failed to spawn compiled websocket_proof.em binary");
  std::thread::sleep(std::time::Duration::from_millis(200));

  let (mut ws, _response) = tungstenite::connect("ws://127.0.0.1:47501/ws")
    .expect("client-side WebSocket handshake against the compiled Emerald server");
  ws.send(tungstenite::Message::Text("hello".into()))
    .expect("send text frame");
  let reply = ws.read().expect("read echoed text frame");
  assert_eq!(
    reply
      .into_text()
      .expect("echoed frame should be Text")
      .as_str(),
    "echo: hello"
  );
  // Plan 102's own Concrete Proof: "then sends a Close frame and
  // asserts the connection closes cleanly." The server's own handler
  // (`websocket_proof.em`) calls `ws.close()` immediately after
  // `.send_text`, so this side's own `.close()` may itself race an
  // already-torn-down socket — real, and harmless (`let _ =`, not
  // `.expect`) — the draining loop below is what actually proves a
  // clean close either way, terminating on any `Err` (the real TCP
  // connection closing) or an explicit `Close` message.
  let _ = ws.close(None);
  loop {
    match ws.read() {
      Ok(tungstenite::Message::Close(_)) => break,
      Ok(_) => continue,
      Err(_) => break,
    }
  }

  let _ = child.kill();
  let _ = child.wait();
  std::fs::remove_file(&output).ok();
}

// Plan 104 (Server-Sent Events): `Http.serve` (plan 101) never
// returns, so — the identical shape `websocket_proof_em_round_trips_
// a_real_echo_over_a_real_socket` immediately above already
// establishes for exactly the same reason — this test spawns the
// compiled binary in the BACKGROUND, drives a real, plain
// `std::net::TcpStream` client directly from Rust against it (no
// external crate needed at all here, unlike the WebSocket proof's own
// `tungstenite` client — SSE is a plain-text format over an ordinary
// HTTP response), reads until the server itself closes the
// connection (`sse_ticker.em`'s own `Sse.close` call drops the
// writer, per plan 104's own Decision log), and asserts on the exact
// byte sequence this plan's own Concrete Proof specifies.
#[test]
fn sse_ticker_em_streams_three_framed_events_over_a_real_socket() {
  let source = workspace_root().join("examples").join("sse_ticker.em");
  let output = std::env::temp_dir().join(format!(
    "emerald_example_sse_ticker_em_{}",
    std::process::id()
  ));

  let status = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&source)
    .arg("-o")
    .arg(&output)
    .status()
    .expect("failed to run emerald-cli");
  assert!(
    status.success(),
    "emerald-cli should succeed on sse_ticker.em"
  );

  let mut child = Command::new(&output)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("failed to spawn compiled sse_ticker.em binary");
  std::thread::sleep(std::time::Duration::from_millis(200));

  use std::io::{Read, Write};
  let mut stream = std::net::TcpStream::connect(("127.0.0.1", 47602))
    .expect("connect to the compiled Emerald SSE server");
  // `Connection: close` — a real, disclosed finding made writing this
  // plan's own `emerald-rt` unit test: `tiny_http`'s internal per-
  // connection worker (entirely separate from the raw writer
  // `Sse.close` drops) otherwise keeps the socket's read half open
  // waiting for a next request that never comes, and `read_to_end`
  // below would hang rather than observing a real EOF.
  stream
    .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    .expect("write request");
  let mut raw = Vec::new();
  stream
    .read_to_end(&mut raw)
    .expect("read until the server closes the connection");
  let response = String::from_utf8(raw).expect("response should be valid UTF-8");

  let (head, body) = response
    .split_once("\r\n\r\n")
    .expect("response should have a header/body split");
  assert!(
    head.starts_with("HTTP/1.1 200 OK"),
    "unexpected status line; full head:\n{head}"
  );
  let head_lower = head.to_ascii_lowercase();
  assert!(head_lower.contains("content-type: text/event-stream"));
  assert!(head_lower.contains("cache-control: no-cache"));
  assert!(head_lower.contains("connection: keep-alive"));
  assert_eq!(
    body,
    "event: tick\ndata: 1\n\nevent: tick\ndata: 2\n\nevent: tick\ndata: 3\n\n"
  );

  let _ = child.kill();
  let _ = child.wait();
  std::fs::remove_file(&output).ok();
}

// Plan 114 (JSON Web Tokens): `Jwt.encode_hs256`/`.verify_hs256`/
// `.verify_hs256_with_issuer`/`.peek_header` — see `jwt_proof.em`'s
// own header comment for the one real, disclosed deviation from this
// plan's own literally-quoted Concrete Proof token (`Hash[String,
// String]`'s string-only claims cannot reproduce the real jwt.io
// token's unquoted numeric `"iat"` byte-for-byte) -- everything else
// (round-trip claim recovery, tamper detection against the real,
// independently-verifiable jwt.io token, header inspection, issuer
// validation) is reproduced exactly. `token`'s own value (line 1) is
// deterministic -- HMAC-SHA256 over the same insertion-ordered claims
// every run -- so it is asserted exactly, not merely checked for shape.
#[test]
fn jwt_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("jwt_proof.em"),
    "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoiMTUxNjIzOTAyMiJ9.-N29Lo09lK26mpCcyv0ig6zURJDbim5L7K4wHnuH6S0\n\
     John Doe\n\
     verification failed\n\
     {\"typ\":\"JWT\",\"alg\":\"HS256\"}\n\
     42\n\
     verification failed\n"
  );
}

// Plan 116 (X.509 Certificate Generation & Parsing): `X509.generate_
// self_signed`/`.parse`, `X509KeyPair#cert_pem`/`#key_pem`,
// `X509Certificate#subject`/`#issuer`/`#not_before`/`#not_after`/
// `#public_key_algorithm` — see `x509_proof.em`'s own header comment
// for the three real, disclosed deviations from this plan's original
// Concrete Proof text (an `X509KeyPair` handle in place of an
// unparseable `Tuple[String, String]`; `rcgen`'s real fixed subject,
// `CN=rcgen self signed cert`, in place of the plan's assumed
// `CN=localhost`; `!=` in place of `<`, since this compiler's own
// codegen supports no `String` ordering operator at all). `not_
// before`/`not_after` and the certificate's own key material are
// freshly generated every run — only their real, checked *properties*
// (they differ; the key PEM is non-empty) are asserted, never their
// literal values; `.public_key_algorithm()` (`id-ecPublicKey`,
// `rcgen`'s own default ECDSA signing algorithm at this pin) and the
// fixed subject line are the two genuinely deterministic values this
// proof can and does assert exactly.
#[test]
fn x509_proof_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("x509_proof.em"),
    "CN=rcgen self signed cert\n\
     true\n\
     true\n\
     id-ecPublicKey\n\
     invalid pem\n\
     invalid certificate\n"
  );
}

// Plan 127 (INI Configuration Files): `Ini.parse` against a real
// multi-section document, `IniDocument#get`/`#key_count` reading two
// keys back by name (the Ok path); a genuinely malformed document (an
// unterminated `[section` header) landing on a real, typed `IniError::
// Syntax`, `rust-ini`'s own real parser output, not a simulated
// failure; and the write API round trip (`IniDocument.new`/`#set`/
// `#write` through a real `Tempfile`, then `Ini.load`/`#get` reading
// it back). `Ini.parse`/`.load` return `Result[IniDocument, IniError]`
// from this plan's own first EXECUTE (plan 195's Typed Domain Errors
// convention applied fresh, unlike `Toml.parse`, which predates it).
#[test]
fn ini_demo_em_prints_expected_sequence() {
  assert_eq!(
    compile_and_run("ini_demo.em"),
    "localhost\n8080\n2\n3:1 expecting \"[Some(']')]\" but found EOF.\ndemo\n"
  );
}

// Plan 185 (OAuth2 Client Flow — Authorization Code Grant with
// Mandatory PKCE): unlike every other example in this table, this
// test does NOT use `compile_and_run` alone — `oauth2_client_flow.em`
// is a real network CLIENT (`OAuth2Client#exchange_code`'s one real
// round trip), so this test itself plays the SERVER role, starting a
// fixed, local, single-purpose mock OAuth2 token endpoint (a real
// `tiny_http::Server`, not a live third-party provider — the same
// "no live network dependency in CI" posture plan 96's own worked
// proof already established) on the exact fixed port (9931) this
// file's own literal `auth_url`/`token_url` text hardcodes, BEFORE
// running the compiled example, mirroring `websocket_proof_em_
// round_trips_a_real_echo_over_a_real_socket`'s own "the Emerald side
// and the Rust side play opposite roles" shape — the reverse role
// from most of this table, since the *Emerald* side here is the
// CLIENT, matching plan 185's own Concrete Proof's own framing.
//
// The mock server itself performs the real, meaningful PKCE check
// plan 185's own Decision log describes: it captures the real
// `code_challenge` query parameter from the GET `/authorize` request
// this example's own `Http.get(url)` call sends (the file's own real,
// disclosed addition simulating the browser step — see its own header
// comment), then on the POST `/token` request, recomputes SHA256/
// base64url of the received `code_verifier` and rejects the exchange
// (a 400 `invalid_grant`) unless it matches — a genuine, working proof
// that a PKCE-paired authorization URL was built and the matching
// verifier was later presented correctly at token-exchange time, not
// a mocked-away shortcut.
#[test]
fn oauth2_client_flow_em_exchanges_a_real_authorization_code_for_a_token() {
  use sha2::{Digest, Sha256};
  use std::sync::{Arc, Mutex};

  fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
      if bytes[i] == b'%' && i + 2 < bytes.len() {
        if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
          out.push(byte);
          i += 3;
          continue;
        }
      }
      out.push(bytes[i]);
      i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
  }

  fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
      let (k, v) = pair.split_once('=')?;
      if k == key {
        return Some(percent_decode(v));
      }
    }
    None
  }

  let server = tiny_http::Server::http("127.0.0.1:9931")
    .expect("bind the fixed mock OAuth2 token endpoint on 127.0.0.1:9931");
  let captured_challenge: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
  let captured = captured_challenge.clone();

  let server_thread = std::thread::spawn(move || {
    // Request 1: GET /authorize?... — the simulated browser step;
    // capture `code_challenge`, respond with an arbitrary 200 (its
    // body is never inspected by the client).
    if let Ok(req1) = server.recv() {
      let url = req1.url().to_string();
      *captured.lock().unwrap() = query_param(&url, "code_challenge");
      let _ = req1.respond(tiny_http::Response::from_string("ok"));
    }

    // Request 2: POST /token — the real token-exchange round trip.
    if let Ok(mut req2) = server.recv() {
      let mut body = String::new();
      let _ = req2.as_reader().read_to_string(&mut body);
      let mut code = None;
      let mut verifier = None;
      for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
          match k {
            "code" => code = Some(percent_decode(v)),
            "code_verifier" => verifier = Some(percent_decode(v)),
            _ => {}
          }
        }
      }
      let expected_challenge = captured.lock().unwrap().clone().unwrap_or_default();
      let ok = match (code.as_deref(), verifier.as_deref()) {
        (Some("fixed-test-code"), Some(verifier)) => {
          use base64::Engine;
          let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
          computed == expected_challenge
        }
        _ => false,
      };
      let response = if ok {
        tiny_http::Response::from_string(
          r#"{"access_token":"mock-access-token-12345","token_type":"bearer","expires_in":3600}"#,
        )
        .with_header(
          "Content-Type: application/json"
            .parse::<tiny_http::Header>()
            .unwrap(),
        )
      } else {
        tiny_http::Response::from_string(r#"{"error":"invalid_grant"}"#).with_status_code(400)
      };
      let _ = req2.respond(response);
    }
  });

  let stdout = compile_and_run("oauth2_client_flow.em");
  server_thread
    .join()
    .expect("mock OAuth2 token endpoint thread should not panic");

  let lines: Vec<&str> = stdout.lines().collect();
  assert_eq!(lines.len(), 3, "expected 3 printed lines, got: {stdout:?}");
  assert!(
    lines[0].starts_with("http://127.0.0.1:9931/authorize?"),
    "line 1 should be the real authorization URL, got: {}",
    lines[0]
  );
  assert!(
    lines[0].contains("code_challenge="),
    "authorization URL should carry a real PKCE code_challenge, got: {}",
    lines[0]
  );
  assert!(
    lines[0].contains("code_challenge_method=S256"),
    "authorization URL should be S256, got: {}",
    lines[0]
  );
  assert!(
    lines[0].contains("state="),
    "authorization URL should carry a real CSRF state, got: {}",
    lines[0]
  );
  assert_eq!(
    lines[1], "true",
    "the generated state string should be non-empty"
  );
  assert_eq!(
    lines[2], "mock-access-token-12345",
    "the real token exchange should recover the mock server's own fixed access token"
  );
}
