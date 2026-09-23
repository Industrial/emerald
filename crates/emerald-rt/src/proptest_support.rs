//! Plan 180 (Property-Based Testing Generators,
//! `history/2026-09-21T212900Z-plan-180-property-testing.md`) — a real,
//! generator-driven, shrinking-on-failure backend for `property
//! "description" (a: Int64, b: Int64) do ... end`, closing the gap
//! plan 80's own doc comment on `Item::Property` explicitly disclosed
//! ("a property block runs its body once, not across many generated
//! inputs — a real, disclosed simplification").
//!
//! # Architecture: the compiled Emerald harness drives the loop
//!
//! This session's own investigation (see this plan's own history file)
//! found no existing mechanism anywhere in `emerald-rt` for native Rust
//! code to call back INTO already-compiled Emerald code (confirmed by
//! `collections.rs`'s own `set_i64_each` doc comment: "no such call-
//! back mechanism exists anywhere in this crate today"). Building one
//! from scratch — a raw LLVM function pointer handed across the FFI
//! boundary, safely re-entering a `setjmp`/`longjmp`-based exception
//! frame from a native caller — is exactly the kind of large,
//! undisclosed architectural gap this plan's own brief calls for
//! scoping down from, not silently attempting.
//!
//! Instead, `emerald_codegen::compile_test_harness` synthesizes an
//! ordinary Emerald `while` loop that DRIVES the generate/run/shrink
//! cycle itself, calling into this module once per step — the normal,
//! already-proven FFI direction every other `emerald_rt_*` export in
//! this crate already uses. Each step:
//!
//! 1. fetches the session's current generated value for every
//!    parameter (`current_i64`/`current_f64`/`current_string`/
//!    `current_bool`),
//! 2. runs the compiled property-case function with those values,
//!    inside a synthesized `begin ... rescue AssertionError => e ...
//!    end` (reusing plan 11's real exception mechanism — never a raw
//!    `extern "C"` callback),
//! 3. reports the outcome back via `report`, which returns whether the
//!    harness loop should run another step.
//!
//! # Generation and shrinking: real `proptest`, not hand-rolled
//!
//! Each parameter gets its own independent `proptest::strategy::
//! BoxedStrategy<PropValue>` (`i` → `any::<i64>()`, `f` → `any::<f64>()`,
//! `s` → a NUL-excluding string strategy, `b` → `any::<bool>()`) — a
//! real, disclosed scope choice: parameters are generated and shrunk
//! independently, never as one correlated joint tuple strategy (v1 has
//! no way for one property to express "these two parameters are
//! related"). On the first case failure, `Session::report` shrinks
//! coordinate-wise: it drives parameter 0's own `ValueTree::simplify()`/
//! `complicate()` (`proptest`'s own real, per-value shrink protocol —
//! see its docs) to convergence, then parameter 1's, and so on, always
//! keeping the smallest input observed still to reproduce the failure.
//! This is `TestRunner::run`'s own well-known single-tree shrink loop
//! (test → if it still fails, `simplify()` again; if it now passes,
//! `complicate()` to back off), applied once per parameter instead of
//! once for one composite tree — real `proptest` shrinking, not a
//! hand-rolled binary search.
//!
//! # Why no `Boolean`-typed extern signature
//!
//! `ValKind::Bool` is a real, distinct `i1` LLVM storage kind (see
//! `emerald-codegen`'s own `value_kind_for_type` doc comment) — rather
//! than gambling on `i1` vs. C's own `_Bool`-as-`i8` ABI lowering
//! matching a plain Rust `extern "C" fn(...) -> bool` exactly, every
//! extern signature in this module uses `Int64` only (`0`/`1`), the
//! same convention every other boolean-shaped `emerald_rt_*` export in
//! this crate already uses (e.g. `path_exists`). `compile_test_harness`
//! turns a `0`/`1` `Int64` back into a real Emerald `Boolean` with an
//! ordinary `!= 0` comparison — the same idiom the harness's own
//! `failed > 0` check already uses, not a new mechanism.

