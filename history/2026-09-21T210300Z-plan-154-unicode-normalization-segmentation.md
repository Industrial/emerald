2026-09-21T21:03:00Z

---
name: Unicode Normalization & Segmentation — `unicode-normalization` + `unicode-segmentation` for Real Character Counting
overview: "`.nfc`/`.nfd`/`.nfkc`/`.nfkd` (canonical/compatibility (de)composition, via `unicode-normalization` 0.1.25) plus `.codepoint_count`/`.grapheme_count`/`.graphemes`/`.word_count`/`.words`/`.sentence_count`/`.sentences` (real Unicode Annex #29 boundary segmentation, via `unicode-segmentation` 1.13.3 — the #3-ranked crate in crates.io's own Text-processing category, 44,483 dependent crates) — closing the gap plan 45's `.length` leaves wide open: `.length` is a direct `strlen()` call (verified against `emerald_runtime.c` by plan 59), a raw byte count, not a codepoint count and certainly not a user-perceived-character (grapheme cluster) count. This plan adds the two counts plan 45 never had a reason to distinguish, and the two Unicode-real crates that compute them correctly rather than approximately."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-rt-crates-and-utf8-policy
    content: "Add `unicode-normalization = \"0.1.25\"` and `unicode-segmentation = \"1.13.3\"` to `crates/emerald-rt/Cargo.toml`. Every function below reads its `String` receiver via `unsafe { CStr::from_ptr(recv) }.to_bytes()` then `String::from_utf8_lossy(...)` — NOT `to_str().unwrap()` — so that a receiver holding legacy-encoded bytes (plan 153's own disclosed `Encoding.encode` output, or any other source of non-UTF-8 bytes this project's runtime never actually prevents) degrades to U+FFFD replacement characters inside `catch_unwind` rather than panicking the Rust side; this is a real, disclosed policy choice this plan is the first to need and states explicitly, one plan downstream of the gap plan 153 itself named."
    status: pending
  - id: leaf-normalization-forms
    content: "`emerald_rt_string_nfc`/`_nfd`/`_nfkc`/`_nfkd(s: *const c_char) -> *mut c_char` — each one `unicode_normalization::UnicodeNormalization::{nfc,nfd,nfkc,nfkd}(&decoded).collect::<String>()`, copied into an `emerald_alloc` buffer via the plan-153-established callback pattern. Sema/codegen expose `String.nfc(self): String` etc. as four more zero-argument String intrinsics dispatched inside the existing `recv_ty == Type::String` `MethodCall` arm plan 45 built."
    status: pending
  - id: leaf-codepoint-and-grapheme-counts
    content: "`emerald_rt_string_codepoint_count(s) -> i64` (`decoded.chars().count()`, no third-party crate needed for this one — Rust `char` iteration is already codepoint-exact) and `emerald_rt_string_grapheme_count(s) -> i64` (`unicode_segmentation::UnicodeSegmentation::graphemes(&decoded, true).count()` — `true` selects the extended grapheme cluster rules of UAX #29, the definition every mainstream language's \"Character\"/\"user-perceived character\" concept actually uses). Exposed as `String.codepoint_count(self): Int64` / `String.grapheme_count(self): Int64`."
    status: pending
  - id: leaf-array-returning-segmentation
    content: "`String.graphemes(self): Array[String]` / `.words(self): Array[String]` / `.sentences(self): Array[String]` — each backed by an `emerald_rt_string_*` function returning a bare `ptr_ty` to a freshly built, contiguous run of `emerald_alloc`'d `char*` elements (see Decision log for why plan 59's user-facing extern allow-list does not constrain this), paired with `.grapheme_split_count`/`.word_split_count`/`.sentence_split_count(self): Int64` companions computed by an independent second pass, mirroring plan 45's own `.split`/`.split_count` two-scan precedent exactly."
    status: pending
  - id: leaf-example-and-rust-tests
    content: "Add `examples/unicode_normalization_segmentation_proof.em` (the Concrete Proof below, containing a literal precomposed \"é\" and a literal \"e\" + COMBINING ACUTE ACCENT (U+0301) sequence) to `emerald-cli/tests/examples.rs`'s checked table. Add `#[test]`s in `emerald-rt` asserting each normalization form and each count against `unicode-normalization`/`unicode-segmentation`'s own documented behavior for the identical fixed strings, plus one asserting `.grapheme_count` on a multi-codepoint emoji ZWJ sequence (e.g. the four-codepoint \"family\" emoji) returns `1`, not the much larger codepoint or byte count."
    status: pending
isProject: false
---

# Plan 154 — Unicode Normalization & Segmentation

