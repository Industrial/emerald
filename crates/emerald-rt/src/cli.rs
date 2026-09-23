//! Plan 182 (Structured CLI Flag Parsing) — `CliParser`/`CliParseResult`,
//! a builder-sequence wrapper over `clap`'s non-derive builder API
//! (`clap::Command`/`clap::Arg`/`clap::ArgAction` — no `derive` feature
//! enabled, since Emerald has no macro/attribute/derive system to
//! mirror `#[derive(Parser)]`'s usual ergonomics; see this plan's own
//! Decision log). Two sibling plan-93 resource-handle newtypes, backed
//! by `crate::handle`'s registry directly (the same shape `Regex`/
//! `Tempfile` already use) — `CliParser` holds a `clap::Command` being
//! built up by `.flag`/`.option`/`.positional` calls; `CliParseResult`
//! holds this module's own private `ParseOutcome`, produced once by
//! `.parse` and never mutated again.
//!
//! `CliParser.parse` consumes `ARGV: Array[String]`/`ARGC: Int64`
//! (plan 45) directly, at the call site — no second, parallel
//! argument-collection mechanism.
//!
//! Builder methods mutate the stored `clap::Command` in place via
//! `crate::handle::handle_get_mut` plus a `std::mem::replace`-with-a-
//! placeholder swap: `clap::Command`'s own builder methods (`.arg`,
//! ...) consume `self` by value and return a new `Command`, so
//! wrapping one behind a stable, mutable handle needs exactly this
//! "take it out, rebuild it, put it back" shape — a real, disclosed
//! workaround, not a hidden inefficiency (see this plan's own
//! Decision log).
//!
//! `.parse` clones the stored `Command` for each call rather than
//! consuming the handle's own slot — an Emerald program may
//! reasonably want to call `.parse` more than once against the same
//! spec (e.g. a test harness driving several fixed `argv` fixtures),
//! so `CliParser` stays reusable after `.parse` returns, unlike a
//! plain `Command::get_matches`-style one-shot consuming API.
//!
//! `--help` never calls `std::process::exit` on Emerald's behalf —
//! `clap`'s own default `get_matches`-family flow does exactly that
//! from inside its own error path; this module uses
//! `try_get_matches_from` instead and inspects the returned `Err`'s
//! own `ErrorKind` directly, surfacing `--help` as an ordinary,
//! inspectable `CliParseResult` field (`.help_requested()`/
//! `.help_text()`) rather than letting `clap` unilaterally terminate
//! the whole host process — exactly the class of hazard plan 92's own
//! `catch_and_raise`-at-every-boundary posture exists to rule out,
//! applied here to a deliberate `exit()` call rather than an
//! unwinding panic. A missing required option (or any other real parse
//! failure) is likewise never raised as a `NativeError` — it is the
//! single most expected, most routine outcome a CLI parser produces,
//! so it travels back as `.error_message(): Option[String]`, an
//! ordinary field an Emerald program branches on with `case`/`when`,
//! not an exception every CLI tool would have to `rescue`.
//!
//! `help_text` is computed unconditionally at `.parse` time (`Command::
//! render_help()`, captured before `ArgMatches` is even produced),
//! never lazily on the first `.help_text()` call — cheap relative to
//! the parse itself, and it means `.help_text()` never needs its own
//! fallible path.
//!
//! `argv`/`argc` marshaling: identical to `process.rs`'s own
//! `Process.run` — `emerald-codegen`'s own call-site codegen skips
//! `Array[String]`'s own `[len: i64]` header and passes the bare
//! element buffer straight through as `argv: *const *const c_char`,
//! alongside the caller-supplied `argc: Int64` travelling as its own
//! explicit parameter (`ARGV`/`ARGC` themselves, unmodified, no copy,
//! no re-derivation via `std::env::args()` — see this plan's own
//! Decision log for why that would silently risk diverging from
//! `ARGV`'s own already-established argv[0]-skipping convention).
//! `clap` itself expects the FIRST item of whatever iterator it parses
//! to be the program's own name (used only for the usage line, never
//! validated against anything) — since `ARGV` already excludes it
//! (plan 45's own convention), this module prepends the `Command`'s
//! own `.get_name()` back on before calling `try_get_matches_from`,
//! rather than losing the real first CLI argument to clap's own
//! name-skipping assumption.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use clap::error::ErrorKind;
use clap::{Arg, ArgAction, Command};
use std::ffi::c_void;
use std::os::raw::c_char;

const PARSER_TAG: &str = "CliParser";
const RESULT_TAG: &str = "CliParseResult";
const OPTION_SOME: i64 = 0;
const OPTION_NONE: i64 = 1;

