//! Plan 124 (XML Parsing) — `quick-xml` 0.42.0 wrapped two ways: a
//! `Xml.parse`/`.parse_file` tree-mode entry point building a
//! compiler-synthesized `XmlNode` enum (the XML-shaped sibling of
//! plan 118's `JsonValue`, registered in `emerald-sema`/`emerald-
//! codegen` the identical placeholder-seeded, self-referential way —
//! `Element(String, Hash[String,String], Array[XmlNode])` contains
//! itself), and a low-level `XmlReader` streaming-event handle (plan
//! 93's resource-handle model) for large documents. `quick-xml`
//! supplies no DOM/tree API of its own — the tree builder below is
//! real, hand-written recursive-descent logic driving the crate's own
//! streaming `Reader`, the same shape plan 91's own zero-dependency
//! proof function is, scaled up over a real dependency.
//!
//! A real, plan-text-contradicting finding from wiring this up: the
//! plan's own text names an `XmlEvent` variant `Text(String)`, but
//! this compiler's enum-variant namespace is GLOBAL, not per-enum
//! (`emerald-sema`'s own `seen_variant_names: HashSet<String>`), and
//! `XmlNode` already has a `Text` variant. `XmlNode::Text` keeps the
//! plan's own name (the plan's own Concrete Proof pattern-matches it
//! directly); `XmlEvent`'s sibling is `TextContent` here instead — see
//! `emerald-sema`/`emerald-codegen`'s own Decision logs for the full
//! reasoning.
//!
//! Byte layout, mirroring `emerald-codegen`'s own `EnumLayout` exactly
//! (`build_enum_layout`'s own doc comment): `[tag: i64][field0: 8
//! bytes][field1: 8 bytes]...` up to the WIDEST variant's field count,
//! shared uniformly by every variant of the same enum. `XmlNode`'s
//! widest variant (`Element`) has 3 fields, so EVERY `XmlNode` block
//! (including `Text`, which only uses field 0) is 32 bytes wide.
//! `XmlEvent`'s widest variant (`StartElement`) has 2 fields, so every
//! `XmlEvent` block is 24 bytes wide, including the field-less `Eof`.
//!
//! `Array[T]`/`Hash[K,V]` buffers this module hand-builds use exactly
//! the same byte-for-byte layouts `json.rs`'s own `lower_array`/
//! `lower_object` already establish and document — `[len: i64][elem:
//! 8 bytes, ...]` for `Array[T]`, `[pair-count: i64][(key_ptr: 8,
//! value_ptr: 8), ...]` for `Hash[K,V]` — reused verbatim, not
//! re-derived.
//!
//! A real, disclosed v1 design decision: `Config::check_end_names`
//! defaults to `true` in `quick-xml` itself, so a mismatched end tag
//! (`<a><b></c></a>`) already surfaces as a real `Err` from the
//! crate's own `read_event`/`read_event_into` — this module adds no
//! bespoke well-formedness re-validation on top of that. `Event::
//! Empty` (a self-closing tag, `<tag/>`) has no dedicated variant in
//! either `XmlNode` or `XmlEvent` (the plan's own 2/4-variant shapes) —
//! tree mode treats it as an `Element` with zero children; streaming
//! mode synthesizes a `StartElement` immediately followed by a queued
//! `EndElement` on the NEXT `.next_event()` call, keeping every real
//! document representable in the plan's own fixed event shape without
//! silently dropping the tag.

use crate::handle::{handle_alloc, handle_get_mut};
use quick_xml::events::{BytesCData, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use std::ffi::c_void;
use std::os::raw::c_char;

const TAG: &str = "XmlReader";

const XMLNODE_MAX_FIELDS: usize = 3;
const NODE_ELEMENT: i64 = 0;
const NODE_TEXT: i64 = 1;

const XMLEVENT_MAX_FIELDS: usize = 2;
const EVT_START: i64 = 0;
const EVT_END: i64 = 1;
const EVT_TEXT: i64 = 2;
const EVT_EOF: i64 = 3;

unsafe fn read_str<'a>(s: *const c_char) -> Result<&'a str, String> {
  if s.is_null() {
    return Err("null string pointer".to_string());
  }
  std::ffi::CStr::from_ptr(s)
    .to_str()
    .map_err(|_| "input is not valid UTF-8".to_string())
}

unsafe fn alloc_block(tag: i64, max_fields: usize) -> *mut i64 {
  let ptr = crate::emerald_alloc(8 + 8 * max_fields as i64) as *mut i64;
  *ptr = tag;
  ptr
}

/// `Hash[String, String]`'s own buffer — attribute maps.
unsafe fn build_string_hash(pairs: &[(String, String)]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 16 * pairs.len() as i64) as *mut i64;
  *ptr = pairs.len() as i64;
  let slots = ptr.add(1) as *mut *mut c_void;
  for (i, (k, v)) in pairs.iter().enumerate() {
    *slots.add(i * 2) = crate::alloc_and_copy_str(k) as *mut c_void;
    *slots.add(i * 2 + 1) = crate::alloc_and_copy_str(v) as *mut c_void;
  }
  ptr as *mut c_void
}

