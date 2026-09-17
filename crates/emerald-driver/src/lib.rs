//! The parse -> type-check -> codegen -> link pipeline (plan 06),
//! extracted into a library crate now that a second caller needs it —
//! `emerald-lsp`/`emerald-mcp` (plan 17) both need to check/compile
//! in-memory source that was never written to disk, something the
//! single binary crate `emerald-cli` couldn't offer on its own (plan
//! 06's own Decision log named exactly this as the extraction
//! trigger).
//!
//! Two source-text entry points: `check` (parse + sema only — what
//! live diagnostics need, the cheapest reject-without-compiling path)
//! and `compile` (the full pipeline, writing a real linked executable
//! to `output_path`). `check_program`/`compile_program` additionally
//! take an already-parsed `Program` directly — plan 46's multi-file
//! `require` splicing (`emerald-cli`'s own `require.rs`, which shipped
//! after this plan was authored) already needs to hand this driver a
//! fully-assembled in-memory `Program`, which the two source-text
//! entry points alone don't cover.

use emerald_parser::{ParseError, Program};
use id_effect::{Effect, run_blocking};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

pub mod cache;
pub mod parallel;
pub mod require_graph;
use cache::{CacheKey, CacheReporter, QueryCache, raw_hash};

// Plan 21's Decision log: `emerald-lsp` keeps depending only on this
// crate, never directly on `emerald-parser`/`emerald-sema` (the same
// boundary plan 17's `leaf-lsp-server` already established) — this
// re-export is what lets it name `SymbolTable`'s own type without a
// second dependency edge.
pub use emerald_sema::{ClassSymbol, FunctionSymbol, SymbolTable};

// Plan 64's `leaf-cli-target-flag-and-linking`: the identical
// re-export boundary immediately above, for the same reason —
// `emerald-cli` names `CodegenTarget` (its own `--target` flag's
// value type) without a second dependency edge straight onto
// `emerald-codegen`.
pub use emerald_codegen::CodegenTarget;

#[derive(Debug)]
pub enum DriverError {
  /// Plan 26: `emerald_parser::parse_named` reports every top-level
  /// `Item` boundary's syntax error in one pass, not just the first.
  Parse(Vec<ParseError>),
  Sema(Vec<emerald_sema::Diagnostic>),
  Codegen(String),
  Link(String),
  /// Plan 49's `leaf-require-graph-leveling`: a require cycle, a
  /// missing/unreadable required file, or (leaf 3) a construct this
  /// plan's parallel multi-file codegen doesn't support yet (see
  /// `require_graph::unsupported_construct`).
  Require(String),
}

fn parse_stage(source: String, name: String) -> Effect<Program, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_parser::parse_named(&source, &name).map_err(DriverError::Parse)
  })
}

// Plan 61's Decision log: `expand_derives` runs here, right before
// `emerald_sema::check_program` — the single real choke point shared by
// every compile path this crate exposes (`compile`/`compile_program`/
// `compile_program_with_libs`/`compile_with_comptime_step_limit`/`check`
// all `flat_map` through this same stage), the same "between require-
// splicing and check_program" positioning the task brief calls for.
// `check_program(program: &Program)`'s own borrowed-reference entry
// point (used by the LSP's live-typing diagnostics) is a real, disclosed
// gap this doesn't cover — it can't mutate its caller's `Program` by
// design, so a `derive Comparable` class checked only that way won't yet
// see its synthesized `==`.
fn check_stage(mut program: Program) -> Effect<Program, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    if let Err(e) = emerald_parser::expand_derives(&mut program) {
      // `Diagnostic::new` is crate-private to `emerald-sema` — its two
      // fields are `pub`, so a plain struct literal is the real,
      // available construction path from here.
      return Err(DriverError::Sema(vec![emerald_sema::Diagnostic {
        message: e,
        span: (0, 0),
      }]));
    }
    // Plan 62's Decision log (a real, disclosed bug fix, not part of
    // the plan's own original design): `RemoteActorError`/
    // `ContractViolation` must exist before `check_program` runs, or a
    // user's own `rescue RemoteActorError => e`/`rescue
    // ContractViolation => e` fails sema with "unknown type" — see
    // `emerald_codegen::ensure_pre_sema_exception_classes`'s own doc
    // comment for the full story.
    emerald_codegen::ensure_pre_sema_exception_classes(&mut program.items);
    emerald_sema::check_program(&program)
      .map(|()| program)
      .map_err(DriverError::Sema)
  })
}

