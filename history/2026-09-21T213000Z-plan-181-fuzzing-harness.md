2026-09-21T21:30:00Z

---
name: Fuzzing Harness Integration
overview: "A `fuzz/` workspace-excluded member fuzzing the Emerald COMPILER's own front end (`emerald_lexer`/`emerald_parser::parse_named` never panics on arbitrary bytes; `emerald_sema::check_program` never panics on any successfully-parsed AST) via `libfuzzer-sys` 0.4 (verified this session: 0.4.13, 5.5M downloads/month, 675 dependent crates) driven by `cargo fuzz` (verified: 0.13.2, requires a nightly Rust compiler and LLVM sanitizer support — x86-64/Aarch64, Unix-like only, per its own README), with `arbitrary` 1.4 (verified: 14.2M downloads/month, 6,738 dependent crates) reserved for a narrower, real second use: generating structured adversarial inputs for typed `emerald_rt` native functions whose own `unsafe` pointer-marshaling code (plan 92's `(ptr, len)` convention) `catch_unwind` cannot protect. This is the one plan in the entire 91-191 batch that adds NO Emerald-facing stdlib surface whatsoever — it is pure compiler/runtime developer tooling, explicitly out of the language's own user-visible feature set, valuable for catching bugs in the emerald-rt crate's own many upcoming native functions and in the compiler's front end before they ship."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-fuzz-workspace-member
    content: "Create `fuzz/Cargo.toml` (cargo-fuzz's own standard generated layout: `[package] publish = false`, a `[[bin]]` per fuzz target, `libfuzzer-sys = \"0.4\"` and `arbitrary = \"1.4\"` as dependencies, `emerald-lexer`/`emerald-parser`/`emerald-sema`/`emerald-rt` as path dependencies) as a workspace member EXCLUDED from the root `Cargo.toml`'s own `[workspace] members` (added instead to a new `exclude = [\"fuzz\"]` entry) — so a plain `cargo build --workspace`/`cargo nextest run --workspace` on this project's pinned stable toolchain (per the `rust-devenv` skill's own stated convention) never attempts to compile `libfuzzer-sys`'s nightly-only intrinsics at all, the same real, disclosed opt-in-tooling posture plan 91 already established for its own recursive-`cargo build` anti-pattern."
    status: pending
  - id: leaf-parser-fuzz-target
    content: "`fuzz/fuzz_targets/parse_program.rs`: `#![no_main]`; `libfuzzer_sys::fuzz_target!(|data: &[u8]| { let src = String::from_utf8_lossy(data); let _ = emerald_parser::parse_named(&src, \"fuzz.em\"); });` — raw fuzzer bytes decoded lossily straight into candidate source text (deliberately NOT going through `arbitrary::Unstructured`, since the goal is \"the parser must survive arbitrary bytes as source,\" not \"generate a structured, already-valid `Program`\" — see Decision log for why `arbitrary` is the wrong tool for this specific target). Any panic anywhere in the lex/parse path is a real, reportable bug regardless of whether the resulting `Result` is `Ok` or a real, well-formed `Err(ErrorRecovery)` — both are acceptable outcomes; a panic is the only failure this target looks for."
    status: pending
  - id: leaf-sema-fuzz-target
    content: "`fuzz/fuzz_targets/sema_check.rs`: identical decode-and-parse step, but only proceeds to `emerald_sema::check_program(&program)` when `parse_named` returns `Ok` — asserting only that a *successfully parsed* (but not necessarily well-typed) AST never causes `check_program` to panic, a real, richer space of \"should reject with a `Diagnostic`, never panic\" inputs than the parser target alone covers."
    status: pending
  - id: leaf-smoke-tests-not-a-real-campaign
    content: "Add `crates/emerald-fuzz-smoke/tests/smoke.rs` (a plain, stable-toolchain, `cargo nextest run --workspace`-compatible crate, NOT inside `fuzz/`) with two `#[test]`s per fuzz target — one fixed, known-good `.em` snippet and one fixed, deliberately-malformed byte sequence (a truncated multi-byte UTF-8 sequence, an unmatched `{`, a NUL byte mid-token) — asserting each of `parse_program`'s/`sema_check`'s own logic (extracted into a plain, testable Rust function each fuzz target calls, per cargo-fuzz's own recommended \"keep fuzz_targets/*.rs as thin wrappers\" pattern) does not panic on either fixed input. This satisfies the batch's own \"Rust-side `#[test]`s\" requirement in the only way honestly available to a fuzzing plan: proving the harness itself is wired correctly on the default, nightly-free, ASan-free CI path — not proving a real fuzzing campaign found zero bugs, which is a materially different, nightly-only, time-unbounded activity this plan does not gate into CI (see Decision log)."
    status: pending
  - id: leaf-manual-campaign-docs
    content: "Document, in `fuzz/README.md`, the manual/opt-in invocation this plan deliberately does not automate: `cargo +nightly fuzz run parse_program -- -max_total_time=60` (and the `sema_check` equivalent) — a real command a developer or a separate, explicitly-opted-in CI job can run locally or on a schedule, requiring a nightly toolchain this project's default pinned-stable build never needs."
    status: pending
