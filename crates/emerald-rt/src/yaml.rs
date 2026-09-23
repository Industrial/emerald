//! Plan 120 (YAML) — `Yaml.parse`/`JsonValue.to_yaml`, wrapping
//! `saphyr` (see this plan's own Decision log for why `saphyr`, not
//! the archived `serde_yaml`, is the crate this module wraps). Reuses
//! plan 118's `JsonValue` tagged-union enum verbatim as the dynamic-
//! value representation via `crate::json`'s own `pub(crate)` tag
//! constants and `alloc_enum_block`/`lift_json_value` helpers — the
//! identical reuse `crate::tomls` (plan 119) already establishes; no
//! second `YamlValue` enum, and no duplicated byte layout.
//!
//! `saphyr::Yaml`'s own real value model (`Value(Scalar)`/`Sequence`/
//! `Mapping`/`Tagged`/`Alias`/`Representation`/`BadValue` — verified
//! directly against the vendored `saphyr` 0.1.0 source, not assumed)
//! is wider than `JsonValue`'s six variants: `Tagged` discards its own
//! tag and lowers the node underneath (this plan's own Decision log);
//! `Alias` is, in practice, never produced by `Yaml::load_from_str`
//! itself — verified directly against `YamlLoader::on_event`'s own
//! `Event::Alias` arm, which clones the anchor's already-resolved node
//! into the tree in place, so an aliased subtree reaches this module's
//! own lowering function pre-duplicated, not as a literal `Alias`
//! variant. The `Alias`/`BadValue`/`Representation` arms below are
//! kept only as a defensive, disclosed fallback (folding to
//! `JsonNull()`) for whichever of those three genuinely-unreachable-
//! under-`load_from_str` shapes a future `saphyr` version might one
//! day route differently — never reached by this module's own tests.
//!
//! A real, disclosed addition beyond this plan's own Decision log,
//! found only by implementing the mapping-lowering walk: YAML mapping
//! keys are not restricted to strings the way JSON object keys and
//! TOML table keys already are (`k: v` is the common case, but `1: v`/
//! `true: v`/a complex key are all real, spec-legal YAML) — `Hash[
//! String, JsonValue]` needs a real `String` for every key regardless.
//! `yaml_key_to_string` below folds a non-string scalar key to its own
//! plain-text form (`36`, `true`, `null`) the same way `toml.rs`'s own
//! `Datetime`-folds-into-`JsonString` decision already accepts a
//! similar lossy simplification; a complex (sequence/mapping) key —
//! vanishingly rare in the hand-written config-file YAML this plan
//! targets — falls back to `Yaml`'s own `Debug` text rather than
//! crashing or silently dropping the pair.

use crate::json::{
  alloc_enum_block, lift_json_value, TAG_ARRAY, TAG_BOOL, TAG_NULL, TAG_NUMBER, TAG_OBJECT,
  TAG_STRING,
};
use ordered_float::OrderedFloat;
use saphyr::{LoadableYamlNode, Mapping, Scalar, Yaml, YamlEmitter};
use std::ffi::c_void;
use std::os::raw::c_char;

// Plan 195 (Typed Domain Errors), applied fresh here per this plan's
// own instruction (it lands after plan 195, so it does not ship
// `Toml.parse`'s own pre-195 `Result[JsonValue, String]` shape): `Yaml
// Error`'s own variant tags, declaration order, matching `emerald-
// sema`/`emerald-codegen`'s own `yaml_error_enum_def` byte-for-byte.
//   0 Syntax(String)   — a real `saphyr::ScanError` from the parser
//   1 EmptyDocument     — a real, disclosed v1 scope cut (see this
//                          plan's own Decision log): `Yaml.parse`
//                          narrows `Yaml::load_from_str`'s own
//                          `Vec<Yaml>` to its first document, and
//                          `Err`s here rather than silently returning
//                          `JsonNull()` when that `Vec` is empty
//   2 Other(String)     — any future classification this match doesn't
//                          yet name individually
const YAML_ERROR_TAG_SYNTAX: i32 = 0;
const YAML_ERROR_TAG_EMPTY_DOCUMENT: i32 = 1;
const YAML_ERROR_TAG_OTHER: i32 = 2;

/// Folds a `saphyr::Yaml` mapping key into a real `String` — see this
/// module's own doc comment for why this is needed at all (YAML
/// mapping keys are not string-only the way JSON/TOML keys already
/// are).
fn yaml_key_to_string(k: &Yaml) -> String {
  match k {
    Yaml::Value(Scalar::String(s)) => s.to_string(),
    Yaml::Value(Scalar::Integer(i)) => i.to_string(),
    Yaml::Value(Scalar::FloatingPoint(f)) => f.into_inner().to_string(),
    Yaml::Value(Scalar::Boolean(b)) => b.to_string(),
    Yaml::Value(Scalar::Null) => "null".to_string(),
    Yaml::Tagged(_, inner) => yaml_key_to_string(inner),
    other => format!("{other:?}"),
  }
}

