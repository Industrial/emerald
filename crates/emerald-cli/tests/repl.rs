//! Real end-to-end proof of plan 47's `leaf-repl` acceptance criteria
//! — drives the actual compiled `emerald repl` binary over a piped
//! stdin, a real subprocess-per-line session, never calling
//! `emerald_driver`/parser internals directly.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_repl(input: &str) -> std::process::Output {
  let mut child = Command::new(env!("CARGO_BIN_EXE_emerald"))
    .arg("repl")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("failed to spawn emerald repl");
  child
    .stdin
    .take()
    .expect("stdin should be piped")
    .write_all(input.as_bytes())
    .expect("should write to stdin");
  child
    .wait_with_output()
    .expect("failed to wait on emerald repl")
}

#[test]
fn the_plan_47_worked_transcript_matches_exactly() {
  let input = "x: Int64 = 10\nx + 5\nx + \"oops\"\nputs x\nexit\n";
  let output = run_repl(input);
  let stdout = String::from_utf8_lossy(&output.stdout);

  // AC1: line 1 -> no output, line 2 -> "15", line 4 -> "10" — a
  // rejected line 3 produces no stdout of its own (its diagnostic goes
  // to stderr, checked separately below).
  assert_eq!(
    stdout,
    "Emerald REPL — each line is compiled and run fresh against the session so far.\n\
     emerald> emerald> 15\n\
     emerald> emerald> 10\n\
     emerald> "
  );

  // Line 3's rejection produced a real diagnostic, not silence.
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(
    !stderr.trim().is_empty(),
    "expected a diagnostic on stderr for line 3"
  );
}

#[test]
fn a_rejected_line_leaves_the_session_unmodified() {
  // AC2: verified indirectly through the session's own observable
  // behavior (line 4 still sees `x == 10`) — the worked transcript
  // test above already proves this end to end, but this test isolates
  // exactly the claim: a bad line changes nothing for what follows.
  let input = "x: Int64 = 10\nx + \"oops\"\nputs x\nexit\n";
  let output = run_repl(input);
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.ends_with("10\nemerald> "), "{stdout}");
}

#[test]
fn a_line_that_fails_to_parse_at_all_is_rejected_the_same_way() {
  // AC3: a genuine parse failure (an unterminated `if`), not just a
  // sema-rejected line, is isolated identically — no crash, no hang,
  // `prelude` untouched (`puts 1` after it still works).
  let input = "if true\nputs 1\nexit\n";
  let output = run_repl(input);
  let stdout = String::from_utf8_lossy(&output.stdout);
  assert!(stdout.contains("1\n"), "{stdout}");
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!stderr.trim().is_empty());
}

#[test]
fn eof_exits_cleanly_with_no_trailing_prompt_hang() {
  let output = run_repl("puts 1\n");
  assert!(output.status.success());
  assert!(String::from_utf8_lossy(&output.stdout).contains("1\n"));
}