/// Plan 35's `leaf-line-table-generation`: `source_info` is `Some((source,
/// name))` only from `compile`'s own source-text entry point — `compile_
/// program`'s already-parsed-`Program` entry point has no source text to
/// derive DWARF line numbers from, so it passes `None` and gets the same
/// debug-info-free object file this crate always produced.
// Plan 61's Decision log: `comptime_step_limit` is `Some(n)` only from
// `compile_with_comptime_step_limit` (`emerald-cli`'s own
// `--comptime-step-limit` flag's real, end-to-end entry point) — every
// other, pre-existing caller passes `None`, so this stage's behavior is
// unchanged for them. Threaded into whichever codegen entry point this
// stage would have called anyway (`compile_to_object_with_debug_info`
// widened to accept it directly — see that function's own Decision-log
// note — since `compile`'s own ordinary CLI path always supplies real
// source text here).
fn codegen_stage(
  program: Program,
  obj_path: PathBuf,
  source_info: Option<(String, String)>,
  comptime_step_limit: Option<u64>,
) -> Effect<PathBuf, DriverError, ()> {
  codegen_stage_with_target(
    program,
    obj_path,
    source_info,
    comptime_step_limit,
    emerald_codegen::CodegenTarget::Native,
  )
}

/// Plan 64's `leaf-target-triple-and-selection`/`leaf-cli-target-flag-
/// and-linking`: `codegen_stage`'s own generalized sibling — every
/// pre-plan-64 caller keeps going through `codegen_stage` above
/// (`CodegenTarget::Native`, unchanged behavior); only `compile_
/// program_with_target`/`compile_with_target` (this plan's own new
/// entry points) ever pass `Wasm32Wasi`.
fn codegen_stage_with_target(
  program: Program,
  obj_path: PathBuf,
  source_info: Option<(String, String)>,
  comptime_step_limit: Option<u64>,
  target: emerald_codegen::CodegenTarget,
) -> Effect<PathBuf, DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    let result = match &source_info {
      Some((source, name)) => emerald_codegen::compile_to_object_with_target(
        &program,
        &obj_path,
        source,
        name,
        comptime_step_limit,
        target,
      ),
      None => emerald_codegen::compile_to_object_with_comptime_step_limit_and_target(
        &program,
        &obj_path,
        comptime_step_limit,
        target,
      ),
    };
    result.map(|()| obj_path).map_err(DriverError::Codegen)
  })
}

/// A real, pre-existing bug this leaf's own new tests surfaced (not
/// caused by them): every one of this crate's temp object-file names
/// used to be keyed on `process::id()` alone — fine for a single
/// compile, but every entry point in this file shares one OS process
/// during `cargo test`'s own default multi-threaded run, so two tests
/// compiling concurrently raced on the exact same path and corrupted
/// each other's object file mid-write. Confirmed via `--test-threads=1`
/// (deterministic pass) vs. the default parallel run (`compile_links_a_
/// real_binary_that_runs_and_prints_42`/`compile_program_compiles_an_
/// already_parsed_program` failing with unrelated stdout, e.g. a
/// DIFFERENT test's own compiled binary's output). Fixed here rather
/// than left as a "known flaky test" — unlike plan 60's own genuine
/// actor-scheduling nondeterminism, this one is a real, fixable bug,
/// not the compiled program's own inherent behavior. `std::thread::
/// current().id()` disambiguates, the same fix this crate's own
/// `emerald-codegen` test helpers already use for the identical reason.
fn obj_file_name(prefix: &str) -> String {
  format!(
    "{prefix}_{}_{:?}.o",
    process::id(),
    std::thread::current().id()
  )
}

/// Plan 27: the compiled runtime archive's real bytes, embedded at
/// *this crate's* compile time (`build.rs`) — moved here unchanged
/// from `emerald-cli`. This is what makes a shipped binary, copied
/// alone with no access to this repo's checkout, still able to link a
/// user's compiled program: the runtime archive travels inside the
/// binary itself.
static RUNTIME_ARCHIVE: &[u8] = include_bytes!(env!("EMERALD_RUNTIME_ARCHIVE"));

