//! Plan 91 — the Rust-native runtime crate. A second static archive,
//! alongside `runtime/emerald_runtime.c`'s existing C one, that
//! `emerald-driver`'s `build.rs` compiles (via a `cargo build -p
//! emerald-rt --release` subprocess, JSON-artifact-discovered — stable
//! Cargo has no artifact-dependency feature to do this natively) and
//! embeds into the final `emerald-cli` binary. Every stdlib-expansion
//! plan from 92 onward adds a module here; this plan's own scope is
//! exactly one proof function, chosen to be independently checkable
//! (FNV-1a is public-domain, specified precisely enough to hand-verify)
//! so what's being proven is the build-and-link *mechanism*, not any
//! algorithm's correctness.
//!
//! # The panic-boundary convention every export in this crate follows
//!
//! `runtime/emerald_runtime.c`'s exceptions are `setjmp`/`longjmp`-based
//! (plan 11) — a Rust panic unwinding past a plain `extern "C" fn`
//! boundary into those C frames is undefined behavior twice over (no
//! unwind tables describe `setjmp`-based C frames at all, and
//! unwinding across a non-`"C-unwind"` `extern "C"` boundary is already
//! UB per Rust's own reference). Every exported function's entire body
//! must therefore be wrapped in `std::panic::catch_unwind`, converting
//! a caught panic into this function's own disclosed error sentinel
//! (`-1` here; plan 92 generalizes this into a real `Result`-style
//! convention project-wide) rather than letting it unwind at all. This
//! is safe only because the workspace keeps its real, un-overridden
//! `panic = "unwind"` default — see this crate's own `Cargo.toml`.

use std::os::raw::c_char;

/// The FNV-1a-32 hash of `s`'s bytes, widened to `i64` (Emerald's own
/// type system has no narrower integer — plan 59's own finding, cited
/// directly rather than re-derived). Returns `-1` on any panic
/// (including a null `s`, guarded explicitly below rather than left to
/// `CStr::from_ptr`'s own null-pointer UB) or invalid UTF-8/non-UTF-8
/// bytes are hashed exactly as `String.length`'s own `strlen`-based
/// byte count already treats them — as raw bytes, not codepoints.
/// # Safety
///
/// `s`, if non-null, must point to a valid, NUL-terminated C string —
/// the same contract every other `*const c_char`-taking runtime
/// function in this project already carries. A null `s` is explicitly
/// handled (returns `-1`), not UB.
#[no_mangle]
pub unsafe extern "C" fn emerald_rt_fnv1a_hash(s: *const c_char) -> i64 {
  std::panic::catch_unwind(|| {
    if s.is_null() {
      return Err(());
    }
    // Safety: `s` is checked non-null above; the caller (Emerald's own
    // compiled output, via `emerald-codegen`'s intrinsic dispatch) is
    // required to pass a valid, NUL-terminated `String`'s backing
    // pointer, the same contract every other `*const c_char`-taking
    // runtime function in this project already carries.
    let bytes = unsafe { std::ffi::CStr::from_ptr(s) }.to_bytes();
    Ok(fnv1a_hash_bytes(bytes))
  })
  .unwrap_or(Err(()))
  .map(|h| h as i64)
  .unwrap_or(-1)
}

/// The actual FNV-1a-32 algorithm — a pure function of `bytes`, with
/// no FFI/panic concerns of its own, so `#[test]` below can assert
/// against a hand-computed value with nothing else in the way.
fn fnv1a_hash_bytes(bytes: &[u8]) -> u32 {
  const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
  const FNV_PRIME: u32 = 0x0100_0193;
  let mut hash = FNV_OFFSET_BASIS;
  for &b in bytes {
    hash ^= u32::from(b);
    hash = hash.wrapping_mul(FNV_PRIME);
  }
  hash
}

#[cfg(test)]
mod tests {
  use super::*;

  // Hand-computed FNV-1a-32 values (offset basis 0x811c9dc5, prime
  // 0x01000193) — independently checkable against any FNV-1a
  // reference table, not just this crate's own arithmetic.
  #[test]
  fn fnv1a_hash_of_hello_matches_the_known_reference_value() {
    assert_eq!(fnv1a_hash_bytes(b"hello"), 0x4f9f_2cab);
  }

  #[test]
  fn fnv1a_hash_of_hello_world_matches_the_known_reference_value() {
    assert_eq!(fnv1a_hash_bytes(b"hello world"), 0xd58b_3fa7);
  }

  #[test]
  fn fnv1a_hash_of_empty_string_is_the_bare_offset_basis() {
    assert_eq!(fnv1a_hash_bytes(b""), 0x811c_9dc5);
  }

  #[test]
  fn the_extern_c_entry_point_matches_the_pure_function_widened_to_i64() {
    let s = std::ffi::CString::new("hello").unwrap();
    assert_eq!(
      unsafe { emerald_rt_fnv1a_hash(s.as_ptr()) },
      0x4f9f_2cab_i64
    );
  }

  #[test]
  fn the_extern_c_entry_point_returns_minus_one_for_a_null_pointer() {
    assert_eq!(unsafe { emerald_rt_fnv1a_hash(std::ptr::null()) }, -1);
  }
}
