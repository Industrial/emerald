//! Plan 146 (Environment Variables) — `Env.get`/`.set`/`.remove`/
//! `.keys`/`.keys_count`, backed entirely by `std::env` (zero
//! third-party crate). The one real design problem: `std::env::
//! set_var`/`remove_var` were reclassified `unsafe fn` in the Rust
//! 2024 edition specifically because mutating the process environment
//! concurrently with another thread reading it is genuine platform-
//! level UB on some targets — and Emerald's own actor model runs a
//! real multi-threaded worker pool (plan 55), so this is not
//! hypothetical. Every function here serializes behind one process-
//! wide `Mutex<()>`, reads included (a concurrent `getenv` racing a
//! concurrent `setenv` is exactly the unsound combination), acquired
//! via `.unwrap_or_else(PoisonError::into_inner)` so one panicking
//! holder can never wedge the lock for every future call.

use std::ffi::c_void;
use std::os::raw::c_char;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, &'a str> {
  if s.is_null() {
    return Err("null string pointer");
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8")
}

/// `Env.get(key: String): Option[String]` — the Rust side returns a
/// bare nullable `*mut c_char`; `emerald-codegen`'s own call site
/// builds the real tagged `Option[String]` value from it, the
/// identical pattern already established for `String.from_cstring`
/// (plan 73's own Decision log).
///
/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn env_get(key: *const c_char) -> *mut c_char {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(_) => return std::ptr::null_mut(),
  };
  let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  match std::env::var(key) {
    Ok(v) => crate::alloc_and_copy_str(&v) as *mut c_char,
    Err(_) => std::ptr::null_mut(),
  }
}

/// `Env.set(key: String, value: String): Void`.
///
/// # Safety
/// `key`/`value`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn env_set(key: *const c_char, value: *const c_char) {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(e),
  };
  let value = match read_str(value) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(e),
  };
  let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  // Safety: real and load-bearing, not boilerplate — see this module's
  // own doc comment. Serialized by `ENV_LOCK` against every other
  // `Env` call in this same process, which is the actual mitigation
  // for the platform-level unsoundness this function's own `unsafe fn`
  // signature (Rust 2024 edition) names.
  unsafe {
    std::env::set_var(key, value);
  }
}

/// `Env.remove(key: String): Void`.
///
/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn env_remove(key: *const c_char) {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(e),
  };
  let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  unsafe {
    std::env::remove_var(key);
  }
}

/// `Env.keys(): Array[String]`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn env_keys() -> *mut c_void {
  let names: Vec<String> = {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    std::env::vars().map(|(k, _)| k).collect()
  };
  let ptr = crate::emerald_alloc(8 + 8 * names.len() as i64) as *mut i64;
  *ptr = names.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, name) in names.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(name);
  }
  ptr as *mut c_void
}

/// `Env.keys_count(): Int64`.
///
/// # Safety
/// Always safe to call.
pub unsafe fn env_keys_count() -> i64 {
  let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
  std::env::vars().count() as i64
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  #[test]
  fn set_then_get_round_trips_a_real_value() {
    unsafe {
      let key = c("EMERALD_RT_ENV_TEST_ROUNDTRIP");
      let value = c("hello");
      env_set(key.as_ptr(), value.as_ptr());
      let got = env_get(key.as_ptr());
      assert!(!got.is_null());
      assert_eq!(std::ffi::CStr::from_ptr(got).to_str().unwrap(), "hello");
      env_remove(key.as_ptr());
    }
  }

  #[test]
  fn remove_then_get_returns_null() {
    unsafe {
      let key = c("EMERALD_RT_ENV_TEST_REMOVE");
      let value = c("temp");
      env_set(key.as_ptr(), value.as_ptr());
      env_remove(key.as_ptr());
      let got = env_get(key.as_ptr());
      assert!(got.is_null());
    }
  }

  #[test]
  fn keys_count_reflects_a_real_set_variable() {
    unsafe {
      let before = env_keys_count();
      let key = c("EMERALD_RT_ENV_TEST_KEYS_COUNT");
      let value = c("x");
      env_set(key.as_ptr(), value.as_ptr());
      let after = env_keys_count();
      assert!(after > before);
      env_remove(key.as_ptr());
    }
  }

  #[test]
  fn concurrent_set_calls_from_multiple_threads_complete_without_a_panic_or_a_poisoned_mutex_failure(
  ) {
    let handles: Vec<_> = (0..8)
      .map(|i| {
        std::thread::spawn(move || unsafe {
          for j in 0..200 {
            let key = c(&format!("EMERALD_RT_ENV_TEST_CONCURRENT_{i}"));
            let value = c(&format!("{j}"));
            env_set(key.as_ptr(), value.as_ptr());
            let _ = env_get(key.as_ptr());
          }
          let key = c(&format!("EMERALD_RT_ENV_TEST_CONCURRENT_{i}"));
          env_remove(key.as_ptr());
        })
      })
      .collect();
    for h in handles {
      h.join().unwrap();
    }
  }
}