/// Plan 64's `leaf-wasi-runtime-and-sequential-actors`: `build.rs`
/// always embeds SOMETHING here — a real, `wasm32-wasip1`-compiled
/// archive when a WASI-capable compiler was configured at *this
/// crate's own* build time (`CC_wasm32_wasip1`), or a real, empty
/// (`.is_empty()`) placeholder otherwise (never a build failure just
/// because nobody asked for WASM support — see `build.rs`'s own doc
/// comment). `link_with_libs_and_target`'s `Wasm32Wasi` branch checks
/// for exactly that emptiness before ever invoking a linker.
static RUNTIME_ARCHIVE_WASM32_WASI: &[u8] =
  include_bytes!(env!("EMERALD_RUNTIME_ARCHIVE_WASM32_WASI"));

/// -no-pie: `emerald-codegen` emits non-PIC code (see its `host_isa`),
/// so the executable must not be a PIE either — otherwise `ld` warns
/// about (harmless but avoidable) DT_TEXTREL relocations. The object
/// file and the extracted runtime archive are both removed once
/// linking is attempted, success or failure.
fn link_stage(obj_path: PathBuf, output_path: PathBuf) -> Effect<(), DriverError, ()> {
  Effect::new(move |_env: &mut ()| link(obj_path.clone(), output_path.clone()))
}

/// Plan 59: `link_stage`'s own generalized sibling — threads
/// `extra_libs` (`emerald.toml`'s `[ffi] link = [...]` table) through
/// to `link_with_libs`. `extra_libs` is owned (not `&[String]`) purely
/// because an `Effect` closure must be `'static`, the same reason
/// `obj_path`/`output_path` are already moved into it above.
fn link_stage_with_libs(
  obj_path: PathBuf,
  output_path: PathBuf,
  extra_libs: Vec<String>,
) -> Effect<(), DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    link_with_libs(obj_path.clone(), output_path.clone(), &extra_libs)
  })
}

/// Plan 59: the real, pure argument-list builder behind `link_with_
/// libs`'s own `Command` — factored out so a test can assert the exact
/// argument vector directly (AC2/AC3) without needing `cc`/a real
/// library actually installed on the machine running the gate. One
/// `-l<name>` per `extra_libs` entry, positioned after the object/
/// runtime-archive arguments and before `-o` — ordinary `cc`/`ld`
/// link-order convention (a library must be listed after the objects
/// that reference its symbols).
fn build_link_args(
  obj_path: &Path,
  runtime_archive_path: &Path,
  output_path: &Path,
  extra_libs: &[String],
) -> Vec<std::ffi::OsString> {
  // Bugfix (benchmark session): pairs with build.rs's `-ffunction-
  // sections`/`-fdata-sections` on the runtime archive — `--gc-sections`
  // is what actually asks the linker to drop the now-per-symbol sections
  // it can prove nothing reachable from `main`/the user's program refers
  // to (e.g. the actor/networking runtime, for a program with no actors).
  // No effect without the build.rs flags; harmless if the linker finds
  // nothing prunable.
  let mut args: Vec<std::ffi::OsString> = vec![
    "-no-pie".into(),
    "-Wl,--gc-sections".into(),
    obj_path.as_os_str().to_os_string(),
    runtime_archive_path.as_os_str().to_os_string(),
  ];
  for lib in extra_libs {
    args.push(format!("-l{lib}").into());
  }
  args.push("-o".into());
  args.push(output_path.as_os_str().to_os_string());
  args
}

/// Plan 64's `leaf-cli-target-flag-and-linking`: `build_link_args`'s
/// own `wasm32-wasi` counterpart — the same argument shape, minus
/// `-no-pie` (an ELF/PIC-specific flag with no WASM equivalent; WASM
/// has no position-independent-executable concept to disable).
fn build_link_args_wasm32_wasi(
  obj_path: &Path,
  runtime_archive_path: &Path,
  output_path: &Path,
  extra_libs: &[String],
) -> Vec<std::ffi::OsString> {
  let mut args: Vec<std::ffi::OsString> = vec![
    obj_path.as_os_str().to_os_string(),
    runtime_archive_path.as_os_str().to_os_string(),
  ];
  for lib in extra_libs {
    args.push(format!("-l{lib}").into());
  }
  args.push("-o".into());
  args.push(output_path.as_os_str().to_os_string());
  args
}