/// `.parse`'s own real output — never `clap::ArgMatches` alone, since
/// a real parse failure or a `--help` request produces no `ArgMatches`
/// at all under `try_get_matches_from`'s own `Result` shape, and this
/// module needs to answer `.flag`/`.value`/`.positional_value` with a
/// plain `false`/`None` (never a panic) on EITHER of those paths, not
/// just the success path.
struct ParseOutcome {
  matches: Option<clap::ArgMatches>,
  help_requested: bool,
  help_text: String,
  error_message: Option<String>,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

unsafe fn alloc_option_string(v: Option<&str>) -> *mut c_void {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  match v {
    Some(s) => {
      *ptr = OPTION_SOME;
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
    }
    None => *ptr = OPTION_NONE,
  }
  ptr as *mut c_void
}

/// `CliParser.new(name: String, version: String): CliParser`.
///
/// # Safety
/// `name`/`version`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn cliparser_new(name: *const c_char, version: *const c_char) -> i64 {
  let name = match read_str(name) {
    Ok(n) => n,
    Err(e) => crate::raise_native_error(&e),
  };
  let version = match read_str(version) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(&e),
  };
  // A real, disclosed correction found only by actually running this
  // plan's own example: `clap` 4's own DEFAULT help template
  // (`clap_builder::output::help_template::DEFAULT_TEMPLATE`) does
  // NOT include a `{name} {version}` banner line at all — a real v4
  // behavior change from v2/v3's own template, verified directly
  // against `clap_builder`'s own source in this exact pinned version
  // — so this plan's own Concrete Proof requirement (the literal
  // string `1.0.0` present in `--help`'s own output, not just
  // `--version`'s) needs an explicit `.help_template` override to
  // reintroduce that banner line.
  let cmd = Command::new(name.to_string())
    .version(version.to_string())
    .help_template(
      "{before-help}{name} {version}\n{about-with-newline}\n{usage-heading} {usage}\n\n{all-args}{after-help}",
    );
  handle_alloc(Box::new(cmd), PARSER_TAG)
}

