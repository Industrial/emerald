//! Plan 154 (Unicode Normalization & Segmentation) — `.nfc`/`.nfd`/
//! `.nfkc`/`.nfkd` (canonical/compatibility (de)composition, wrapping
//! `unicode-normalization`, Unicode Standard Annex #15) plus
//! `.codepoint_count`/`.grapheme_count`/`.graphemes`/`.words`/
//! `.sentences`/`.grapheme_split_count`/`.word_split_count`/
//! `.sentence_split_count` (real Unicode Annex #29 boundary
//! segmentation, wrapping `unicode-segmentation`) — closing the gap
//! plan 45's `.length` (a raw `strlen()` byte count, per plan 59's own
//! finding) leaves wide open: neither a byte count nor a codepoint
//! count answers "how many characters does a person see here" — only
//! real Unicode Annex #29 grapheme-cluster segmentation does.
//!
//! **UTF-8 policy**: every function here decodes its `*const c_char`
//! receiver via `CStr::from_ptr(..).to_bytes()` then
//! `String::from_utf8_lossy(..)` — NOT `.to_str().unwrap()` — so that
//! a receiver holding plan 153's own disclosed non-UTF-8
//! `Encoding.encode` output (or any other source of non-UTF-8 bytes
//! this project's runtime never actually prevents) degrades to
//! U+FFFD replacement characters inside `catch_unwind` rather than
//! panicking the Rust side. This is a real, disclosed policy choice
//! this plan is the first to need — see this plan's own history doc
//! Decision log.
//!
//! **`Array[String]`-returning functions** (`.graphemes`/`.words`/
//! `.sentences`) build the `[length: Int64][elements: *mut c_void, ...]`
//! header-prefixed layout `build_array_lit`'s own current doc comment
//! documents (plan 42's `leaf-array-length-header`), the same layout
//! `regex.rs`'s own private `build_string_array` helper already builds
//! for `.find_all`/`.captures`/`.split` — duplicated here rather than
//! shared cross-module (that helper is private to `regex.rs`, matching
//! this crate's existing per-module-duplication style, not a shared
//! `crate`-level utility). **A real, disclosed correction against this
//! plan's own text**: this plan's own Decision log describes
//! `Array[T]` as "no length prefix, no bounds checking", quoting an
//! old `build_array_lit` doc comment — that comment is stale as of
//! plan 42, which added the length header both to `build_array_lit`
//! and to `emerald_string_split` (the C runtime's own `.split`, fixing
//! what its own comment calls "a real, disclosed regression" from a
//! pre-plan-42 header-less buffer). The header-prefixed layout below
//! matches the CURRENT, real convention every other `Array[T]`-valued
//! runtime call already follows, not the plan's own stale
//! description.

use std::ffi::c_void;
use std::os::raw::c_char;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Decodes `s` into an owned, always-valid-UTF-8 `String`, lossily —
/// see this module's own doc comment for why `from_utf8_lossy`, not
/// `to_str().unwrap()`, is this plan's deliberate policy. A null `s`
/// decodes to the empty string, matching this crate's other String
/// intrinsics' null-is-empty convention.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
unsafe fn decode(s: *const c_char) -> String {
  if s.is_null() {
    return String::new();
  }
  let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
  String::from_utf8_lossy(bytes).into_owned()
}

/// Builds the `[length: Int64][elements: *mut c_void, ...]` header-
/// prefixed `Array[String]` buffer — see this module's own doc
/// comment for why this layout, not a bare header-less pointer.
unsafe fn build_string_array(items: &[&str]) -> *mut c_void {
  let ptr = crate::emerald_alloc(8 + 8 * items.len() as i64) as *mut i64;
  *ptr = items.len() as i64;
  let elems = ptr.add(1) as *mut *const c_char;
  for (i, s) in items.iter().enumerate() {
    *elems.add(i) = crate::alloc_and_copy_str(s);
  }
  ptr as *mut c_void
}

/// `String.nfc(self): String` — canonical composition (UAX #15).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_nfc(s: *const c_char) -> *const c_char {
  let out: String = decode(s).nfc().collect();
  crate::alloc_and_copy_str(&out)
}

/// `String.nfd(self): String` — canonical decomposition (UAX #15).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_nfd(s: *const c_char) -> *const c_char {
  let out: String = decode(s).nfd().collect();
  crate::alloc_and_copy_str(&out)
}

/// `String.nfkc(self): String` — compatibility composition (UAX #15).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_nfkc(s: *const c_char) -> *const c_char {
  let out: String = decode(s).nfkc().collect();
  crate::alloc_and_copy_str(&out)
}