/// Plan 64's `leaf-cli-target-flag-and-linking`: `link_with_libs`'s own
/// target-dispatched sibling — `Native` delegates unchanged; `Wasm32Wasi`
/// invokes the WASI-capable compiler resolved the same way `build.rs`
/// resolves one to cross-compile the runtime archive (`CC_wasm32_wasip1`),
/// against `RUNTIME_ARCHIVE_WASM32_WASI` instead of the native archive.
/// A missing WASI toolchain (this workspace's own `devenv shell`,
/// verified this session — see `build.rs`'s own doc comment) is a real,
/// named `DriverError::Link`, not a confusing linker-not-found failure
/// or a silent fallback to native.
pub fn link_with_libs_and_target(
  obj_path: PathBuf,
  output_path: PathBuf,
  extra_libs: &[String],
  target: emerald_codegen::CodegenTarget,
) -> Result<(), DriverError> {
  match target {
    emerald_codegen::CodegenTarget::Native => link_with_libs(obj_path, output_path, extra_libs),
    emerald_codegen::CodegenTarget::Wasm32Wasi => {
      if RUNTIME_ARCHIVE_WASM32_WASI.is_empty() {
        std::fs::remove_file(&obj_path).ok();
        return Err(DriverError::Link(
          "no WASI toolchain was configured when this `emerald` binary was built — \
           `--target wasm32-wasi` is unavailable (see spec/COMPILER.md's per-target \
           restrictions table)"
            .to_string(),
        ));
      }
      let Ok(cc) = std::env::var("CC_wasm32_wasip1") else {
        std::fs::remove_file(&obj_path).ok();
        return Err(DriverError::Link(
          "no WASI-capable compiler configured (CC_wasm32_wasip1) — cannot link a \
           wasm32-wasi build"
            .to_string(),
        ));
      };
      let runtime_archive_path = std::env::temp_dir().join(format!(
        "libemerald_runtime_wasm32_wasi_{}.a",
        process::id()
      ));
      if let Err(e) = std::fs::write(&runtime_archive_path, RUNTIME_ARCHIVE_WASM32_WASI) {
        std::fs::remove_file(&obj_path).ok();
        return Err(DriverError::Link(format!(
          "failed to extract the embedded wasm32-wasi runtime archive: {e}"
        )));
      }
      let args =
        build_link_args_wasm32_wasi(&obj_path, &runtime_archive_path, &output_path, extra_libs);
      let link_result = Command::new(&cc).args(&args).status();
      std::fs::remove_file(&obj_path).ok();
      std::fs::remove_file(&runtime_archive_path).ok();
      match link_result {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(DriverError::Link("linking failed".to_string())),
        Err(e) => Err(DriverError::Link(format!("failed to invoke {cc}: {e}"))),
      }
    }
  }
}

fn link_stage_with_target(
  obj_path: PathBuf,
  output_path: PathBuf,
  extra_libs: Vec<String>,
  target: emerald_codegen::CodegenTarget,
) -> Effect<(), DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    link_with_libs_and_target(obj_path.clone(), output_path.clone(), &extra_libs, target)
  })
}

/// Plan 48: `link_stage`'s own body, factored into a plain function so
/// the new `*_cached` entry points (which don't build an `Effect`
/// pipeline for their already-cached parse/check/codegen stages) can
/// still call it directly — `link_stage` above is now a thin `Effect`
/// wrapper over this, not a second implementation. Never cached (see
/// the plan's own Decision log: caching `cc` would need its own key
/// surface and artifact store for a stage that's a small fraction of
/// total build time compared to LLVM codegen).
///
/// Plan 59: now itself a thin wrapper over `link_with_libs` with
/// `extra_libs: &[]` — produces byte-for-byte the same `Command`
/// argument list it always has (`build_link_args`'s own regression
/// test), so every pre-existing call site is unchanged by construction.
fn link(obj_path: PathBuf, output_path: PathBuf) -> Result<(), DriverError> {
  link_with_libs(obj_path, output_path, &[])
}

