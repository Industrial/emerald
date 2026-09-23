# Plan 130 (Gzip/Deflate/Zlib Compression) — `Gzip`/`Deflate`/`Zlib`
# `.compress`/`.decompress`, wrapping `flate2`.
#
# Real, disclosed adaptations from the plan's own literal Concrete
# Proof text, found only by checking this grammar's/`Bytes`'s own real
# surface before writing this example (the same class of correction
# plan 109's `crypto_hashing_proof.em`/plan 123's `base64_hex_
# encoding.em` already disclose, not new to this plan):
#
# 1. A `.method(...)` call's receiver can never be a string literal
#    directly (`"x".to_bytes()` is a real parse error — confirmed
#    against `grammar.lalrpop`'s `PrimaryExpr`/`StmtPrimaryExpr`/
#    `CondPrimaryExpr` productions), and never the bare result of a
#    previous, non-block-attached `.method(...)` call (`Gzip.compress(
#    x).length` is equally a parse error — a `MethodCall`'s own result
#    is not itself an `Ident`). Every intermediate value below is
#    therefore bound to a local first, then chained off that local.
# 2. `Bytes` (plan 109) exposes exactly one instance method today,
#    `.to_hex(): String` — no `.length` and no content-aware `==` (a
#    bare `==` between two `Bytes` newtype handles would compare their
#    own underlying heap POINTERS, not their content, which would make
#    a correct round trip through two *different* heap allocations
#    print `false`). This plan's own Concrete Proof text assumed a
#    richer `Bytes` surface than actually exists; the real, provable-
#    today equivalent compares each `Bytes` value's own `.to_hex()`
#    `String` instead — `String`'s own `==`/`.length` are real content
#    comparisons/real lengths, and two equal-content `Bytes` values
#    always hex-encode to the same `String`, so this is a faithful
#    stand-in for the plan's own `Bytes.length`/`Bytes == Bytes`
#    (real compression still shows as a shorter hex string; a real
#    round trip still shows as an equal hex string).
# 3. `puts` accepts only `Int64`/`Float64`/`String` (`examples/
#    regex_dates.em`'s own already-disclosed finding, reused verbatim
#    by plan 193's own `set_deque_priority_queue.em`) — every `<`/`==`
#    comparison below is a `Boolean`, so each needs string
#    interpolation (`"#{...}"`) to print, not a bare `puts`.

original: String = "the quick brown fox jumps over the lazy dog, the quick brown fox jumps over the lazy dog"
original_bytes: Bytes = original.to_bytes()
original_hex: String = original_bytes.to_hex()

compressed: Bytes = Gzip.compress(original_bytes)
compressed_hex: String = compressed.to_hex()
puts "#{compressed_hex.length < original_hex.length}"

restored: Bytes = Gzip.decompress(compressed)
restored_hex: String = restored.to_hex()
puts "#{restored_hex == original_hex}"

deflated: Bytes = Deflate.compress(original_bytes)
deflated_restored: Bytes = Deflate.decompress(deflated)
deflated_restored_hex: String = deflated_restored.to_hex()
puts "#{deflated_restored_hex == original_hex}"

zlibbed: Bytes = Zlib.compress(original_bytes)
zlibbed_restored: Bytes = Zlib.decompress(zlibbed)
zlibbed_restored_hex: String = zlibbed_restored.to_hex()
puts "#{zlibbed_restored_hex == original_hex}"
