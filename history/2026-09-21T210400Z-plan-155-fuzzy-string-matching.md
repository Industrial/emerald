2026-09-21T21:04:00Z

---
name: Fuzzy String Matching — `strsim` for Typo-Tolerant Comparison
overview: "`String.levenshtein_distance`/`.damerau_levenshtein_distance(other): Int64` and `.jaro_similarity`/`.jaro_winkler_similarity`/`.sorensen_dice_similarity(other): Float64` — five pairwise string-distance/similarity intrinsics wrapping `strsim` 0.11.1 (now maintained under the `rapidfuzz` GitHub organization, `unsafe forbidden`, 69.1 million downloads/month, #7 in crates.io's Algorithms category) — for typo-tolerant \"did you mean\" suggestions, near-duplicate record detection, and fuzzy search ranking, entirely as pure scalar `String, String -> Int64 | Float64` functions with zero new types and zero new FFI surface beyond what plan 59 already allow-lists."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-strsim-rt-crate
    content: "Add `strsim = \"0.11.1\"` to `crates/emerald-rt/Cargo.toml`. Both string arguments are read via the plan-154-established `CStr::from_ptr(...).to_bytes()` + `String::from_utf8_lossy(...)` policy before being handed to `strsim`'s own functions, which internally operate over `char`s (Unicode codepoints, via `str::chars()`), not bytes — so distances/similarities computed by this plan are already codepoint-aware, not byte-aware, with no extra work: a real, free benefit of `strsim`'s own implementation choice, not something this plan had to build."
    status: pending
  - id: leaf-distance-functions
    content: "`emerald_rt_string_levenshtein_distance(a, b) -> i64` (`strsim::levenshtein(&a, &b) as i64`) and `emerald_rt_string_damerau_levenshtein_distance(a, b) -> i64` (`strsim::damerau_levenshtein(&a, &b) as i64` — the real, unrestricted Damerau-Levenshtein distance, which counts one adjacent transposition as a single edit, unlike plain Levenshtein). Exposed as `String.levenshtein_distance(self, other: String): Int64` / `.damerau_levenshtein_distance(self, other: String): Int64`, dispatched inside plan 45's existing `recv_ty == Type::String` `MethodCall` arm."
    status: pending
  - id: leaf-similarity-functions
    content: "`emerald_rt_string_jaro_similarity(a, b) -> f64` (`strsim::jaro(&a, &b)`), `emerald_rt_string_jaro_winkler_similarity(a, b) -> f64` (`strsim::jaro_winkler(&a, &b)`), `emerald_rt_string_sorensen_dice_similarity(a, b) -> f64` (`strsim::sorensen_dice(&a, &b)`) — all three already return `f64` in `[0.0, 1.0]` where `1.0` means an exact match, requiring zero scaling/inversion; exposed as `Float64`-returning `String` methods, the same zero-marshaling path plan 59 already established for every existing `Float64` extern."
    status: pending
  - id: leaf-example-and-rust-tests
    content: "Add `examples/fuzzy_string_matching_proof.em` (the Concrete Proof below) to `emerald-cli/tests/examples.rs`'s checked table. Add `#[test]`s in `emerald-rt` asserting each of the five functions against the textbook canonical values used in the proof (`levenshtein(\"kitten\", \"sitting\") == 3`, `jaro(\"MARTHA\", \"MARHTA\")` against Winkler's own 1990 paper's worked example, etc.) directly against `strsim`'s own computed output, not hand-derived numbers."
    status: pending
isProject: false
---

# Plan 155 — Fuzzy String Matching

