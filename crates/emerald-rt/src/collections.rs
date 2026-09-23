//! Plan 193 (`Set[T]`, `Deque[T]`, `PriorityQueue[T]`) — the three
//! "missing collection primitives" inception-3 §4.3 names, routed
//! through plan 93's `crate::handle` resource-handle registry rather
//! than hand-rolled LLVM memory the way `Array[T]`/`Hash[K,V]`
//! themselves are (see this plan's own Decision log,
//! `history/2026-09-22T223900Z-plan-193-set-deque-priority-queue.md`,
//! for why that split is deliberate, not an oversight). Each of the
//! three wraps Rust's own `std::collections` type directly — `HashSet`,
//! `VecDeque`, `BinaryHeap` — no external crate.
//!
//! Monomorphized per-element-type dispatch (Decision log): every
//! `(kind, element type)` pair gets its own concrete instantiation and
//! its own `emerald_rt_<kind>_<elemtype>_<method>` export (wired in
//! `lib.rs`, not here — this file holds the real, plain-Rust-named
//! logic every wrapper there calls, the same split `regex.rs`/`aead.rs`
//! already establish). The generic private helpers below (`set_new`,
//! `deque_push_front`, ...) exist purely to keep those concrete
//! exports one-line thin wrappers — never exported themselves, and
//! never crossing the FFI boundary directly (an `extern "C" fn` needs
//! a concrete, non-generic signature).
//!
//! Scope, per the Decision log: `Set[T]`/`PriorityQueue[T]` are
//! `Int64`/`String` only (`Float64`'s `f64` has no `Eq`/`Hash`/`Ord` —
//! a real Rust limitation, not a self-imposed one; `Boolean` is a
//! low-value element out of v1 scope). `Deque[T]` carries neither
//! constraint, so it covers all four of `derive Serializable`'s own
//! already-supported primitive types: `Int64`/`Float64`/`String`/
//! `Boolean`.
//!
//! `PriorityQueue[T]` is a max-heap — Rust's own `BinaryHeap` default,
//! unmodified (no `std::cmp::Reverse`-wrapped min-heap variant here;
//! Decision log).

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use std::collections::{BinaryHeap, HashSet, VecDeque};
use std::ffi::c_void;
use std::hash::Hash;
use std::os::raw::c_char;

const SET_I64_TAG: &str = "Set_i64";
const SET_STRING_TAG: &str = "Set_string";
const DEQUE_I64_TAG: &str = "Deque_i64";
const DEQUE_F64_TAG: &str = "Deque_f64";
const DEQUE_STRING_TAG: &str = "Deque_string";
const DEQUE_BOOL_TAG: &str = "Deque_bool";
const PQ_I64_TAG: &str = "PriorityQueue_i64";
const PQ_STRING_TAG: &str = "PriorityQueue_string";

/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
/// A private, per-module copy of `regex.rs`'s own identically-named
/// helper — this crate's own established "duplicate the bookkeeping
/// per domain module" convention (see `emerald-codegen`'s `mangle_
/// type_expr` doc comment for the same disclosed precedent), not an
/// oversight.
unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Array[Int64]`'s own raw layout (`[len: i64][elements: i64, ...]`,
/// `emerald-codegen`'s `build_array_lit` convention) — `Set[Int64]
/// #each`'s materialized snapshot reuses this byte-for-byte, the same
/// way `regex.rs`'s `.find_all`/`.split` already reuse it for
/// `Array[String]`.
unsafe fn build_i64_array(items: &[i64]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  for (i, v) in items.iter().enumerate() {
    *ptr.add(1 + i) = *v;
  }
  ptr as *mut c_void
}

/// `Array[String]`'s own raw layout — copied verbatim from `regex.rs`'s
/// own `build_string_array`.
unsafe fn build_string_array(items: &[&str]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, s) in items.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(s);
  }
  ptr as *mut c_void
}

// ---------------------------------------------------------------------
// Set[T] — Int64, String.
// ---------------------------------------------------------------------

fn set_new<T: 'static + Eq + Hash + Send>(tag: &'static str) -> i64 {
  handle_alloc(Box::new(HashSet::<T>::new()), tag)
}

