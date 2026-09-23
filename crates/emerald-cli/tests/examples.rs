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
