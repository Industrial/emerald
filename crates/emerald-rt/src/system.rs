//! Plan 152 (System Information) — a `System` compiler-known
//! namespace over `sysinfo`: CPU/memory/disk/process introspection,
//! read-only, full stop (no kill/renice/priority-change surface
//! anywhere in this module, a deliberate design boundary, not merely
//! an accident of what `sysinfo` itself exposes). No long-lived
//! cached snapshot is held across calls — every function builds,
//! refreshes, reads, and drops its own short-lived `sysinfo` instance,
//! a real, disclosed simplicity-over-throughput tradeoff (a cached,
//! explicitly-refreshable handle is a natural future optimization,
//! not attempted here).
//!
//! `process_ids(): Array[Int64]` is this batch's first array-of-
//! scalars intrinsic (every prior `Array[T]`-returning function
//! returned `Array[String]`) — mechanically identical to `Array[
//! String]`'s own `[len: i64][elem, ...]` layout (`json.rs`'s own
//! `lower_array`), just storing a raw `i64` value per slot instead of
//! a pointer; no new codegen mechanism is actually needed, since
//! `Array[T]` construction already stores whatever `BasicValueEnum`
//! build_expr produces, regardless of kind.

use std::ffi::c_void;
use std::os::raw::c_char;
use sysinfo::{Disks, Pid, System};

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

fn build_string_array(items: &[String]) -> *mut c_void {
  unsafe {
    let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
    *ptr = items.len() as i64;
    let elems = ptr.add(1) as *mut *const c_char;
    for (i, s) in items.iter().enumerate() {
      *elems.add(i) = crate::alloc_and_copy_str(s);
    }
    ptr as *mut c_void
  }
}

fn build_int64_array(items: &[i64]) -> *mut c_void {
  unsafe {
    let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
    *ptr = items.len() as i64;
    let elems = ptr.add(1);
    for (i, v) in items.iter().enumerate() {
      *elems.add(i) = *v;
    }
    ptr as *mut c_void
  }
}

pub fn cpu_count() -> i64 {
  System::new_with_specifics(
    sysinfo::RefreshKind::nothing().with_cpu(sysinfo::CpuRefreshKind::everything()),
  )
  .cpus()
  .len() as i64
}

pub fn total_memory_bytes() -> i64 {
  let mut sys = System::new_all();
  sys.refresh_memory();
  sys.total_memory() as i64
}

pub fn used_memory_bytes() -> i64 {
  let mut sys = System::new_all();
  sys.refresh_memory();
  sys.used_memory() as i64
}

pub fn disk_names() -> *mut c_void {
  let disks = Disks::new_with_refreshed_list();
  let names: Vec<String> = disks
    .iter()
    .map(|d| d.name().to_string_lossy().into_owned())
    .collect();
  build_string_array(&names)
}

pub fn disk_names_count() -> i64 {
  Disks::new_with_refreshed_list().iter().count() as i64
}

/// # Safety
/// `name`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn disk_total_bytes(name: *const c_char) -> i64 {
  let name = match read_str(name) {
    Ok(n) => n,
    Err(_) => return -1,
  };
  let disks = Disks::new_with_refreshed_list();
  disks
    .iter()
    .find(|d| d.name().to_string_lossy() == name)
    .map(|d| d.total_space() as i64)
    .unwrap_or(-1)
}

/// # Safety
/// `name`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn disk_available_bytes(name: *const c_char) -> i64 {
  let name = match read_str(name) {
    Ok(n) => n,
    Err(_) => return -1,
  };
  let disks = Disks::new_with_refreshed_list();
  disks
    .iter()
    .find(|d| d.name().to_string_lossy() == name)
    .map(|d| d.available_space() as i64)
    .unwrap_or(-1)
}

pub fn process_ids() -> *mut c_void {
  let mut sys = System::new_all();
  sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
  let ids: Vec<i64> = sys.processes().keys().map(|p| p.as_u32() as i64).collect();
  build_int64_array(&ids)
}

pub fn process_ids_count() -> i64 {
  let mut sys = System::new_all();
  sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
  sys.processes().len() as i64
}

/// `System.process_name(pid: Int64): Option[String]` — the Rust side
/// returns a bare nullable `*mut c_char`; `emerald-codegen`'s own
/// call site builds the real tagged `Option[String]` value from it,
/// the same pattern `Env.get`/`String.from_cstring` already establish.
/// A miss (the process exited between enumeration and this lookup, a
/// real TOCTOU-shaped race inherent to process introspection on every
/// OS) returns null, not a panic.
pub fn process_name(pid: i64) -> *mut c_char {
  let mut sys = System::new_all();
  sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
  match sys.process(Pid::from_u32(pid as u32)) {
    Some(p) => unsafe { crate::alloc_and_copy_str(&p.name().to_string_lossy()) as *mut c_char },
    None => std::ptr::null_mut(),
  }
}

/// `System.process_memory_bytes(pid: Int64): Int64` — `-1` on a miss,
/// explicitly provisional pending plan 92's own general scalar-return
/// error convention (the identical disclosed placeholder plan 91's
/// own proof function already used).
pub fn process_memory_bytes(pid: i64) -> i64 {
  let mut sys = System::new_all();
  sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
  sys
    .process(Pid::from_u32(pid as u32))
    .map(|p| p.memory() as i64)
    .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cpu_count_is_positive_on_the_real_test_machine() {
    assert!(cpu_count() > 0);
  }

  #[test]
  fn total_memory_is_at_least_used_memory() {
    assert!(total_memory_bytes() >= used_memory_bytes());
  }

  #[test]
  fn process_ids_contains_this_real_test_processs_own_pid() {
    let own_pid = std::process::id() as i64;
    let ids_ptr = process_ids() as *const i64;
    unsafe {
      let len = *ids_ptr;
      let elems = std::slice::from_raw_parts(ids_ptr.add(1), len as usize);
      assert!(
        elems.contains(&own_pid),
        "expected process_ids() to contain this test's own pid {own_pid}"
      );
    }
  }

  #[test]
  fn a_lookup_against_a_pid_that_does_not_exist_returns_the_disclosed_sentinel() {
    let bogus_pid = 999_999_999i64;
    let name_ptr = process_name(bogus_pid);
    assert!(name_ptr.is_null());
    assert_eq!(process_memory_bytes(bogus_pid), -1);
  }
}