State the gap plainly, since AGENTS.md itself records a past false-bug-report
from an unverified claim: plan 59 read `emerald_runtime.c` directly and found
`emerald_string_length` (L331-333) is `return (long long) strlen(s);` —
`.length` (plan 45) is a **byte count**, full stop. For plain ASCII text this
is indistinguishable from a codepoint count or a user-perceived-character
count, which is exactly why the gap is easy to miss until real-world Unicode
text arrives. It stops being invisible the moment a string contains anything
outside ASCII: the same visual character "é" can be one codepoint
(U+00E9, PRECOMPOSED LATIN SMALL LETTER E WITH ACUTE, 2 UTF-8 bytes) or two
codepoints (U+0065 "e" + U+0301 COMBINING ACUTE ACCENT, 3 UTF-8 bytes) —
both are legal, both render identically in a real font, and `.length` reports
2 for one and 3 for the other despite a human reading either string seeing
exactly one letter. Go further — a family emoji ("👨‍👩‍👧‍👦") is *four* base
emoji codepoints joined by three ZERO WIDTH JOINER codepoints, 7 codepoints
and 25 UTF-8 bytes, rendered by any real terminal or UI as one glyph. Neither
"codepoint count" nor "byte count" is "how many characters does a person see
here" — only a real Unicode Annex #29 grapheme-cluster segmentation answers
that question, and nothing in this compiler computes one today. This plan
adds the two crates that do it correctly (`unicode-normalization` for
canonical-equivalence-aware comparison, `unicode-segmentation` for real
grapheme/word/sentence boundaries) as `String` methods that sit *alongside*
`.length`, not in place of it — three genuinely different, all individually
useful numbers, named clearly enough that a caller picks the right one on
purpose instead of by accident.