isProject: false
---

# Plan 181 — Fuzzing Harness Integration

Every other plan in the 91-191 batch adds a real, new capability to
Emerald *programs* — something an Emerald developer's own `.em` source
can call. This plan adds nothing an Emerald program can ever see or
call. It exists one level up: hardening the *compiler itself* (the
lexer/parser/sema front end that turns arbitrary `.em` source text into
a checked `Program`) and, looking forward, the many `unsafe`-marshaling
native functions this batch's other plans and the ~100 domain plans that
follow will add to `emerald-rt`, against inputs no unit test happens to
have thought of. State this plainly, since it is a real, deliberate
scope distinction from every sibling plan in this batch: **this plan
ships zero new Emerald-facing stdlib surface.** No class, no method, no
grammar production, no `emerald_rt_*` export callable from `.em` source
exists anywhere in this plan's own scope.

## Concrete proof this plan targets

There is no `.em` example and no expected-stdout claim, because this
plan has no Emerald-facing behavior to demonstrate — the "proof" for a
dev-tooling plan is the fuzz harness itself compiling, running, and
finding what it's built to find:

```
$ cargo +nightly fuzz run parse_program -- -runs=100000
...
Done 100000 runs in 4s
$ cargo nextest run -p emerald-fuzz-smoke
running 4 tests
test parse_program_survives_known_good_input ... ok
test parse_program_survives_truncated_utf8 ... ok
test sema_check_survives_known_good_input ... ok
test sema_check_survives_unmatched_brace ... ok
```

The first line (nightly-only, opt-in, never part of the default gate) is
this plan's real "did fuzzing find a crash" proof; the second (stable,
default, `cargo nextest run --workspace`-compatible) is this plan's
required, CI-gated proof that the harness itself is correctly wired —
the distinction between the two is the load-bearing point of this
plan's whole design, not a formality.

## Decision log

- **Scope, stated up front and firmly: zero Emerald-facing surface, the
  one plan in this batch like this.** Every leaf in this plan touches
  only `fuzz/` and a small, separate smoke-test crate — no change to
  `grammar.lalrpop`, `emerald-sema`'s `classes`/type-checking logic, or
  any `emerald_rt_*` export's Emerald-visible dispatch. A reviewer
  checking this plan against the batch's own "specific Rust crate(s) +
  exact Emerald API surface" requirement should find the API-surface
  answer is genuinely "none" here, not an oversight.
- **`arbitrary`/`cargo-fuzz`/`libfuzzer-sys`, verified current and
  actively maintained this session, not assumed.** `lib.rs`'s real
  crate pages: `arbitrary` `1.4.2` (14 Aug 2025), 14,220,716
  downloads/month, 6,738 dependent crates, maintained by the
  `rust-fuzz` GitHub org; `cargo-fuzz` `0.13.2` (9 Jun 2026), maintained
  by the same org, whose own README states plainly: "libFuzzer needs
  LLVM sanitizer support, so this only works on x86-64 and Aarch64, and
  only on Unix-like operating systems (not Windows)... This also needs
  a nightly compiler"; `libfuzzer-sys` `0.4.13` (4 Jun 2026), 5,500,367
  downloads/month, 675 dependent crates — the real Rust binding to
  LLVM's own libFuzzer C++ runtime that `cargo fuzz` drives. All three
  are the standard, canonical toolchain for fuzzing Rust code — no
  competing option was evaluated, since this is a de facto
  single-ecosystem choice (AFL.rs is the only real alternative driver,
  and `rust-fuzz`'s own `arbitrary` crate is designed to pair with
  either — this plan picks libFuzzer/`cargo fuzz` as the more actively
  maintained, better-documented default of the two).
- **The two front-end targets are the highest-value, cheapest starting
  point — not an arbitrary pair.** `emerald_lexer`/`emerald_parser::
  parse_named` is the single largest surface in this entire compiler
  that must accept genuinely untrusted, arbitrary-byte input by
  definition (any `.em` file on disk, by construction, since Emerald
  source is never itself sandboxed or pre-validated before compilation
  begins) — fuzzing it requires no per-function harness-writing effort
  the way a future `emerald_rt_*` native function target would (no
  `#[derive(Arbitrary)]` struct to design, no domain-specific input
  shape to model — raw bytes decoded lossily into a candidate source
  string covers the entire front-end surface at once). `emerald_sema::
  check_program` is the natural second target: a successfully-parsed
  but potentially ill-typed `Program` is a much richer space of "must
  reject via `Diagnostic`, never panic" inputs than raw un-parseable
  byte soup covers.