/// `Array[XmlNode]`'s own buffer — an element's children.
unsafe fn build_ptr_array(items: &[*mut c_void]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *mut c_void;
  for (i, p) in items.iter().enumerate() {
    *elems.add(i) = *p;
  }
  ptr as *mut c_void
}

unsafe fn alloc_element(
  tag: &str,
  attrs: &[(String, String)],
  children: &[*mut c_void],
) -> *mut c_void {
  let ptr = alloc_block(NODE_ELEMENT, XMLNODE_MAX_FIELDS);
  *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(tag);
  *(ptr.add(2) as *mut *mut c_void) = build_string_hash(attrs);
  *(ptr.add(3) as *mut *mut c_void) = build_ptr_array(children);
  ptr as *mut c_void
}

unsafe fn alloc_text_node(text: &str) -> *mut c_void {
  let ptr = alloc_block(NODE_TEXT, XMLNODE_MAX_FIELDS);
  *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(text);
  ptr as *mut c_void
}

unsafe fn alloc_start_event(tag: &str, attrs: &[(String, String)]) -> *mut c_void {
  let ptr = alloc_block(EVT_START, XMLEVENT_MAX_FIELDS);
  *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(tag);
  *(ptr.add(2) as *mut *mut c_void) = build_string_hash(attrs);
  ptr as *mut c_void
}

unsafe fn alloc_end_event(tag: &str) -> *mut c_void {
  let ptr = alloc_block(EVT_END, XMLEVENT_MAX_FIELDS);
  *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(tag);
  ptr as *mut c_void
}

unsafe fn alloc_text_event(text: &str) -> *mut c_void {
  let ptr = alloc_block(EVT_TEXT, XMLEVENT_MAX_FIELDS);
  *(ptr.add(1) as *mut *const c_char) = crate::alloc_and_copy_str(text);
  ptr as *mut c_void
}

unsafe fn alloc_eof_event() -> *mut c_void {
  alloc_block(EVT_EOF, XMLEVENT_MAX_FIELDS) as *mut c_void
}

fn read_attrs(e: &BytesStart) -> Result<Vec<(String, String)>, String> {
  let mut out = Vec::new();
  for a in e.attributes() {
    let a = a.map_err(|err| err.to_string())?;
    let key = a.key.as_ref().to_string();
    let value = a
      .normalized_value(quick_xml::XmlVersion::Implicit1_0)
      .map_err(|err| err.to_string())?
      .into_owned();
    out.push((key, value));
  }
  Ok(out)
}

fn text_content(t: &BytesText) -> String {
  t.xml10_content().into_owned()
}

fn cdata_content(t: &BytesCData) -> String {
  t.xml10_content().into_owned()
}

// --- Tree mode: `Xml.parse`/`.parse_file` ------------------------------
//
// Drives quick-xml's own borrowing `Reader<&[u8]>` (via `Reader::
// from_str`) to EOF internally, building nested `XmlNode` values on
// the Rust side and returning one fully-built tree — no tree-
// construction API is ever exposed to Emerald source, so this reader
// never needs the `crate::handle` disposal story `XmlReader` below
// does (it never outlives this one function call).

