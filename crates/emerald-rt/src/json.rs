// Plan 118 (JSON) — `serde_json` wrapped behind a compiler-synthesized,
// non-generic `JsonValue` tagged-union enum (`emerald-sema`/`emerald-
// codegen`'s own `check_program`/`compile_to_object_impl` register it
// directly, mirroring `Option[T]`'s own synthesized-`EnumDef`
// precedent — see either crate's Decision log). This module hand-
// lowers a real `serde_json::Value` tree into Emerald's own `EnumLayout`
// byte format (`[tag: i64][payload: 8 bytes]`, since every `JsonValue`
// variant carries at most one field) — the exact byte-for-byte shape
// `emerald-codegen`'s own enum codegen already produces for any other
// enum, so a `JsonValue` this module hands back is indistinguishable,
// from Emerald's own `match`, from one a real Emerald program
// constructed directly (`JsonBool(true)`, etc.).
//
// Variant tags (declaration order, matching `emerald-sema`/`emerald-
// codegen`'s own `EnumDef.variants` list byte-for-byte — see both
// crates' own `json_value_enum_def`):
//   0 JsonNull       (no payload)
//   1 JsonBool       (payload: 1-byte bool at offset 8, rest zeroed)
//   2 JsonNumber     (payload: 8-byte f64 at offset 8)
//   3 JsonString     (payload: pointer to a NUL-terminated buffer)
//   4 JsonArray      (payload: pointer to an Array[JsonValue] buffer)
//   5 JsonObject     (payload: pointer to a Hash[String,JsonValue] buffer)
//
// Every JSON number becomes `Float64` — Emerald has no unified numeric
// type (plan 59's own finding), so `36` widens to `36.0`, a disclosed,
// deliberate lossy round-trip, not an oversight.

use std::ffi::c_void;
use std::os::raw::c_char;

// `pub(crate)`, not private: plan 119's `toml.rs` reuses these tags and
// `alloc_enum_block`/`lift_json_value` directly rather than duplicating
// the `JsonValue` byte layout a second time.
pub(crate) const TAG_NULL: i64 = 0;
pub(crate) const TAG_BOOL: i64 = 1;
pub(crate) const TAG_NUMBER: i64 = 2;
pub(crate) const TAG_STRING: i64 = 3;
pub(crate) const TAG_ARRAY: i64 = 4;
pub(crate) const TAG_OBJECT: i64 = 5;

// Plan 195 (Typed Domain Errors): `JsonError`'s own variant tags,
// declaration order, matching `emerald-sema`/`emerald-codegen`'s own
// `json_error_enum_def` byte-for-byte — the identical convention
// `TAG_NULL`..`TAG_OBJECT` above already establish for `JsonValue`.
//   0 Syntax(String)        — serde_json::error::Category::Syntax
//   1 UnexpectedEnd          — serde_json::error::Category::Eof
//   2 Other(String)          — Category::Io | Category::Data, or any
//                              future category this match doesn't
//                              yet name individually
const JSON_ERROR_TAG_SYNTAX: i32 = 0;
const JSON_ERROR_TAG_UNEXPECTED_END: i32 = 1;
const JSON_ERROR_TAG_OTHER: i32 = 2;

pub(crate) unsafe fn alloc_enum_block(tag: i64) -> *mut i64 {
  let ptr = crate::emerald_alloc(16) as *mut i64;
  *ptr = tag;
  ptr
}

/// Lowers one `serde_json::Value` (and, recursively, everything it
/// contains) into a real, `emerald_alloc`-backed `JsonValue` block —
/// the same allocator `emerald-codegen`'s own generated code already
/// uses for every other value this program allocates, not a second
/// one.
unsafe fn lower_json_value(v: &serde_json::Value) -> *mut c_void {
  match v {
    serde_json::Value::Null => alloc_enum_block(TAG_NULL) as *mut c_void,
    serde_json::Value::Bool(b) => {
      let ptr = alloc_enum_block(TAG_BOOL);
      *(ptr.add(1) as *mut u8) = u8::from(*b);
      ptr as *mut c_void
    }
    serde_json::Value::Number(n) => {
      let ptr = alloc_enum_block(TAG_NUMBER);
      *(ptr.add(1) as *mut f64) = n.as_f64().unwrap_or(f64::NAN);
      ptr as *mut c_void
    }
    serde_json::Value::String(s) => {
      let ptr = alloc_enum_block(TAG_STRING);
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
      ptr as *mut c_void
    }
    serde_json::Value::Array(items) => {
      let ptr = alloc_enum_block(TAG_ARRAY);
      *(ptr.add(1) as *mut *mut c_void) = lower_array(items);
      ptr as *mut c_void
    }
    serde_json::Value::Object(map) => {
      let ptr = alloc_enum_block(TAG_OBJECT);
      *(ptr.add(1) as *mut *mut c_void) = lower_object(map);
      ptr as *mut c_void
    }
  }
}