- **Real, distinct value over plan 92's `catch_unwind` boundary — memory-
  safety bugs inside `unsafe` blocks, not ordinary panics.** Plan 92's
  `emerald_rt_fn!` macro catches every ordinary Rust panic at every
  `emerald_rt_*` export's boundary — a ratified, working, and entirely
  different safety mechanism than what fuzzing-plus-ASan (libFuzzer's
  usual pairing with AddressSanitizer, real and standard practice for
  any `cargo fuzz` target) catches: a buffer over-read, an out-of-bounds
  pointer dereference inside a hand-written `unsafe { *ptr.offset(n) }`
  block that never trips one of Rust's own bounds-checked panic paths
  at all. `catch_unwind` cannot catch what never panics in the first
  place — a`n `unsafe` pointer computation gone wrong from a
  maliciously-mismatched `(ptr, len)` pair (plan 92's own binary-safe
  convention, the exact shape a future domain plan's `unsafe` marshaling
  code would need to get right) is precisely the class of bug this
  distinction names, and precisely where a future `#[derive(Arbitrary)]`-
  driven structured-input fuzz target (deliberately generating a
  plausible-looking but adversarially mismatched length/pointer
  argument pair) would earn its keep, once a concrete `(ptr, len)`-
  taking native function exists to target — not built by this plan,
  since no such function ships in this batch's own six plans, but named
  here as the real, concrete reason `arbitrary` the crate is listed
  alongside `cargo-fuzz`/`libfuzzer-sys` at all, not merely bundled by
  convention.
- **`arbitrary`'s actual role here is narrower than "decode all fuzzer
  bytes" — a real, disclosed distinction from the front-end targets.**
  `leaf-parser-fuzz-target`/`leaf-sema-fuzz-target` deliberately do
  *not* use `Unstructured::arbitrary::<String>()` to decode fuzzer
  bytes — `String::from_utf8_lossy` is used instead, since the goal is
  "feed the parser genuinely arbitrary byte sequences as source text,"
  not "feed it a structured value `arbitrary`'s own blanket `String`
  impl happens to produce" (the two are not the same distribution of
  inputs, and the raw-bytes-as-source-text approach is both simpler and
  closer to the real threat model — a `.em` file is just bytes on disk).
  `arbitrary`'s real, load-bearing use is reserved for the future
  typed-native-function targets the previous bullet names.
- **Nightly-toolchain tension, resolved exactly like plan 91's own
  recursive-`cargo build` anti-pattern — disclosed, not hidden.**
  `AGENTS.md`'s own build/test/lint/format commands are all stable-
  toolchain commands; `cargo-fuzz` needs nightly for its own unstable
  sanitizer flags. `fuzz/`'s exclusion from `[workspace] members` (via
  root `Cargo.toml`'s `exclude = [...]`, a real, standard Cargo
  mechanism) means `cargo build --workspace`/`cargo nextest run
  --workspace` on the project's pinned stable channel never even
  attempts to parse or compile anything inside `fuzz/` — the nightly
  requirement is real and permanent, not a temporary gap, and is
  handled by scoping it out of the default build graph entirely rather
  than by any conditional-compilation trick inside a shared crate.
- **The "Rust-side `#[test]`s" requirement is satisfied differently
  here than in every sibling plan — disclosed explicitly, not silently
  reinterpreted.** A `cargo fuzz run` invocation is not a `#[test]`; it
  is a long-running, non-deterministic, nightly-only, ASan-instrumented
  process that this project's own `cargo nextest run --workspace` gate
  neither can nor should invoke by default. `leaf-smoke-tests-not-a-
  real-campaign`'s stable-toolchain smoke tests are this plan's real,
  disclosed substitute — they prove the harness's own logic (extracted
  into plain, directly-testable functions) doesn't panic on two fixed,
  version-controlled inputs, which is a meaningfully weaker claim than
  "a real fuzzing campaign ran and found nothing," and this plan does
  not pretend otherwise.
- **Out of scope.** No fuzzing of `emerald-cli`'s other subcommands
  (`lint`/`format`/`doc`/`repl`) in v1 — a real, disclosed narrowing;
  parser and sema are this plan's own two highest-value initial
  targets, and a later plan could extend the same pattern to them. No
  CI-gating of a real, time-unbounded fuzzing campaign (explicitly kept
  manual/opt-in, documented in `fuzz/README.md`, never wired into any
  default pipeline). No automatic corpus management, crash triage, or
  minimization pipeline beyond what `cargo fuzz`'s own tooling already
  provides out of the box. No structured `#[derive(Arbitrary)]` fuzz
  target for any specific `emerald_rt_*` native function in this
  plan's own scope (named as real future work above, not built here,
  since none of this batch's six plans ships an `unsafe`, raw-pointer-
  marshaling function whose adversarial-input space would justify one
  yet). No Emerald-facing stdlib surface of any kind, restated as the
  final word on this plan's scope.