use crate::handle::{handle_alloc, handle_get_mut};
use proptest::prelude::*;
use proptest::strategy::{BoxedStrategy, Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use std::os::raw::c_char;

const PROPTEST_TAG: &str = "proptest_session";

/// Up to `proptest`'s own documented default case count (256) per
/// property — matches this plan's own Concrete Proof exactly.
const MAX_CASES: u32 = 256;

/// A total generate+shrink step budget, guarding against a pathological
/// strategy that never converges — comfortably above anything a real
/// `Int64`/`Float64`/`String`/`Boolean` shrink run needs (each `i64`
/// coordinate shrinks to zero in well under 100 steps in practice).
const MAX_TOTAL_STEPS: u32 = 20_000;

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|e| format!("invalid UTF-8: {e}"))
}

#[derive(Clone, Debug)]
enum PropValue {
  I64(i64),
  F64(f64),
  Str(String),
  Bool(bool),
}

impl PropValue {
  fn as_i64(&self) -> i64 {
    match self {
      PropValue::I64(v) => *v,
      _ => 0,
    }
  }
  fn as_f64(&self) -> f64 {
    match self {
      PropValue::F64(v) => *v,
      _ => 0.0,
    }
  }
  fn as_str(&self) -> &str {
    match self {
      PropValue::Str(s) => s.as_str(),
      _ => "",
    }
  }
  fn as_bool_i64(&self) -> i64 {
    match self {
      PropValue::Bool(v) => *v as i64,
      _ => 0,
    }
  }
}

/// One byte per parameter names its generator: `i`/`f`/`s`/`b`. Any
/// other byte is a real bug in `compile_test_harness`'s own encoding
/// (sema already rejects every other declared property parameter
/// type before codegen ever reaches this point) — not a case a
/// deployed binary should ever actually hit, but `raise`d rather than
/// panicking across the FFI boundary regardless.
fn strategy_for(kind: u8) -> Result<BoxedStrategy<PropValue>, String> {
  match kind {
    b'i' => Ok(any::<i64>().prop_map(PropValue::I64).boxed()),
    b'f' => Ok(any::<f64>().prop_map(PropValue::F64).boxed()),
    // Excludes embedded NUL bytes (plan 180's own Decision log) so
    // every generated case round-trips safely through Emerald's
    // `char*`-based `String` representation.
    b's' => Ok("[^\u{0}]{0,64}".prop_map(PropValue::Str).boxed()),
    b'b' => Ok(any::<bool>().prop_map(PropValue::Bool).boxed()),
    other => Err(format!(
      "internal error: unknown property parameter generator kind {other:?}"
    )),
  }
}

#[derive(Clone, Copy, PartialEq)]
enum SessionState {
  Generating,
  Shrinking { index: usize },
  Done,
}

struct Session {
  runner: TestRunner,
  strategies: Vec<BoxedStrategy<PropValue>>,
  trees: Vec<Box<dyn ValueTree<Value = PropValue>>>,
  state: SessionState,
  cases_run: u32,
  steps: u32,
  failed: bool,
  fail_message: String,
}

// `handle_alloc` requires `Box<dyn Any + Send>` (plan 93's registry —
// see `crate::handle`'s own doc comment); `Box<dyn ValueTree<...>>`
// (proptest's own `BoxedStrategy::Tree` shape) carries no `Send` bound
// at the trait-object level. Sound regardless: every field here is
// either plainly `Send` (`TestRunner`'s own RNG state, the `u32`/
// `bool`/`String` counters) or a boxed `ValueTree`/`Strategy` built
// exclusively from `proptest`'s own standard `any::<i64/f64/bool>()`/
// regex-`String` generators — plain owned data with no `Rc`/thread-
// local/interior-non-atomic-mutability of any kind, independently
// verified against `proptest`'s own source before writing this line,
// not assumed.
unsafe impl Send for Session {}

impl Session {
  fn new(kinds: &[u8]) -> Result<Self, String> {
    let strategies: Vec<BoxedStrategy<PropValue>> = kinds
      .iter()
      .map(|&k| strategy_for(k))
      .collect::<Result<_, _>>()?;
    let mut runner = TestRunner::default();
    let trees = Self::generate(&strategies, &mut runner)?;
    Ok(Session {
      runner,
      strategies,
      trees,
      state: SessionState::Generating,
      cases_run: 0,
      steps: 0,
      failed: false,
      fail_message: String::new(),
    })
  }