/// `Array[JsonValue]`'s own buffer: `[length: i64][elements: *mut
/// c_void, one per item]` — plan 42's array-length-header layout,
/// verified against `build_array_lit`'s own doc comment, byte-for-byte.
unsafe fn lower_array(items: &[serde_json::Value]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *mut c_void;
  for (i, item) in items.iter().enumerate() {
    *elems.add(i) = lower_json_value(item);
  }
  ptr as *mut c_void
}

/// `Hash[String, JsonValue]`'s own buffer: `[pair-count: i64][(key_ptr:
/// 8, value_ptr: 8), ...]` — verified against `build_hash_lit`'s own
/// doc comment/body, byte-for-byte (the same layout `leaf-json-get-
/// and-to-s-intrinsics`'s own `.get` reads directly, below).
unsafe fn lower_object(map: &serde_json::Map<String, serde_json::Value>) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 16 * map.len() as i64) as *mut i64;
  *ptr = map.len() as i64;
  let pairs = ptr.add(1) as *mut *mut c_void;
  for (i, (k, v)) in map.iter().enumerate() {
    *pairs.add(i * 2) = crate::alloc_and_copy_str(k) as *mut c_void;
    *pairs.add(i * 2 + 1) = lower_json_value(v);
  }
  ptr as *mut c_void
}

/// Re-lifts a real `JsonValue` block (this module's own, or one a
/// compiled Emerald program built directly via `JsonBool(true)` etc. —
/// indistinguishable, by design) back into an owned `serde_json::Value`,
/// for `.to_s`'s own `serde_json::to_string` call.
pub(crate) unsafe fn lift_json_value(ptr: *const c_void) -> serde_json::Value {
  let block = ptr as *const i64;
  let tag = *block;
  match tag {
    TAG_NULL => serde_json::Value::Null,
    TAG_BOOL => serde_json::Value::Bool(*(block.add(1) as *const u8) != 0),
    TAG_NUMBER => {
      let f = *(block.add(1) as *const f64);
      serde_json::Number::from_f64(f).map_or(serde_json::Value::Null, serde_json::Value::Number)
    }
    TAG_STRING => {
      let s_ptr = *(block.add(1) as *const *const c_char);
      let s = std::ffi::CStr::from_ptr(s_ptr)
        .to_string_lossy()
        .into_owned();
      serde_json::Value::String(s)
    }
    TAG_ARRAY => {
      let arr_ptr = *(block.add(1) as *const *const i64);
      let len = *arr_ptr;
      let elems = arr_ptr.add(1) as *const *const c_void;
      let mut out = Vec::with_capacity(len as usize);
      for i in 0..len {
        out.push(lift_json_value(*elems.add(i as usize)));
      }
      serde_json::Value::Array(out)
    }
    TAG_OBJECT => {
      let obj_ptr = *(block.add(1) as *const *const i64);
      let count = *obj_ptr;
      let pairs = obj_ptr.add(1) as *const *const c_void;
      let mut out = serde_json::Map::with_capacity(count as usize);
      for i in 0..count {
        let key_ptr = *pairs.add((i * 2) as usize) as *const c_char;
        let key = std::ffi::CStr::from_ptr(key_ptr)
          .to_string_lossy()
          .into_owned();
        let value_ptr = *pairs.add((i * 2 + 1) as usize);
        out.insert(key, lift_json_value(value_ptr));
      }
      serde_json::Value::Object(out)
    }
    _ => serde_json::Value::Null,
  }
}

