//! Plan 121 (CSV) — `Csv.parse`/`.parse_with_headers`/`.write`,
//! wrapping the `csv` crate (BurntSushi's canonical, official Rust CSV
//! reader/writer). Deliberately does NOT reuse plan 118's `JsonValue`
//! tree — a CSV document is a flat table (rows of plain strings,
//! positionally or header-keyed), never nested, so `Array[Array[
//! String]]`/`Array[Hash[String, String]]` say exactly what it is
//! directly in Emerald's own existing collection types.
//!
//! Reuses `Array[String]`'s own real `[len: i64][elem: *const
//! c_char]*n` buffer layout (`Env.keys`'s own established construction
//! pattern, `env.rs`) and `Hash[String,V]`'s own real `[pair-count:
//! i64][(key_ptr, value_ptr), ...]` layout (plan 118's `lower_object`,
//! reused in shape, not code — the value here is a bare `*const
//! c_char`, never a `JsonValue` block).
//!
//! Named `csvs`, not `csv`, in `lib.rs`'s own `#[path]` declaration —
//! this crate's own `mod csv` would shadow the external `csv` crate
//! this module wraps, the identical collision `aead.rs`/`url.rs`/
//! `toml.rs` already hit and disclosed.

use std::ffi::c_void;
use std::os::raw::c_char;

unsafe fn alloc_string_array(strings: &[String]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * strings.len() as i64) as *mut i64;
  *ptr = strings.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, s) in strings.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(s);
  }
  ptr as *mut c_void
}

unsafe fn alloc_ptr_array(ptrs: &[*mut c_void]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * ptrs.len() as i64) as *mut i64;
  *ptr = ptrs.len() as i64;
  let elems = ptr.add(1) as *mut *mut c_void;
  for (i, p) in ptrs.iter().enumerate() {
    *elems.add(i) = *p;
  }
  ptr as *mut c_void
}

unsafe fn alloc_string_hash(pairs: &[(String, String)]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 16 * pairs.len() as i64) as *mut i64;
  *ptr = pairs.len() as i64;
  let slots = ptr.add(1) as *mut *mut c_void;
  for (i, (k, v)) in pairs.iter().enumerate() {
    *slots.add(i * 2) = crate::alloc_and_copy_str(k) as *mut c_void;
    *slots.add(i * 2 + 1) = crate::alloc_and_copy_str(v) as *mut c_void;
  }
  ptr as *mut c_void
}

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

/// `Csv.parse(s: String): Result[Array[Array[String]], String]` —
/// every row, header included, positionally. `flexible(true)`: a
/// short/long row is never rejected for its field count alone (this
/// plan's own Decision log — unlike `.parse_with_headers` below, a
/// positional row has no header to be inconsistent with).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn csv_parse(s: *const c_char) -> *mut c_void {
  let text = match read_str(s) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  let mut rdr = csv::ReaderBuilder::new()
    .has_headers(false)
    .flexible(true)
    .from_reader(text.as_bytes());
  let mut rows: Vec<*mut c_void> = Vec::new();
  for result in rdr.records() {
    match result {
      Ok(record) => {
        let fields: Vec<String> = record.iter().map(|f| f.to_string()).collect();
        rows.push(alloc_string_array(&fields));
      }
      Err(e) => return crate::emerald_rt_result_err_str(&e.to_string()),
    }
  }
  let ptr = alloc_ptr_array(&rows);
  crate::emerald_rt_result_ok(ptr as i64)
}

/// `Csv.parse_with_headers(s: String): Result[Array[Hash[String,
/// String]], String]` — the crate's own real default posture
/// (`has_headers(true)`, `flexible(false)`): a data row whose field
/// count doesn't match the header is a real `Err`, never silently
/// padded or truncated (this plan's own Decision log).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn csv_parse_with_headers(s: *const c_char) -> *mut c_void {
  let text = match read_str(s) {
    Ok(t) => t,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  let mut rdr = csv::ReaderBuilder::new().from_reader(text.as_bytes());
  let headers: Vec<String> = match rdr.headers() {
    Ok(h) => h.iter().map(|f| f.to_string()).collect(),
    Err(e) => return crate::emerald_rt_result_err_str(&e.to_string()),
  };
  let mut records: Vec<*mut c_void> = Vec::new();
  for result in rdr.records() {
    match result {
      Ok(record) => {
        let pairs: Vec<(String, String)> = headers
          .iter()
          .zip(record.iter())
          .map(|(k, v)| (k.clone(), v.to_string()))
          .collect();
        records.push(alloc_string_hash(&pairs));
      }
      Err(e) => return crate::emerald_rt_result_err_str(&e.to_string()),
    }
  }
  let ptr = alloc_ptr_array(&records);
  crate::emerald_rt_result_ok(ptr as i64)
}