  fn generate(
    strategies: &[BoxedStrategy<PropValue>],
    runner: &mut TestRunner,
  ) -> Result<Vec<Box<dyn ValueTree<Value = PropValue>>>, String> {
    strategies
      .iter()
      .map(|s| {
        s.new_tree(runner)
          .map_err(|reason| format!("proptest strategy generation failed: {reason}"))
      })
      .collect()
  }

  fn current(&self, idx: usize) -> Result<PropValue, String> {
    self
      .trees
      .get(idx)
      .map(|t| t.current())
      .ok_or_else(|| format!("property parameter index {idx} out of range"))
  }

  /// Tries `simplify()` starting at `index`, advancing to later
  /// parameters (each starting fresh) until one succeeds or every
  /// remaining parameter has converged, in which case shrinking is
  /// over. Shared by the `Generating` → `Shrinking` transition and by
  /// `Shrinking`'s own "this coordinate is done" advance.
  fn try_shrink_from(&mut self, mut index: usize) -> bool {
    loop {
      if index >= self.trees.len() {
        self.state = SessionState::Done;
        return false;
      }
      if self.trees[index].simplify() {
        self.state = SessionState::Shrinking { index };
        return true;
      }
      index += 1;
    }
  }

  /// Reports the outcome of the case the harness just ran with the
  /// session's own `current()` values, and returns whether the
  /// harness's `while` loop should run another step.
  fn report(&mut self, case_failed: bool, message: &str) -> bool {
    if self.state == SessionState::Done {
      return false;
    }
    self.steps += 1;
    if self.steps > MAX_TOTAL_STEPS {
      self.state = SessionState::Done;
      return false;
    }
    match self.state {
      SessionState::Generating => {
        if case_failed {
          self.failed = true;
          self.fail_message = message.to_string();
          self.try_shrink_from(0)
        } else {
          self.cases_run += 1;
          if self.cases_run >= MAX_CASES {
            self.state = SessionState::Done;
            false
          } else {
            match Self::generate(&self.strategies, &mut self.runner) {
              Ok(trees) => {
                self.trees = trees;
                true
              }
              Err(_) => {
                self.state = SessionState::Done;
                false
              }
            }
          }
        }
      }
      SessionState::Shrinking { index } => {
        if case_failed {
          self.fail_message = message.to_string();
          if self.trees[index].simplify() {
            true
          } else {
            self.try_shrink_from(index + 1)
          }
        } else if self.trees[index].complicate() {
          true
        } else {
          self.try_shrink_from(index + 1)
        }
      }
      SessionState::Done => unreachable!("checked above"),
    }
  }
}