/// `Json.parse(s: String): Result[JsonValue, JsonError]` — plan 195's
/// retrofit of this signature's original, plan-118-shipped `Result[
/// JsonValue, String]` shape (a disclosed, real breaking change to
/// plan 118's public surface — see plan 195's own history file). `Ok`'s
/// own payload is `lower_json_value`'s returned pointer, widened to
/// `i64` exactly the way `emerald_rt_result_ok` already expects any
/// Ptr-kind Ok payload (a pointer IS an `i64` bit pattern in this
/// backend's own representation, verified against `ValKind::Ptr`'s own
/// single-word storage). `Err`'s payload is a real `JsonError` enum
/// value — classified via `serde_json::Error::classify()`'s own real,
/// already-vendored `Category` (`Category::Eof` → `UnexpectedEnd`,
/// `Category::Syntax` → `Syntax(msg)`, `Category::Io`/`Category::Data`
/// → `Other(msg)`) — carrying `serde_json::Error`'s own real `Display`
/// text on every variant, never an empty string or a crash.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn json_parse(s: *const c_char) -> *mut c_void {
  if s.is_null() {
    return crate::emerald_rt_result_err_tagged_str(
      JSON_ERROR_TAG_OTHER,
      "Json.parse: null string pointer",
    );
  }
  let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
  match serde_json::from_slice::<serde_json::Value>(bytes) {
    Ok(v) => {
      let ptr = lower_json_value(&v);
      crate::emerald_rt_result_ok(ptr as i64)
    }
    Err(e) => {
      let tag = match e.classify() {
        serde_json::error::Category::Eof => JSON_ERROR_TAG_UNEXPECTED_END,
        serde_json::error::Category::Syntax => JSON_ERROR_TAG_SYNTAX,
        serde_json::error::Category::Io | serde_json::error::Category::Data => JSON_ERROR_TAG_OTHER,
      };
      crate::emerald_rt_result_err_tagged_str(tag, &e.to_string())
    }
  }
}

/// `JsonValue.get(self, key: String): JsonValue?` — reads `Hash[String,
/// JsonValue]`'s own already-stable buffer format directly (see
/// `lower_object`'s own doc comment for the exact layout), never
/// routing through `build_hash_lookup`/`build_index` at all — those
/// are `Int64`-key-only today (`leaf-fix-hash-generic-indexing`'s own
/// separate, disclosed fix). Returns a real `Option[JsonValue]` block
/// (`[tag: i64][payload: 8]`, `Some` = 0, `None` = 1 — plan 73's own
/// fixed encoding, identical regardless of `T`) — `self` must actually
/// be a `JsonObject` block; any other variant returns `None` rather
/// than raising, since "the wrong shape" is exactly the kind of
/// expected, routinely-checked outcome plan 92's own Result-vs-
/// exception rule assigns away from `NativeError`.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block this module (or a
/// compiled Emerald program's own `JsonObject(...)` construction)
/// produced. `key`, if non-null, must point to a valid, NUL-terminated
/// C string.
pub unsafe fn json_object_get(obj: *const c_void, key: *const c_char) -> *mut c_void {
  const OPTION_SOME: i64 = 0;
  const OPTION_NONE: i64 = 1;
  let none = || {
    let ptr = crate::emerald_alloc(16) as *mut i64;
    *ptr = OPTION_NONE;
    ptr as *mut c_void
  };
  if obj.is_null() || key.is_null() {
    return none();
  }
  let block = obj as *const i64;
  if *block != TAG_OBJECT {
    return none();
  }
  let requested = std::ffi::CStr::from_ptr(key);
  let obj_ptr = *(block.add(1) as *const *const i64);
  let count = *obj_ptr;
  let pairs = obj_ptr.add(1) as *const *const c_void;
  for i in 0..count {
    let key_ptr = *pairs.add((i * 2) as usize) as *const c_char;
    if std::ffi::CStr::from_ptr(key_ptr) == requested {
      let value_ptr = *pairs.add((i * 2 + 1) as usize);
      let some_ptr = crate::emerald_alloc(16) as *mut i64;
      *some_ptr = OPTION_SOME;
      *(some_ptr.add(1) as *mut *mut c_void) = value_ptr as *mut c_void;
      return some_ptr as *mut c_void;
    }
  }
  none()
}

