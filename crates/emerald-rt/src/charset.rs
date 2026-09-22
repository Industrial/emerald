//! Plan 153 (Character Set / Encoding Conversion) — `Encoding.decode`/
//! `.decode_strict`/`.encode`, wrapping `encoding_rs` (the actual
//! WHATWG Encoding Standard implementation shipping inside Firefox/
//! Gecko) to move text between Emerald's native UTF-8 `String` and
//! legacy single- and multi-byte encodings (Latin-1/windows-1252,
//! Shift_JIS, GBK, EUC-JP, and every other WHATWG-registered label).
//!
//! Named `charset`, not `encoding` — `crates/emerald-rt/src/
//! encoding.rs` is already taken by plan 123's Base64/Hex module (that
//! file's own module doc explains its name predates this plan and
//! wraps a different pair of namespaces, `Base64`/`Hex`, entirely).
//! `mod charset;` in `lib.rs` avoids that collision the same way
//! `csvs`/`tomls`/`urls` already dodge their own external-crate-name
//! collisions — this file's real, Emerald-facing surface is the
//! `Encoding` namespace, the module's own internal Rust name is just
//! not that literal spelling.
//!
//! **`String`'s documented UTF-8 invariant is knowingly, deliberately
//! violated by `Encoding.encode`'s own return value** — see this
//! plan's own history doc (`history/2026-09-21T210200Z-plan-153-
//! charset-encoding.md`) Decision log for the full account. Nothing in
//! `emerald_runtime.c` or `emerald-codegen` validates a `String`'s
//! bytes as UTF-8 anywhere (`emerald_alloc` is a bare `malloc`
//! wrapper); this plan is simply the first to *produce* a non-UTF-8
//! `String` on purpose rather than merely tolerate one. Never call a
//! Unicode-aware `String` method on an `Encoding.encode` result — only
//! `Encoding.decode`/`.decode_strict` and `File.write` are safe
//! consumers of it.

use std::os::raw::c_char;

/// WHATWG label matching (`encoding_rs::Encoding::for_label`'s own
/// ASCII-lowercasing/trimming/alias resolution) — e.g. `"iso-8859-1"`
/// deliberately resolves to windows-1252, not genuine ISO/IEC 8859-1,
/// per the Encoding Standard's own Web-compat mandate.
///
/// # Safety
/// `label_ptr`, if non-null, must point to a valid, NUL-terminated C
/// string.
unsafe fn resolve(label_ptr: *const c_char) -> Option<&'static encoding_rs::Encoding> {
  if label_ptr.is_null() {
    return None;
  }
  let bytes = std::ffi::CStr::from_ptr(label_ptr).to_bytes();
  encoding_rs::Encoding::for_label(bytes)
}

/// Renders `label_ptr` for an error message without requiring it to be
/// valid UTF-8 (a garbage label is exactly the case being reported).
///
/// # Safety
/// `label_ptr`, if non-null, must point to a valid, NUL-terminated C
/// string.
unsafe fn label_for_display(label_ptr: *const c_char) -> String {
  if label_ptr.is_null() {
    return "<null>".to_string();
  }
  std::ffi::CStr::from_ptr(label_ptr)
    .to_string_lossy()
    .into_owned()
}

/// Copies raw, not-necessarily-valid-UTF-8 bytes plus a manually
/// appended trailing NUL into a fresh `emerald_alloc` buffer —
/// `Encoding.encode`'s own return value is exactly this: a `String`-
/// shaped carrier for bytes this plan's own module doc discloses may
/// not be valid UTF-8 at all, so `crate::alloc_and_copy_str` (which
/// requires a real `&str`) cannot be reused here.
unsafe fn alloc_and_copy_bytes(data: &[u8]) -> *const c_char {
  let buf = crate::emerald_alloc(data.len() as i64 + 1) as *mut u8;
  std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
  *buf.add(data.len()) = 0;
  buf as *const c_char
}