/// Plan 59's Decision log: the literal generalization `link` above
/// wraps — one `-l<name>` linker argument per `extra_libs` entry
/// (`emerald.toml`'s `[ffi] link = [...]` table), letting a real
/// `unsafe extern "C" { ... }` declaration that needs a named
/// third-party C library actually resolve at link time instead of
/// failing with an unresolved-symbol error and no way in `emerald.toml`
/// to fix it.
pub fn link_with_libs(
  obj_path: PathBuf,
  output_path: PathBuf,
  extra_libs: &[String],
) -> Result<(), DriverError> {
  let runtime_archive_path =
    std::env::temp_dir().join(format!("libemerald_runtime_{}.a", process::id()));
  if let Err(e) = std::fs::write(&runtime_archive_path, RUNTIME_ARCHIVE) {
    std::fs::remove_file(&obj_path).ok();
    return Err(DriverError::Link(format!(
      "failed to extract the embedded runtime archive: {e}"
    )));
  }
  let args = build_link_args(&obj_path, &runtime_archive_path, &output_path, extra_libs);
  let link_result = Command::new("cc").args(&args).status();
  std::fs::remove_file(&obj_path).ok();
  std::fs::remove_file(&runtime_archive_path).ok();
  match link_result {
    Ok(status) if status.success() => Ok(()),
    Ok(_) => Err(DriverError::Link("linking failed".to_string())),
    Err(e) => Err(DriverError::Link(format!("failed to invoke cc: {e}"))),
  }
}

/// Parses and type-checks `source` in memory — no codegen, no link,
/// no filesystem write of any kind.
pub fn check(source: &str, name: &str) -> Result<(), DriverError> {
  let pipeline = parse_stage(source.to_string(), name.to_string()).flat_map(check_stage);
  run_blocking(pipeline, ()).map(|_program| ())
}

/// The full pipeline: parse, check, codegen, link — writes a real
/// executable to `output_path`.
pub fn compile(source: &str, name: &str, output_path: &Path) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let source_info = Some((source.to_string(), name.to_string()));
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_stage(program, obj_path, source_info, None))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));
  run_blocking(pipeline, ())
}

/// Plan 64's `leaf-cli-target-flag-and-linking`: identical to
/// `compile`, except `target` selects the real target machine and
/// linker/runtime-archive pair — `emerald-cli`'s own `--target
/// wasm32-wasi` flag's real, end-to-end entry point for the legacy
/// single-file (`emerald <file>.em`) path.
pub fn compile_with_target(
  source: &str,
  name: &str,
  output_path: &Path,
  target: emerald_codegen::CodegenTarget,
) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let source_info = Some((source.to_string(), name.to_string()));
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| {
      codegen_stage_with_target(program, obj_path, source_info, None, target)
    })
    .flat_map(move |obj_path| link_stage_with_target(obj_path, output_path, Vec::new(), target));
  run_blocking(pipeline, ())
}

/// Plan 61's Decision log: identical to `compile`, except `limit`
/// overrides `ComptimeInterpreter`'s default `1_000_000`-step ceiling —
/// `emerald-cli`'s own `--comptime-step-limit` flag's real, end-to-end
/// entry point.
pub fn compile_with_comptime_step_limit(
  source: &str,
  name: &str,
  output_path: &Path,
  limit: u64,
) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let source_info = Some((source.to_string(), name.to_string()));
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_stage(program, obj_path, source_info, Some(limit)))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));
  run_blocking(pipeline, ())
}

/// Plan 47's `leaf-test-runner`: parses + checks `source` exactly like
/// `compile` does, then hands the result to
/// `emerald_codegen::compile_test_harness` (not `compile_to_object`)
/// and links the result — a thin passthrough, mirroring how `compile`
/// itself already wraps `compile_to_object`. Returns the number of
/// `test` blocks found.
pub fn compile_test(source: &str, name: &str, output_path: &Path) -> Result<usize, DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald_test"));
  let output_path = output_path.to_path_buf();
  let pipeline = parse_stage(source.to_string(), name.to_string())
    .flat_map(check_stage)
    .flat_map(move |program| codegen_test_stage(program, obj_path))
    .flat_map(move |(obj_path, count)| {
      let output_path = output_path.clone();
      link_stage(obj_path, output_path).flat_map(move |()| {
        Effect::new(move |_env: &mut ()| -> Result<usize, DriverError> { Ok(count) })
      })
    });
  run_blocking(pipeline, ())
}

fn codegen_test_stage(
  program: Program,
  obj_path: PathBuf,
) -> Effect<(PathBuf, usize), DriverError, ()> {
  Effect::new(move |_env: &mut ()| {
    emerald_codegen::compile_test_harness(&program, &obj_path)
      .map(|count| (obj_path.clone(), count))
      .map_err(DriverError::Codegen)
  })
}

