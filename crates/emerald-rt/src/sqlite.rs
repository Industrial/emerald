//! Plan 137 (SQLite) — `Sqlite.open`/`.open_memory`/`.close`/
//! `.execute_direct`/`.prepare`/`.bind_string`/`.bind_int64`/
//! `.bind_float64`/`.bind_null`/`.execute`/`.query`/`.step`/
//! `.column_string`/`.column_int64`/`.column_float64`/`.begin`/
//! `.commit`/`.rollback`, wrapping `rusqlite` — this batch's one
//! deliberate, plan-95-sanctioned C-binding exception (see this
//! plan's own history file's Decision log for the full account of
//! why, checked directly against `turso` first).
//!
//! Every handle in this surface (connection, prepared statement,
//! result cursor) is a bare `i64` id in plan 93's own `crate::handle`
//! registry — never a `Type::Newtype` (see `emerald-sema`'s own
//! Decision log for this plan). Three distinct type tags
//! (`CONN_TAG`/`STMT_TAG`/`CURSOR_TAG`) share that one registry, the
//! same multi-tag-one-registry shape plan 93 itself already permits.
//!
//! A prepared statement (`SqliteStatement`) holds no live `rusqlite::
//! Statement` at all — only owned SQL text and an owned, positionally
//! indexed parameter buffer (`Vec<SqliteValue>`), filled by `bind_*`.
//! Only `sqlite_execute`/`sqlite_query` ever call `conn.prepare(&sql)`
//! — never `sqlite_prepare` itself — bind, run to completion, and for
//! `sqlite_query`, eagerly drain every row into an owned `Vec<Vec<
//! SqliteValue>>` (`SqliteCursor`), all inside that one Rust function
//! body, before the borrow on `Connection` ever has to survive a
//! return. This is what makes the whole surface soundly buildable
//! with zero `unsafe` lifetime extension across the real `rusqlite::
//! Statement<'conn>` self-referential problem — see this plan's own
//! Decision log for the full account.
//!
//! `sqlite_close` refuses to run (raising a real `NativeError`) while
//! any statement/cursor prepared against that connection is still
//! "live": a freshly prepared statement counts as live from `prepare`
//! until its first `execute`/`query` call (after which it holds no
//! `rusqlite` object anyway, so it stops counting — a statement can
//! still be re-run any number of times afterward, it just stops being
//! what blocks `close`); a cursor counts as live from `query` until
//! `step` has walked past its last buffered row. `SqliteConnection`
//! tracks this itself via an `open_children: i64` field, adjusted by
//! `conn_child_inc`/`conn_child_dec` below — never a second, separate
//! side-table. Matching `handle_close`'s own convention (`handle.rs`'s
//! doc comment), closing an already-closed or unknown connection is a
//! harmless no-op, never a raise — only a genuinely live connection
//! with live children refuses to close.
//!
//! No `Bytes`/blob support in v1 (`emerald-sema`'s Decision log,
//! mirroring plan 59's own finding) — a `BLOB` column value is stored
//! internally (`SqliteValue::Blob`) but every `column_*` accessor
//! raises a `NativeError` naming the column if called against one,
//! rather than silently returning invalid UTF-8 or garbage.
//!
//! Parameterized queries are the only path that carries a caller
//! value into SQL text: `bind_string`/`bind_int64`/`bind_float64`/
//! `bind_null` write into an owned parameter buffer bound through
//! `rusqlite`'s own `?`-positional slots, never string-concatenated
//! into `sql`. `execute_direct` exists only for parameter-free DDL —
//! see this plan's own Decision log for why no value-carrying overload
//! of it exists.
//!
//! Error handling: every `rusqlite::Error` (constraint violation,
//! malformed SQL, a type-coercion failure on a `column_*` call) and
//! every closed/unknown-handle use raises a real, catchable
//! `NativeError` via `crate::raise_native_error` — no function in
//! this surface returns `Result`, matching `Tempfile.create`/`File`'s
//! own functions/plan 145's `Process.run` (see this plan's own
//! Decision log: transactions are three thin `BEGIN`/`COMMIT`/
//! `ROLLBACK` statement calls, not a new control-flow construct, so
//! there is no natural `Result`-typed boundary to hang this behind
//! either).

