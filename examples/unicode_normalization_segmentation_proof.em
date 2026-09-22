# Plan 154 (Unicode Normalization & Segmentation) -- `String.nfc`/
# `.nfd`/`.nfkc`/`.nfkd`, wrapping `unicode-normalization`; `.codepoint_
# count`/`.grapheme_count`, wrapping `unicode-segmentation`.
#
# `composed` is "café" with a single precomposed U+00E9
# (LATIN SMALL LETTER E WITH ACUTE) -- 4 codepoints, 5 UTF-8 bytes.
# `decomposed` is the letters "cafe" followed by a literal COMBINING
# ACUTE ACCENT (U+0301) -- 5 codepoints, 6 UTF-8 bytes. Both render
# identically to a human reader and both contain exactly 4 user-
# perceived characters (the combining accent attaches to the preceding
# "e" as one extended grapheme cluster, UAX #29's whole reason to
# exist) -- `.length` (a byte count, unaffected by this plan) and
# `.codepoint_count` both disagree between the two strings for reasons
# that have nothing to do with how many characters are actually there;
# `.grapheme_count` is the one number that agrees with what a person
# reading either string would say. `decomposed.nfc` (canonical
# composition) recombines "e" + U+0301 into precomposed "café",
# making its `.length` collapse to `5` -- byte-identical to `composed`
# -- proving normalization is what actually resolves the ambiguity
# `.length`/`.codepoint_count` alone cannot.
#
# Zero-argument method calls are written without parentheses
# throughout (`composed.length`, `decomposed.nfc`), matching this
# codebase's own established convention for `.upcase`/`.strip`/etc.
# elsewhere in `examples/` -- not this plan's own Concrete Proof text,
# which inconsistently wrote a trailing `()` on `.nfc` alone.

composed: String = "café"
decomposed: String = "café"
puts composed.length
puts decomposed.length
puts composed.codepoint_count
puts decomposed.codepoint_count
puts composed.grapheme_count
puts decomposed.grapheme_count
normalized: String = decomposed.nfc
puts normalized.length
