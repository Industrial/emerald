# Plan 131 (Zstandard Compression) — `Zstd.compress`/`.decompress`,
# wrapping `zstd`.
#
# Real, disclosed adaptations from the plan's own literal Concrete
# Proof text — the identical class of correction plan 130's own
# `gzip_roundtrip.em` already discloses, reused verbatim here rather
# than rediscovered:
#
# 1. A `.method(...)` call's receiver can never be a string literal
#    directly (`"x".to_bytes()` is a real parse error), and never the
#    bare result of a previous, non-block-attached `.method(...)` call.
#    Every intermediate value below is therefore bound to a local
#    first, then chained off that local.
# 2. `Bytes` (plan 109) exposes exactly one instance method today,
#    `.to_hex(): String` — no `.length` and no content-aware `==` (a
#    bare `==` between two `Bytes` newtype handles compares their own
#    underlying heap POINTERS, not their content). Every comparison
#    below therefore compares each `Bytes` value's own `.to_hex()`
#    `String` instead — a faithful stand-in for the plan's own
#    `Bytes.length`/`Bytes == Bytes` (real compression still shows as a
#    shorter hex string; a real round trip still shows as an equal hex
#    string; a higher level's hex string is still never longer than a
#    lower level's).
# 3. `puts` accepts only `Int64`/`Float64`/`String` — every `<`/`==`
#    comparison below is a `Boolean`, so each needs string
#    interpolation (`"#{...}"`) to print, not a bare `puts`.

original: String = "the quick brown fox jumps over the lazy dog, the quick brown fox jumps over the lazy dog"
original_bytes: Bytes = original.to_bytes()
original_hex: String = original_bytes.to_hex()

compressed: Bytes = Zstd.compress(original_bytes, 3)
compressed_hex: String = compressed.to_hex()
puts "#{compressed_hex.length < original_hex.length}"

restored: Bytes = Zstd.decompress(compressed)
restored_hex: String = restored.to_hex()
puts "#{restored_hex == original_hex}"

high: Bytes = Zstd.compress(original_bytes, 19)
high_hex: String = high.to_hex()
high_restored: Bytes = Zstd.decompress(high)
high_restored_hex: String = high_restored.to_hex()
puts "#{high_restored_hex == original_hex}"

puts "#{high_hex.length <= compressed_hex.length}"
