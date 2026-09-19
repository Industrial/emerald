//! Plan 35's `leaf-debugger-verification`: a real, scripted, non-
//! interactive `gdb` session against a compiled Emerald binary, proving
//! the DWARF line-table debug info `emerald_codegen::
//! compile_to_object_with_debug_info` attaches (`leaf-line-table-
//! generation`) isn't just present and well-formed — a real debugger
//! can use it to stop execution at the right source line.
//!
//! `#[ignore]`d — depends on an external `gdb` binary being present,
//! same convention as plan 15's benchmark runner. Run explicitly:
//!
//! ```text
//! cargo test --release -p emerald-cli -- --ignored --nocapture debug_info
//! ```

use std::process::Command;

/// Plan 35's own worked example — the same source this plan's Decision
/// log names, byte-for-byte, so the breakpoint target line this test
/// computes always matches the real compiled line.
const WORKED_EXAMPLE: &str =
  "fn add(a: Int64, b: Int64): Int64 do\n  sum: Int64 = a + b\n  sum\nend\n\nputs add(20, 22)\n";

#[test]
#[ignore]
fn breakpoint_at_sum_line_actually_stops_execution_there() {
  let gdb = Command::new("gdb").arg("--version").output();
  if gdb.is_err() {
    eprintln!(
      "gdb not found on PATH — skipping (see leaf-debugger-verification's own disclosed fallback: the DWARF line table's real, correct content was already verified via `readelf --debug-dump=decodedline` this session)."
    );
    return;
  }

  // Plan 35's own line-number rule: a 1-based count of the real source
  // text, never hardcoded — this line must track `WORKED_EXAMPLE`
  // exactly, the same discipline `line_for_offset` uses in
  // `emerald-codegen`.
  let target_line = WORKED_EXAMPLE
    .lines()
    .position(|l| l.contains("sum: Int64 = a + b"))
    .expect("WORKED_EXAMPLE must contain the breakpoint target statement")
    + 1;

  let dir = std::env::temp_dir();
  let src_path = dir.join(format!("emerald_debug_info_{}.em", std::process::id()));
  let bin_path = dir.join(format!("emerald_debug_info_bin_{}", std::process::id()));
  std::fs::write(&src_path, WORKED_EXAMPLE).unwrap();

  let compile = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg(&src_path)
    .arg("-o")
    .arg(&bin_path)
    .status()
    .expect("failed to run emerald-cli");
  assert!(
    compile.success(),
    "emerald-cli should compile the worked example"
  );

  // DWARF stores the file name as `compile_to_object_with_debug_info`'s
  // `file_name` param's own basename (see `emerald-codegen`'s
  // `compile_to_object_impl`) — the driver passes the source path it
  // was given, so `gdb`'s `break <basename>:<line>` must match that,
  // not the scratch dir's full absolute path.
  let src_basename = src_path.file_name().unwrap().to_str().unwrap();
  let break_spec = format!("{src_basename}:{target_line}");

  let gdb_run = Command::new("gdb")
    .arg("-batch")
    .arg("-ex")
    .arg(format!("break {break_spec}"))
    .arg("-ex")
    .arg("run")
    .arg("-ex")
    .arg("info line")
    .arg("-q")
    .arg(&bin_path)
    .output()
    .expect("failed to run gdb");

  let stdout = String::from_utf8_lossy(&gdb_run.stdout);
  let stderr = String::from_utf8_lossy(&gdb_run.stderr);
  let combined = format!("{stdout}\n{stderr}");
  eprintln!("--- gdb session output ---\n{combined}\n--- end ---");

  assert!(
    !combined.contains("No symbol table") && !combined.contains("No source file named"),
    "gdb should resolve the breakpoint against real debug info: {combined}"
  );
  assert!(
    combined.contains(&break_spec) || combined.contains(&format!("line {target_line}")),
    "gdb should report the breakpoint's real source line ({target_line}): {combined}"
  );
  // A real, observed program halt at the breakpoint — not merely a
  // resolved address that's never hit: `run` must report the program
  // stopped at a "Breakpoint" (gdb's own wording), and the program's
  // real stdout ("42") must NOT have already been printed, since `add`
  // is called (and so must stop) before `puts` ever executes.
  assert!(
    combined.contains("Breakpoint") && combined.contains(src_basename),
    "gdb should report stopping at a breakpoint in the compiled source file: {combined}"
  );
  assert!(
    !combined.contains("\n42\n") && !stdout.trim_end().ends_with("42"),
    "execution must stop at the breakpoint before `puts` ever runs — the program's own output must not appear: {combined}"
  );

  std::fs::remove_file(&src_path).ok();
  std::fs::remove_file(&bin_path).ok();
}
