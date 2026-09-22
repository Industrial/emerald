//! Plan 113 (Cryptographically Secure Random Number Generation) —
//! `Random.secure_hex`/`.secure_token` (CSPRNG-backed, safe for keys/
//! tokens/nonces/salts) and `Random.int`/`.shuffle` (fast, non-
//! cryptographic, safe only for simulations/games/sampling) —
//! deliberately separate top-level names, not one generator behind a
//! boolean flag, per this plan's own Decision log.
//!
//! Real, disclosed simplification vs. this plan's own literal design:
//! `Random.secure_hex`/`.secure_token` call `getrandom::fill` directly
//! (this crate's own existing dependency, already used for plan 110's
//! AEAD nonce/key generation) rather than adding `rand` 0.10's
//! `SysRng` wrapper — `SysRng` is itself described, by the crate's own
//! CHANGELOG, as "backed by the `getrandom` crate", so calling
//! `getrandom` directly reaches the identical OS entropy source one
//! layer more directly, and avoids a second, semver-incompatible major
//! version of `rand` alongside the `rand` 0.8 plan 111 already added
//! for `rsa`'s own older `rand_core` pin. `Random.int`/`.shuffle` use
//! that existing `rand` 0.8's own `rand::thread_rng()` (the direct
//! ancestor of 0.10's `ThreadRng`, identical fast/non-cryptographic
//! contract) for the same reason.
//!
//! `Random.shuffle` mutates its argument in place — a real, disclosed
//! exception to plan 45's "no method mutates its receiver" rule (see
//! this plan's own Decision log): it reads/writes an `Array[Int64]`
//! value's own `[len: i64][elem: i64]*n` heap layout directly (plan
//! 42's `leaf-array-length-header`), the same header-inclusive layout
//! every other `Array[T]`-producing intrinsic in this crate already
//! assumes.

use rand::Rng;

/// `Random.secure_hex(n: Int64): String` — `n` random bytes, hex-
/// encoded (`2*n` ASCII hex characters).
///
/// # Safety
/// Always safe to call.
pub unsafe fn random_secure_hex(n: i64) -> *const std::os::raw::c_char {
  let bytes = random_bytes(n);
  crate::alloc_and_copy_str(&hex::encode(&bytes))
}

/// `Random.secure_token(n: Int64): String` — the same `n` random
/// bytes as `secure_hex`, URL-safe-base64-encoded instead (shorter,
/// still safely embeddable in a URL path/query segment).
///
/// # Safety
/// Always safe to call.
pub unsafe fn random_secure_token(n: i64) -> *const std::os::raw::c_char {
  use base64::engine::general_purpose::URL_SAFE_NO_PAD;
  use base64::Engine as _;
  let bytes = random_bytes(n);
  crate::alloc_and_copy_str(&URL_SAFE_NO_PAD.encode(&bytes))
}

unsafe fn random_bytes(n: i64) -> Vec<u8> {
  if n <= 0 {
    crate::raise_native_error("Random.secure_hex/.secure_token: n must be positive");
  }
  let mut buf = vec![0u8; n as usize];
  getrandom::fill(&mut buf).expect("emerald-rt: OS CSPRNG failure generating random bytes");
  buf
}

/// `Random.int(min: Int64, max: Int64): Int64` — inclusive range,
/// `rand::thread_rng()`-backed (fast, non-cryptographic — never for
/// keys/tokens/nonces).
///
/// # Safety
/// Always safe to call.
pub unsafe fn random_int(min: i64, max: i64) -> i64 {
  if min > max {
    crate::raise_native_error("Random.int: min must be <= max");
  }
  rand::thread_rng().gen_range(min..=max)
}

/// `Random.shuffle(arr: Array[Int64]): Void` — in-place Fisher-Yates,
/// `rand::thread_rng()`-backed. Reads/writes `arr`'s own `[len: i64]
/// [elem: i64]*n` heap layout directly.
///
/// # Safety
/// `arr` must be a live `Array[Int64]` value (a pointer this
/// compiler's own array-literal/array-building codegen produced).
pub unsafe fn random_shuffle(arr: *mut i64) {
  if arr.is_null() {
    crate::raise_native_error("Random.shuffle: null array pointer");
  }
  let count = *arr as usize;
  let elems = arr.add(1);
  let mut rng = rand::thread_rng();
  for i in (1..count).rev() {
    let j = rng.gen_range(0..=i);
    if i != j {
      std::ptr::swap(elems.add(i), elems.add(j));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  unsafe fn hex_str(id: *const std::os::raw::c_char) -> String {
    std::ffi::CStr::from_ptr(id).to_str().unwrap().to_string()
  }

  #[test]
  fn secure_hex_returns_the_correct_length_and_never_repeats() {
    unsafe {
      let a = hex_str(random_secure_hex(16));
      let b = hex_str(random_secure_hex(16));
      assert_eq!(a.len(), 32);
      assert_eq!(b.len(), 32);
      assert_ne!(a, b, "two consecutive CSPRNG calls collided");
    }
  }

  #[test]
  fn secure_token_is_url_safe_and_never_repeats() {
    unsafe {
      let a = hex_str(random_secure_token(16));
      let b = hex_str(random_secure_token(16));
      assert!(!a.contains('+') && !a.contains('/') && !a.contains('='));
      assert_ne!(a, b, "two consecutive CSPRNG calls collided");
    }
  }

  #[test]
  fn int_stays_within_the_inclusive_range_across_many_draws() {
    unsafe {
      for _ in 0..1000 {
        let roll = random_int(1, 6);
        assert!((1..=6).contains(&roll));
      }
    }
  }

  #[test]
  fn shuffle_preserves_every_element_just_reorders_them() {
    unsafe {
      let mut buf = [5i64, 1, 2, 3, 4, 5];
      random_shuffle(buf.as_mut_ptr());
      let mut rest = buf[1..].to_vec();
      rest.sort();
      assert_eq!(rest, vec![1, 2, 3, 4, 5]);
    }
  }
}