/// Plan 21's `leaf-symbol-table`: parses `source` and returns
/// `emerald_sema::collect_symbols`'s best-effort table — sitting
/// alongside `check`/`compile`, reusing the existing `DriverError::
/// Parse` variant for a parse failure rather than inventing a new
/// error path. `emerald-lsp` reaches this instead of depending on
/// `emerald-parser`/`emerald-sema` directly (plan 17's own boundary).
pub fn symbols(source: &str, name: &str) -> Result<emerald_sema::SymbolTable, DriverError> {
  let program = emerald_parser::parse_named(source, name).map_err(DriverError::Parse)?;
  Ok(emerald_sema::collect_symbols(&program))
}

/// Type-checks an already-parsed `Program` directly.
pub fn check_program(program: &Program) -> Result<(), DriverError> {
  emerald_sema::check_program(program).map_err(DriverError::Sema)
}

/// Compiles and links an already-parsed `Program` directly.
pub fn compile_program(program: Program, output_path: &Path) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let pipeline = check_stage(program)
    .flat_map(move |program| codegen_stage(program, obj_path, None, None))
    .flat_map(move |obj_path| link_stage(obj_path, output_path));
  run_blocking(pipeline, ())
}

/// Plan 59: `compile_program`'s own generalized sibling — threads
/// `extra_libs` (`emerald.toml`'s `[ffi] link = [...]` table) through
/// to the link stage. `emerald-cli`'s `cmd_build` (already loads the
/// manifest) calls this instead of `compile_program` directly.
pub fn compile_program_with_libs(
  program: Program,
  output_path: &Path,
  extra_libs: &[String],
) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let extra_libs = extra_libs.to_vec();
  let pipeline = check_stage(program)
    .flat_map(move |program| codegen_stage(program, obj_path, None, None))
    .flat_map(move |obj_path| link_stage_with_libs(obj_path, output_path, extra_libs));
  run_blocking(pipeline, ())
}

/// Plan 64's `leaf-cli-target-flag-and-linking`: identical to
/// `compile_program_with_libs`, except `target` selects the real
/// target machine and linker/runtime-archive pair — `emerald-cli`'s
/// own `cmd_build`'s real entry point for `emerald build --target
/// wasm32-wasi` (project mode, a `require`-spliced `Program` with no
/// single coherent source string, hence `codegen_stage_with_target`'s
/// own `source_info: None` branch).
pub fn compile_program_with_libs_and_target(
  program: Program,
  output_path: &Path,
  extra_libs: &[String],
  target: emerald_codegen::CodegenTarget,
) -> Result<(), DriverError> {
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  let output_path = output_path.to_path_buf();
  let extra_libs = extra_libs.to_vec();
  let pipeline = check_stage(program)
    .flat_map(move |program| codegen_stage_with_target(program, obj_path, None, None, target))
    .flat_map(move |obj_path| link_stage_with_target(obj_path, output_path, extra_libs, target));
  run_blocking(pipeline, ())
}

// Plan 48's `leaf-query-cache-core`/`leaf-require-graph-cache-keys`:
// cached siblings of `check`/`compile`/`compile_program` above. Every
// one of `check`/`compile`/`compile_program` is left completely
// unchanged — no cache-related flag means the exact pre-this-plan code
// path, by construction, satisfying `leaf-verbose-flag-and-consumer-
// wiring`'s AC4 regression requirement trivially. These `_cached`
// variants are additive, opt-in entry points `emerald-cli` reaches
// only when `--verbose-cache`/`--history-cache` is actually passed.

/// The cached sibling of `check` — parses and type-checks `source` via
/// `cache`'s memoized queries instead of calling `emerald_parser`/
/// `emerald_sema` unconditionally.
pub fn check_cached(
  source: &str,
  name: &str,
  cache: &QueryCache,
  reporter: &dyn CacheReporter,
) -> Result<(), DriverError> {
  let program = cache
    .parse_query(name, source, reporter)
    .map_err(DriverError::Parse)?;
  let key = cache.key_for(raw_hash(source.as_bytes()));
  cache
    .type_check_query(key, &program, name, reporter)
    .map_err(DriverError::Sema)
}

/// The cached sibling of `compile` — parse/check/codegen all go
/// through `cache`; `link_stage`'s own `cc` invocation is never
/// cached (see the plan's own Decision log).
pub fn compile_cached(
  source: &str,
  name: &str,
  output_path: &Path,
  cache: &QueryCache,
  reporter: &dyn CacheReporter,
) -> Result<(), DriverError> {
  let program = cache
    .parse_query(name, source, reporter)
    .map_err(DriverError::Parse)?;
  let key = cache.key_for(raw_hash(source.as_bytes()));
  cache
    .type_check_query(key, &program, name, reporter)
    .map_err(DriverError::Sema)?;
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  cache
    .codegen_query(key, &program, &obj_path, name, reporter)
    .map_err(DriverError::Codegen)?;
  link(obj_path, output_path.to_path_buf())
}