/// `String.nfkd(self): String` — compatibility decomposition (UAX #15).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_nfkd(s: *const c_char) -> *const c_char {
  let out: String = decode(s).nfkd().collect();
  crate::alloc_and_copy_str(&out)
}

/// `String.codepoint_count(self): Int64` — `chars().count()` is
/// already codepoint-exact (a Rust `char` IS a Unicode scalar value);
/// no third-party crate is needed for this one, per this plan's own
/// Decision log.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_codepoint_count(s: *const c_char) -> i64 {
  decode(s).chars().count() as i64
}

/// `String.grapheme_count(self): Int64` — extended grapheme clusters
/// (`graphemes(true)`, UAX #29's modern default), never legacy
/// clusters — see this plan's own Decision log for why only the
/// extended mode is exposed.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_grapheme_count(s: *const c_char) -> i64 {
  decode(s).graphemes(true).count() as i64
}

/// `String.graphemes(self): Array[String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_graphemes(s: *const c_char) -> *mut c_void {
  let decoded = decode(s);
  let items: Vec<&str> = decoded.graphemes(true).collect();
  build_string_array(&items)
}

/// `String.words(self): Array[String]` — `unicode_words()`, a real
/// behavioral upgrade over `.split(" ")` (plan 45): internal
/// apostrophes don't split a contraction, and whitespace/punctuation
/// runs don't produce empty segments — see this plan's own Decision
/// log.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_words(s: *const c_char) -> *mut c_void {
  let decoded = decode(s);
  let items: Vec<&str> = decoded.unicode_words().collect();
  build_string_array(&items)
}

/// `String.sentences(self): Array[String]`.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_sentences(s: *const c_char) -> *mut c_void {
  let decoded = decode(s);
  let items: Vec<&str> = decoded.unicode_sentences().collect();
  build_string_array(&items)
}

/// `String.grapheme_split_count(self): Int64` — the `.graphemes`
/// companion scalar (`Array[T]` carries no runtime length metadata a
/// caller can query independently — plan 45's own `.split`/
/// `.split_count` precedent), computed by an independent second pass
/// over the input rather than by reading back `.graphemes`' own
/// result length, mirroring that precedent exactly. Identical in
/// value (not in implementation shape) to `.grapheme_count` above —
/// a real, unavoidable consequence of "extended grapheme cluster" and
/// "user-perceived character" being the same concept; both names are
/// kept because they answer two different questions on purpose
/// (a scalar in isolation vs. the count paired with `.graphemes`'
/// own array).
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_grapheme_split_count(s: *const c_char) -> i64 {
  decode(s).graphemes(true).count() as i64
}

/// `String.word_split_count(self): Int64` — the `.words` companion
/// scalar, independent second pass, same shape as `.grapheme_split_
/// count` above.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_word_split_count(s: *const c_char) -> i64 {
  decode(s).unicode_words().count() as i64
}