/// Mutates the stored `Command` in place — see this module's own doc
/// comment for the `mem::replace`-with-a-placeholder shape every
/// builder method here shares (`clap::Command`'s own builder methods
/// consume `self` by value; a live handle needs a stable slot).
unsafe fn mutate_command(id: i64, f: impl FnOnce(Command) -> Command) {
  match handle_get_mut::<Command, ()>(id, PARSER_TAG, |slot| {
    let cmd = std::mem::replace(slot, Command::new(""));
    *slot = f(cmd);
  }) {
    Ok(()) => {}
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.flag(self, long: String, short: String, help: String): Void`.
///
/// # Safety
/// `long`/`short`/`help`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn cliparser_flag(
  id: i64,
  long: *const c_char,
  short: *const c_char,
  help: *const c_char,
) {
  let long = match read_str(long) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let short = match read_str(short) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let help = match read_str(help) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_command(id, move |cmd| {
    let mut arg = Arg::new(long.clone())
      .long(long)
      .help(help)
      .action(ArgAction::SetTrue);
    if let Some(c) = short.chars().next() {
      arg = arg.short(c);
    }
    cmd.arg(arg)
  });
}

/// `.option(self, long: String, short: String, help: String, required:
/// Boolean): Void`.
///
/// # Safety
/// `long`/`short`/`help`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn cliparser_option(
  id: i64,
  long: *const c_char,
  short: *const c_char,
  help: *const c_char,
  required: i64,
) {
  let long = match read_str(long) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let short = match read_str(short) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let help = match read_str(help) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_command(id, move |cmd| {
    let mut arg = Arg::new(long.clone())
      .long(long)
      .help(help)
      .action(ArgAction::Set)
      .required(required != 0);
    if let Some(c) = short.chars().next() {
      arg = arg.short(c);
    }
    cmd.arg(arg)
  });
}

/// `.positional(self, name: String, help: String, required: Boolean):
/// Void` — no `.long()`/`.short()`, clap's own convention for a
/// positional argument.
///
/// # Safety
/// `name`/`help`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn cliparser_positional(
  id: i64,
  name: *const c_char,
  help: *const c_char,
  required: i64,
) {
  let name = match read_str(name) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  let help = match read_str(help) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  mutate_command(id, move |cmd| {
    cmd.arg(Arg::new(name).help(help).required(required != 0))
  });
}

/// `.parse(self, argv: Array[String], argc: Int64): CliParseResult`.
///
/// # Safety
/// `id` must be a live `CliParser` handle. `argv` must point to a
/// buffer of at least `argc` valid, NUL-terminated C string pointers —
/// `emerald-codegen`'s own call-site codegen guarantees this (see this
/// module's own doc comment); when `argc` is `0`, `argv` is never
/// dereferenced and may be null.
pub unsafe fn cliparser_parse(id: i64, argv: *const *const c_char, argc: i64) -> i64 {
  let cmd = match handle_get_mut::<Command, Command>(id, PARSER_TAG, |c| c.clone()) {
    Ok(c) => c,
    Err(e) => crate::raise_native_error(&e),
  };

  let mut items: Vec<String> = Vec::with_capacity(argc.max(0) as usize + 1);
  items.push(cmd.get_name().to_string());
  for i in 0..argc {
    let arg_ptr = *argv.add(i as usize);
    items.push(
      std::ffi::CStr::from_ptr(arg_ptr)
        .to_string_lossy()
        .into_owned(),
    );
  }

  // Rendered from a fresh clone, before `try_get_matches_from` below
  // consumes `cmd` itself — captured unconditionally, per this
  // module's own doc comment, not only on an actual `--help` request.
  // `render_long_help` (the real `--help` rendering), never `render_
  // help` (`-h`'s own short form) — a real, disclosed correction
  // found only by actually running this plan's own example: `clap`'s
  // own short-help template omits the `{name} {version}` banner line
  // entirely, so this plan's own Concrete Proof requirement (the
  // literal string `1.0.0` present in `.help_text()`'s own output)
  // fails under the short form even though both forms otherwise list
  // every registered flag.
  let help_text = cmd.clone().render_long_help().to_string();

  let outcome = match cmd.try_get_matches_from(items) {
    Ok(matches) => ParseOutcome {
      matches: Some(matches),
      help_requested: false,
      help_text,
      error_message: None,
    },
    Err(e) if e.kind() == ErrorKind::DisplayHelp => ParseOutcome {
      matches: None,
      help_requested: true,
      help_text,
      error_message: None,
    },
    Err(e) => ParseOutcome {
      matches: None,
      help_requested: false,
      help_text,
      error_message: Some(e.to_string()),
    },
  };

  handle_alloc(Box::new(outcome), RESULT_TAG)
}

/// `.close(self): Void` — see `crate::handle::handle_close`'s own
/// doc comment: double-close is a harmless no-op, never an error.
pub fn cliparser_close(id: i64) {
  handle_close(id);
}

/// `.flag(self, long: String): Boolean` — a plain `bool`, never
/// `Option` (a flag not passed is simply `false`).
///
/// # Safety
/// `id` must be a live `CliParseResult` handle. `long`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn cliparseresult_flag(id: i64, long: *const c_char) -> i64 {
  let long = match read_str(long) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ParseOutcome, bool>(id, RESULT_TAG, |o| {
    o.matches
      .as_ref()
      .map(|m| m.get_flag(long))
      .unwrap_or(false)
  }) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.value(self, long: String): Option[String]`.
///
/// # Safety
/// `id` must be a live `CliParseResult` handle. `long`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn cliparseresult_value(id: i64, long: *const c_char) -> *mut c_void {
  let long = match read_str(long) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ParseOutcome, Option<String>>(id, RESULT_TAG, |o| {
    o.matches
      .as_ref()
      .and_then(|m| m.get_one::<String>(long).cloned())
  }) {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.positional_value(self, name: String): Option[String]`.
///
/// # Safety
/// `id` must be a live `CliParseResult` handle. `name`, if non-null,
/// must point to a valid, NUL-terminated C string.
pub unsafe fn cliparseresult_positional_value(id: i64, name: *const c_char) -> *mut c_void {
  let name = match read_str(name) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<ParseOutcome, Option<String>>(id, RESULT_TAG, |o| {
    o.matches
      .as_ref()
      .and_then(|m| m.get_one::<String>(name).cloned())
  }) {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.help_requested(self): Boolean`.
///
/// # Safety
/// `id` must be a live `CliParseResult` handle.
pub unsafe fn cliparseresult_help_requested(id: i64) -> i64 {
  match handle_get_mut::<ParseOutcome, bool>(id, RESULT_TAG, |o| o.help_requested) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.help_text(self): String`.
///
/// # Safety
/// `id` must be a live `CliParseResult` handle.
pub unsafe fn cliparseresult_help_text(id: i64) -> *const c_char {
  match handle_get_mut::<ParseOutcome, String>(id, RESULT_TAG, |o| o.help_text.clone()) {
    Ok(s) => crate::alloc_and_copy_str(&s),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.error_message(self): Option[String]`.
///
/// # Safety
/// `id` must be a live `CliParseResult` handle.
pub unsafe fn cliparseresult_error_message(id: i64) -> *mut c_void {
  match handle_get_mut::<ParseOutcome, Option<String>>(id, RESULT_TAG, |o| o.error_message.clone())
  {
    Ok(v) => alloc_option_string(v.as_deref()),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.close(self): Void`.
pub fn cliparseresult_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn argv_of(items: &[CString]) -> Vec<*const c_char> {
    items.iter().map(|c| c.as_ptr()).collect()
  }

  unsafe fn opt_string(ptr: *const i64) -> Option<String> {
    if *ptr == OPTION_SOME {
      let s_ptr = *(ptr.add(1) as *const *const c_char);
      Some(
        std::ffi::CStr::from_ptr(s_ptr)
          .to_str()
          .unwrap()
          .to_string(),
      )
    } else {
      None
    }
  }

  #[test]
  fn a_full_successful_parse_reads_back_the_flag_option_and_positional() {
    unsafe {
      let name = c("greet");
      let version = c("1.0.0");
      let id = cliparser_new(name.as_ptr(), version.as_ptr());

      let (long, short, help) = (c("verbose"), c("v"), c("print extra detail"));
      cliparser_flag(id, long.as_ptr(), short.as_ptr(), help.as_ptr());

      let (long, short, help) = (c("name"), c("n"), c("who to greet"));
      cliparser_option(id, long.as_ptr(), short.as_ptr(), help.as_ptr(), 1);

      let (pname, phelp) = (c("suffix"), c("a trailing word"));
      cliparser_positional(id, pname.as_ptr(), phelp.as_ptr(), 0);

      let args = [c("--name"), c("Ada"), c("--verbose"), c("extra")];
      let argv = argv_of(&args);
      let result_id = cliparser_parse(id, argv.as_ptr(), argv.len() as i64);

      assert_eq!(cliparseresult_help_requested(result_id), 0);
      let err = cliparseresult_error_message(result_id) as *const i64;
      assert!(opt_string(err).is_none());

      let name_val = cliparseresult_value(result_id, c("name").as_ptr()) as *const i64;
      assert_eq!(opt_string(name_val), Some("Ada".to_string()));

      assert_eq!(cliparseresult_flag(result_id, c("verbose").as_ptr()), 1);

      let suffix_val =
        cliparseresult_positional_value(result_id, c("suffix").as_ptr()) as *const i64;
      assert_eq!(opt_string(suffix_val), Some("extra".to_string()));

      cliparser_close(id);
      cliparseresult_close(result_id);
    }
  }

  #[test]
  fn a_missing_required_option_is_a_real_error_message_not_a_raise() {
    unsafe {
      let name = c("greet");
      let version = c("1.0.0");
      let id = cliparser_new(name.as_ptr(), version.as_ptr());
      let (long, short, help) = (c("name"), c("n"), c("who to greet"));
      cliparser_option(id, long.as_ptr(), short.as_ptr(), help.as_ptr(), 1);

      let result_id = cliparser_parse(id, std::ptr::null(), 0);

      assert_eq!(cliparseresult_help_requested(result_id), 0);
      let err = cliparseresult_error_message(result_id) as *const i64;
      let msg = opt_string(err).expect("missing required option should produce an error message");
      assert!(!msg.is_empty());

      cliparser_close(id);
      cliparseresult_close(result_id);
    }
  }

  #[test]
  fn a_help_flag_sets_help_requested_and_a_non_empty_help_text() {
    unsafe {
      let name = c("greet");
      let version = c("1.0.0");
      let id = cliparser_new(name.as_ptr(), version.as_ptr());
      let (long, short, help) = (c("verbose"), c("v"), c("print extra detail"));
      cliparser_flag(id, long.as_ptr(), short.as_ptr(), help.as_ptr());

      let args = [c("--help")];
      let argv = argv_of(&args);
      let result_id = cliparser_parse(id, argv.as_ptr(), argv.len() as i64);

      assert_eq!(cliparseresult_help_requested(result_id), 1);
      let help_text_ptr = cliparseresult_help_text(result_id);
      let help_text = std::ffi::CStr::from_ptr(help_text_ptr).to_str().unwrap();
      assert!(!help_text.is_empty());
      assert!(help_text.contains("greet"));
      // A real, disclosed correction found only by actually running
      // this plan's own example (`examples/cli_flag_parsing.em`):
      // clap 4's own default help template omits the `{name}
      // {version}` banner line entirely, unlike v2/v3 — `cliparser_
      // new`'s own explicit `.help_template` override reintroduces it,
      // so the plan's own Concrete Proof requirement (`--help`'s own
      // output containing the literal `1.0.0`) holds; asserted here
      // too, not just in the end-to-end example test.
      assert!(help_text.contains("1.0.0"));

      cliparser_close(id);
      cliparseresult_close(result_id);
    }
  }

  #[test]
  fn double_close_on_both_handles_is_a_harmless_no_op() {
    unsafe {
      let name = c("greet");
      let version = c("1.0.0");
      let id = cliparser_new(name.as_ptr(), version.as_ptr());
      let result_id = cliparser_parse(id, std::ptr::null(), 0);
      cliparser_close(id);
      cliparser_close(id);
      cliparseresult_close(result_id);
      cliparseresult_close(result_id);
    }
  }
}