use crate::handle::{handle_alloc, handle_close, handle_get_mut};
use rusqlite::types::{ToSql, ToSqlOutput, Value, ValueRef};
use std::os::raw::c_char;

const CONN_TAG: &str = "SqliteConnection";
const STMT_TAG: &str = "SqliteStatement";
const CURSOR_TAG: &str = "SqliteCursor";

/// One value of one bound parameter, or one column of one buffered
/// result row — an owned, `'static` enum (`Blob`/`Text` own their own
/// bytes), never a borrowed `rusqlite::types::ValueRef<'_>`, the plain
/// owned-data shape this module's own doc comment requires throughout.
#[derive(Debug, Clone)]
enum SqliteValue {
  Null,
  Integer(i64),
  Real(f64),
  Text(String),
  Blob(Vec<u8>),
}

impl ToSql for SqliteValue {
  fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
    Ok(match self {
      SqliteValue::Null => ToSqlOutput::Borrowed(ValueRef::Null),
      SqliteValue::Integer(i) => ToSqlOutput::Owned(Value::Integer(*i)),
      SqliteValue::Real(f) => ToSqlOutput::Owned(Value::Real(*f)),
      SqliteValue::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
      SqliteValue::Blob(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b.as_slice())),
    })
  }
}

impl From<ValueRef<'_>> for SqliteValue {
  fn from(v: ValueRef<'_>) -> Self {
    match v {
      ValueRef::Null => SqliteValue::Null,
      ValueRef::Integer(i) => SqliteValue::Integer(i),
      ValueRef::Real(f) => SqliteValue::Real(f),
      ValueRef::Text(t) => SqliteValue::Text(String::from_utf8_lossy(t).into_owned()),
      ValueRef::Blob(b) => SqliteValue::Blob(b.to_vec()),
    }
  }
}

fn sqlite_type_name(v: &SqliteValue) -> &'static str {
  match v {
    SqliteValue::Null => "NULL",
    SqliteValue::Integer(_) => "INTEGER",
    SqliteValue::Real(_) => "REAL",
    SqliteValue::Text(_) => "TEXT",
    SqliteValue::Blob(_) => "BLOB",
  }
}

/// A real, live `rusqlite::Connection` plus `open_children`, the
/// live-statement/cursor reference count `sqlite_close` checks — see
/// this module's own doc comment.
struct SqliteConnection {
  conn: rusqlite::Connection,
  open_children: i64,
}

/// Owned SQL text plus an owned, positionally indexed parameter
/// buffer — no live `rusqlite::Statement` (this module's own doc
/// comment). `active` tracks whether this statement still counts
/// toward its own connection's `open_children`: true from `prepare`
/// until the first `execute`/`query` call.
struct SqliteStatement {
  conn_id: i64,
  sql: String,
  params: Vec<SqliteValue>,
  active: bool,
}

/// Every row of a `SELECT`, eagerly drained at `query` time (this
/// module's own doc comment). `pos` starts at `-1` (before the first
/// row); `step` advances it. `active` tracks whether this cursor still
/// counts toward its own connection's `open_children`: true until
/// `step` walks past the last row.
struct SqliteCursor {
  conn_id: i64,
  rows: Vec<Vec<SqliteValue>>,
  pos: i64,
  active: bool,
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

fn conn_child_inc(conn_id: i64) -> Result<(), String> {
  handle_get_mut::<SqliteConnection, ()>(conn_id, CONN_TAG, |c| {
    c.open_children += 1;
  })
}

fn conn_child_dec(conn_id: i64) -> Result<(), String> {
  handle_get_mut::<SqliteConnection, ()>(conn_id, CONN_TAG, |c| {
    c.open_children -= 1;
  })
}

/// `Sqlite.open(path: String): Int64`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn sqlite_open(path: *const c_char) -> i64 {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => crate::raise_native_error(&e),
  };
  match rusqlite::Connection::open(path) {
    Ok(conn) => handle_alloc(
      Box::new(SqliteConnection {
        conn,
        open_children: 0,
      }),
      CONN_TAG,
    ),
    Err(e) => crate::raise_native_error(&format!("Sqlite.open: {e}")),
  }
}