/// `Encoding.decode(raw: String, label: String): String` — lossy:
/// malformed/unmappable byte sequences become U+FFFD per the WHATWG
/// spec, which never fails to decode SOME text. An unresolvable
/// `label` is a real, disclosed runtime abort (`fprintf(stderr, ...);
/// exit(1)`, plan 45's own File-I/O-error precedent), not a raised,
/// rescuable exception — the same fatal-misconfiguration posture
/// `File.read`/`File.write` already take for an unopenable path.
///
/// # Safety
/// `raw`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings. `raw` is read as a raw byte sequence, not required to be
/// valid UTF-8 — that is the whole point of this function.
pub unsafe fn encoding_decode(raw: *const c_char, label: *const c_char) -> *const c_char {
  let Some(enc) = resolve(label) else {
    eprintln!(
      "uncaught error: Encoding.decode unknown label '{}'",
      label_for_display(label)
    );
    std::process::exit(1);
  };
  if raw.is_null() {
    eprintln!("uncaught error: Encoding.decode null string pointer");
    std::process::exit(1);
  }
  let bytes = std::ffi::CStr::from_ptr(raw).to_bytes();
  let (decoded, _enc, _had_errors) = enc.decode(bytes);
  crate::alloc_and_copy_str(decoded.as_ref())
}

/// `Encoding.decode_strict(raw: String, label: String): String?` —
/// the same underlying `encoding_rs::Encoding::decode` call as
/// [`encoding_decode`], reacting differently to its `had_errors` flag:
/// a null pointer (Emerald `nil`, plan 43/59's null-pointer-is-`nil`
/// convergence) instead of the lossy replacement-character output. An
/// unresolvable `label` folds into the same `nil`, rather than
/// aborting — from the caller's perspective both are "asked for valid
/// text back and didn't get it," a real, disclosed asymmetry against
/// `encode`/`decode`'s own abort-on-bad-label policy, not an
/// inconsistency (see this plan's own Decision log).
///
/// # Safety
/// `raw`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings. `raw` is read as a raw byte sequence, not required to be
/// valid UTF-8.
pub unsafe fn encoding_decode_strict(raw: *const c_char, label: *const c_char) -> *mut c_char {
  let Some(enc) = resolve(label) else {
    return std::ptr::null_mut();
  };
  if raw.is_null() {
    return std::ptr::null_mut();
  }
  let bytes = std::ffi::CStr::from_ptr(raw).to_bytes();
  let (decoded, _enc, had_errors) = enc.decode(bytes);
  if had_errors {
    return std::ptr::null_mut();
  }
  crate::alloc_and_copy_str(decoded.as_ref()) as *mut c_char
}

