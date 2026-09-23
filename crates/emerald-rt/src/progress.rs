//! Plan 191 (Progress Bars & Terminal Formatting) — wraps `indicatif`
//! 0.18.6 (`ProgressBar`) and `console` 0.16.6 (`Style`/`Term`), both
//! from the same `console-rs` GitHub organization (`indicatif`'s own
//! progress-bar rendering depends on `console` internally for
//! terminal width/cursor control — a real, checkable upstream
//! coupling, not an arbitrary batching choice; see this plan's own
//! Decision log).
//!
//! `ProgressBar` (the Emerald-visible newtype) is a plan-93
//! resource-handle wrapper over `indicatif::ProgressBar` — genuinely
//! stateful (position, message, render-refresh-timing) over its
//! lifetime, backed by `crate::handle`'s registry the same way
//! `Regex`/`XmlReader`/`CliParser` already are. `.new(total)`/
//! `.new_spinner()` are its two constructors, sharing one handle type
//! (a spinner is simply a `ProgressBar` with no known length, not a
//! separate class — see this plan's own Decision log).
//!
//! `.finish()` folds plan 93's usual explicit `.close()` discipline
//! into the domain-natural call itself (see this plan's own Decision
//! log, the same narrow, disclosed exception plan 147's `Tempfile`
//! auto-cleanup precedent already established): a progress bar's
//! natural end-of-life event (the tracked operation completing) and
//! its resource-cleanup event (stopping the background redraw state)
//! are always the same real moment, so `.finish()` calls
//! `indicatif::ProgressBar::finish()` (which itself stops the
//! background steady-tick and renders the bar's final frame) and then
//! `crate::handle::handle_close` in the same call — no separate
//! `.close()` exists on this class at all.
//!
//! `indicatif::ProgressBar::new`/`::new_spinner` both default to
//! `ProgressDrawTarget::stderr()` (indicatif's own documented
//! default) — kept, not overridden, matching every other CLI tool
//! built on this crate pair. TTY-noise suppression is handled
//! correctly by construction, not by anything this module adds:
//! `ProgressDrawTarget::is_hidden` (`indicatif`'s own
//! `draw_target.rs`, verified directly against the vendored 0.18.6
//! source this session) returns `!term.is_term()` for a real
//! `Term`-backed target, so a bar drawing into a piped/redirected
//! stderr already renders nothing at all — no escape-code noise —
//! with zero extra wiring here.
//!
//! `Console.styled(text, color)` wraps `console::Style` — a bare
//! reserved-namespace static call (`Console` is never a real
//! `ModuleDef`, the identical shape `Json`/`Toml`/`Xml` already use),
//! never a handle: `console::Style::new().<color>().apply_to(text)`
//! auto-detects color support via the crate's own `colors_enabled()`
//! (itself checking `console::Term::stdout().is_term()` plus the
//! `NO_COLOR`/`CLICOLOR_FORCE` environment convention, verified
//! directly against `console`'s own vendored `utils.rs` this
//! session), so under CI's own piped stdout this returns the plain,
//! escape-code-free text automatically — never forced on or off by
//! this wrapper. Eight named colors are supported (`black`/`red`/
//! `green`/`yellow`/`blue`/`magenta`/`cyan`/`white`) — `console::
//! Style`'s own real, complete set of basic ANSI foreground-color
//! builder methods, verified against its vendored `utils.rs`; an
//! unrecognized color name raises `NativeError` naming it, rather
//! than silently falling back to an unstyled string.
//!
//! `Console.is_terminal(): Boolean` wraps `console::Term::stdout().
//! is_term()` directly — a first-class, separately-callable check
//! (this plan's own Decision log), so a caller can decide whether to
//! even construct a `ProgressBar` at all when output isn't a
//! terminal, not just rely on `.styled`'s own internal auto-detection.

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use console::Style;
use indicatif::ProgressBar as IndicatifBar;
use std::os::raw::c_char;

const TAG: &str = "ProgressBar";

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `ProgressBar.new(total: Int64): ProgressBar` — a negative `total`
/// is clamped to `0` (an empty bar, not a native error: `indicatif`'s
/// own `u64` length has no natural negative representation, and a
/// caller-computed length that happens to be negative is far more
/// likely a harmless off-by-one than a program that actually wants a
/// raised exception here).
pub fn progressbar_new(total: i64) -> i64 {
  let bar = IndicatifBar::new(total.max(0) as u64);
  handle_alloc(Box::new(bar), TAG)
}

/// `ProgressBar.new_spinner(): ProgressBar` — for operations with no
/// known total ahead of time (e.g. plan 187's index-building).
pub fn progressbar_new_spinner() -> i64 {
  let bar = IndicatifBar::new_spinner();
  handle_alloc(Box::new(bar), TAG)
}