/// `Sqlite.open_memory(): Int64`.
pub fn sqlite_open_memory() -> i64 {
  match rusqlite::Connection::open_in_memory() {
    Ok(conn) => handle_alloc(
      Box::new(SqliteConnection {
        conn,
        open_children: 0,
      }),
      CONN_TAG,
    ),
    Err(e) => unsafe { crate::raise_native_error(&format!("Sqlite.open_memory: {e}")) },
  }
}

/// `Sqlite.close(conn: Int64): Void` — refuses to run (raising a real
/// `NativeError`) while any statement/cursor prepared against `conn`
/// is still live; an already-closed or unknown `conn` is a harmless
/// no-op, matching `handle_close`'s own convention — see this
/// module's own doc comment.
pub fn sqlite_close(conn: i64) {
  let open = handle_get_mut::<SqliteConnection, i64>(conn, CONN_TAG, |c| c.open_children);
  match open {
    Ok(n) if n > 0 => unsafe {
      crate::raise_native_error(&format!(
        "Sqlite.close: connection {conn} still has {n} open statement/cursor handle(s)"
      ))
    },
    _ => {
      handle_close(conn);
    }
  }
}

/// `Sqlite.execute_direct(conn: Int64, sql: String): Void` —
/// parameter-free DDL only (`CREATE TABLE`/`CREATE INDEX`); see this
/// plan's own Decision log for why no value-carrying overload of this
/// function exists.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle; `sql`, if
/// non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn sqlite_execute_direct(conn: i64, sql: *const c_char) {
  let sql = match read_str(sql) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  match handle_get_mut::<SqliteConnection, rusqlite::Result<usize>>(conn, CONN_TAG, |c| {
    c.conn.execute(sql, rusqlite::params![])
  }) {
    Ok(Ok(_)) => {}
    Ok(Err(e)) => crate::raise_native_error(&format!("Sqlite.execute_direct: {e}")),
    Err(e) => crate::raise_native_error(&e),
  }
}

/// `Sqlite.prepare(conn: Int64, sql: String): Int64` — allocates a
/// statement handle holding only owned SQL text and an empty
/// parameter buffer; no live `rusqlite::Statement` yet (this module's
/// own doc comment) — `conn`'s own liveness is still checked (via
/// `conn_child_inc`), but `conn.prepare` itself is never called here.
///
/// # Safety
/// `conn` must be a live `Sqlite` connection handle; `sql`, if
/// non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn sqlite_prepare(conn: i64, sql: *const c_char) -> i64 {
  let sql = match read_str(sql) {
    Ok(s) => s.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  if let Err(e) = conn_child_inc(conn) {
    crate::raise_native_error(&e);
  }
  handle_alloc(
    Box::new(SqliteStatement {
      conn_id: conn,
      sql,
      params: Vec::new(),
      active: true,
    }),
    STMT_TAG,
  )
}

fn bind_at(stmt: i64, index: i64, value: SqliteValue) -> Result<(), String> {
  if index < 1 {
    return Err(format!(
      "Sqlite.bind_*: parameter index must be >= 1, got {index}"
    ));
  }
  handle_get_mut::<SqliteStatement, ()>(stmt, STMT_TAG, |s| {
    let idx = (index - 1) as usize;
    if s.params.len() <= idx {
      s.params.resize_with(idx + 1, || SqliteValue::Null);
    }
    s.params[idx] = value;
  })
}