Both crates are maintained under the `unicode-rs` GitHub organization
(overlapping ownership with `rust-lang`-adjacent maintainers — Manish
Goregaokar, Riad Wahby, Simon Sapin, Huon Wilson appear as owners/co-owners
of both, verified this session against each crate's own `lib.rs` listing),
`no_std`-compatible, and both implement Unicode Standard Annexes directly
(`unicode-normalization`: UAX #15; `unicode-segmentation`: UAX #29) rather
than approximating them — `unicode-segmentation` alone is used by 44,483
crates and ranks #3 in crates.io's Text-processing category, ahead of nearly
every other crate in this entire 101-plan batch's dependency list.

## Concrete proof this plan targets

```ruby
composed: String = "café"
decomposed: String = "cafe´"
puts composed.length
puts decomposed.length
puts composed.codepoint_count
puts decomposed.codepoint_count
puts composed.grapheme_count
puts decomposed.grapheme_count
normalized: String = decomposed.nfc()
puts normalized.length
```

(In the real `.em` source file, `decomposed`'s literal is authored as the
five characters `c`, `a`, `f`, `e`, then a real COMBINING ACUTE ACCENT
(U+0301) placed directly after the `e` — not an escape sequence, a literal
UTF-8 byte sequence in the file, which every existing string-literal
mechanism already handles with zero special casing, per plan 59's own
"`String` needs no byte-level conversion" finding.)

Expected output:
```
5
6
4
5
4
4
5
```

`composed` ("café" with precomposed é, U+00E9) is 4 codepoints / 5 UTF-8
bytes. `decomposed` ("cafe" + combining acute) is 5 codepoints / 6 UTF-8
bytes — a real, `.length`-visible byte-count difference between two strings
that look identical. Both have `grapheme_count == 4`: the combining acute
accent attaches to the preceding "e" as a single extended grapheme cluster
(UAX #29's whole reason to exist), so both strings really do contain four
user-perceived characters, exactly matching what a person reading either one
would say. `decomposed.nfc()` (canonical composition) recombines "e" +
U+0301 into precomposed "é", making its `.length` collapse to `5` — now
byte-identical to `composed`, proving normalization is what actually
resolves the ambiguity `.length`/`.codepoint_count` alone cannot.

## Decision log

- **Three counts, three names, no default — `.length` keeps meaning
  "bytes" exactly as plan 45 already shipped it; this plan adds
  `.codepoint_count` and `.grapheme_count` beside it rather than
  redefining or deprecating anything.** Silently changing `.length`'s
  existing meaning would be a breaking change to plan 45's own shipped
  contract for no compiler-detectable reason (every caller of `.length`
  today keeps compiling and keeps getting the answer it already got);
  adding two new, precisely-named methods is strictly additive and lets a
  caller choose deliberately. `.codepoint_count` needs no third-party
  crate at all — `s.chars().count()` is already exact, since Rust's own
  `char` type *is* a Unicode scalar value — a detail worth stating since
  it means half of this plan's counting surface is essentially free,
  and the real crate dependency buys exactly the grapheme-cluster half.
- **`.graphemes`/`.words`/`.sentences` return `Array[String]` by reusing
  the compiler's own internal runtime-call path, not the user-facing
  `unsafe extern \"C\"` mechanism — so plan 59's real, verified
  `{Int64, Float64, String, CString, Void}` extern-signature allow-list
  does not constrain them.** That allow-list gates sema's check on
  *user-written* `unsafe extern "C" { ... }` declarations specifically
  (plan 59's own Decision log: the allow-list exists because nothing
  narrower than `Int64` exists in the real `Type` enum to marshal into);
  it says nothing about the fixed set of runtime calls `emerald-codegen`
  already declares and calls directly today — several of which,
  `emerald_alloc` foremost, already return a bare `ptr_ty`, not a value
  from that allow-list at all. `Array[T]`'s own documented representation
  (plan 45, quoting `build_array_lit`'s doc comment directly: "no length
  prefix, no bounds checking") is exactly a contiguous run of elements
  behind a bare pointer — for `Array[String]`, a contiguous run of
  `char*`. These intrinsics are dispatched the same hard-coded way plan
  45's own `.split` already is: a new `emerald_rt_string_graphemes`/
  `_words`/`_sentences` function (declared with a bare `ptr_ty` return,
  exactly like `emerald_alloc`) builds a heap buffer of freshly
  `emerald_alloc`'d `char*` elements Rust-side and hands back the
  buffer's address; codegen treats that address as the `Array[String]`
  value directly. The matching `_split_count`-style companion is computed
  by a second, independent pass over the same input, for the identical
  reason plan 45's own Decision log already gives for `.split`/
  `.split_count`: "every runtime call in `emerald_runtime.c` returns
  exactly one value... inventing a struct-returning ABI... is strictly
  more machinery than this plan needs" — that reasoning applies to
  `emerald-rt`'s calls exactly as much as it applies to the C runtime's.
- **`.words`/`.word_split_count` are a real, disclosed *behavioral*
  upgrade over plan 45's `.split(\" \")`, not just a Unicode-correctness
  one.** `unicode_words()` implements UAX #29 word-boundary rules —
  `"can't"` segments as one word (an internal apostrophe doesn't split
  it), consecutive punctuation and whitespace runs are dropped rather
  than producing empty segments, and word boundaries are computed from
  real Unicode word-break properties, not literal-substring matching.
  Plan 45's `.split(\" \")` is, by its own Decision log's admission,
  "always literal-substring splitting... it does not collapse runs of
  whitespace" and does not understand contractions or punctuation at
  all. This plan does not replace `.split` — a caller who wants exact
  literal-separator splitting still has it — `.words` is a genuinely
  different operation with a genuinely different, better-for-natural-
  language-text contract, named differently on purpose.
- **Extended grapheme clusters (`graphemes(true)`), not legacy grapheme
  clusters (`graphemes(false)`), are the only mode this plan exposes.**
  `unicode-segmentation`'s own API distinguishes "extended" (the modern
  UAX #29 default nearly every consumer wants — matches how Swift's
  `Character`, for instance, behaves) from "legacy" grapheme clusters (an
  older, narrower rule set kept for backward compatibility with software
  predating Unicode 9's tailoring). This plan hard-codes `true` rather
  than exposing a second boolean-flag'd method pair, for the same reason
  plan 153 gives for not exposing a lossy/strict flag on one function:
  `Boolean` isn't in the extern-signature allow-list for user-facing
  externs, and here there is no real caller demand for the legacy variant
  to justify a second full intrinsic anyway.
- **`.nfc`/`.nfd`/`.nfkc`/`.nfkd` operate on the receiver only — this
  plan does not add a `String.==` override or any comparison intrinsic
  that normalizes implicitly before comparing.** Two canonically-
  equivalent strings that differ only in normalization form are, and
  remain, different byte sequences and (whatever the real comparison
  semantics turn out to be for `Type::String` — not independently
  re-verified in this plan) are not silently treated as equal by this
  plan. A caller who wants canonical-equivalence comparison normalizes
  both sides explicitly (`a.nfc() == b.nfc()`) — an honest, visible cost
  at the call site, not a hidden behavior change to how `==` already
  works for every other `String` value in every other program.
- **UTF-8 validity of the receiver is assumed best-effort, not
  guaranteed, and this plan's `from_utf8_lossy` policy is the first
  place that distinction actually matters operationally.** Plan 153
  disclosed that `Encoding.encode`'s output is a `String`-typed value
  that is deliberately *not* valid UTF-8, and warned callers never to
  call a Unicode-aware method on it. This plan is that warning's
  enforcement mechanism in practice: every function here decodes with
  `String::from_utf8_lossy`, so a legacy-encoded buffer accidentally
  routed through `.nfc()`/`.grapheme_count`/etc. does not crash the
  process (`catch_unwind` never even needs to fire) — it silently
  produces U+FFFD-laden nonsense instead, a degraded-but-safe outcome
  consistent with this project's "no ordinary bug should crash the
  process" identity (plan 59's own framing), at the cost of a wrong
  answer being less loud than a crash would be. Stated directly so a
  future reader doesn't mistake silent U+FFFD corruption for a bug in
  this plan rather than a caller violating plan 153's own disclosed
  contract.
- **Out of scope.** Unicode collation / locale-aware sort ordering (a
  much larger problem — needs CLDR tailoring data, not just UAX #15/#29
  — `unic-locale`/ICU-style crates territory, a distinct future plan),
  case folding beyond plan 45's existing byte-wise ASCII `.upcase`/
  `.downcase` (full Unicode case conversion needs its own crate,
  `unicode-case-mapping` or similar, and its own plan — this plan only
  touches normalization and segmentation), bidirectional text algorithm
  support, and any change to how string *literals* are lexed or stored —
  this plan is purely new `String` methods operating on values that
  already exist by the time any of them run.
