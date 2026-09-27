# Plan 134 (LZ4 Compression) — `Lz4.compress`/`.decompress`, wrapping
# `lz4_flex`'s block format, round-tripping a real, repetitive input
# plus a zero-length edge case (the size-header convention `compress_
# prepend_size`/`decompress_size_prepended` share is exactly the kind
# of mechanism a length-zero input could plausibly break if implemented
# carelessly — this plan's own Rust-side `#[test]` suite in `lz4.rs`
# checks this same case directly against the real crate too).
#
# Real, disclosed adaptations, reused verbatim from plan 130's own
# `gzip_roundtrip.em`/plan 131's own `zstd_roundtrip.em`:
#
# 1. A `.method(...)` call's receiver can never be a string literal
#    directly (`"x".to_bytes()` is a real parse error), and never the
#    bare result of a previous, non-block-attached `.method(...)` call
#    — every intermediate value below is bound to a local first, then
#    chained off that local.
# 2. `Bytes` (plan 109) exposes a real `.length: Int64` (plan 125) but
#    still no content-aware `==` (a bare `==` between two `Bytes`
#    newtype handles compares their own underlying heap POINTERS, not
#    their content) — every equality check below compares each `Bytes`
#    value's own `.to_hex()` `String` instead, while `.length` itself
#    is used directly for the compression-ratio check (unlike plan
#    130's/131's own `gzip_roundtrip.em`/`zstd_roundtrip.em`, which
#    still routed even THAT check through `.to_hex().length`, since
#    this plan executes after `Bytes.length` had already landed).
# 3. `puts` accepts only `Int64`/`Float64`/`String` — every `<`/`==`
#    comparison below is a `Boolean`, so it needs string interpolation
#    (`"#{...}"`) to print, not a bare `puts`.

original_str: String = "the quick brown fox jumps over the lazy dog, the quick brown fox jumps over the lazy dog"
original: Bytes = original_str.to_bytes()
original_hex: String = original.to_hex()

compressed: Bytes = Lz4.compress(original)
puts "#{compressed.length < original.length}"

restored: Bytes = Lz4.decompress(compressed)
restored_hex: String = restored.to_hex()
puts "#{restored_hex == original_hex}"

empty_str: String = ""
empty: Bytes = empty_str.to_bytes()
empty_hex: String = empty.to_hex()
empty_compressed: Bytes = Lz4.compress(empty)
empty_restored: Bytes = Lz4.decompress(empty_compressed)
empty_restored_hex: String = empty_restored.to_hex()
puts "#{empty_restored_hex == empty_hex}"