/// `Sqlite.bind_string(stmt: Int64, index: Int64, value: String): Void`.
///
/// # Safety
/// `stmt` must be a live `Sqlite` statement handle; `value`, if
/// non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn sqlite_bind_string(stmt: i64, index: i64, value: *const c_char) {
  let value = match read_str(value) {
    Ok(v) => v.to_string(),
    Err(e) => crate::raise_native_error(&e),
  };
  if let Err(e) = bind_at(stmt, index, SqliteValue::Text(value)) {
    crate::raise_native_error(&e);
  }
}

/// `Sqlite.bind_int64(stmt: Int64, index: Int64, value: Int64): Void`.
pub fn sqlite_bind_int64(stmt: i64, index: i64, value: i64) {
  if let Err(e) = bind_at(stmt, index, SqliteValue::Integer(value)) {
    unsafe { crate::raise_native_error(&e) };
  }
}

/// `Sqlite.bind_float64(stmt: Int64, index: Int64, value: Float64): Void`.
pub fn sqlite_bind_float64(stmt: i64, index: i64, value: f64) {
  if let Err(e) = bind_at(stmt, index, SqliteValue::Real(value)) {
    unsafe { crate::raise_native_error(&e) };
  }
}

/// `Sqlite.bind_null(stmt: Int64, index: Int64): Void`.
pub fn sqlite_bind_null(stmt: i64, index: i64) {
  if let Err(e) = bind_at(stmt, index, SqliteValue::Null) {
    unsafe { crate::raise_native_error(&e) };
  }
}

// Reads (conn_id, sql, params) out of a statement handle and, if this
// is the statement's first execute/query, marks it inactive and
// decrements its connection's open_children — shared by
// sqlite_execute/sqlite_query, both of which materialize a real
// rusqlite::Statement immediately after calling this.
fn take_statement_for_run(stmt: i64) -> Result<(i64, String, Vec<SqliteValue>), String> {
  let (conn_id, sql, params, was_active) = handle_get_mut::<
    SqliteStatement,
    (i64, String, Vec<SqliteValue>, bool),
  >(stmt, STMT_TAG, |s| {
    let was_active = s.active;
    s.active = false;
    (s.conn_id, s.sql.clone(), s.params.clone(), was_active)
  })?;
  if was_active {
    conn_child_dec(conn_id)?;
  }
  Ok((conn_id, sql, params))
}