fn parse_tree(input: &str) -> Result<*mut c_void, String> {
  let mut reader = Reader::from_str(input);
  reader.config_mut().trim_text(true);
  loop {
    let ev = reader.read_event().map_err(|e| e.to_string())?;
    match ev {
      Event::Eof => return Err("Xml.parse: document has no root element".to_string()),
      Event::Start(e) => {
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        let children = parse_children(&mut reader, &tag)?;
        return Ok(unsafe { alloc_element(&tag, &attrs, &children) });
      }
      Event::Empty(e) => {
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        return Ok(unsafe { alloc_element(&tag, &attrs, &[]) });
      }
      Event::End(e) => {
        let tag = e.name().as_ref().to_string();
        return Err(format!(
          "Xml.parse: unexpected end tag `</{tag}>` before any start tag"
        ));
      }
      // Decl/Comment/PI/DocType/whitespace-only Text before the root
      // element are silently skipped, per this plan's own Decision
      // log (comments/PIs are never surfaced as `XmlNode`s at all).
      _ => continue,
    }
  }
}

fn parse_children(
  reader: &mut Reader<&[u8]>,
  parent_tag: &str,
) -> Result<Vec<*mut c_void>, String> {
  let mut children: Vec<*mut c_void> = Vec::new();
  let mut text_buf = String::new();
  let flush = |buf: &mut String, children: &mut Vec<*mut c_void>| {
    if !buf.is_empty() {
      children.push(unsafe { alloc_text_node(buf) });
      buf.clear();
    }
  };
  loop {
    let ev = reader.read_event().map_err(|e| e.to_string())?;
    match ev {
      Event::Eof => {
        return Err(format!(
          "Xml.parse: unexpected end of document inside <{parent_tag}>"
        ))
      }
      Event::Start(e) => {
        flush(&mut text_buf, &mut children);
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        let sub_children = parse_children(reader, &tag)?;
        children.push(unsafe { alloc_element(&tag, &attrs, &sub_children) });
      }
      Event::Empty(e) => {
        flush(&mut text_buf, &mut children);
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        children.push(unsafe { alloc_element(&tag, &attrs, &[]) });
      }
      Event::End(_) => {
        flush(&mut text_buf, &mut children);
        return Ok(children);
      }
      Event::Text(t) => text_buf.push_str(&text_content(&t)),
      Event::CData(t) => text_buf.push_str(&cdata_content(&t)),
      Event::GeneralRef(r) => {
        if let Ok(Some(ch)) = r.resolve_char_ref() {
          text_buf.push(ch);
        }
      }
      // Comment/Decl/PI/DocType silently skipped, per this plan's own
      // Decision log.
      _ => {}
    }
  }
}

/// `Xml.parse(s: String): Result[XmlNode, String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn xml_parse(s: *const c_char) -> *mut c_void {
  let input = match read_str(s) {
    Ok(s) => s,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match parse_tree(input) {
    Ok(ptr) => crate::emerald_rt_result_ok(ptr as i64),
    Err(e) => crate::emerald_rt_result_err_str(&e),
  }
}

/// `Xml.parse_file(path: String): Result[XmlNode, String]` — reads the
/// whole file to a `String` first (a real, disclosed simplification
/// over driving `Reader::from_file` directly: this plan's own tree
/// mode already builds its ENTIRE result in memory regardless, so
/// there is no streaming benefit to preserve here, unlike `XmlReader`
/// below), so a file-not-found or non-UTF-8 file surfaces as the same
/// `Result::Err(String)` channel a malformed document does.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn xml_parse_file(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  let contents = match std::fs::read_to_string(path) {
    Ok(c) => c,
    Err(e) => return crate::emerald_rt_result_err_str(&e.to_string()),
  };
  match parse_tree(&contents) {
    Ok(ptr) => crate::emerald_rt_result_ok(ptr as i64),
    Err(e) => crate::emerald_rt_result_err_str(&e),
  }
}

// --- Streaming mode: `XmlReader` resource handle ------------------------
//
// Unlike tree mode's borrowing `Reader<&[u8]>` above, a handle that
// outlives one function call must be `'static` (`Box<dyn Any + Send>`,
// `handle.rs`'s own bound) — so this owns its input bytes (`Cursor<
// Vec<u8>>` for a string source, `BufReader<File>` for a file source,
// both real owned types quick-xml's generic `Reader<R: BufRead>`
// already supports) rather than borrowing them, sidestepping the
// self-referential-struct problem entirely.

enum XmlSource {
  Str(Reader<std::io::Cursor<Vec<u8>>>),
  File(Reader<std::io::BufReader<std::fs::File>>),
}

