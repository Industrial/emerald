//! Plan 145 (Process Spawning & Control): `Process.run(cmd: String,
//! args: Array[String], argc: Int64, stdin_data: String): ProcessResult`
//! — a synchronous, blocking child-process spawn wrapping plain
//! `std::process::Command`, zero third-party crate (`std::process`
//! already does everything this plan needs: spawn, capture stdout/
//! stderr, exit code, write-then-close stdin).
//!
//! Error handling: NOT `Result[T, String]`/plan 195's Typed Domain
//! Errors convention — this plan's own Decision log draws a
//! deliberate line between two different classes of event.
//! `Command::spawn()` failing (the executable doesn't exist, isn't
//! executable, ...) means the child never ran at all; there is no
//! `ProcessResult` to construct, so this raises a real, catchable
//! `NativeError` exception via `crate::raise_native_error` — the same
//! plain-raise convention `File`'s own functions already use (`path.
//! rs`'s own module doc explains why `Path`/`Decimal` are the
//! `Result`-returning exception to that rule, not the rule itself). A
//! nonzero exit code is NOT an error under this plan's design —
//! `grep` finding no match, `diff` finding a difference are completely
//! routine, expected outcomes for many real programs — so it is a
//! plain `Int64` field on the returned `ProcessResult`, read back via
//! `.exit_code()`/`.success()`, never wrapped in any error type.
//!
//! `ProcessResult` — a compiler-synthesized `Type::Class` (the
//! identical "opaque heap pointer + hardcoded `ClassInfo`/`ClassDef`
//! registration, no dedicated `ValKind`" shape `Decimal`/`DateTime`/
//! `FileMetadata` already established — no per-domain `ValKind`
//! mechanism exists anywhere in this codebase for any sibling
//! compiler-synthesized value type, so this plan's own original text
//! proposing a dedicated `ValKind::ProcessResult` is superseded the
//! same way `path.rs`'s own module doc already discloses for
//! `FileMetadata`) — a heap pointer to a packed `[stdout: *const
//! c_char][stderr: *const c_char][exit_code: i64]` block (24 bytes).
//! `.stdout()`/`.stderr()` read the two `String` pointers straight
//! back; `.exit_code()` reads the `Int64` straight back; `.success()`
//! has no backing field at all — it's computed here as `exit_code ==
//! 0`, crossing the FFI boundary as a plain `i64` (0/1), narrowed to a
//! real `i1` by `emerald-codegen`'s own call site, the same direction
//! `FileMetadata`'s own Boolean accessors already narrow.
//!
//! `args`/`argc` marshaling: `emerald-codegen`'s own call-site codegen
//! (not this module) skips the already-realized `Array[String]`
//! value's own `[len: i64]` header (`build_array_lit`'s own layout)
//! and passes the bare element buffer straight through as `argv:
//! *const *const c_char`, alongside the caller-supplied `argc: Int64`
//! travelling as its own explicit argument — the third confirmed
//! sighting of `Array[T]`'s missing length metadata forcing an
//! explicit companion count (plan 45's `ARGV`/`ARGC`, plan 144's `Dir.
//! entries`/`.entries_count`), this occurrence crossing the FFI
//! boundary as a call argument rather than a second return value.
//!
//! No shell is ever invoked — `Command::new(cmd).args(argv)` runs
//! `cmd` directly via `execvp`-family semantics, never through `/bin/
//! sh -c`, avoiding shell-metacharacter injection by construction (a
//! caller who genuinely wants shell semantics can still get them by
//! explicitly spawning `Process.run("/bin/sh", ["-c", "..."], 2, "")`).
//! Captured stdout/stderr are raw bytes reinterpreted as `String` via
//! `String::from_utf8_lossy` — a child process's real output is not
//! guaranteed valid UTF-8, and this plan deliberately replaces invalid
//! byte sequences with `U+FFFD` rather than failing the whole call
//! over a handful of bad bytes, matching `.upcase`/`.downcase`'s own
//! already-disclosed forgiving-by-design precedent (plan 45).

