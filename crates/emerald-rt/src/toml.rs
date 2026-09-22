//! Plan 119 (TOML) — `Toml.parse`/`JsonValue.to_toml`, wrapping the
//! `toml` crate (already an in-tree, already-vetted dependency of
//! `emerald-cli` itself, which uses it to parse every `emerald.toml`
//! manifest since plan 46 — see `Cargo.toml`'s own comment on this
//! dependency for the real, disclosed version-skew note). Reuses plan
//! 118's `JsonValue` tagged-union enum verbatim as the dynamic-value
//! representation via `crate::json`'s own now-`pub(crate)` tag
//! constants and `alloc_enum_block`/`lift_json_value` helpers — no
//! second `TomlValue` enum, and no duplicated byte layout.
//!
//! TOML's own value model (string/integer/float/boolean/datetime/
//! array/table) is a strict subset of `JsonValue`'s six variants once
//! `Datetime` folds into `JsonString` (TOML's own canonical RFC 3339
//! text, via `Datetime`'s own real `Display` impl) — TOML has no null
//! literal at all, so `TAG_NULL` is never produced by `lower_toml_
//! value`, only consumed (as a real, disclosed `Err`) by `.to_toml`.
//!
//! Named `tomls`, not `toml`, in `lib.rs`'s own `#[path]` declaration —
//! this crate's own `mod toml` would shadow the external `toml` crate
//! this module wraps, exactly the collision `url.rs`/`aead.rs` already
//! hit and disclosed.

use crate::json::{
  alloc_enum_block, lift_json_value, TAG_ARRAY, TAG_BOOL, TAG_NUMBER, TAG_OBJECT, TAG_STRING,
};
use std::ffi::c_void;
use std::os::raw::c_char;

unsafe fn lower_toml_value(v: &toml::Value) -> *mut c_void {
  match v {
    toml::Value::String(s) => {
      let ptr = alloc_enum_block(TAG_STRING);
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
      ptr as *mut c_void
    }
    toml::Value::Integer(i) => {
      let ptr = alloc_enum_block(TAG_NUMBER);
      *(ptr.add(1) as *mut f64) = *i as f64;
      ptr as *mut c_void
    }
    toml::Value::Float(f) => {
      let ptr = alloc_enum_block(TAG_NUMBER);
      *(ptr.add(1) as *mut f64) = *f;
      ptr as *mut c_void
    }
    toml::Value::Boolean(b) => {
      let ptr = alloc_enum_block(TAG_BOOL);
      *(ptr.add(1) as *mut u8) = u8::from(*b);
      ptr as *mut c_void
    }
    // Real, disclosed folding, per this plan's own Decision log: a
    // round-tripped TOML datetime becomes an ordinary Emerald String,
    // not a value a `match` arm could tell apart from a plain string
    // that merely looks like a timestamp.
    toml::Value::Datetime(dt) => {
      let ptr = alloc_enum_block(TAG_STRING);
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(&dt.to_string());
      ptr as *mut c_void
    }
    toml::Value::Array(items) => {
      let ptr = alloc_enum_block(TAG_ARRAY);
      let arr_ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
      *arr_ptr = items.len() as i64;
      let elems = arr_ptr.add(1) as *mut *mut c_void;
      for (i, item) in items.iter().enumerate() {
        *elems.add(i) = lower_toml_value(item);
      }
      *(ptr.add(1) as *mut *mut c_void) = arr_ptr as *mut c_void;
      ptr as *mut c_void
    }
    toml::Value::Table(map) => {
      let ptr = alloc_enum_block(TAG_OBJECT);
      let obj_ptr = crate::emerald_alloc(8 + 16 * map.len() as i64) as *mut i64;
      *obj_ptr = map.len() as i64;
      let pairs = obj_ptr.add(1) as *mut *mut c_void;
      for (i, (k, v)) in map.iter().enumerate() {
        *pairs.add(i * 2) = crate::alloc_and_copy_str(k) as *mut c_void;
        *pairs.add(i * 2 + 1) = lower_toml_value(v);
      }
      *(ptr.add(1) as *mut *mut c_void) = obj_ptr as *mut c_void;
      ptr as *mut c_void
    }
  }
}