/// Lowers one `saphyr::Yaml` node (and, recursively, everything it
/// contains) into a real, `emerald_alloc`-backed `JsonValue` block —
/// the identical byte-for-byte shape `crate::json::lower_json_value`/
/// `crate::tomls`'s own `lower_toml_value` already produce, so a
/// `JsonValue` this module hands back is indistinguishable, from
/// Emerald's own `match`, from one either of those already-shipped
/// sibling plans produced. Every YAML integer/float, like every JSON
/// number, widens to `Float64` — the same disclosed, deliberate
/// lossy round-trip `json.rs`'s own module doc already states.
unsafe fn lower_yaml_value(v: &Yaml) -> *mut c_void {
  match v {
    Yaml::Value(Scalar::Null) => alloc_enum_block(TAG_NULL) as *mut c_void,
    Yaml::Value(Scalar::Boolean(b)) => {
      let ptr = alloc_enum_block(TAG_BOOL);
      *(ptr.add(1) as *mut u8) = u8::from(*b);
      ptr as *mut c_void
    }
    Yaml::Value(Scalar::Integer(i)) => {
      let ptr = alloc_enum_block(TAG_NUMBER);
      *(ptr.add(1) as *mut f64) = *i as f64;
      ptr as *mut c_void
    }
    Yaml::Value(Scalar::FloatingPoint(f)) => {
      let ptr = alloc_enum_block(TAG_NUMBER);
      *(ptr.add(1) as *mut f64) = f.into_inner();
      ptr as *mut c_void
    }
    Yaml::Value(Scalar::String(s)) => {
      let ptr = alloc_enum_block(TAG_STRING);
      *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(s);
      ptr as *mut c_void
    }
    Yaml::Sequence(items) => {
      let ptr = alloc_enum_block(TAG_ARRAY);
      let arr_ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
      *arr_ptr = items.len() as i64;
      let elems = arr_ptr.add(1) as *mut *mut c_void;
      for (i, item) in items.iter().enumerate() {
        *elems.add(i) = lower_yaml_value(item);
      }
      *(ptr.add(1) as *mut *mut c_void) = arr_ptr as *mut c_void;
      ptr as *mut c_void
    }
    Yaml::Mapping(map) => {
      let ptr = alloc_enum_block(TAG_OBJECT);
      let obj_ptr = crate::emerald_alloc(8 + 16 * map.len() as i64) as *mut i64;
      *obj_ptr = map.len() as i64;
      let pairs = obj_ptr.add(1) as *mut *mut c_void;
      for (i, (k, v)) in map.iter().enumerate() {
        *pairs.add(i * 2) = crate::alloc_and_copy_str(&yaml_key_to_string(k)) as *mut c_void;
        *pairs.add(i * 2 + 1) = lower_yaml_value(v);
      }
      *(ptr.add(1) as *mut *mut c_void) = obj_ptr as *mut c_void;
      ptr as *mut c_void
    }
    // Plan 120's Decision log: a `!Tag`'s own tag name is discarded;
    // the node underneath lowers exactly as it would untagged.
    Yaml::Tagged(_, inner) => lower_yaml_value(inner),
    // Defensive fallback only — see this module's own doc comment for
    // why `Alias`/`Representation`/`BadValue` are not, in practice,
    // ever produced by `Yaml::load_from_str`.
    Yaml::Alias(_) | Yaml::Representation(..) | Yaml::BadValue => {
      alloc_enum_block(TAG_NULL) as *mut c_void
    }
  }
}

/// `Yaml.parse(s: String): Result[JsonValue, YamlError]` — parses via
/// `saphyr::Yaml::load_from_str`, narrows the real `Vec<Yaml>` multi-
/// document result to its first document (see this plan's own
/// Decision log for the disclosed v1 scope cut), and lowers it the
/// same way `Json.parse`/`Toml.parse` already do.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn yaml_parse(s: *const c_char) -> *mut c_void {
  if s.is_null() {
    return crate::emerald_rt_result_err_tagged_str(
      YAML_ERROR_TAG_OTHER,
      "Yaml.parse: null string pointer",
    );
  }
  let text = match std::ffi::CStr::from_ptr(s).to_str() {
    Ok(t) => t,
    Err(_) => {
      return crate::emerald_rt_result_err_tagged_str(
        YAML_ERROR_TAG_OTHER,
        "Yaml.parse: input is not valid UTF-8",
      )
    }
  };
  let docs = match Yaml::load_from_str(text) {
    Ok(docs) => docs,
    Err(e) => {
      return crate::emerald_rt_result_err_tagged_str(YAML_ERROR_TAG_SYNTAX, &e.to_string())
    }
  };
  match docs.first() {
    Some(doc) => {
      let ptr = lower_yaml_value(doc);
      crate::emerald_rt_result_ok(ptr as i64)
    }
    None => crate::emerald_rt_result_err_tagged_str(
      YAML_ERROR_TAG_EMPTY_DOCUMENT,
      "empty YAML document stream",
    ),
  }
}