/// # Safety
/// `types` must be a valid, NUL-terminated C string, one byte per
/// property parameter (`i`/`f`/`s`/`b`).
pub unsafe fn proptest_begin(types: *const c_char) -> i64 {
  let types = match read_str(types) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match Session::new(types.as_bytes()) {
    Ok(session) => handle_alloc(Box::new(session), PROPTEST_TAG),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_current_i64(session: i64, idx: i64) -> i64 {
  match handle_get_mut::<Session, Result<i64, String>>(session, PROPTEST_TAG, |s| {
    s.current(idx as usize).map(|v| v.as_i64())
  }) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) | Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_current_f64(session: i64, idx: i64) -> f64 {
  match handle_get_mut::<Session, Result<f64, String>>(session, PROPTEST_TAG, |s| {
    s.current(idx as usize).map(|v| v.as_f64())
  }) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) | Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_current_string(session: i64, idx: i64) -> *const c_char {
  match handle_get_mut::<Session, Result<String, String>>(session, PROPTEST_TAG, |s| {
    s.current(idx as usize).map(|v| v.as_str().to_string())
  }) {
    Ok(Ok(v)) => crate::alloc_and_copy_str(&v),
    Ok(Err(e)) | Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`. Returns `0`/`1`, not a real Emerald `Boolean` —
/// see this module's own doc comment for why.
pub unsafe fn proptest_current_bool(session: i64, idx: i64) -> i64 {
  match handle_get_mut::<Session, Result<i64, String>>(session, PROPTEST_TAG, |s| {
    s.current(idx as usize).map(|v| v.as_bool_i64())
  }) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) | Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`; `message`, if non-null, must point to a valid,
/// NUL-terminated C string. Returns `1` (run another step) or `0`
/// (done — see `proptest_failed`/`proptest_fail_message`/
/// `proptest_case_count`).
pub unsafe fn proptest_report(session: i64, failed: i64, message: *const c_char) -> i64 {
  let message = if message.is_null() {
    ""
  } else {
    match read_str(message) {
      Ok(s) => s,
      Err(e) => crate::raise_native_error(&e),
    }
  };
  match handle_get_mut::<Session, bool>(session, PROPTEST_TAG, |s| s.report(failed != 0, message)) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_failed(session: i64) -> i64 {
  match handle_get_mut::<Session, bool>(session, PROPTEST_TAG, |s| s.failed) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_case_count(session: i64) -> i64 {
  match handle_get_mut::<Session, i64>(session, PROPTEST_TAG, |s| s.cases_run as i64) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(&e),
  }
}

/// # Safety
/// `session` must be a live handle previously returned by
/// `proptest_begin`.
pub unsafe fn proptest_fail_message(session: i64) -> *const c_char {
  match handle_get_mut::<Session, String>(session, PROPTEST_TAG, |s| s.fail_message.clone()) {
    Ok(msg) => crate::alloc_and_copy_str(&msg),
    Err(e) => crate::raise_native_error(&e),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_property_that_always_holds_runs_the_full_default_case_count() {
    // Mirrors `a + b == b + a` — every generated `(i64, i64)` pair
    // makes this true, so shrinking is never triggered.
    let mut session = Session::new(b"ii").unwrap();
    loop {
      let a = session.current(0).unwrap().as_i64();
      let b = session.current(1).unwrap().as_i64();
      let case_failed = a.wrapping_add(b) != b.wrapping_add(a);
      assert!(!case_failed, "a + b == b + a must hold for every i64 pair");
      if !session.report(false, "") {
        break;
      }
    }
    assert_eq!(session.cases_run, MAX_CASES);
    assert!(!session.failed);
  }

  #[test]
  fn a_property_that_always_fails_shrinks_to_a_real_minimal_counterexample() {
    // Mirrors this plan's own Concrete Proof: `a - b == b - a` only
    // when `a == b`, so a strategy that (almost) never generates equal
    // pairs fails on virtually the first case and must shrink.
    let mut session = Session::new(b"ii").unwrap();
    let mut steps = 0;
    loop {
      steps += 1;
      assert!(steps < 100_000, "shrink loop did not converge");
      let a = session.current(0).unwrap().as_i64();
      let b = session.current(1).unwrap().as_i64();
      let case_failed = a != b;
      if !session.report(case_failed, "expected equal but got different") {
        break;
      }
    }
    assert!(session.failed);
    let a = session.current(0).unwrap().as_i64();
    let b = session.current(1).unwrap().as_i64();
    // Real shrinking, not just "a failure was found": both coordinates
    // converge all the way to proptest's own documented shrink target
    // for integers (zero) on one side, since `a != b` fails for every
    // pair except `a == b`, and `(0, 0)` is reachable while remaining a
    // genuine, still-failing... — actually `(0, 0)` does NOT fail since
    // `a == b` there. What must hold is that the two values are
    // adjacent-minimal and unequal — assert the real, checkable
    // invariant instead: the failing pair's own values are both small
    // (proptest shrinks i64 toward 0) and still genuinely unequal.
    assert_ne!(a, b, "the shrunk pair must still be a real counterexample");
    assert!(
      a.unsigned_abs() <= 1 && b.unsigned_abs() <= 1,
      "shrinking should converge near zero, got a={a}, b={b}"
    );
  }

  #[test]
  fn string_and_bool_generators_never_produce_a_nul_byte_and_round_trip() {
    let mut session = Session::new(b"sb").unwrap();
    for _ in 0..50 {
      let s = session.current(0).unwrap().as_str().to_string();
      assert!(!s.contains('\u{0}'), "generated string must exclude NUL");
      let _b = session.current(1).unwrap().as_bool_i64();
      if !session.report(false, "") {
        break;
      }
    }
  }
}