/// `String.sentence_split_count(self): Int64` — the `.sentences`
/// companion scalar, independent second pass, same shape as
/// `.grapheme_split_count` above.
///
/// # Safety
/// `s`, if non-null, must point to a valid, NUL-terminated C string.
pub unsafe fn string_sentence_split_count(s: *const c_char) -> i64 {
  decode(s).unicode_sentences().count() as i64
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

  unsafe fn read_array(ptr: *mut c_void) -> Vec<String> {
    let base = ptr as *mut i64;
    let count = *base as usize;
    let elems = (base.add(1)) as *mut *const c_char;
    (0..count).map(|i| cstr(*elems.add(i))).collect()
  }

  // Same fixed strings this plan's own Concrete Proof example uses:
  // `composed` = "café" with the precomposed U+00E9; `decomposed` =
  // "cafe" + a literal COMBINING ACUTE ACCENT (U+0301).
  fn composed() -> CString {
    c("caf\u{e9}")
  }
  fn decomposed() -> CString {
    c("cafe\u{301}")
  }

  #[test]
  fn codepoint_count_distinguishes_precomposed_from_decomposed() {
    unsafe {
      assert_eq!(string_codepoint_count(composed().as_ptr()), 4);
      assert_eq!(string_codepoint_count(decomposed().as_ptr()), 5);
    }
  }

  #[test]
  fn grapheme_count_is_four_for_both_forms() {
    unsafe {
      assert_eq!(string_grapheme_count(composed().as_ptr()), 4);
      assert_eq!(string_grapheme_count(decomposed().as_ptr()), 4);
      assert_eq!(string_grapheme_split_count(composed().as_ptr()), 4);
      assert_eq!(string_grapheme_split_count(decomposed().as_ptr()), 4);
    }
  }

  #[test]
  fn nfc_recomposes_the_decomposed_form_to_match_the_precomposed_one() {
    unsafe {
      let normalized_ptr = string_nfc(decomposed().as_ptr());
      let normalized = cstr(normalized_ptr);
      assert_eq!(normalized, "caf\u{e9}");
      assert_eq!(string_codepoint_count(normalized_ptr), 4);
    }
  }

  #[test]
  fn nfd_decomposes_the_precomposed_form_to_match_the_decomposed_one() {
    unsafe {
      let decomposed_ptr = string_nfd(composed().as_ptr());
      assert_eq!(cstr(decomposed_ptr), "cafe\u{301}");
    }
  }

  #[test]
  fn nfkc_and_nfkd_round_trip_a_compatibility_ligature() {
    unsafe {
      // U+FB01 LATIN SMALL LIGATURE FI has no canonical decomposition
      // (NFD leaves it untouched) but DOES have a compatibility
      // decomposition to "fi" (two ordinary letters) - the real,
      // documented distinction between canonical and compatibility
      // (de)composition this plan's own two extra intrinsics exist to
      // expose.
      let ligature = c("\u{fb01}");
      let nfkd_ptr = string_nfkd(ligature.as_ptr());
      assert_eq!(cstr(nfkd_ptr), "fi");
      let nfd_ptr = string_nfd(ligature.as_ptr());
      assert_eq!(cstr(nfd_ptr), "\u{fb01}");
      let nfkc_ptr = string_nfkc(ligature.as_ptr());
      assert_eq!(cstr(nfkc_ptr), "fi");
    }
  }

  #[test]
  fn grapheme_count_treats_a_family_zwj_emoji_sequence_as_one_character() {
    unsafe {
      // "👨‍👩‍👧‍👦" - four base emoji codepoints joined by three ZERO
      // WIDTH JOINER codepoints, seven codepoints total, one grapheme.
      let family = c("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}");
      assert_eq!(string_codepoint_count(family.as_ptr()), 7);
      assert_eq!(string_grapheme_count(family.as_ptr()), 1);
    }
  }

  #[test]
  fn graphemes_splits_a_combining_sequence_into_one_element_per_letter() {
    unsafe {
      let items = read_array(string_graphemes(decomposed().as_ptr()));
      assert_eq!(items, vec!["c", "a", "f", "e\u{301}"]);
    }
  }

  #[test]
  fn words_does_not_split_an_internal_apostrophe_contraction() {
    unsafe {
      let s = c("The quick (\"brown\") fox can't jump 32.3 feet, right?");
      let items = read_array(string_words(s.as_ptr()));
      assert_eq!(
        items,
        vec!["The", "quick", "brown", "fox", "can't", "jump", "32.3", "feet", "right"]
      );
      assert_eq!(string_word_split_count(s.as_ptr()), 9);
    }
  }

  #[test]
  fn sentences_splits_on_real_sentence_boundaries() {
    unsafe {
      // Deliberately avoids an abbreviation like "Mr." — UAX #29's
      // sentence-break rules have no abbreviation dictionary, so a
      // period after "Mr" is itself a real sentence boundary as far
      // as this algorithm is concerned (confirmed empirically: an
      // earlier version of this test used exactly that sentence and
      // got 3 segments, not the 2 a human reader would say).
      let s = c("The quick fox jumps. The lazy dog sleeps!");
      let items = read_array(string_sentences(s.as_ptr()));
      assert_eq!(items.len(), 2);
      assert_eq!(string_sentence_split_count(s.as_ptr()), 2);
    }
  }

  #[test]
  fn a_non_utf8_receiver_degrades_to_replacement_characters_rather_than_panicking() {
    unsafe {
      // Plan 153's own disclosed non-UTF-8 `Encoding.encode` output —
      // this module's own `from_utf8_lossy` policy is what keeps a
      // caller who accidentally routes such a value through a
      // Unicode-aware method from crashing the process.
      let windows_1252_bytes: &[u8] = &[0x63, 0x61, 0x66, 0xe9, 0x00];
      let raw = windows_1252_bytes.as_ptr() as *const c_char;
      // Must not panic - the only assertion this test needs to make.
      let _ = string_codepoint_count(raw);
      let _ = string_grapheme_count(raw);
    }
  }
}
