//! Plan 142 (Embedded ACID Database, redb) — `Redb.open`/`.close`/
//! `.table`/`.begin_write`/`.begin_read`/`.table_insert`/`.table_get`/
//! `.table_remove`/`.commit`/`.abort`, wrapping `redb` — this batch's
//! recommended default embedded key-value store over plan 141's
//! stalled `sled` (this plan's own history file's Decision log has
//! the full comparative vetting).
//!
//! Every handle in this surface (database, table schema, write
//! transaction, read transaction) is a bare `i64` id in plan 93's own
//! `crate::handle` registry — never a `Type::Newtype` (this plan's
//! own, and `emerald-sema`'s own, Decision log), the identical shape
//! plan 137's `Sqlite` already established. Four distinct type tags
//! (`DB_TAG`/`TABLE_TAG`/`WRITE_TXN_TAG`/`READ_TXN_TAG`) share that
//! one registry, the same multi-tag-one-registry shape `sqlite.rs`
//! already uses.
//!
//! Table names need runtime interning to satisfy a real Rust API
//! constraint: `redb::TableDefinition<'static, K, V>` requires a
//! `'static` name, but Emerald table names only exist as runtime
//! `String` values. `redb_table` resolves this with a one-time-per-
//! distinct-name `Box::leak`, cached in a global `Mutex<HashMap<
//! String, i64>>` keyed by the name itself — calling `Redb.table(
//! "users")` any number of times returns the SAME handle, never a
//! fresh leak per call (this plan's own Decision log).
//!
//! `WriteTransaction::commit`/`.abort` both consume `self` by value
//! (redb's real, current API, re-verified directly against its own
//! vendored source rather than assumed) — `RedbWriteTxnHandle`
//! therefore wraps an `Option<redb::WriteTransaction>`, `.take()`n by
//! whichever of `redb_commit`/`redb_abort` runs first; a second
//! `commit`/`abort` (or a `table_insert`/`table_get`/`table_remove`
//! after either) raises a real `NativeError` naming the transaction
//! already consumed, and the handle itself is separately marked
//! closed via `handle_close` so any further use gets plan 93's own
//! "use of closed" diagnostic instead.
//!
//! `table_get` is valid against either a write or a read transaction
//! (a write transaction can read its own uncommitted writes, redb's
//! real MVCC semantics): it tries the write-transaction tag first,
//! then the read-transaction tag. `table_insert`/`table_remove` only
//! ever look up the write-transaction tag — hand either a genuine
//! read-transaction handle and `handle_get_mut`'s own type-tag check
//! raises plan 93's own "holds a different resource type" diagnostic,
//! the concrete, existing mechanism this plan's own Decision log
//! leans on for "table_insert/table_remove are plan-93 errors if the
//! handle names a read transaction", not bespoke new code.
//!
//! Values marshal as `String`, not raw bytes — the same disclosed
//! `Bytes`-type gap every other storage plan in this batch (137, 138,
//! 139, 140, 141) already hits (`emerald-sema`'s own Decision log for
//! this plan). `table_insert`/`table_get`/`table_remove` all return
//! `Option[String]`, mirroring `redb`'s own real `Option<AccessGuard<
//! V>>` return shape for `insert`/`get`/`remove` alike — the previous
//! value for `insert`/`remove`, the current value for `get`.
//!
//! No function in this surface is `Result`-wrapped — a real `redb`
//! error (a write attempted through a read transaction, a closed or
//! unknown handle, a genuine storage error) raises a plain, catchable
//! `NativeError` instead, the same convention `Sqlite.*`/`Tempfile.
//! create`/plan 145's `Process.run` already use.

use crate::handle::{handle_alloc, handle_close, handle_get, handle_get_mut};
use redb::{ReadableDatabase, ReadableTable};
use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::{Mutex, OnceLock};

const DB_TAG: &str = "RedbDatabase";
const TABLE_TAG: &str = "RedbTable";
const WRITE_TXN_TAG: &str = "RedbWriteTxn";
const READ_TXN_TAG: &str = "RedbReadTxn";

type StrTableDef = redb::TableDefinition<'static, &'static str, &'static str>;

/// A real, live `redb::Database` — see this module's own doc comment.
struct RedbDatabaseHandle {
  db: redb::Database,
}