/// Lifts a `serde_json::Value` (itself lifted from a real `JsonValue`
/// block by `crate::json::lift_json_value`) into an owned `saphyr::
/// Yaml<'static>` — a total function (see this plan's own Decision
/// log): every `JsonValue` variant, including `JsonNull()`, has a
/// direct YAML equivalent, unlike `crate::tomls::json_to_toml_value`'s
/// `Err`-on-null.
fn json_to_yaml_value(v: &serde_json::Value) -> Yaml<'static> {
  match v {
    serde_json::Value::Null => Yaml::Value(Scalar::Null),
    serde_json::Value::Bool(b) => Yaml::Value(Scalar::Boolean(*b)),
    serde_json::Value::Number(n) => Yaml::Value(Scalar::FloatingPoint(OrderedFloat(
      n.as_f64().unwrap_or(f64::NAN),
    ))),
    serde_json::Value::String(s) => Yaml::Value(Scalar::String(s.clone().into())),
    serde_json::Value::Array(items) => {
      Yaml::Sequence(items.iter().map(json_to_yaml_value).collect())
    }
    serde_json::Value::Object(map) => {
      let mut out = Mapping::new();
      for (k, v) in map {
        out.insert(
          Yaml::Value(Scalar::String(k.clone().into())),
          json_to_yaml_value(v),
        );
      }
      Yaml::Mapping(out)
    }
  }
}