/// The cached sibling of `compile_program` — for a multi-file
/// `require`-spliced program, `key` must already be the merged
/// hash-of-hashes `emerald-cli`'s `require.rs` computes over the
/// visited files' own content hashes (`QueryCache::key_for_many`) —
/// this function has no source text of its own to hash, matching
/// `compile_program`'s own already-parsed-`Program` shape.
pub fn compile_program_cached(
  program: Program,
  key: CacheKey,
  label: &str,
  output_path: &Path,
  cache: &QueryCache,
  reporter: &dyn CacheReporter,
) -> Result<(), DriverError> {
  cache
    .type_check_query(key, &program, label, reporter)
    .map_err(DriverError::Sema)?;
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  cache
    .codegen_query(key, &program, &obj_path, label, reporter)
    .map_err(DriverError::Codegen)?;
  link(obj_path, output_path.to_path_buf())
}

/// Plan 59: `compile_program_cached`'s own generalized sibling —
/// threads `extra_libs` through to `link_with_libs` instead of `link`.
/// `emerald-cli`'s `cmd_build` calls this instead when `--verbose-
/// cache`/`--history-cache` is passed.
#[allow(clippy::too_many_arguments)]
pub fn compile_program_cached_with_libs(
  program: Program,
  key: CacheKey,
  label: &str,
  output_path: &Path,
  cache: &QueryCache,
  reporter: &dyn CacheReporter,
  extra_libs: &[String],
) -> Result<(), DriverError> {
  cache
    .type_check_query(key, &program, label, reporter)
    .map_err(DriverError::Sema)?;
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald"));
  cache
    .codegen_query(key, &program, &obj_path, label, reporter)
    .map_err(DriverError::Codegen)?;
  link_with_libs(obj_path, output_path.to_path_buf(), extra_libs)
}