/// `JsonValue.to_s(self): String` — re-lifts `self`'s own bytes back
/// into a `serde_json::Value` and serializes it with `serde_json::
/// to_string`, so this module's own lowering and Emerald source
/// constructing a `JsonValue` directly print identically.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn json_to_string(obj: *const c_void) -> *const c_char {
  if obj.is_null() {
    return crate::alloc_and_copy_str("null");
  }
  let v = lift_json_value(obj);
  let s = serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string());
  crate::alloc_and_copy_str(&s)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse_ok(json: &str) -> *mut c_void {
    let cstr = std::ffi::CString::new(json).unwrap();
    let result = unsafe { json_parse(cstr.as_ptr()) } as *const i64;
    let discriminant = unsafe { *result };
    assert_eq!(discriminant, 0, "expected Ok for input: {json}");
    (unsafe { *(result.add(1)) }) as *mut c_void
  }

  #[test]
  fn parse_null_lowers_to_the_null_tag() {
    let doc = parse_ok("null") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_NULL);
  }

  #[test]
  fn parse_bool_lowers_the_real_boolean_value() {
    let doc = parse_ok("true") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_BOOL);
    assert_eq!(unsafe { *(doc.add(1) as *const u8) }, 1);
  }

  #[test]
  fn parse_number_widens_to_a_real_f64() {
    let doc = parse_ok("36") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_NUMBER);
    assert_eq!(unsafe { *(doc.add(1) as *const f64) }, 36.0);
  }

  #[test]
  fn parse_string_round_trips_the_real_bytes() {
    let doc = parse_ok("\"Ada\"") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_STRING);
    let s_ptr = unsafe { *(doc.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "Ada");
  }

  #[test]
  fn parse_array_lowers_a_real_length_and_each_real_element() {
    let doc = parse_ok("[\"math\", \"cs\"]") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_ARRAY);
    let arr_ptr = unsafe { *(doc.add(1) as *const *const i64) };
    let len = unsafe { *arr_ptr };
    assert_eq!(len, 2);
    let elems = unsafe { arr_ptr.add(1) as *const *const c_void };
    let first = unsafe { *elems } as *const i64;
    assert_eq!(unsafe { *first }, TAG_STRING);
  }

  #[test]
  fn parse_nested_three_level_document_lowers_correctly() {
    let doc =
      parse_ok("{\"name\": \"Ada\", \"age\": 36, \"active\": true, \"tags\": [\"math\", \"cs\"]}")
        as *const i64;
    assert_eq!(unsafe { *doc }, TAG_OBJECT);
    let name = unsafe { json_object_get(doc as *const c_void, c_key("name")) } as *const i64;
    assert_eq!(unsafe { *name }, 0, "expected Some for `name`");
    let payload = unsafe { *(name.add(1)) } as *const i64;
    assert_eq!(unsafe { *payload }, TAG_STRING);
    let missing = unsafe { json_object_get(doc as *const c_void, c_key("nickname")) } as *const i64;
    assert_eq!(unsafe { *missing }, 1, "expected None for a missing key");
  }

  #[test]
  fn to_string_round_trips_a_parsed_document() {
    let doc = parse_ok("{\"a\":1.0,\"b\":[true,null]}");
    let out_ptr = unsafe { json_to_string(doc as *const c_void) };
    let out = unsafe { std::ffi::CStr::from_ptr(out_ptr) }
      .to_str()
      .unwrap();
    assert_eq!(out, "{\"a\":1.0,\"b\":[true,null]}");
  }

  /// Reads a `Result[JsonValue, JsonError]`'s `Err` arm back into
  /// `(tag, message)` — the `JsonError` enum block's own `[tag: i64]
  /// [msg: *const c_char]` layout, `emerald_rt_result_err_tagged`'s
  /// own doc comment.
  fn err_tag_and_message(result: *const i64) -> (i64, String) {
    assert_eq!(unsafe { *result }, 1, "expected Err");
    let err_block = unsafe { *(result.add(1)) } as *const i64;
    let tag = unsafe { *err_block };
    let msg_ptr = unsafe { *(err_block.add(1) as *const *const c_char) };
    let msg = unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
      .to_str()
      .unwrap()
      .to_string();
    (tag, msg)
  }

  #[test]
  fn parse_invalid_json_returns_a_real_non_empty_err_message() {
    let cstr = std::ffi::CString::new("{not json").unwrap();
    let result = unsafe { json_parse(cstr.as_ptr()) } as *const i64;
    let (_tag, msg) = err_tag_and_message(result);
    assert!(!msg.is_empty());
  }

  #[test]
  fn parse_syntax_error_is_tagged_syntax() {
    // A stray comma is a real syntax error, not a truncation — serde_
    // json's own `Category::Syntax`.
    let cstr = std::ffi::CString::new("{\"a\":1,}").unwrap();
    let result = unsafe { json_parse(cstr.as_ptr()) } as *const i64;
    let (tag, _msg) = err_tag_and_message(result);
    assert_eq!(tag, JSON_ERROR_TAG_SYNTAX as i64);
  }

  #[test]
  fn parse_truncated_input_is_tagged_unexpected_end() {
    // Valid so far, but cut off mid-object — serde_json's own
    // `Category::Eof`.
    let cstr = std::ffi::CString::new("{\"a\":").unwrap();
    let result = unsafe { json_parse(cstr.as_ptr()) } as *const i64;
    let (tag, _msg) = err_tag_and_message(result);
    assert_eq!(tag, JSON_ERROR_TAG_UNEXPECTED_END as i64);
  }

  fn c_key(s: &str) -> *const c_char {
    // Leaked deliberately — test-only, lives for the process lifetime,
    // matching every other test helper's own throwaway allocation
    // style in this crate (`test_stubs::emerald_alloc`'s own doc
    // comment states the identical convention).
    Box::leak(std::ffi::CString::new(s).unwrap().into_boxed_c_str()).as_ptr()
  }
}