/// `true` (`1`) iff `v` was not already present, mirroring `HashSet::
/// insert`'s own real return value — a genuinely useful signal, not
/// just discarded `Void`.
unsafe fn set_add<T: 'static + Eq + Hash + Send>(id: i64, tag: &'static str, v: T) -> i64 {
  match handle_get_mut::<HashSet<T>, bool>(id, tag, |s| s.insert(v)) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn set_contains<T: 'static + Eq + Hash + Send>(id: i64, tag: &'static str, v: T) -> i64 {
  match handle_get_mut::<HashSet<T>, bool>(id, tag, |s| s.contains(&v)) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

/// `true` (`1`) iff `v` was present (and so actually removed),
/// mirroring `HashSet::remove`'s own real return value.
unsafe fn set_remove<T: 'static + Eq + Hash + Send>(id: i64, tag: &'static str, v: T) -> i64 {
  match handle_get_mut::<HashSet<T>, bool>(id, tag, |s| s.remove(&v)) {
    Ok(true) => 1,
    Ok(false) => 0,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn set_count<T: 'static + Eq + Hash + Send>(id: i64, tag: &'static str) -> i64 {
  match handle_get_mut::<HashSet<T>, i64>(id, tag, |s| s.len() as i64) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

pub fn set_i64_new() -> i64 {
  set_new::<i64>(SET_I64_TAG)
}
pub unsafe fn set_i64_add(id: i64, v: i64) -> i64 {
  set_add(id, SET_I64_TAG, v)
}
pub unsafe fn set_i64_contains(id: i64, v: i64) -> i64 {
  set_contains(id, SET_I64_TAG, v)
}
pub unsafe fn set_i64_remove(id: i64, v: i64) -> i64 {
  set_remove(id, SET_I64_TAG, v)
}
pub unsafe fn set_i64_count(id: i64) -> i64 {
  set_count::<i64>(id, SET_I64_TAG)
}
/// `.each(proc)` — `emerald-codegen`'s own dispatch materializes this
/// into a real `Array[Int64]` snapshot (this exact raw layout) and
/// then reuses `Array[T]#each`'s already-proven loop-and-call-block
/// codegen directly over it, rather than this crate calling back into
/// an Emerald closure across the FFI boundary itself — a real,
/// disclosed simplification (no such call-back mechanism exists
/// anywhere in this crate today), not a functional gap: the block
/// still runs once per element, in insertion-order-unspecified
/// `HashSet` iteration order, exactly as `HashSet::iter` gives it.
///
/// # Safety
/// Always safe to call with any `i64` `id` — raises rather than
/// dereferencing on a closed/unknown handle.
pub unsafe fn set_i64_each(id: i64) -> *mut c_void {
  match handle_get_mut::<HashSet<i64>, Vec<i64>>(id, SET_I64_TAG, |s| s.iter().copied().collect()) {
    Ok(items) => build_i64_array(&items),
    Err(msg) => crate::raise_native_error(&msg),
  }
}
pub fn set_i64_close(id: i64) {
  handle_close(id);
}

pub fn set_string_new() -> i64 {
  set_new::<String>(SET_STRING_TAG)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn set_string_add(id: i64, v: *const c_char) -> i64 {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  set_add(id, SET_STRING_TAG, v)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn set_string_contains(id: i64, v: *const c_char) -> i64 {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  set_contains(id, SET_STRING_TAG, v)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn set_string_remove(id: i64, v: *const c_char) -> i64 {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  set_remove(id, SET_STRING_TAG, v)
}
pub unsafe fn set_string_count(id: i64) -> i64 {
  set_count::<String>(id, SET_STRING_TAG)
}
/// # Safety
/// Always safe to call with any `i64` `id`.
pub unsafe fn set_string_each(id: i64) -> *mut c_void {
  match handle_get_mut::<HashSet<String>, Vec<String>>(id, SET_STRING_TAG, |s| {
    s.iter().cloned().collect()
  }) {
    Ok(items) => {
      let refs: Vec<&str> = items.iter().map(String::as_str).collect();
      build_string_array(&refs)
    }
    Err(msg) => crate::raise_native_error(&msg),
  }
}
pub fn set_string_close(id: i64) {
  handle_close(id);
}

// ---------------------------------------------------------------------
// Deque[T] — Int64, Float64, String, Boolean.
// ---------------------------------------------------------------------

fn deque_new<T: 'static + Send>(tag: &'static str) -> i64 {
  handle_alloc(Box::new(VecDeque::<T>::new()), tag)
}

unsafe fn deque_push_front<T: 'static + Send>(id: i64, tag: &'static str, v: T) {
  if let Err(msg) = handle_get_mut::<VecDeque<T>, ()>(id, tag, |d| d.push_front(v)) {
    crate::raise_native_error(&msg);
  }
}

unsafe fn deque_push_back<T: 'static + Send>(id: i64, tag: &'static str, v: T) {
  if let Err(msg) = handle_get_mut::<VecDeque<T>, ()>(id, tag, |d| d.push_back(v)) {
    crate::raise_native_error(&msg);
  }
}

/// Raises (rather than returning some sentinel `T` a caller could
/// mistake for a real element) on an empty `Deque` — the same "never a
/// silent no-op" posture `crate::handle`'s own closed/unknown-handle
/// diagnostics already establish, extended to this domain's own real
/// empty-collection failure mode.
unsafe fn deque_pop_front<T: 'static + Send>(id: i64, tag: &'static str) -> T {
  match handle_get_mut::<VecDeque<T>, Option<T>>(id, tag, |d| d.pop_front()) {
    Ok(Some(v)) => v,
    Ok(None) => crate::raise_native_error("pop_front from an empty Deque"),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn deque_pop_back<T: 'static + Send>(id: i64, tag: &'static str) -> T {
  match handle_get_mut::<VecDeque<T>, Option<T>>(id, tag, |d| d.pop_back()) {
    Ok(Some(v)) => v,
    Ok(None) => crate::raise_native_error("pop_back from an empty Deque"),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn deque_count<T: 'static + Send>(id: i64, tag: &'static str) -> i64 {
  match handle_get_mut::<VecDeque<T>, i64>(id, tag, |d| d.len() as i64) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

pub fn deque_i64_new() -> i64 {
  deque_new::<i64>(DEQUE_I64_TAG)
}
pub unsafe fn deque_i64_push_front(id: i64, v: i64) {
  deque_push_front(id, DEQUE_I64_TAG, v)
}
pub unsafe fn deque_i64_push_back(id: i64, v: i64) {
  deque_push_back(id, DEQUE_I64_TAG, v)
}
pub unsafe fn deque_i64_pop_front(id: i64) -> i64 {
  deque_pop_front::<i64>(id, DEQUE_I64_TAG)
}
pub unsafe fn deque_i64_pop_back(id: i64) -> i64 {
  deque_pop_back::<i64>(id, DEQUE_I64_TAG)
}
pub unsafe fn deque_i64_count(id: i64) -> i64 {
  deque_count::<i64>(id, DEQUE_I64_TAG)
}
pub fn deque_i64_close(id: i64) {
  handle_close(id);
}

pub fn deque_f64_new() -> i64 {
  deque_new::<f64>(DEQUE_F64_TAG)
}
pub unsafe fn deque_f64_push_front(id: i64, v: f64) {
  deque_push_front(id, DEQUE_F64_TAG, v)
}
pub unsafe fn deque_f64_push_back(id: i64, v: f64) {
  deque_push_back(id, DEQUE_F64_TAG, v)
}
pub unsafe fn deque_f64_pop_front(id: i64) -> f64 {
  deque_pop_front::<f64>(id, DEQUE_F64_TAG)
}
pub unsafe fn deque_f64_pop_back(id: i64) -> f64 {
  deque_pop_back::<f64>(id, DEQUE_F64_TAG)
}
pub unsafe fn deque_f64_count(id: i64) -> i64 {
  deque_count::<f64>(id, DEQUE_F64_TAG)
}
pub fn deque_f64_close(id: i64) {
  handle_close(id);
}

pub fn deque_string_new() -> i64 {
  deque_new::<String>(DEQUE_STRING_TAG)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn deque_string_push_front(id: i64, v: *const c_char) {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  deque_push_front(id, DEQUE_STRING_TAG, v)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn deque_string_push_back(id: i64, v: *const c_char) {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  deque_push_back(id, DEQUE_STRING_TAG, v)
}
pub unsafe fn deque_string_pop_front(id: i64) -> *const c_char {
  crate::alloc_and_copy_str(&deque_pop_front::<String>(id, DEQUE_STRING_TAG))
}
pub unsafe fn deque_string_pop_back(id: i64) -> *const c_char {
  crate::alloc_and_copy_str(&deque_pop_back::<String>(id, DEQUE_STRING_TAG))
}
pub unsafe fn deque_string_count(id: i64) -> i64 {
  deque_count::<String>(id, DEQUE_STRING_TAG)
}
pub fn deque_string_close(id: i64) {
  handle_close(id);
}

pub fn deque_bool_new() -> i64 {
  deque_new::<bool>(DEQUE_BOOL_TAG)
}
pub unsafe fn deque_bool_push_front(id: i64, v: i64) {
  deque_push_front(id, DEQUE_BOOL_TAG, v != 0)
}
pub unsafe fn deque_bool_push_back(id: i64, v: i64) {
  deque_push_back(id, DEQUE_BOOL_TAG, v != 0)
}
pub unsafe fn deque_bool_pop_front(id: i64) -> i64 {
  i64::from(deque_pop_front::<bool>(id, DEQUE_BOOL_TAG))
}
pub unsafe fn deque_bool_pop_back(id: i64) -> i64 {
  i64::from(deque_pop_back::<bool>(id, DEQUE_BOOL_TAG))
}
pub unsafe fn deque_bool_count(id: i64) -> i64 {
  deque_count::<bool>(id, DEQUE_BOOL_TAG)
}
pub fn deque_bool_close(id: i64) {
  handle_close(id);
}

// ---------------------------------------------------------------------
// PriorityQueue[T] — Int64, String. Max-heap (Rust `BinaryHeap`'s own
// default `Ord`), unmodified — Decision log.
// ---------------------------------------------------------------------

fn pq_new<T: 'static + Send + Ord>(tag: &'static str) -> i64 {
  handle_alloc(Box::new(BinaryHeap::<T>::new()), tag)
}

unsafe fn pq_push<T: 'static + Send + Ord>(id: i64, tag: &'static str, v: T) {
  if let Err(msg) = handle_get_mut::<BinaryHeap<T>, ()>(id, tag, |h| h.push(v)) {
    crate::raise_native_error(&msg);
  }
}

unsafe fn pq_pop<T: 'static + Send + Ord>(id: i64, tag: &'static str) -> T {
  match handle_get_mut::<BinaryHeap<T>, Option<T>>(id, tag, |h| h.pop()) {
    Ok(Some(v)) => v,
    Ok(None) => crate::raise_native_error("pop from an empty PriorityQueue"),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn pq_peek<T: 'static + Send + Ord + Clone>(id: i64, tag: &'static str) -> T {
  match handle_get_mut::<BinaryHeap<T>, Option<T>>(id, tag, |h| h.peek().cloned()) {
    Ok(Some(v)) => v,
    Ok(None) => crate::raise_native_error("peek on an empty PriorityQueue"),
    Err(msg) => crate::raise_native_error(&msg),
  }
}

unsafe fn pq_count<T: 'static + Send + Ord>(id: i64, tag: &'static str) -> i64 {
  match handle_get_mut::<BinaryHeap<T>, i64>(id, tag, |h| h.len() as i64) {
    Ok(n) => n,
    Err(msg) => crate::raise_native_error(&msg),
  }
}

pub fn priority_queue_i64_new() -> i64 {
  pq_new::<i64>(PQ_I64_TAG)
}
pub unsafe fn priority_queue_i64_push(id: i64, v: i64) {
  pq_push(id, PQ_I64_TAG, v)
}
pub unsafe fn priority_queue_i64_pop(id: i64) -> i64 {
  pq_pop::<i64>(id, PQ_I64_TAG)
}
pub unsafe fn priority_queue_i64_peek(id: i64) -> i64 {
  pq_peek::<i64>(id, PQ_I64_TAG)
}
pub unsafe fn priority_queue_i64_count(id: i64) -> i64 {
  pq_count::<i64>(id, PQ_I64_TAG)
}
pub fn priority_queue_i64_close(id: i64) {
  handle_close(id);
}

pub fn priority_queue_string_new() -> i64 {
  pq_new::<String>(PQ_STRING_TAG)
}
/// # Safety
/// `v`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn priority_queue_string_push(id: i64, v: *const c_char) {
  let v = match read_str(v) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  pq_push(id, PQ_STRING_TAG, v)
}
pub unsafe fn priority_queue_string_pop(id: i64) -> *const c_char {
  crate::alloc_and_copy_str(&pq_pop::<String>(id, PQ_STRING_TAG))
}
pub unsafe fn priority_queue_string_peek(id: i64) -> *const c_char {
  crate::alloc_and_copy_str(&pq_peek::<String>(id, PQ_STRING_TAG))
}
pub unsafe fn priority_queue_string_count(id: i64) -> i64 {
  pq_count::<String>(id, PQ_STRING_TAG)
}
pub fn priority_queue_string_close(id: i64) {
  handle_close(id);
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  // Real, disclosed finding while writing these (matching `regex.rs`'s
  // own established omission, and `emerald-rt/src/lib.rs`'s own
  // `emerald_raise` test stub doc comment, verified directly this
  // session): `raise_native_error` never returns via an ordinary,
  // catchable Rust panic — it's a real `longjmp`-shaped divergence to
  // `emerald_raise`, and the test build's own stub for that symbol
  // calls `std::process::abort()` outright rather than pretend to
  // unwind across what's really a C ABI boundary. So the "use of a
  // CLOSED handle"/"unknown handle" diagnostics below are verified the
  // same way `handle.rs`'s OWN tests verify them — a direct
  // `handle_get_mut`/`handle_close` call, never through one of this
  // module's own `raise_native_error`-calling public wrappers (doing
  // so would abort the whole test process, not fail one test) — and
  // the real end-to-end raise path is verified only by the plan's own
  // Concrete Proof `.em` example, run through the real CLI.

  // --- Set[Int64] ---

  #[test]
  fn set_i64_add_dedupes_and_count_reflects_the_real_unique_total() {
    unsafe {
      let id = set_i64_new();
      assert_eq!(set_i64_add(id, 10), 1, "newly inserted");
      assert_eq!(set_i64_add(id, 20), 1);
      assert_eq!(set_i64_add(id, 10), 0, "already present");
      assert_eq!(set_i64_count(id), 2);
      assert_eq!(set_i64_contains(id, 20), 1);
      assert_eq!(set_i64_contains(id, 99), 0);
      set_i64_close(id);
    }
  }

  #[test]
  fn set_i64_remove_reports_whether_the_value_was_actually_present() {
    unsafe {
      let id = set_i64_new();
      set_i64_add(id, 1);
      assert_eq!(set_i64_remove(id, 1), 1);
      assert_eq!(set_i64_remove(id, 1), 0);
      assert_eq!(set_i64_count(id), 0);
      set_i64_close(id);
    }
  }

  #[test]
  fn set_i64_each_materializes_every_member_exactly_once() {
    unsafe {
      let id = set_i64_new();
      set_i64_add(id, 1);
      set_i64_add(id, 2);
      set_i64_add(id, 3);
      let arr = set_i64_each(id) as *const i64;
      let len = *arr;
      assert_eq!(len, 3);
      let mut seen: Vec<i64> = (0..len).map(|i| *arr.add(1 + i as usize)).collect();
      seen.sort();
      assert_eq!(seen, vec![1, 2, 3]);
      set_i64_close(id);
    }
  }

  #[test]
  fn set_i64_double_close_is_a_harmless_no_op() {
    let id = set_i64_new();
    set_i64_close(id);
    set_i64_close(id);
  }

  #[test]
  fn set_i64_use_after_close_is_a_real_diagnostic() {
    let id = set_i64_new();
    set_i64_close(id);
    let err = handle_get_mut::<HashSet<i64>, i64>(id, SET_I64_TAG, |s| s.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed Set_i64 handle");
  }

  #[test]
  fn set_i64_use_of_an_unknown_handle_is_a_real_diagnostic() {
    let err = handle_get_mut::<HashSet<i64>, i64>(999_999_999, SET_I64_TAG, |s| s.len() as i64)
      .unwrap_err();
    assert_eq!(
      err,
      "unknown Set_i64 handle: this process never issued id 999999999"
    );
  }

  // --- Set[String] ---

  #[test]
  fn set_string_add_dedupes_by_real_string_equality() {
    unsafe {
      let id = set_string_new();
      let a = c("a");
      let b = c("b");
      assert_eq!(set_string_add(id, a.as_ptr()), 1);
      assert_eq!(set_string_add(id, a.as_ptr()), 0);
      assert_eq!(set_string_add(id, b.as_ptr()), 1);
      assert_eq!(set_string_count(id), 2);
      assert_eq!(set_string_contains(id, a.as_ptr()), 1);
      set_string_close(id);
    }
  }

  #[test]
  fn set_string_each_materializes_every_member_exactly_once() {
    unsafe {
      let id = set_string_new();
      let a = c("alpha");
      set_string_add(id, a.as_ptr());
      let arr = set_string_each(id) as *const i64;
      assert_eq!(*arr, 1);
      let elems = arr.add(1) as *const *const c_char;
      let s = std::ffi::CStr::from_ptr(*elems).to_str().unwrap();
      assert_eq!(s, "alpha");
      set_string_close(id);
    }
  }

  #[test]
  fn set_string_use_after_close_is_a_real_diagnostic() {
    let id = set_string_new();
    set_string_close(id);
    let err =
      handle_get_mut::<HashSet<String>, i64>(id, SET_STRING_TAG, |s| s.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed Set_string handle");
  }

  // --- Deque[Int64] ---

  #[test]
  fn deque_i64_pushes_and_pops_from_both_ends_in_real_fifo_lifo_order() {
    unsafe {
      let id = deque_i64_new();
      deque_i64_push_back(id, 1);
      deque_i64_push_back(id, 2);
      deque_i64_push_front(id, 0);
      assert_eq!(deque_i64_count(id), 3);
      assert_eq!(deque_i64_pop_front(id), 0);
      assert_eq!(deque_i64_pop_back(id), 2);
      assert_eq!(deque_i64_pop_front(id), 1);
      assert_eq!(deque_i64_count(id), 0);
      deque_i64_close(id);
    }
  }

  #[test]
  fn deque_i64_double_close_is_a_harmless_no_op() {
    let id = deque_i64_new();
    deque_i64_close(id);
    deque_i64_close(id);
  }

  #[test]
  fn deque_i64_use_after_close_is_a_real_diagnostic() {
    let id = deque_i64_new();
    deque_i64_close(id);
    let err =
      handle_get_mut::<VecDeque<i64>, i64>(id, DEQUE_I64_TAG, |d| d.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed Deque_i64 handle");
  }

  #[test]
  fn deque_i64_use_of_an_unknown_handle_is_a_real_diagnostic() {
    let err = handle_get_mut::<VecDeque<i64>, i64>(999_999_999, DEQUE_I64_TAG, |d| d.len() as i64)
      .unwrap_err();
    assert_eq!(
      err,
      "unknown Deque_i64 handle: this process never issued id 999999999"
    );
  }

  // --- Deque[Float64] ---

  #[test]
  fn deque_f64_round_trips_real_floating_point_values() {
    unsafe {
      let id = deque_f64_new();
      deque_f64_push_back(id, 1.5);
      deque_f64_push_front(id, 0.5);
      assert_eq!(deque_f64_count(id), 2);
      assert_eq!(deque_f64_pop_front(id), 0.5);
      assert_eq!(deque_f64_pop_back(id), 1.5);
      deque_f64_close(id);
    }
  }

  #[test]
  fn deque_f64_use_after_close_is_a_real_diagnostic() {
    let id = deque_f64_new();
    deque_f64_close(id);
    let err =
      handle_get_mut::<VecDeque<f64>, i64>(id, DEQUE_F64_TAG, |d| d.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed Deque_f64 handle");
  }

  // --- Deque[String] ---

  #[test]
  fn deque_string_pushes_and_pops_real_owned_strings() {
    unsafe {
      let id = deque_string_new();
      let a = c("a");
      let z = c("z");
      deque_string_push_back(id, a.as_ptr());
      deque_string_push_front(id, z.as_ptr());
      assert_eq!(deque_string_count(id), 2);
      let front = deque_string_pop_front(id);
      assert_eq!(std::ffi::CStr::from_ptr(front).to_str().unwrap(), "z");
      let back = deque_string_pop_back(id);
      assert_eq!(std::ffi::CStr::from_ptr(back).to_str().unwrap(), "a");
      deque_string_close(id);
    }
  }

  #[test]
  fn deque_string_use_after_close_is_a_real_diagnostic() {
    let id = deque_string_new();
    deque_string_close(id);
    let err = handle_get_mut::<VecDeque<String>, i64>(id, DEQUE_STRING_TAG, |d| d.len() as i64)
      .unwrap_err();
    assert_eq!(err, "use of closed Deque_string handle");
  }

  // --- Deque[Boolean] ---

  #[test]
  fn deque_bool_round_trips_both_boolean_values() {
    unsafe {
      let id = deque_bool_new();
      deque_bool_push_back(id, 1);
      deque_bool_push_back(id, 0);
      assert_eq!(deque_bool_count(id), 2);
      assert_eq!(deque_bool_pop_front(id), 1);
      assert_eq!(deque_bool_pop_front(id), 0);
      deque_bool_close(id);
    }
  }

  #[test]
  fn deque_bool_use_of_an_unknown_handle_is_a_real_diagnostic() {
    let err =
      handle_get_mut::<VecDeque<bool>, i64>(999_999_999, DEQUE_BOOL_TAG, |d| d.len() as i64)
        .unwrap_err();
    assert_eq!(
      err,
      "unknown Deque_bool handle: this process never issued id 999999999"
    );
  }

  // --- PriorityQueue[Int64] ---

  #[test]
  fn priority_queue_i64_pop_always_returns_the_current_maximum() {
    unsafe {
      let id = priority_queue_i64_new();
      priority_queue_i64_push(id, 5);
      priority_queue_i64_push(id, 1);
      priority_queue_i64_push(id, 9);
      priority_queue_i64_push(id, 3);
      assert_eq!(priority_queue_i64_count(id), 4);
      assert_eq!(priority_queue_i64_peek(id), 9, "peek doesn't remove");
      assert_eq!(priority_queue_i64_count(id), 4);
      assert_eq!(priority_queue_i64_pop(id), 9);
      assert_eq!(priority_queue_i64_pop(id), 5);
      assert_eq!(priority_queue_i64_pop(id), 3);
      assert_eq!(priority_queue_i64_pop(id), 1);
      priority_queue_i64_close(id);
    }
  }

  #[test]
  fn priority_queue_i64_use_after_close_is_a_real_diagnostic() {
    let id = priority_queue_i64_new();
    priority_queue_i64_close(id);
    let err =
      handle_get_mut::<BinaryHeap<i64>, i64>(id, PQ_I64_TAG, |h| h.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed PriorityQueue_i64 handle");
  }

  #[test]
  fn priority_queue_i64_use_of_an_unknown_handle_is_a_real_diagnostic() {
    let err = handle_get_mut::<BinaryHeap<i64>, i64>(999_999_999, PQ_I64_TAG, |h| h.len() as i64)
      .unwrap_err();
    assert_eq!(
      err,
      "unknown PriorityQueue_i64 handle: this process never issued id 999999999"
    );
  }

  // --- PriorityQueue[String] ---

  #[test]
  fn priority_queue_string_pop_always_returns_the_lexicographic_maximum() {
    unsafe {
      let id = priority_queue_string_new();
      for w in ["mango", "apple", "zebra", "banana"] {
        let s = c(w);
        priority_queue_string_push(id, s.as_ptr());
      }
      assert_eq!(priority_queue_string_count(id), 4);
      let top = priority_queue_string_pop(id);
      assert_eq!(std::ffi::CStr::from_ptr(top).to_str().unwrap(), "zebra");
      priority_queue_string_close(id);
    }
  }

  #[test]
  fn priority_queue_string_use_after_close_is_a_real_diagnostic() {
    let id = priority_queue_string_new();
    priority_queue_string_close(id);
    let err =
      handle_get_mut::<BinaryHeap<String>, i64>(id, PQ_STRING_TAG, |h| h.len() as i64).unwrap_err();
    assert_eq!(err, "use of closed PriorityQueue_string handle");
  }
}