/// `Toml.parse(s: String): Result[JsonValue, String]` — parses via
/// `toml::Table`'s own real `FromStr` impl (the crate's own documented
/// entry point for a whole document, which is always a table at the
/// top level) and lowers the result the same way `Json.parse` does.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn toml_parse(s: *const c_char) -> *mut c_void {
  if s.is_null() {
    return crate::emerald_rt_result_err_str("Toml.parse: null string pointer");
  }
  let text = match std::ffi::CStr::from_ptr(s).to_str() {
    Ok(t) => t,
    Err(_) => return crate::emerald_rt_result_err_str("Toml.parse: input is not valid UTF-8"),
  };
  match text.parse::<toml::Table>() {
    Ok(table) => {
      let ptr = lower_toml_value(&toml::Value::Table(table));
      crate::emerald_rt_result_ok(ptr as i64)
    }
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

/// Lifts a `serde_json::Value` (itself lifted from a real `JsonValue`
/// block by `crate::json::lift_json_value`) into a `toml::Value` —
/// `Err` on `Null`, TOML's one real representational gap versus JSON
/// (see this plan's own Decision log).
fn json_to_toml_value(v: &serde_json::Value) -> Result<toml::Value, String> {
  match v {
    serde_json::Value::Null => Err(
      "JsonValue.to_toml: TOML has no null literal; a JsonNull value cannot be serialized to TOML"
        .to_string(),
    ),
    serde_json::Value::Bool(b) => Ok(toml::Value::Boolean(*b)),
    serde_json::Value::Number(n) => Ok(toml::Value::Float(n.as_f64().unwrap_or(f64::NAN))),
    serde_json::Value::String(s) => Ok(toml::Value::String(s.clone())),
    serde_json::Value::Array(items) => {
      let mut out = Vec::with_capacity(items.len());
      for item in items {
        out.push(json_to_toml_value(item)?);
      }
      Ok(toml::Value::Array(out))
    }
    serde_json::Value::Object(map) => {
      let mut out = toml::map::Map::new();
      for (k, v) in map {
        out.insert(k.clone(), json_to_toml_value(v)?);
      }
      Ok(toml::Value::Table(out))
    }
  }
}

/// `JsonValue.to_toml(self): Result[String, String]` — `Err`, not a
/// total `String`, because not every `JsonValue` a JSON document can
/// produce is representable in TOML (see `json_to_toml_value`'s own
/// doc comment).
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn json_to_toml(obj: *const c_void) -> *mut c_void {
  if obj.is_null() {
    return crate::emerald_rt_result_err_str("JsonValue.to_toml: null pointer");
  }
  let json_val = lift_json_value(obj);
  let toml_val = match json_to_toml_value(&json_val) {
    Ok(v) => v,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match toml::to_string(&toml_val) {
    Ok(s) => crate::emerald_rt_result_ok(crate::alloc_and_copy_str(&s) as i64),
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::json::json_object_get;

  fn c(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
  }

  fn parse_ok(input: &str) -> *mut c_void {
    let cstr = c(input);
    let result = unsafe { toml_parse(cstr.as_ptr()) } as *const i64;
    let discriminant = unsafe { *result };
    assert_eq!(discriminant, 0, "expected Ok for input: {input}");
    (unsafe { *(result.add(1)) }) as *mut c_void
  }

  #[test]
  fn parse_string_lowers_a_real_string_value() {
    let doc = parse_ok("s = 'bar'") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_OBJECT);
    let key = c("s");
    let v = unsafe { json_object_get(doc as *const c_void, key.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *v }, 0, "expected Some");
    let payload = unsafe { *(v.add(1)) } as *const i64;
    assert_eq!(unsafe { *payload }, TAG_STRING);
    let s_ptr = unsafe { *(payload.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "bar");
  }

  #[test]
  fn parse_integer_and_float_both_widen_to_a_real_f64() {
    let doc = parse_ok("i = 36\nf = 1.5") as *const i64;
    let i_key = c("i");
    let i_v = unsafe { json_object_get(doc as *const c_void, i_key.as_ptr()) } as *const i64;
    let i_payload = unsafe { *(i_v.add(1)) } as *const i64;
    assert_eq!(unsafe { *i_payload }, TAG_NUMBER);
    assert_eq!(unsafe { *(i_payload.add(1) as *const f64) }, 36.0);

    let f_key = c("f");
    let f_v = unsafe { json_object_get(doc as *const c_void, f_key.as_ptr()) } as *const i64;
    let f_payload = unsafe { *(f_v.add(1)) } as *const i64;
    assert_eq!(unsafe { *f_payload }, TAG_NUMBER);
    assert_eq!(unsafe { *(f_payload.add(1) as *const f64) }, 1.5);
  }

  #[test]
  fn parse_datetime_folds_into_a_real_string() {
    let doc = parse_ok("d = 1979-05-27T07:32:00Z") as *const i64;
    let key = c("d");
    let v = unsafe { json_object_get(doc as *const c_void, key.as_ptr()) } as *const i64;
    let payload = unsafe { *(v.add(1)) } as *const i64;
    assert_eq!(unsafe { *payload }, TAG_STRING);
    let s_ptr = unsafe { *(payload.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "1979-05-27T07:32:00Z");
  }

  #[test]
  fn parse_nested_table_round_trips_through_json_object_get() {
    let doc = parse_ok("[package]\nname = \"demo\"\n") as *const i64;
    let pkg_key = c("package");
    let pkg = unsafe { json_object_get(doc as *const c_void, pkg_key.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *pkg }, 0, "expected Some for `package`");
    let pkg_payload = unsafe { *(pkg.add(1)) } as *const i64;
    assert_eq!(unsafe { *pkg_payload }, TAG_OBJECT);
    let name_key = c("name");
    let name =
      unsafe { json_object_get(pkg_payload as *const c_void, name_key.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *name }, 0, "expected Some for `name`");
    let name_payload = unsafe { *(name.add(1)) } as *const i64;
    assert_eq!(unsafe { *name_payload }, TAG_STRING);
    let s_ptr = unsafe { *(name_payload.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "demo");
  }

  #[test]
  fn parse_invalid_toml_returns_a_real_non_empty_err_message() {
    let cstr = c("not = valid = toml");
    let result = unsafe { toml_parse(cstr.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *result }, 1, "expected Err");
    let msg_ptr = unsafe { *(result.add(1)) } as *const c_char;
    let msg = unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
      .to_str()
      .unwrap();
    assert!(!msg.is_empty());
  }

  #[test]
  fn to_toml_round_trips_a_simple_object() {
    let json = c("{\"name\":\"demo\",\"version\":\"0.1.0\"}");
    let parsed = unsafe { crate::json::json_parse(json.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *parsed }, 0, "expected Ok");
    let obj = unsafe { *(parsed.add(1)) } as *const c_void;
    let result = unsafe { json_to_toml(obj) } as *const i64;
    assert_eq!(unsafe { *result }, 0, "expected Ok");
    let s_ptr = unsafe { *(result.add(1)) } as *const c_char;
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert!(s.contains("name = \"demo\""));
    assert!(s.contains("version = \"0.1.0\""));
  }

  #[test]
  fn to_toml_rejects_a_null_value_with_a_real_err() {
    let json = c("{\"x\":null}");
    let parsed = unsafe { crate::json::json_parse(json.as_ptr()) } as *const i64;
    let obj = unsafe { *(parsed.add(1)) } as *const c_void;
    let result = unsafe { json_to_toml(obj) } as *const i64;
    assert_eq!(unsafe { *result }, 1, "expected Err for a null field");
  }
}