/// Plan 50's `leaf-escape-instrumentation-and-report`: identical to
/// `compile`, additionally returning how many `ClassName.new(...)`
/// sites were stack- vs. heap-allocated — `emerald-cli`'s `--emit=
/// escape-report` flag prints this before proceeding with linking as
/// normal. Not built on the `Effect` pipeline `compile`/`check` share
/// (there is no existing `codegen_stage` shape that returns anything
/// but `PathBuf`) — a small, direct sequence instead, matching
/// `compile_program_cached`'s own directness just above.
pub fn compile_with_escape_report(
  source: &str,
  name: &str,
  output_path: &Path,
) -> Result<emerald_codegen::EscapeStats, DriverError> {
  let program = emerald_parser::parse_named(source, name).map_err(DriverError::Parse)?;
  emerald_sema::check_program(&program).map_err(DriverError::Sema)?;
  let obj_path = std::env::temp_dir().join(obj_file_name("emerald_escape"));
  let stats = emerald_codegen::compile_to_object_with_stats(&program, &obj_path)
    .map_err(DriverError::Codegen)?;
  link(obj_path, output_path.to_path_buf())?;
  Ok(stats)
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_SRC: &str =
    "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn check_accepts_hello_em_in_memory_with_no_filesystem_write() {
    assert!(check(HELLO_SRC, "hello.em").is_ok());
  }

  #[test]
  fn check_rejects_a_sema_error() {
    let source = "x: Int64 = \"nope\"\n";
    assert!(matches!(check(source, "bad.em"), Err(DriverError::Sema(_))));
  }

  #[test]
  fn check_rejects_a_parse_error() {
    let source = "def add(a: Int64\n";
    assert!(matches!(
      check(source, "bad.em"),
      Err(DriverError::Parse(_))
    ));
  }

  #[test]
  fn compile_links_a_real_binary_that_runs_and_prints_42() {
    let dir = std::env::temp_dir().join(format!("emerald-driver-compile-test-{}", process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("hello_out");
    compile(HELLO_SRC, "hello.em", &output).unwrap();
    let run = Command::new(&output).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    std::fs::remove_dir_all(&dir).ok();
  }

  // Plan 61 (comptime execution) — `leaf-derive-comparable`'s own
  // worked proof: `p1 == p2` (equal instances) prints `true`; `p3 == p4`
  // (unequal instances) prints `false`. Real, executed proof the
  // generated `==` method is correct, not just present — and the
  // pipeline placement proof that `check_stage`'s own `expand_derives`
  // call actually reaches every real compile path.
  #[test]
  fn derive_comparable_worked_example_prints_true_then_false() {
    let src = "class Point derive Comparable\n  x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np1: Point = Point.new(1, 2)\np2: Point = Point.new(1, 2)\np3: Point = Point.new(1, 2)\np4: Point = Point.new(3, 4)\nif p1 == p2\n  puts \"true\"\nelse\n  puts \"false\"\nend\nif p3 == p4\n  puts \"true\"\nelse\n  puts \"false\"\nend\n";
    let dir = std::env::temp_dir().join(format!(
      "emerald-driver-derive-comparable-test-{}",
      process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("derive_out");
    compile(src, "derive.em", &output).unwrap();
    let run = Command::new(&output).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "true\nfalse\n");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn compile_program_compiles_an_already_parsed_program() {
    let dir = std::env::temp_dir().join(format!(
      "emerald-driver-compile-program-test-{}",
      process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("hello_out");
    let program = emerald_parser::parse_named(HELLO_SRC, "hello.em").unwrap();
    assert!(check_program(&program).is_ok());
    compile_program(program, &output).unwrap();
    let run = Command::new(&output).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    std::fs::remove_dir_all(&dir).ok();
  }

  // Plan 21 (LSP symbols and navigation).

  #[test]
  fn symbols_returns_a_table_for_hello_em_in_memory_with_no_filesystem_write() {
    let table = symbols(HELLO_SRC, "hello.em").expect("should collect symbols");
    let add = table.functions.get("add").expect("`add` should be present");
    assert_eq!(add.params.len(), 2);
  }

  #[test]
  fn symbols_returns_a_table_for_classes_em() {
    let src = "class Counter\n  value: Int64\n\n  def initialize(start: Int64) -> Void\n    @value = start\n  end\nend\n";
    let table = symbols(src, "classes.em").expect("should collect symbols");
    assert!(table.classes.contains_key("Counter"));
  }

  #[test]
  fn symbols_rejects_an_unparseable_buffer() {
    let source = "def add(a: Int64\n";
    assert!(matches!(
      symbols(source, "bad.em"),
      Err(DriverError::Parse(_))
    ));
  }

  // Plan 59 (C FFI) — `leaf-manifest-linking`.

  #[test]
  fn build_link_args_with_no_extra_libs_is_byte_for_byte_the_original_five_argument_list() {
    // Bugfix (benchmark session): name kept for history, but this is now
    // the six-argument list including `-Wl,--gc-sections` — see that
    // flag's own doc comment on `build_link_args` for why.
    let obj = Path::new("/tmp/x.o");
    let archive = Path::new("/tmp/libemerald_runtime.a");
    let out = Path::new("/tmp/out");
    let args = build_link_args(obj, archive, out, &[]);
    assert_eq!(
      args,
      vec![
        std::ffi::OsString::from("-no-pie"),
        std::ffi::OsString::from("-Wl,--gc-sections"),
        std::ffi::OsString::from("/tmp/x.o"),
        std::ffi::OsString::from("/tmp/libemerald_runtime.a"),
        std::ffi::OsString::from("-o"),
        std::ffi::OsString::from("/tmp/out"),
      ]
    );
  }

  #[test]
  fn build_link_args_with_extra_libs_inserts_l_flags_after_the_objects_and_before_o() {
    let obj = Path::new("/tmp/x.o");
    let archive = Path::new("/tmp/libemerald_runtime.a");
    let out = Path::new("/tmp/out");
    let args = build_link_args(obj, archive, out, &["sqlite3".to_string()]);
    assert_eq!(
      args,
      vec![
        std::ffi::OsString::from("-no-pie"),
        std::ffi::OsString::from("-Wl,--gc-sections"),
        std::ffi::OsString::from("/tmp/x.o"),
        std::ffi::OsString::from("/tmp/libemerald_runtime.a"),
        std::ffi::OsString::from("-lsqlite3"),
        std::ffi::OsString::from("-o"),
        std::ffi::OsString::from("/tmp/out"),
      ]
    );
  }
}