/// `Sqlite.execute(stmt: Int64): Int64` — INSERT/UPDATE/DELETE,
/// returns the real affected-row count.
pub fn sqlite_execute(stmt: i64) -> i64 {
  let (conn_id, sql, params) = match take_statement_for_run(stmt) {
    Ok(v) => v,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  let result =
    handle_get_mut::<SqliteConnection, rusqlite::Result<usize>>(conn_id, CONN_TAG, |c| {
      let mut prepared = c.conn.prepare(&sql)?;
      prepared.execute(rusqlite::params_from_iter(params.iter().cloned()))
    });
  match result {
    Ok(Ok(n)) => n as i64,
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Sqlite.execute: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Sqlite.query(stmt: Int64): Int64` — SELECT, returns a new
/// result-cursor handle with every row already eagerly buffered (this
/// module's own doc comment).
pub fn sqlite_query(stmt: i64) -> i64 {
  let (conn_id, sql, params) = match take_statement_for_run(stmt) {
    Ok(v) => v,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  let result = handle_get_mut::<SqliteConnection, rusqlite::Result<Vec<Vec<SqliteValue>>>>(
    conn_id,
    CONN_TAG,
    |c| {
      let mut prepared = c.conn.prepare(&sql)?;
      let col_count = prepared.column_count();
      let mut rows_out = Vec::new();
      let mut rows = prepared.query(rusqlite::params_from_iter(params.iter().cloned()))?;
      while let Some(row) = rows.next()? {
        let mut cols = Vec::with_capacity(col_count);
        for i in 0..col_count {
          cols.push(SqliteValue::from(row.get_ref(i)?));
        }
        rows_out.push(cols);
      }
      Ok(rows_out)
    },
  );
  let rows = match result {
    Ok(Ok(rows)) => rows,
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Sqlite.query: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  if let Err(e) = conn_child_inc(conn_id) {
    unsafe { crate::raise_native_error(&e) };
  }
  handle_alloc(
    Box::new(SqliteCursor {
      conn_id,
      rows,
      pos: -1,
      active: true,
    }),
    CURSOR_TAG,
  )
}

/// `Sqlite.step(cursor: Int64): Boolean` — advances to the next
/// buffered row, returning whether one exists; crosses the FFI
/// boundary as a plain `i64` (0/1), narrowed to a real `i1` by
/// `emerald-codegen`'s own call site, the same "widen the other
/// direction" convention `Regex#is_match` already establishes.
pub fn sqlite_step(cursor: i64) -> i64 {
  let result = handle_get_mut::<SqliteCursor, (bool, i64, bool)>(cursor, CURSOR_TAG, |c| {
    c.pos += 1;
    let has_row = (c.pos as usize) < c.rows.len();
    let just_exhausted = !has_row && c.active;
    if just_exhausted {
      c.active = false;
    }
    (has_row, c.conn_id, just_exhausted)
  });
  let (has_row, conn_id, just_exhausted) = match result {
    Ok(v) => v,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  if just_exhausted {
    if let Err(e) = conn_child_dec(conn_id) {
      unsafe { crate::raise_native_error(&e) };
    }
  }
  if has_row {
    1
  } else {
    0
  }
}

fn column_at(cursor: i64, col: i64) -> Result<SqliteValue, String> {
  handle_get_mut::<SqliteCursor, Result<SqliteValue, String>>(cursor, CURSOR_TAG, |c| {
    if c.pos < 0 || (c.pos as usize) >= c.rows.len() {
      return Err(format!(
        "Sqlite: cursor {cursor} has no current row (call .step first)"
      ));
    }
    let row = &c.rows[c.pos as usize];
    let idx = col as usize;
    row.get(idx).cloned().ok_or_else(|| {
      format!(
        "Sqlite: column index {col} out of range ({} columns)",
        row.len()
      )
    })
  })?
}

/// `Sqlite.column_string(cursor: Int64, col: Int64): String`.
///
/// # Safety
/// `cursor` must be a live `Sqlite` cursor handle, currently
/// positioned on a real row (a prior `.step` returned `true`).
pub unsafe fn sqlite_column_string(cursor: i64, col: i64) -> *const c_char {
  let value = match column_at(cursor, col) {
    Ok(v) => v,
    Err(e) => crate::raise_native_error(&e),
  };
  let s = match value {
    SqliteValue::Text(s) => s,
    SqliteValue::Integer(i) => i.to_string(),
    SqliteValue::Real(f) => f.to_string(),
    SqliteValue::Null => {
      crate::raise_native_error(&format!("Sqlite.column_string: column {col} is NULL"))
    }
    SqliteValue::Blob(_) => crate::raise_native_error(&format!(
      "Sqlite.column_string: column {col} is a BLOB, not supported (v1 has no Bytes/Blob column type — see this plan's own Decision log)"
    )),
  };
  crate::alloc_and_copy_str(&s)
}

/// `Sqlite.column_int64(cursor: Int64, col: Int64): Int64`.
pub fn sqlite_column_int64(cursor: i64, col: i64) -> i64 {
  let value = match column_at(cursor, col) {
    Ok(v) => v,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  match value {
    SqliteValue::Integer(i) => i,
    SqliteValue::Real(f) => f as i64,
    SqliteValue::Text(s) => match s.parse::<i64>() {
      Ok(i) => i,
      Err(_) => unsafe {
        crate::raise_native_error(&format!(
          "Sqlite.column_int64: column {col} holds non-numeric text ({s:?})"
        ))
      },
    },
    other => unsafe {
      crate::raise_native_error(&format!(
        "Sqlite.column_int64: column {col} is {}, cannot coerce to Int64",
        sqlite_type_name(&other)
      ))
    },
  }
}

/// `Sqlite.column_float64(cursor: Int64, col: Int64): Float64`.
pub fn sqlite_column_float64(cursor: i64, col: i64) -> f64 {
  let value = match column_at(cursor, col) {
    Ok(v) => v,
    Err(e) => unsafe { crate::raise_native_error(&e) },
  };
  match value {
    SqliteValue::Real(f) => f,
    SqliteValue::Integer(i) => i as f64,
    SqliteValue::Text(s) => match s.parse::<f64>() {
      Ok(f) => f,
      Err(_) => unsafe {
        crate::raise_native_error(&format!(
          "Sqlite.column_float64: column {col} holds non-numeric text ({s:?})"
        ))
      },
    },
    other => unsafe {
      crate::raise_native_error(&format!(
        "Sqlite.column_float64: column {col} is {}, cannot coerce to Float64",
        sqlite_type_name(&other)
      ))
    },
  }
}

fn run_txn_stmt(conn: i64, sql: &str) {
  match handle_get_mut::<SqliteConnection, rusqlite::Result<()>>(conn, CONN_TAG, |c| {
    c.conn.execute(sql, rusqlite::params![]).map(|_| ())
  }) {
    Ok(Ok(())) => {}
    Ok(Err(e)) => unsafe { crate::raise_native_error(&format!("Sqlite.{sql}: {e}")) },
    Err(e) => unsafe { crate::raise_native_error(&e) },
  }
}

/// `Sqlite.begin(conn: Int64): Void` — issues a literal `BEGIN`
/// through the connection directly; see this plan's own Decision log
/// for why transactions are three thin statement-shaped calls, not a
/// new control-flow construct.
pub fn sqlite_begin(conn: i64) {
  run_txn_stmt(conn, "BEGIN")
}

/// `Sqlite.commit(conn: Int64): Void`.
pub fn sqlite_commit(conn: i64) {
  run_txn_stmt(conn, "COMMIT")
}

/// `Sqlite.rollback(conn: Int64): Void`.
pub fn sqlite_rollback(conn: i64) {
  run_txn_stmt(conn, "ROLLBACK")
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  #[test]
  fn open_memory_create_insert_query_round_trip_the_concrete_proof_shape() {
    unsafe {
      let db = sqlite_open_memory();
      let create = c(
        "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL)",
      );
      sqlite_execute_direct(db, create.as_ptr());

      let insert_sql = c("INSERT INTO todos (title, done) VALUES (?, ?)");
      let insert = sqlite_prepare(db, insert_sql.as_ptr());
      let title = c("write plan 137");
      sqlite_bind_string(insert, 1, title.as_ptr());
      sqlite_bind_int64(insert, 2, 0);
      assert_eq!(sqlite_execute(insert), 1);

      let select_sql = c("SELECT id, title FROM todos WHERE done = ?");
      let select = sqlite_prepare(db, select_sql.as_ptr());
      sqlite_bind_int64(select, 1, 0);
      let cursor = sqlite_query(select);
      assert_eq!(sqlite_step(cursor), 1);
      assert_eq!(sqlite_column_int64(cursor, 0), 1);
      let title_out = std::ffi::CStr::from_ptr(sqlite_column_string(cursor, 1))
        .to_str()
        .unwrap()
        .to_string();
      assert_eq!(title_out, "write plan 137");
      assert_eq!(sqlite_step(cursor), 0);

      sqlite_close(db);
    }
  }

  #[test]
  fn bind_null_and_float64_and_multiple_rows_round_trip() {
    unsafe {
      let db = sqlite_open_memory();
      let create = c("CREATE TABLE measurements (label TEXT, value REAL)");
      sqlite_execute_direct(db, create.as_ptr());

      let insert_sql = c("INSERT INTO measurements (label, value) VALUES (?, ?)");
      let insert = sqlite_prepare(db, insert_sql.as_ptr());
      let label = c("temp");
      sqlite_bind_string(insert, 1, label.as_ptr());
      sqlite_bind_float64(insert, 2, 98.6);
      assert_eq!(sqlite_execute(insert), 1);

      let insert2 = sqlite_prepare(db, insert_sql.as_ptr());
      let label2 = c("unknown");
      sqlite_bind_string(insert2, 1, label2.as_ptr());
      sqlite_bind_null(insert2, 2);
      assert_eq!(sqlite_execute(insert2), 1);

      let select_sql = c("SELECT label, value FROM measurements ORDER BY label");
      let select = sqlite_prepare(db, select_sql.as_ptr());
      let cursor = sqlite_query(select);

      assert_eq!(sqlite_step(cursor), 1);
      let l0 = std::ffi::CStr::from_ptr(sqlite_column_string(cursor, 0))
        .to_str()
        .unwrap()
        .to_string();
      assert_eq!(l0, "temp");
      assert!((sqlite_column_float64(cursor, 1) - 98.6).abs() < 1e-9);

      assert_eq!(sqlite_step(cursor), 1);
      let l1 = std::ffi::CStr::from_ptr(sqlite_column_string(cursor, 0))
        .to_str()
        .unwrap()
        .to_string();
      assert_eq!(l1, "unknown");

      assert_eq!(sqlite_step(cursor), 0);
      sqlite_close(db);
    }
  }

  #[test]
  fn execute_returns_the_real_affected_row_count() {
    unsafe {
      let db = sqlite_open_memory();
      let create = c("CREATE TABLE t (n INTEGER)");
      sqlite_execute_direct(db, create.as_ptr());
      let insert_sql = c("INSERT INTO t (n) VALUES (1), (2), (3)");
      let insert = sqlite_prepare(db, insert_sql.as_ptr());
      assert_eq!(sqlite_execute(insert), 3);
      let update_sql = c("UPDATE t SET n = n + 1");
      let update = sqlite_prepare(db, update_sql.as_ptr());
      assert_eq!(sqlite_execute(update), 3);
      sqlite_close(db);
    }
  }

  #[test]
  fn double_close_is_a_harmless_no_op() {
    let db = sqlite_open_memory();
    sqlite_close(db);
    sqlite_close(db);
  }

  #[test]
  fn begin_commit_and_rollback_issue_real_transaction_control_sql() {
    unsafe {
      let db = sqlite_open_memory();
      let create = c("CREATE TABLE t (n INTEGER)");
      sqlite_execute_direct(db, create.as_ptr());
      let insert_sql = c("INSERT INTO t (n) VALUES (1)");
      let count_sql = c("SELECT COUNT(*) FROM t");

      sqlite_begin(db);
      let insert = sqlite_prepare(db, insert_sql.as_ptr());
      sqlite_execute(insert);
      sqlite_rollback(db);

      let count_stmt = sqlite_prepare(db, count_sql.as_ptr());
      let cursor = sqlite_query(count_stmt);
      assert_eq!(sqlite_step(cursor), 1);
      assert_eq!(
        sqlite_column_int64(cursor, 0),
        0,
        "rollback should have undone the insert"
      );
      assert_eq!(sqlite_step(cursor), 0);

      sqlite_begin(db);
      let insert2 = sqlite_prepare(db, insert_sql.as_ptr());
      sqlite_execute(insert2);
      sqlite_commit(db);

      let count_stmt2 = sqlite_prepare(db, count_sql.as_ptr());
      let cursor2 = sqlite_query(count_stmt2);
      assert_eq!(sqlite_step(cursor2), 1);
      assert_eq!(
        sqlite_column_int64(cursor2, 0),
        1,
        "commit should have kept the insert"
      );
      assert_eq!(sqlite_step(cursor2), 0);

      sqlite_close(db);
    }
  }
}