/// `Encoding.encode(text: String, label: String): String` — returns a
/// `String`-typed value whose bytes are, in general, NOT valid UTF-8
/// (see this module's own doc comment). An unresolvable `label` aborts
/// exactly like [`encoding_decode`]'s own policy. `text` itself is
/// expected to be a real, valid-UTF-8 Emerald `String` (an ordinary
/// source-level string literal, never a prior `Encoding.encode`
/// result — see this plan's own Decision log for why re-encoding an
/// already-encoded value is out of scope); a `text` that somehow isn't
/// valid UTF-8 is handled via a lossy conversion rather than a panic,
/// matching `encoding_rs::Encoding::encode`'s own total-function
/// posture over its input domain.
///
/// # Safety
/// `text`/`label`, if non-null, must point to valid, NUL-terminated C
/// strings.
pub unsafe fn encoding_encode(text: *const c_char, label: *const c_char) -> *const c_char {
  let Some(enc) = resolve(label) else {
    eprintln!(
      "uncaught error: Encoding.encode unknown label '{}'",
      label_for_display(label)
    );
    std::process::exit(1);
  };
  if text.is_null() {
    eprintln!("uncaught error: Encoding.encode null string pointer");
    std::process::exit(1);
  }
  let text = std::ffi::CStr::from_ptr(text).to_string_lossy();
  let (encoded, _enc, _had_errors) = enc.encode(&text);
  alloc_and_copy_bytes(encoded.as_ref())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::ffi::CString;

  fn c(s: &str) -> CString {
    CString::new(s).unwrap()
  }

  unsafe fn cstr(p: *const c_char) -> String {
    std::ffi::CStr::from_ptr(p).to_str().unwrap().to_string()
  }

  #[test]
  fn windows_1252_round_trips_a_non_ascii_string() {
    unsafe {
      let text = c("café");
      let label = c("windows-1252");
      let encoded_ptr = encoding_encode(text.as_ptr(), label.as_ptr());
      // "café" encodes to the 4 single bytes 63 61 66 e9 in
      // windows-1252 — the trailing 0xe9 is not a valid standalone
      // UTF-8 byte, proving the returned `String` really does carry
      // non-UTF-8 bytes.
      let encoded_bytes = std::ffi::CStr::from_ptr(encoded_ptr).to_bytes();
      assert_eq!(encoded_bytes, &[0x63, 0x61, 0x66, 0xe9]);

      let encoded_cstr = CString::new(encoded_bytes).unwrap();
      let decoded_ptr = encoding_decode(encoded_cstr.as_ptr(), label.as_ptr());
      assert_eq!(cstr(decoded_ptr), "café");
    }
  }

  #[test]
  fn shift_jis_round_trips_japanese_text() {
    unsafe {
      let text = c("日本語");
      let label = c("shift_jis");
      let encoded_ptr = encoding_encode(text.as_ptr(), label.as_ptr());
      let encoded_bytes = std::ffi::CStr::from_ptr(encoded_ptr).to_bytes().to_vec();
      // Real Shift_JIS is emphatically not UTF-8 for this text (each
      // of these three characters is 2 bytes in Shift_JIS vs 3 in
      // UTF-8) — assert the actual byte count differs from the
      // source's own UTF-8 length as a sanity check that real
      // transcoding happened, not a no-op copy.
      assert_ne!(encoded_bytes.len(), text.as_bytes().len());

      let encoded_cstr = CString::new(encoded_bytes).unwrap();
      let decoded_ptr = encoding_decode(encoded_cstr.as_ptr(), label.as_ptr());
      assert_eq!(cstr(decoded_ptr), "日本語");
    }
  }

  #[test]
  fn decode_strict_returns_null_for_malformed_utf8_under_the_utf8_label() {
    unsafe {
      let label = c("windows-1252");
      let text = c("café");
      let encoded_ptr = encoding_encode(text.as_ptr(), label.as_ptr());
      let encoded_bytes = std::ffi::CStr::from_ptr(encoded_ptr).to_bytes().to_vec();
      let encoded_cstr = CString::new(encoded_bytes).unwrap();

      let utf8_label = c("utf-8");
      let result = encoding_decode_strict(encoded_cstr.as_ptr(), utf8_label.as_ptr());
      assert!(result.is_null(), "windows-1252 bytes are not valid UTF-8");

      let correct_label = c("windows-1252");
      let result = encoding_decode_strict(encoded_cstr.as_ptr(), correct_label.as_ptr());
      assert!(!result.is_null());
      assert_eq!(cstr(result as *const c_char), "café");
    }
  }

  #[test]
  fn decode_strict_returns_null_for_an_unresolvable_label() {
    unsafe {
      let text = c("hello");
      let bogus_label = c("not-a-real-encoding-label");
      let result = encoding_decode_strict(text.as_ptr(), bogus_label.as_ptr());
      assert!(result.is_null());
    }
  }

  #[test]
  fn iso_8859_1_label_resolves_to_windows_1252_per_the_encoding_standard() {
    // A real, disclosed WHATWG Web-compat quirk, not a bug: the
    // `"iso-8859-1"` label resolves to windows-1252, not genuine
    // ISO/IEC 8859-1.
    unsafe {
      let label = c("iso-8859-1");
      assert_eq!(
        resolve(label.as_ptr()).map(|e| e.name()),
        Some("windows-1252")
      );
    }
  }
}