struct XmlReaderState {
  source: XmlSource,
  buf: Vec<u8>,
  /// Set when the previous event was a self-closing `<tag/>` (`Event::
  /// Empty`) — quick-xml collapses that to ONE event, but neither
  /// `XmlEvent` variant this plan's own text defines has a "self-
  /// closing" shape, so this module synthesizes the `StartElement`
  /// immediately and queues the matching `EndElement` here, returned
  /// on the very next `.next_event()` call instead.
  pending_end: Option<String>,
}

fn next_event_inner(state: &mut XmlReaderState) -> Result<*mut c_void, String> {
  if let Some(tag) = state.pending_end.take() {
    return Ok(unsafe { alloc_end_event(&tag) });
  }
  let XmlReaderState {
    source,
    buf,
    pending_end,
  } = state;
  loop {
    buf.clear();
    let ev = match source {
      XmlSource::Str(r) => r.read_event_into(buf),
      XmlSource::File(r) => r.read_event_into(buf),
    }
    .map_err(|e| e.to_string())?;
    match ev {
      Event::Eof => return Ok(unsafe { alloc_eof_event() }),
      Event::Start(e) => {
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        return Ok(unsafe { alloc_start_event(&tag, &attrs) });
      }
      Event::Empty(e) => {
        let tag = e.name().as_ref().to_string();
        let attrs = read_attrs(&e)?;
        *pending_end = Some(tag.clone());
        return Ok(unsafe { alloc_start_event(&tag, &attrs) });
      }
      Event::End(e) => {
        let tag = e.name().as_ref().to_string();
        return Ok(unsafe { alloc_end_event(&tag) });
      }
      Event::Text(t) => {
        let text = text_content(&t);
        return Ok(unsafe { alloc_text_event(&text) });
      }
      Event::CData(t) => {
        let text = cdata_content(&t);
        return Ok(unsafe { alloc_text_event(&text) });
      }
      // Comment/Decl/PI/DocType/GeneralRef silently skipped, matching
      // tree mode's own Decision log (a general reference outside any
      // text run has no natural home in this plan's own 4-variant
      // `XmlEvent` shape, and no worked proof needs one).
      _ => continue,
    }
  }
}

/// `Xml.reader_from_string(s: String): XmlReader` — never `Result`
/// (see `emerald-sema`'s own Decision log): constructing a reader over
/// an in-memory string does no I/O and no parsing.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn xml_reader_from_string(s: *const c_char) -> i64 {
  let input = match read_str(s) {
    Ok(s) => s,
    Err(e) => crate::raise_native_error(&e),
  };
  let bytes = input.as_bytes().to_vec();
  let mut reader = Reader::from_reader(std::io::Cursor::new(bytes));
  reader.config_mut().trim_text(true);
  let state = XmlReaderState {
    source: XmlSource::Str(reader),
    buf: Vec::new(),
    pending_end: None,
  };
  handle_alloc(Box::new(state), TAG)
}

/// `Xml.reader_from_file(path: String): Result[XmlReader, String]`.
///
/// # Safety
/// `path`, if non-null, must point to a valid, NUL-terminated C
/// string.
pub unsafe fn xml_reader_from_file(path: *const c_char) -> *mut c_void {
  let path = match read_str(path) {
    Ok(p) => p,
    Err(e) => return crate::emerald_rt_result_err_str(&e),
  };
  match Reader::from_file(path) {
    Ok(mut reader) => {
      reader.config_mut().trim_text(true);
      let state = XmlReaderState {
        source: XmlSource::File(reader),
        buf: Vec::new(),
        pending_end: None,
      };
      let id = handle_alloc(Box::new(state), TAG);
      crate::emerald_rt_result_ok(id)
    }
    Err(e) => crate::emerald_rt_result_err_str(&e.to_string()),
  }
}

