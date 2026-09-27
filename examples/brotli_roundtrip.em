# Plan 135 (Brotli Compression) — `Brotli.compress`/`.decompress`,
# wrapping `brotli`, round-tripping a real, repetitive input at both a
# fast (1) and the slowest/best (11) real brotli quality level.
#
# Real, disclosed adaptations, reused verbatim from plan 130's own
# `gzip_roundtrip.em`/plan 131's own `zstd_roundtrip.em`/plan 134's own
# `lz4_roundtrip.em`:
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
#    is used directly for the quality-vs-size check, the same posture
#    plan 134's own `lz4_roundtrip.em` already takes since this plan
#    also executes after `Bytes.length` had already landed.
# 3. `puts` accepts only `Int64`/`Float64`/`String` — every `<=`/`==`
#    comparison below is a `Boolean`, so it needs string interpolation
#    (`"#{...}"`) to print, not a bare `puts`.

original_str: String = "the quick brown fox jumps over the lazy dog, the quick brown fox jumps over the lazy dog"
original: Bytes = original_str.to_bytes()
original_hex: String = original.to_hex()

fast: Bytes = Brotli.compress(original, 1)
fast_restored: Bytes = Brotli.decompress(fast)
fast_restored_hex: String = fast_restored.to_hex()
puts "#{fast_restored_hex == original_hex}"

best: Bytes = Brotli.compress(original, 11)
best_restored: Bytes = Brotli.decompress(best)
best_restored_hex: String = best_restored.to_hex()
puts "#{best_restored_hex == original_hex}"

puts "#{best.length <= fast.length}"