/// `JsonValue.to_yaml(self): String` — `saphyr::YamlEmitter`'s own
/// `.dump`, total per this plan's own Decision log (never `Result`,
/// unlike `.to_toml`). `EmitError` (a bare `fmt::Error` wrapper) is
/// not reachable writing into an in-memory `String` — folded to a
/// `"~\n"` (YAML's own null literal) fallback rather than unwrapping,
/// so this function can never panic regardless. The emitted text
/// always begins with `saphyr::YamlEmitter::dump`'s own `---\n`
/// document-start marker — a real, disclosed detail of this crate's
/// own emitter, not this module's addition.
///
/// # Safety
/// `obj` must point to a real `JsonValue` block.
pub unsafe fn json_to_yaml(obj: *const c_void) -> *const c_char {
  if obj.is_null() {
    return crate::alloc_and_copy_str("--- ~\n");
  }
  let json_val = lift_json_value(obj);
  let yaml_val = json_to_yaml_value(&json_val);
  let mut out = String::new();
  let mut emitter = YamlEmitter::new(&mut out);
  if emitter.dump(&yaml_val).is_err() {
    return crate::alloc_and_copy_str("--- ~\n");
  }
  crate::alloc_and_copy_str(&out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse_ok(yaml: &str) -> *mut c_void {
    let cstr = std::ffi::CString::new(yaml).unwrap();
    let result = unsafe { yaml_parse(cstr.as_ptr()) } as *const i64;
    let discriminant = unsafe { *result };
    assert_eq!(discriminant, 0, "expected Ok for input: {yaml}");
    (unsafe { *(result.add(1)) }) as *mut c_void
  }

  fn parse_err_tag(yaml: &str) -> i32 {
    let cstr = std::ffi::CString::new(yaml).unwrap();
    let result = unsafe { yaml_parse(cstr.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *result }, 1, "expected Err for input: {yaml}");
    let err_block = unsafe { *(result.add(1)) } as *const i32;
    unsafe { *err_block }
  }

  #[test]
  fn parse_null_lowers_to_the_null_tag() {
    let doc = parse_ok("~") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_NULL);
  }

  #[test]
  fn parse_bool_lowers_the_real_boolean_value() {
    let doc = parse_ok("true") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_BOOL);
    assert_eq!(unsafe { *(doc.add(1) as *const u8) }, 1);
  }

  #[test]
  fn parse_integer_widens_to_a_real_f64() {
    let doc = parse_ok("36") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_NUMBER);
    assert_eq!(unsafe { *(doc.add(1) as *const f64) }, 36.0);
  }

  #[test]
  fn parse_float_lowers_a_real_f64() {
    let doc = parse_ok("1.5") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_NUMBER);
    assert_eq!(unsafe { *(doc.add(1) as *const f64) }, 1.5);
  }

  #[test]
  fn parse_string_round_trips_the_real_bytes() {
    let doc = parse_ok("Ada") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_STRING);
    let s_ptr = unsafe { *(doc.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "Ada");
  }

  #[test]
  fn parse_block_sequence_lowers_a_real_length_and_each_real_element() {
    let doc = parse_ok("- math\n- cs\n") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_ARRAY);
    let arr_ptr = unsafe { *(doc.add(1) as *const *const i64) };
    let len = unsafe { *arr_ptr };
    assert_eq!(len, 2);
    let elems = unsafe { arr_ptr.add(1) as *const *const c_void };
    let first = unsafe { *elems } as *const i64;
    assert_eq!(unsafe { *first }, TAG_STRING);
  }

  #[test]
  fn parse_block_mapping_round_trips_through_json_object_get() {
    let doc = parse_ok("name: Ada\nactive: true\n") as *const i64;
    assert_eq!(unsafe { *doc }, TAG_OBJECT);
    let key = std::ffi::CString::new("name").unwrap();
    let v =
      unsafe { crate::json::json_object_get(doc as *const c_void, key.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *v }, 0, "expected Some");
    let payload = unsafe { *(v.add(1)) } as *const i64;
    assert_eq!(unsafe { *payload }, TAG_STRING);
    let s_ptr = unsafe { *(payload.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(s, "Ada");
  }

  #[test]
  fn parse_anchor_and_alias_resolve_to_the_same_real_value() {
    let doc = parse_ok("a: &anchor Ada\nb: *anchor\n") as *const i64;
    let b_key = std::ffi::CString::new("b").unwrap();
    let v =
      unsafe { crate::json::json_object_get(doc as *const c_void, b_key.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *v }, 0, "expected Some for `b`");
    let payload = unsafe { *(v.add(1)) } as *const i64;
    assert_eq!(unsafe { *payload }, TAG_STRING);
    let s_ptr = unsafe { *(payload.add(1) as *const *const c_char) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert_eq!(
      s, "Ada",
      "the alias should resolve to the anchor's own real value"
    );
  }

  #[test]
  fn parse_empty_document_stream_returns_a_real_empty_document_err() {
    assert_eq!(parse_err_tag(""), YAML_ERROR_TAG_EMPTY_DOCUMENT);
  }

  #[test]
  fn parse_invalid_indentation_returns_a_real_syntax_err() {
    // A real YAML indentation error: mixing a sequence item and a
    // scalar at the same nesting level under `key:` — this plan's own
    // second Concrete Proof negative example.
    assert_eq!(parse_err_tag("key:\n  - a\n  b"), YAML_ERROR_TAG_SYNTAX);
  }

  #[test]
  fn scratch_print_full_demo_doc_to_yaml() {
    let yaml_in = "name: Ada\nlanguages:\n  - math\n  - cs\nactive: true\n";
    let doc = parse_ok(yaml_in);
    let s_ptr = unsafe { json_to_yaml(doc) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    eprintln!("SCRATCH_TO_YAML_OUTPUT_START\n{s}\nSCRATCH_TO_YAML_OUTPUT_END");
    let cstr = std::ffi::CString::new("key:\n  - a\n  b").unwrap();
    let result = unsafe { yaml_parse(cstr.as_ptr()) } as *const i64;
    let err_block = unsafe { *(result.add(1)) } as *const i64;
    let msg_ptr = unsafe { *(err_block.add(1)) } as *const c_char;
    let msg = unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
      .to_str()
      .unwrap();
    eprintln!("SCRATCH_SYNTAX_ERR_START\n{msg}\nSCRATCH_SYNTAX_ERR_END");
  }

  #[test]
  fn to_yaml_round_trips_a_simple_object() {
    let json = std::ffi::CString::new("{\"name\":\"Ada\",\"active\":true}").unwrap();
    let parsed = unsafe { crate::json::json_parse(json.as_ptr()) } as *const i64;
    assert_eq!(unsafe { *parsed }, 0, "expected Ok");
    let obj = unsafe { *(parsed.add(1)) } as *const c_void;
    let s_ptr = unsafe { json_to_yaml(obj) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert!(s.contains("name: Ada"), "unexpected output: {s}");
    assert!(s.contains("active: true"), "unexpected output: {s}");
  }

  #[test]
  fn to_yaml_represents_a_null_value_totally_never_erring() {
    let json = std::ffi::CString::new("{\"x\":null}").unwrap();
    let parsed = unsafe { crate::json::json_parse(json.as_ptr()) } as *const i64;
    let obj = unsafe { *(parsed.add(1)) } as *const c_void;
    let s_ptr = unsafe { json_to_yaml(obj) };
    let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
    assert!(s.contains("x:"), "unexpected output: {s}");
  }
}