/// `inner` goes to `None` once `commit`/`abort` has consumed the real
/// `redb::WriteTransaction` — see this module's own doc comment.
struct RedbWriteTxnHandle {
  inner: Option<redb::WriteTransaction>,
}

/// No `Option` wrapper needed: a `redb::ReadTransaction` is never
/// consumed by any operation this surface exposes (only `get`, never
/// `commit`/`abort`) — see this module's own doc comment.
struct RedbReadTxnHandle {
  inner: redb::ReadTransaction,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// The interned-name-to-handle cache `redb_table`'s own doc comment
/// describes — keyed by the table name itself, never a fresh entry
/// per call.
fn table_registry() -> &'static Mutex<HashMap<String, i64>> {
  static REGISTRY: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
  REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn table_def(table: i64) -> Result<StrTableDef, String> {
  handle_get::<StrTableDef>(table, TABLE_TAG)
}

/// `Redb.open(path: String): Int64` — `redb::Database::create`
/// creates the file if absent, opens it if present (redb's own
/// documented behavior for this exact function).
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn redb_open(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match redb::Database::create(path) {
    Ok(db) => handle_alloc(Box::new(RedbDatabaseHandle { db }), DB_TAG),
    Err(e) => crate::raise_native_error(&format!("Redb.open: {e}")),
  }
}

/// `Redb.close(db: Int64): Void` — removes and drops the entry; an
/// already-closed or unknown handle is a harmless no-op, matching
/// `handle_close`'s own convention.
pub fn redb_close(db: i64) {
  handle_close(db);
}

/// `Redb.table(name: String): Int64` — see this module's own doc
/// comment for the interning/caching discipline.
///
/// # Safety
/// `name`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn redb_table(name: *const c_char) -> i64 {
  let name = match read_str(name) {
    Ok(n) => n,
    Err(e) => crate::raise_native_error(&e),
  };
  let mut reg = table_registry().lock().unwrap_or_else(|p| p.into_inner());
  if let Some(&id) = reg.get(name) {
    return id;
  }
  let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
  let def: StrTableDef = redb::TableDefinition::new(leaked);
  let id = handle_alloc(Box::new(def), TABLE_TAG);
  reg.insert(leaked.to_string(), id);
  id
}