/// `.increment(self, n: Int64): Void` — a negative `n` is clamped to
/// `0` (a no-op step), the identical reasoning `.new`'s own `total`
/// clamp above documents.
pub fn progressbar_increment(id: i64, n: i64) {
  match handle_get_mut::<IndicatifBar, ()>(id, TAG, |bar| bar.inc(n.max(0) as u64)) {
    Ok(()) => {}
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `.set_message(self, text: String): Void`.
///
/// # Safety
/// `text`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn progressbar_set_message(id: i64, text: *const c_char) {
  let text = match read_str(text) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<IndicatifBar, ()>(id, TAG, |bar| bar.set_message(text)) {
    Ok(()) => {}
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `.finish(self): Void` — see this module's own doc comment: folds
/// plan 93's usual `.close()` into this one domain-natural call.
pub fn progressbar_finish(id: i64) {
  match handle_get_mut::<IndicatifBar, ()>(id, TAG, |bar| bar.finish()) {
    Ok(()) => {}
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
  handle_close(id);
}

fn style_for(color: &str) -> Result<Style, String> {
  let s = Style::new();
  Ok(match color {
    "black" => s.black(),
    "red" => s.red(),
    "green" => s.green(),
    "yellow" => s.yellow(),
    "blue" => s.blue(),
    "magenta" => s.magenta(),
    "cyan" => s.cyan(),
    "white" => s.white(),
    other => {
      return Err(format!(
        "Console.styled: unknown color `{other}` (expected one of black/red/green/yellow/blue/magenta/cyan/white)"
      ))
    }
  })
}

/// `Console.styled(text: String, color: String): String`.
///
/// # Safety
/// `text`/`color`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn console_styled(text: *const c_char, color: *const c_char) -> *const c_char {
  let text = match read_str(text) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let color = match read_str(color) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let style = match style_for(color) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let styled = style.apply_to(text).to_string();
  crate::alloc_and_copy_str(&styled)
}

/// `Console.is_terminal(): Boolean`.
pub fn console_is_terminal() -> i64 {
  if console::Term::stdout().is_term() {
    1
  } else {
    0
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  #[test]
  fn a_fresh_bar_increments_without_error() {
    let id = progressbar_new(100);
    progressbar_increment(id, 1);
    progressbar_increment(id, 9);
    progressbar_finish(id);
  }

  #[test]
  fn a_spinner_accepts_a_message_and_finishes() {
    let id = progressbar_new_spinner();
    let msg = c("indexing...");
    unsafe { progressbar_set_message(id, msg.as_ptr()) };
    progressbar_finish(id);
  }

  #[test]
  fn finish_closes_the_handle_a_second_use_then_fails() {
    let id = progressbar_new(10);
    progressbar_finish(id);
    let err =
      crate::handle::handle_get_mut::<IndicatifBar, ()>(id, TAG, |bar| bar.inc(1)).unwrap_err();
    assert_eq!(err, "use of closed ProgressBar handle");
  }

  // `console::set_colors_enabled` is a real, documented, PROCESS-WIDE
  // global override `console` itself exposes for exactly this purpose
  // — real assertions against known ANSI escape sequences (not a
  // visual-only check). Both the "forced on" and "forced off"
  // assertions live in ONE test function, not two, and a real,
  // disclosed finding from actually running this suite: `cargo test`
  // runs `#[test]` functions on separate threads by default, and two
  // separate tests each flipping this same global `AtomicBool` raced
  // each other nondeterministically (`forced_on`'s own assertion
  // observing `forced_off`'s "disabled" value mid-run) — genuinely
  // flaky, not a one-off fluke, confirmed by re-running the pair
  // repeatedly. Serializing both assertions inside one test function
  // (no other thread's code runs between them) is the fix, not a
  // `Mutex`/`#[serial]` harness addition this crate doesn't otherwise
  // depend on.
  #[test]
  fn styled_respects_the_forced_color_enabled_flag_in_both_directions() {
    let text = c("done");
    let color = c("green");

    console::set_colors_enabled(true);
    let styled_on = unsafe {
      std::ffi::CStr::from_ptr(console_styled(text.as_ptr(), color.as_ptr()))
        .to_str()
        .unwrap()
        .to_string()
    };
    // `console::Color::Green`'s real SGR code is `32`; `console`'s own
    // `Style::apply_to` wraps the text in `\x1b[<codes>m...\x1b[0m`.
    assert_eq!(styled_on, "\x1b[32mdone\x1b[0m");

    console::set_colors_enabled(false);
    let styled_off = unsafe {
      std::ffi::CStr::from_ptr(console_styled(text.as_ptr(), color.as_ptr()))
        .to_str()
        .unwrap()
        .to_string()
    };
    assert_eq!(styled_off, "done");
  }

  #[test]
  fn an_unknown_color_name_raises_rather_than_silently_falling_back() {
    let err = style_for("chartreuse").unwrap_err();
    assert!(err.contains("chartreuse"));
  }
}
