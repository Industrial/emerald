//! Plan 69 — the no-`--jobs` half of the gap plan 65 explicitly left
//! open: `run_legacy` (`emerald-cli/src/main.rs`'s bare `emerald
//! <file>.em -o out` path, no `emerald.toml`, no `--jobs`) used to hand
//! its raw source straight to `emerald_driver::compile` without ever
//! looking at `require` items — `emerald_sema` treats `Item::Require`
//! as a no-op (see `emerald-parser/src/ast.rs`'s own doc comment), so
//! every symbol a required file was supposed to bring into scope came
//! back "unknown type"/"undefined variable"/"undefined function" at
//! typecheck instead of splicing in. `require.rs`'s splicer (plan 23)
//! itself was never broken — `cmd_build`'s `emerald build` path (which
//! needs an `emerald.toml`) was simply the only caller ever reaching
//! it; `run_legacy` never called it at all, for ANY required
//! definition shape (plain function, ordinary class, or actor), not
//! just actors — confirmed directly against this session's build
//! before fixing anything, contradicting this plan's own inherited
//! assumption that ordinary requires already worked here.
//!
//! Real end-to-end proof: spawns the actual compiled `emerald` binary
//! as a subprocess, exactly like `query_cache.rs`'s own tests do, never
//! calling `emerald-driver`/`require.rs` in-process.

use std::path::PathBuf;
use std::process::Command;

fn fresh_dir(tag: &str) -> PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "emerald-cli-require-no-jobs-test-{tag}-{}",
    std::process::id()
  ));
  std::fs::create_dir_all(&dir).unwrap();
  dir
}

fn compile_and_run(dir: &std::path::Path, entry: &str) -> (bool, String, String) {
  let output = dir.join("out");
  let compiled = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(dir.join(entry))
    .arg("-o")
    .arg(&output)
    .output()
    .expect("failed to spawn emerald-cli");
  if !compiled.status.success() {
    return (
      false,
      String::new(),
      String::from_utf8_lossy(&compiled.stderr).into_owned(),
    );
  }
  let run = Command::new(&output)
    .output()
    .expect("failed to run compiled binary");
  (
    run.status.success(),
    String::from_utf8_lossy(&run.stdout).into_owned(),
    String::from_utf8_lossy(&run.stderr).into_owned(),
  )
}

/// `leaf-confirm-scope-of-the-gap`'s own finding, kept as a permanent
/// regression: a bare, no-`--jobs` `require` of a file declaring only
/// an ordinary top-level function — plan 23's original shape, with no
/// class/actor involved at all — must still typecheck, link, and run.
#[test]
fn a_bare_no_jobs_require_of_a_plain_function_splices_and_runs() {
  let dir = fresh_dir("plain-function");
  std::fs::write(dir.join("helper.em"), "fn helper(): Int64 do\n  5\nend\n").unwrap();
  std::fs::write(dir.join("main.em"), "require helper\nputs helper() + 1\n").unwrap();

  let (ok, stdout, stderr) = compile_and_run(&dir, "main.em");
  assert!(
    ok,
    "plain-function no-jobs require should succeed: {stderr}"
  );
  assert_eq!(stdout, "6\n");

  std::fs::remove_dir_all(&dir).ok();
}

/// The plan's own concrete proof: `examples/counter_actor.em`-shaped
/// `actor Counter`, required by a single-file program with no
/// `--jobs`. Before this fix: typecheck failure citing `unknown type
/// 'Counter'` and `undefined variable 'c'`, exactly as `examples/
/// README.md` and this plan's own file documented. Mirrors
/// `emerald-driver/tests/parallel_jobs.rs`'s own
/// `an_actor_shared_across_require_d_files_links_and_runs_correctly_
/// under_jobs` fixture byte-for-byte (same source, same expected `3`)
/// so the `--jobs` and no-`--jobs` paths are proven against the exact
/// same program, just the flag differing — the plan's own worked
/// example calls `puts c.report` instead of the bare `c.report` used
/// here, but `report`'s own `-> Void` return can't be `puts`'d at all
/// (a real, separate, pre-existing "`puts` of a non-`Void` actor-call
/// `Result`" limitation unrelated to require-splicing, out of this
/// plan's scope) — `examples/host.em`/`client.em` and the `--jobs`
/// regression test both already call actor methods this same bare way.
#[test]
fn a_bare_no_jobs_require_of_an_actor_declaring_file_splices_links_and_runs() {
  let dir = fresh_dir("actor");
  std::fs::write(
    dir.join("counter_actor.em"),
    "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn report: Void do\n    puts @count\n  end\nend\n",
  )
  .unwrap();
  std::fs::write(
    dir.join("main.em"),
    "require counter_actor\n\nc: Counter = Counter.spawn(0)\nc.increment\nc.increment\nc.increment\nc.report\n",
  )
  .unwrap();

  let (ok, stdout, stderr) = compile_and_run(&dir, "main.em");
  assert!(ok, "actor no-jobs require should succeed: {stderr}");
  assert_eq!(stdout, "3\n");

  std::fs::remove_dir_all(&dir).ok();
}

/// Regression (the exact pre-fix symptom, pinned so a future change
/// can't silently reopen it): the same actor fixture, but this time
/// asserting the OLD failure's exact diagnostics are gone — no
/// "unknown type" and no "undefined variable" in stderr on success.
#[test]
fn a_bare_no_jobs_actor_require_no_longer_reports_unknown_type_or_undefined_variable() {
  let dir = fresh_dir("actor-diagnostics");
  std::fs::write(
    dir.join("counter_actor.em"),
    "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn report: Void do\n    puts @count\n  end\nend\n",
  )
  .unwrap();
  std::fs::write(
    dir.join("main.em"),
    "require counter_actor\n\nc: Counter = Counter.spawn(0)\nc.increment\nc.report\n",
  )
  .unwrap();

  let (ok, _stdout, stderr) = compile_and_run(&dir, "main.em");
  assert!(ok, "actor no-jobs require should succeed: {stderr}");
  assert!(
    !stderr.contains("unknown type") && !stderr.contains("undefined variable"),
    "the pre-fix diagnostics must not reappear: {stderr}"
  );

  std::fs::remove_dir_all(&dir).ok();
}