/// `XmlReader#next_event(self): XmlEvent`.
pub fn xml_reader_next_event(id: i64) -> *mut c_void {
  match handle_get_mut::<XmlReaderState, Result<*mut c_void, String>>(id, TAG, next_event_inner) {
    Ok(Ok(ptr)) => ptr,
    Ok(Err(msg)) => unsafe { crate::raise_native_error(&msg) },
    Err(msg) => unsafe { crate::raise_native_error(&msg) },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::{CStr, CString};

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn parse_ok(xml: &str) -> *const i64 {
    let cstr = c(xml);
    let result = xml_parse(cstr.as_ptr()) as *const i64;
    assert_eq!(*result, 0, "expected Ok for input: {xml}");
    (*(result.add(1))) as *const i64
  }

  unsafe fn field(block: *const i64, i: usize) -> *const i64 {
    *(block.add(1 + i) as *const *const i64)
  }

  unsafe fn c_str_field(block: *const i64, i: usize) -> String {
    let ptr = *(block.add(1 + i) as *const *const c_char);
    CStr::from_ptr(ptr).to_str().unwrap().to_string()
  }

  const DOC: &str = "<library><book title=\"Dune\" author=\"Herbert\" /><book title=\"1984\" author=\"Orwell\" /></library>";

  #[test]
  fn tree_mode_root_element_has_the_real_tag_and_two_children() {
    unsafe {
      let root = parse_ok(DOC);
      assert_eq!(*root, NODE_ELEMENT);
      assert_eq!(c_str_field(root, 0), "library");
      let children_ptr = *(root.add(3) as *const *const i64);
      let len = *children_ptr;
      assert_eq!(len, 2, "two <book> children");
    }
  }

  #[test]
  fn tree_mode_a_self_closing_child_carries_its_real_attributes() {
    unsafe {
      let root = parse_ok(DOC);
      let children_ptr = *(root.add(3) as *const *const i64);
      let elems = children_ptr.add(1) as *const *const i64;
      let first_book = *elems;
      assert_eq!(*first_book, NODE_ELEMENT);
      assert_eq!(c_str_field(first_book, 0), "book");
      let attrs_ptr = *(first_book.add(2) as *const *const i64);
      let pair_count = *attrs_ptr;
      assert_eq!(pair_count, 2, "title + author");
      let pairs = attrs_ptr.add(1) as *const *const c_char;
      let key0 = CStr::from_ptr(*pairs).to_str().unwrap();
      let val0 = CStr::from_ptr(*pairs.add(1)).to_str().unwrap();
      assert_eq!(key0, "title");
      assert_eq!(val0, "Dune");
    }
  }

  #[test]
  fn tree_mode_text_content_between_tags_becomes_a_text_node() {
    unsafe {
      let root = parse_ok("<a>hello</a>");
      assert_eq!(*root, NODE_ELEMENT);
      let children_ptr = *(root.add(3) as *const *const i64);
      let len = *children_ptr;
      assert_eq!(len, 1);
      let text_node = *(children_ptr.add(1) as *const *const i64);
      assert_eq!(*text_node, NODE_TEXT);
      assert_eq!(c_str_field(text_node, 0), "hello");
    }
  }

  #[test]
  fn a_mismatched_end_tag_is_a_real_err_not_a_panic() {
    unsafe {
      let cstr = c("<a><b></c></a>");
      let result = xml_parse(cstr.as_ptr()) as *const i64;
      assert_eq!(*result, 1, "expected Err");
      let msg_ptr = *(result.add(1)) as *const c_char;
      assert!(!CStr::from_ptr(msg_ptr).to_str().unwrap().is_empty());
    }
  }

  #[test]
  fn an_unclosed_document_is_a_real_err_not_a_panic() {
    unsafe {
      let cstr = c("<a><b>");
      let result = xml_parse(cstr.as_ptr()) as *const i64;
      assert_eq!(*result, 1, "expected Err");
    }
  }

  #[test]
  fn streaming_mode_yields_the_real_event_sequence_for_a_self_closing_tag() {
    unsafe {
      let cstr = c("<a><b x=\"1\"/></a>");
      let id = xml_reader_from_string(cstr.as_ptr());

      let e1 = xml_reader_next_event(id) as *const i64;
      assert_eq!(*e1, EVT_START);
      assert_eq!(c_str_field(e1, 0), "a");

      let e2 = xml_reader_next_event(id) as *const i64;
      assert_eq!(*e2, EVT_START, "self-closing <b/> starts as StartElement");
      assert_eq!(c_str_field(e2, 0), "b");
      let attrs_ptr = field(e2, 1);
      assert_eq!(*attrs_ptr, 1, "one attribute");

      let e3 = xml_reader_next_event(id) as *const i64;
      assert_eq!(
        *e3, EVT_END,
        "the queued synthetic EndElement for <b/> comes next"
      );
      assert_eq!(c_str_field(e3, 0), "b");

      let e4 = xml_reader_next_event(id) as *const i64;
      assert_eq!(*e4, EVT_END);
      assert_eq!(c_str_field(e4, 0), "a");

      let e5 = xml_reader_next_event(id) as *const i64;
      assert_eq!(*e5, EVT_EOF);
    }
  }
}