Exact string comparison (`==`, or plan 45's literal-substring `.split`) is the
wrong tool for two extremely common real-world tasks: telling a user what
they probably meant to type when a flag, command, or identifier is
misspelled, and deciding whether two records ("Jon Smith" / "John Smith",
"ACME Corp." / "ACME Corporation") describe the same real-world entity
closely enough to flag as a likely duplicate. Both need a *distance* or
*similarity* metric between two strings, not a boolean equality check. This
plan adds five of the standard ones — two edit-distance metrics (Levenshtein,
Damerau-Levenshtein) and three similarity-ratio metrics (Jaro, Jaro-Winkler,
Sørensen-Dice) — via `strsim`, a small (43KB, 965 lines), dependency-minimal,
`#![forbid(unsafe_code)]` crate. It began life under Danny Guo's ownership
and now lists Max Bachmann (of the `rapidfuzz` project, a well-known
cross-language fuzzy-matching effort) as a co-owner under the `rapidfuzz`
GitHub organization, verified this session directly against the crate's own
`lib.rs` listing (version 0.11.1, 69.1 million downloads/month, 5,912
dependent crates, #7 in crates.io's own Algorithms category) — an actively
maintained, not abandoned, small utility crate. The same style of typo
suggestion this plan targets — computing a similarity score between a
mistyped identifier and each known-good candidate to print "did you mean
X?" — is a well-established pattern in Rust CLI tooling; `clap`, the
ecosystem's dominant argument parser, has shipped exactly this kind of
suggestion feature internally for years, built on the same class of
algorithm this plan wraps.

## Concrete proof this plan targets

```ruby
a: String = "kitten"
b: String = "sitting"
puts a.levenshtein_distance(b)

x: String = "MARTHA"
y: String = "MARHTA"
puts x.jaro_similarity(y)
puts x.jaro_winkler_similarity(y)

typo: String = "wrold"
correct: String = "world"
puts typo.damerau_levenshtein_distance(correct)

puts "night".sorensen_dice_similarity("nacht")
```

Expected output:
```
3
0.9444444444444445
0.9611111111111111
1
0.25
```

`"kitten"` → `"sitting"` is the standard textbook Levenshtein example (3
edits: substitute k→s, substitute e→i, insert g). `"MARTHA"`/`"MARHTA"` is
Winkler's own original worked example from his Jaro-Winkler paper — Jaro
similarity 0.9444..., Jaro-Winkler (which boosts scores for a shared prefix,
here the 3-character prefix "MAR") 0.9611... `"wrold"`/`"world"` differ by
exactly one adjacent transposition (r↔o) — plain Levenshtein would need 2
edits (two substitutions), but Damerau-Levenshtein counts a transposition as
a single edit, giving `1` — the concrete reason this plan wraps both metrics
rather than only Levenshtein. `"night"`/`"nacht"` share exactly one bigram
("ht") out of four bigrams each, giving a Sørensen-Dice coefficient of
`2×1/(4+4) = 0.25`.

## Decision log

- **This plan is five pure `(String, String) -> scalar` functions and
  nothing else — no aggregate \"find the closest match among these N
  candidates\" convenience method, on purpose.** An aggregate helper would
  need to accept a caller-supplied list of candidates, and the natural
  shape for that in this stdlib is `Array[String]` — but adding a *new*
  native intrinsic to consume one is more compiler surface than this
  plan needs to justify: ordinary, already-shipped Emerald code
  (`while i < n ... if s.jaro_winkler_similarity(candidates[i]) > best
  ... end`, using plan 45's own `while`/indexing) already computes the
  identical result today with zero new language or runtime surface. This
  plan declines to add a redundant native shortcut for something
  existing, already-shipped features already compose to build.
- **`strsim` iterates by Unicode codepoint (`char`), not by byte or by
  grapheme cluster — this plan inherits that choice rather than
  re-implementing distance metrics over grapheme clusters instead.**
  Plan 154 draws a careful three-way distinction between bytes,
  codepoints, and grapheme clusters; `strsim`'s own functions are written
  against `str::chars()`, i.e. codepoints. A combining-character sequence
  therefore counts as multiple edit-distance units even though it is one
  visual grapheme (`.grapheme_count`, plan 154, would say otherwise). This
  is `strsim`'s own, real, upstream behavior — not a marshaling choice
  this plan introduces — and is disclosed here rather than silently
  inherited without comment.
- **`Boolean` case-sensitivity flags are not exposed — every function is
  case-sensitive, full stop.** `strsim` itself has no case-insensitive
  variant of any function; a caller wanting case-insensitive fuzzy
  matching calls `.downcase` (plan 45) on both operands first. Not
  duplicating that transformation inside this plan's five functions
  keeps each one a thin, direct wrapper over exactly one `strsim`
  function, with the exact same "no config knob `strsim` itself doesn't
  offer" posture plan 157 (slug) uses for the same reason.
- **Hamming distance and plain (non-Damerau) \"optimal string
  alignment\" distance — both real functions `strsim` also exports — are
  declined for this first pass, not merged into the five above.** Hamming
  distance requires equal-length inputs (`strsim::hamming` returns a
  `Result`, erroring on length mismatch — a real, different failure shape
  none of this plan's other five functions have, since Levenshtein-family
  and Jaro-family metrics are defined for any two lengths); OSA distance
  is a strict subset of what Damerau-Levenshtein already gives (OSA
  forbids a transposed pair from being edited again afterward — a
  narrower, more specialized metric with real but rarer use cases).
  Neither is wired up here; both are straightforward, low-risk additions
  to fold into a future revision of this same plan rather than reasons to
  delay the five that cover the common cases now.
- **Return types are `Int64` for the two distance metrics and `Float64`
  for the three similarity metrics — both already inside plan 59's real,
  verified extern-signature allow-list, so this plan needs zero new
  marshaling code.** `strsim::levenshtein`/`damerau_levenshtein` return
  `usize`, cast directly to `i64` (no risk of overflow in any realistic
  string-length regime this compiler's own `Array[T]`/`String` values
  could represent); `jaro`/`jaro_winkler`/`sorensen_dice` already return
  `f64` in `[0.0, 1.0]`. This is the simplest possible FFI shape in this
  entire batch of plans — no `emerald_alloc` callback, no pointer
  return, no lossy-UTF-8 output-encoding question, because nothing here
  allocates or returns a new `String` at all.
- **Out of scope.** Aggregate best-match search over `Array[String]`
  (see above — a real, deferred follow-on once/if a native intrinsic
  ever needs to consume caller-supplied arrays directly, not merely
  produce them the way plan 154's `.graphemes` does), phonetic algorithms
  (Soundex, Metaphone, Double Metaphone — a different algorithm family
  entirely, a different crate, a different plan), approximate substring
  search / fuzzy regex (bitap, Myers bit-vector algorithms — belongs
  with a hypothetical future `Regexp` plan, which plan 45's own Decision
  log already named as explicitly out of scope for that plan too), and
  `Hamming`/OSA distance (see above).