use std::ffi::c_void;
use std::io::Write;
use std::os::raw::c_char;
use std::process::{Command, Stdio};

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Process.run(cmd: String, args: Array[String], argc: Int64,
/// stdin_data: String): ProcessResult`.
///
/// # Safety
/// `cmd`/`stdin_data`, if non-null, must point to valid, NUL-
/// terminated C strings. `argv` must point to a buffer of at least
/// `argc` valid, NUL-terminated C string pointers — `emerald-codegen`'s
/// own call-site codegen guarantees this (see this module's own doc
/// comment for the header-skipping convention that produces it); when
/// `argc` is `0`, `argv` is never dereferenced and may be null.
pub unsafe fn process_run(
  cmd: *const c_char,
  argv: *const *const c_char,
  argc: i64,
  stdin_data: *const c_char,
) -> *mut c_void {
  let cmd = match read_str(cmd) {
    Ok(c) => c,
    Err(e) => crate::raise_native_error(&e),
  };
  let stdin_data = match read_str(stdin_data) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };

  let mut args: Vec<String> = Vec::with_capacity(argc.max(0) as usize);
  for i in 0..argc {
    let arg_ptr = *argv.add(i as usize);
    args.push(
      std::ffi::CStr::from_ptr(arg_ptr)
        .to_string_lossy()
        .into_owned(),
    );
  }

  let mut child = match Command::new(cmd)
    .args(&args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
  {
    Ok(c) => c,
    Err(e) => crate::raise_native_error(&format!("Process.run: {cmd}: {e}")),
  };

  // Write the entirety of `stdin_data`, then drop the handle to close
  // the pipe — the child sees a real EOF, not a hang. `wait_with_
  // output` reads stdout/stderr concurrently on separate threads
  // internally (`std::process`'s own documented behavior), so no
  // hand-rolled pipe-deadlock avoidance is needed here.
  if let Some(mut stdin) = child.stdin.take() {
    // A child that exits before consuming all of `stdin_data` (or
    // that never reads stdin at all, e.g. `echo`) makes this write
    // fail with a broken-pipe error — real, expected, and deliberately
    // ignored: the child's own captured output/exit code still fully
    // answers the caller's question, so failing the whole call over a
    // benign broken pipe would reject a real, common case.
    let _ = stdin.write_all(stdin_data.as_bytes());
  }

  let output = match child.wait_with_output() {
    Ok(o) => o,
    Err(e) => crate::raise_native_error(&format!("Process.run: {cmd}: {e}")),
  };

  let stdout_text = String::from_utf8_lossy(&output.stdout);
  let stderr_text = String::from_utf8_lossy(&output.stderr);
  // A process killed by a signal (Unix) has no real exit code —
  // `ExitStatus::code()` returns `None` in that case; `-1` is a real,
  // disclosed sentinel for that case, distinguishable from any real
  // exit code a well-behaved process returns (0-255 on every platform
  // this compiler targets).
  let exit_code = output.status.code().unwrap_or(-1) as i64;

  let ptr = crate::emerald_alloc(24) as *mut i64;
  *ptr = crate::alloc_and_copy_str(&stdout_text) as i64;
  *ptr.add(1) = crate::alloc_and_copy_str(&stderr_text) as i64;
  *ptr.add(2) = exit_code;
  ptr as *mut c_void
}

/// `.stdout(self): String`.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
pub unsafe fn processresult_stdout(ptr: *const i64) -> *const c_char {
  *ptr as *const c_char
}

/// `.stderr(self): String`.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
pub unsafe fn processresult_stderr(ptr: *const i64) -> *const c_char {
  *ptr.add(1) as *const c_char
}

/// `.exit_code(self): Int64`.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
pub unsafe fn processresult_exit_code(ptr: *const i64) -> i64 {
  *ptr.add(2)
}

/// `.success(self): Boolean` — `exit_code == 0`, not a stored field;
/// crosses the FFI boundary as `i64` 0/1, narrowed to `i1` by
/// `emerald-codegen`'s own call site.
///
/// # Safety
/// `ptr` must point to a live, 24-byte, `emerald_alloc`-backed
/// `ProcessResult` block.
pub unsafe fn processresult_success(ptr: *const i64) -> i64 {
  i64::from(*ptr.add(2) == 0)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  #[test]
  fn echo_captures_real_stdout_and_a_zero_exit_code() {
    unsafe {
      let cmd = c("echo");
      let a0 = c("hello");
      let a1 = c("from");
      let a2 = c("emerald");
      let argv = [a0.as_ptr(), a1.as_ptr(), a2.as_ptr()];
      let stdin_data = c("");
      let result_ptr =
        process_run(cmd.as_ptr(), argv.as_ptr(), 3, stdin_data.as_ptr()) as *const i64;
      let stdout = std::ffi::CStr::from_ptr(processresult_stdout(result_ptr))
        .to_str()
        .unwrap();
      assert_eq!(stdout, "hello from emerald\n");
      assert_eq!(processresult_exit_code(result_ptr), 0);
      assert_eq!(processresult_success(result_ptr), 1);
    }
  }

  #[test]
  fn a_nonzero_exit_is_a_real_field_not_an_error() {
    unsafe {
      // `false` is a real, always-present POSIX utility that always
      // exits `1` and never touches stdout/stderr — proves the plain-
      // field path for a genuine nonzero exit without depending on
      // `grep`'s own output-matching semantics.
      let cmd = c("false");
      let stdin_data = c("");
      let result_ptr =
        process_run(cmd.as_ptr(), std::ptr::null(), 0, stdin_data.as_ptr()) as *const i64;
      assert_eq!(processresult_exit_code(result_ptr), 1);
      assert_eq!(processresult_success(result_ptr), 0);
    }
  }

  #[test]
  fn stdin_bytes_are_written_then_closed_and_echoed_back_through_cat() {
    unsafe {
      let cmd = c("cat");
      let stdin_data = c("round trip me\n");
      let result_ptr =
        process_run(cmd.as_ptr(), std::ptr::null(), 0, stdin_data.as_ptr()) as *const i64;
      let stdout = std::ffi::CStr::from_ptr(processresult_stdout(result_ptr))
        .to_str()
        .unwrap();
      assert_eq!(stdout, "round trip me\n");
      assert_eq!(processresult_exit_code(result_ptr), 0);
    }
  }

  #[test]
  fn spawning_a_binary_that_does_not_exist_is_a_real_trigger_for_the_raise_path() {
    // This is the real trigger `process_run`'s own `Command::spawn()`
    // failure path turns into a raised `NativeError` — verified here
    // only up through `std::process::Command`'s own real `Err`, never
    // through `process_run` itself: `raise_native_error` calls
    // `emerald_raise`, an `extern "C" fn` whose real body is only
    // linked into the real `--release` staticlib; this crate's own
    // `#[cfg(test)]` `test_stubs::emerald_raise` aborts the process
    // rather than unwind across the `extern "C"` boundary (see this
    // crate's own `lib.rs` for why), so the full raise-then-rescue
    // path is verified separately via a real `.em` example run
    // through the real CLI (`examples/process_spawning_proof.em`,
    // this plan's own Concrete Proof), not here.
    let result =
      std::process::Command::new("definitely-not-a-real-binary-xyz-emerald-test").spawn();
    assert!(result.is_err());
  }
}