/// `Csv.write(rows: Array[Array[String]]): String` — the crate's own
/// `QuoteStyle::Necessary` default decides quoting; never hand-rolled
/// (this plan's own Decision log).
///
/// # Safety
/// `rows` must point to a real `Array[Array[String]]` buffer — `Env.
/// keys`'s own `[len: i64][elem: *const c_char]*n` shape, one level up
/// (each outer element itself points to an inner buffer of that same
/// shape).
pub unsafe fn csv_write(rows: *const c_void) -> *const c_char {
  if rows.is_null() {
    crate::raise_native_error("Csv.write: null pointer");
  }
  let outer = rows as *const i64;
  let row_count = *outer;
  let row_ptrs = outer.add(1) as *const *const i64;
  let mut wtr = csv::WriterBuilder::new().from_writer(Vec::new());
  for i in 0..row_count {
    let row_ptr = *row_ptrs.add(i as usize);
    let field_count = *row_ptr;
    let field_ptrs = row_ptr.add(1) as *const *const c_char;
    let mut fields: Vec<String> = Vec::with_capacity(field_count as usize);
    for j in 0..field_count {
      let field_ptr = *field_ptrs.add(j as usize);
      let field = std::ffi::CStr::from_ptr(field_ptr)
        .to_string_lossy()
        .into_owned();
      fields.push(field);
    }
    if let Err(e) = wtr.write_record(&fields) {
      crate::raise_native_error(&format!("Csv.write: {e}"));
    }
  }
  if let Err(e) = wtr.flush() {
    crate::raise_native_error(&format!("Csv.write: {e}"));
  }
  let bytes = match wtr.into_inner() {
    Ok(b) => b,
    Err(e) => crate::raise_native_error(&format!("Csv.write: {e}")),
  };
  let text = String::from_utf8_lossy(&bytes).into_owned();
  crate::alloc_and_copy_str(&text)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn c(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
  }

  fn parse_ok(input: &str) -> *mut c_void {
    let cstr = c(input);
    let result = unsafe { csv_parse(cstr.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *result }, 0, "expected Ok for input: {input}");
    (unsafe { *(result.add(1)) }) as *mut c_void
  }

  unsafe fn row_field(rows: *const c_void, row: usize, field: usize) -> String {
    let outer = rows as *const i64;
    let row_ptrs = outer.add(1) as *const *const i64;
    let row_ptr = *row_ptrs.add(row);
    let field_ptrs = row_ptr.add(1) as *const *const c_char;
    let field_ptr = *field_ptrs.add(field);
    std::ffi::CStr::from_ptr(field_ptr)
      .to_str()
      .unwrap()
      .to_string()
  }

  #[test]
  fn quoted_field_containing_the_delimiter_is_not_split() {
    let doc = parse_ok("name,role\nAda,\"engineer, lead\"\n");
    assert_eq!(unsafe { row_field(doc, 1, 1) }, "engineer, lead");
  }

  #[test]
  fn quoted_field_containing_an_embedded_newline_is_preserved() {
    let doc = parse_ok("name,notes\nAda,\"wrote \"\"the\nfirst\"\" program\"\n");
    assert_eq!(
      unsafe { row_field(doc, 1, 1) },
      "wrote \"the\nfirst\" program"
    );
  }

  #[test]
  fn short_row_under_plain_parse_is_just_a_shorter_array_not_an_error() {
    let doc = parse_ok("a,b,c\n1,2\n") as *const i64;
    let outer = doc;
    let row_ptrs = unsafe { outer.add(1) as *const *const i64 };
    let second_row = unsafe { *row_ptrs.add(1) };
    assert_eq!(
      unsafe { *second_row },
      2,
      "short row keeps its own real field count"
    );
  }

  #[test]
  fn parse_with_headers_builds_one_hash_per_row_keyed_by_the_header() {
    let cstr = c("name,role\nAda,engineer\nGrace,admiral\n");
    let result = unsafe { csv_parse_with_headers(cstr.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *result }, 0, "expected Ok");
    let records = unsafe { *(result.add(1)) } as *const i64;
    let count = unsafe { *records };
    assert_eq!(count, 2, "two data rows, header consumed separately");
  }

  #[test]
  fn parse_with_headers_rejects_a_row_with_the_wrong_field_count() {
    let cstr = c("a,b,c\n1,2\n");
    let result = unsafe { csv_parse_with_headers(cstr.as_ptr()) } as *const i64;
    assert_eq!(
      unsafe { *result },
      1,
      "expected Err for a short row under headers mode"
    );
  }

  #[test]
  fn write_round_trips_through_parse() {
    let doc = parse_ok("name,role\nAda,\"engineer, lead\"\n");
    let out_ptr = unsafe { csv_write(doc) };
    let out = unsafe { std::ffi::CStr::from_ptr(out_ptr) }
      .to_str()
      .unwrap();
    let reparsed = parse_ok(out);
    assert_eq!(unsafe { row_field(reparsed, 1, 1) }, "engineer, lead");
  }
}