/// `Redb.begin_write(db: Int64): Int64`.
pub fn redb_begin_write(db: i64) -> i64 {
  let result = handle_get_mut::<
    RedbDatabaseHandle,
    Result<redb::WriteTransaction, redb::TransactionError>,
  >(db, DB_TAG, |d| d.db.begin_write());
  match result {
    Ok(Ok(txn)) => handle_alloc(
      Box::new(RedbWriteTxnHandle { inner: Some(txn) }),
      WRITE_TXN_TAG,
    ),
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Redb.begin_write: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Redb.begin_read(db: Int64): Int64`.
pub fn redb_begin_read(db: i64) -> i64 {
  let result = handle_get_mut::<
    RedbDatabaseHandle,
    Result<redb::ReadTransaction, redb::TransactionError>,
  >(db, DB_TAG, |d| d.db.begin_read());
  match result {
    Ok(Ok(txn)) => handle_alloc(Box::new(RedbReadTxnHandle { inner: txn }), READ_TXN_TAG),
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Redb.begin_read: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Redb.table_insert(txn: Int64, table: Int64, key: String, value:
/// String): String?` — the previous value, if the key was already
/// present; a plan-93 error (via `handle_get_mut`'s own type-tag
/// check) if `txn` names a read transaction, not a write one.
///
/// # Safety
/// `key`/`value`, if non-null, must each point to a valid,
/// NUL-terminated C string.
pub unsafe fn redb_table_insert(
  txn: i64,
  table: i64,
  key: *const c_char,
  value: *const c_char,
) -> *mut c_char {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  let value = match read_str(value) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(&e),
  };
  let def = match table_def(table) {
    Ok(d) => d,
    Err(e) => crate::raise_native_error(&e),
  };
  let result =
    handle_get_mut::<RedbWriteTxnHandle, Result<Option<String>, String>>(txn, WRITE_TXN_TAG, |w| {
      let wt = w.inner.as_ref().ok_or_else(|| {
        "Redb.table_insert: write transaction already committed or aborted".to_string()
      })?;
      let mut t = wt.open_table(def).map_err(|e| e.to_string())?;
      let old = t.insert(key, value).map_err(|e| e.to_string())?;
      Ok(old.map(|g| g.value().to_string()))
    });
  match result {
    Ok(Ok(Some(s))) => crate::alloc_and_copy_str(&s) as *mut c_char,
    Ok(Ok(None)) => std::ptr::null_mut(),
    Ok(Err(e)) => crate::raise_native_error(&format!("Redb.table_insert: {e}")),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Redb.table_get(txn: Int64, table: Int64, key: String): String?` —
/// valid against either a write or a read transaction handle (this
/// module's own doc comment).
///
/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn redb_table_get(txn: i64, table: i64, key: *const c_char) -> *mut c_char {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  let def = match table_def(table) {
    Ok(d) => d,
    Err(e) => crate::raise_native_error(&e),
  };
  let write_result =
    handle_get_mut::<RedbWriteTxnHandle, Result<Option<String>, String>>(txn, WRITE_TXN_TAG, |w| {
      let wt = w.inner.as_ref().ok_or_else(|| {
        "Redb.table_get: write transaction already committed or aborted".to_string()
      })?;
      let t = wt.open_table(def).map_err(|e| e.to_string())?;
      let val = t.get(key).map_err(|e| e.to_string())?;
      Ok(val.map(|g| g.value().to_string()))
    });
  let result = match write_result {
    Ok(inner) => inner,
    // Not a live write-transaction handle — try a read transaction,
    // the "either transaction kind" behavior this module's own doc
    // comment describes.
    Err(_) => {
      handle_get_mut::<RedbReadTxnHandle, Result<Option<String>, String>>(txn, READ_TXN_TAG, |r| {
        let t = r.inner.open_table(def).map_err(|e| e.to_string())?;
        let val = t.get(key).map_err(|e| e.to_string())?;
        Ok(val.map(|g| g.value().to_string()))
      })
      .and_then(|inner| inner)
    }
  };
  match result {
    Ok(Some(s)) => crate::alloc_and_copy_str(&s) as *mut c_char,
    Ok(None) => std::ptr::null_mut(),
    Err(e) => crate::raise_native_error(&format!("Redb.table_get: {e}")),
  }
}

/// `Redb.table_remove(txn: Int64, table: Int64, key: String):
/// String?` — the removed value, if the key was present; a plan-93
/// error (via `handle_get_mut`'s own type-tag check) if `txn` names a
/// read transaction, not a write one.
///
/// # Safety
/// `key`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn redb_table_remove(txn: i64, table: i64, key: *const c_char) -> *mut c_char {
  let key = match read_str(key) {
    Ok(k) => k,
    Err(e) => crate::raise_native_error(&e),
  };
  let def = match table_def(table) {
    Ok(d) => d,
    Err(e) => crate::raise_native_error(&e),
  };
  let result =
    handle_get_mut::<RedbWriteTxnHandle, Result<Option<String>, String>>(txn, WRITE_TXN_TAG, |w| {
      let wt = w.inner.as_ref().ok_or_else(|| {
        "Redb.table_remove: write transaction already committed or aborted".to_string()
      })?;
      let mut t = wt.open_table(def).map_err(|e| e.to_string())?;
      let old = t.remove(key).map_err(|e| e.to_string())?;
      Ok(old.map(|g| g.value().to_string()))
    });
  match result {
    Ok(Ok(Some(s))) => crate::alloc_and_copy_str(&s) as *mut c_char,
    Ok(Ok(None)) => std::ptr::null_mut(),
    Ok(Err(e)) => crate::raise_native_error(&format!("Redb.table_remove: {e}")),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Redb.commit(txn: Int64): Void` — consumes and commits the write
/// transaction; see this module's own doc comment.
pub fn redb_commit(txn: i64) {
  let result = handle_get_mut::<RedbWriteTxnHandle, Result<(), String>>(txn, WRITE_TXN_TAG, |w| {
    match w.inner.take() {
      Some(inner) => inner.commit().map_err(|e| e.to_string()),
      None => Err("Redb.commit: write transaction already committed or aborted".to_string()),
    }
  });
  match result {
    Ok(Ok(())) => {
      handle_close(txn);
    }
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Redb.commit: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Redb.abort(txn: Int64): Void` — explicitly rolls back the write
/// transaction; see this module's own doc comment.
pub fn redb_abort(txn: i64) {
  let result = handle_get_mut::<RedbWriteTxnHandle, Result<(), String>>(txn, WRITE_TXN_TAG, |w| {
    match w.inner.take() {
      Some(inner) => inner.abort().map_err(|e| e.to_string()),
      None => Err("Redb.abort: write transaction already committed or aborted".to_string()),
    }
  });
  match result {
    Ok(Ok(())) => {
      handle_close(txn);
    }
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Redb.abort: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_db_path(name: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    // Leak the tempdir so the file survives past this function's own
    // return — every test below opens its own uniquely named file, so
    // nothing collides across the `#[test]` run.
    std::mem::forget(dir);
    path
  }

  fn cstr(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
  }

  #[test]
  fn write_then_commit_then_read_back_round_trips() {
    unsafe {
      let path = temp_db_path("plan142_roundtrip.redb");
      let path_c = cstr(path.to_str().unwrap());
      let db = redb_open(path_c.as_ptr());
      let table = redb_table(cstr("users").as_ptr());

      let write_txn = redb_begin_write(db);
      let old = redb_table_insert(write_txn, table, cstr("1").as_ptr(), cstr("Ada").as_ptr());
      assert!(old.is_null(), "fresh key should have no previous value");
      redb_table_insert(write_txn, table, cstr("2").as_ptr(), cstr("Grace").as_ptr());
      redb_commit(write_txn);

      let read_txn = redb_begin_read(db);
      let first = redb_table_get(read_txn, table, cstr("1").as_ptr());
      assert!(!first.is_null());
      assert_eq!(std::ffi::CStr::from_ptr(first).to_str().unwrap(), "Ada");

      let second = redb_table_get(read_txn, table, cstr("2").as_ptr());
      assert_eq!(std::ffi::CStr::from_ptr(second).to_str().unwrap(), "Grace");

      let missing = redb_table_get(read_txn, table, cstr("3").as_ptr());
      assert!(missing.is_null());

      redb_close(db);
    }
  }

  #[test]
  fn an_aborted_write_transactions_changes_are_never_visible_to_a_later_read() {
    unsafe {
      let path = temp_db_path("plan142_abort.redb");
      let path_c = cstr(path.to_str().unwrap());
      let db = redb_open(path_c.as_ptr());
      let table = redb_table(cstr("users").as_ptr());

      // Seed a committed baseline row first, so the table itself
      // exists for the later read transaction to open (redb raises if
      // a read transaction opens a table that was never created).
      let seed_txn = redb_begin_write(db);
      redb_table_insert(seed_txn, table, cstr("1").as_ptr(), cstr("Ada").as_ptr());
      redb_commit(seed_txn);

      let write_txn = redb_begin_write(db);
      redb_table_insert(write_txn, table, cstr("2").as_ptr(), cstr("Grace").as_ptr());
      redb_abort(write_txn);

      let read_txn = redb_begin_read(db);
      let first = redb_table_get(read_txn, table, cstr("1").as_ptr());
      assert_eq!(std::ffi::CStr::from_ptr(first).to_str().unwrap(), "Ada");
      let aborted = redb_table_get(read_txn, table, cstr("2").as_ptr());
      assert!(
        aborted.is_null(),
        "an aborted write transaction's changes must not be visible"
      );

      redb_close(db);
    }
  }

  #[test]
  fn calling_table_with_the_same_name_twice_returns_the_same_handle() {
    unsafe {
      let a = redb_table(cstr("same_name").as_ptr());
      let b = redb_table(cstr("same_name").as_ptr());
      assert_eq!(a, b);
    }
  }

  // `table_insert`/`table_remove` raising a real `NativeError` when
  // `txn` names a read transaction (via `handle_get_mut`'s own
  // type-tag check) is NOT exercised as a Rust `#[test]` here —
  // `crate::raise_native_error`'s real raise mechanism is a C-level
  // `longjmp` with no safe in-process stand-in; this crate's own
  // `test_stubs::emerald_raise` deliberately calls
  // `std::process::abort()` rather than panic (see that stub's own
  // doc comment), so a Rust-level `catch_unwind` around a call that
  // reaches it aborts the whole test binary rather than catching
  // anything. Matching `sqlite.rs`'s own established precedent (no
  // error-raising path is unit-tested there either), this plan's own
  // raise path is left to `examples/redb_kv.em`'s real, full-pipeline
  // CLI run instead.
}
