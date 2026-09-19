# Plan 70 (enumerable stdlib completion): `map`/`select`/`filter`/
# `reduce`/`inject`/`each_with_index`/`count`/`sum`/`sort` on
# `Array[T]`, plus the K,V-appropriate subset (`map`/`reduce`/
# `each_with_index`/`count`) on `Hash[K,V]` — using this plan's own new
# block-attached-call syntax (`recv.method { |params| body }`), not the
# pre-existing plan 42 workaround of binding a named `Proc` first.
#
# Two real, disclosed narrowings from this plan's own concrete-proof
# sketch, found implementing it against the real grammar/type-checker
# (see `examples/README.md`'s "Real bugs found" section for the full
# writeup):
#   - A block's parameters need an explicit type (`|x: Int64|`, not a
#     bare `|x|`) — this language has no call-site-driven type
#     inference for an unannotated block parameter.
#   - No chaining (inherited from plan 42's own original scope,
#     restated because it's easy to assume otherwise once real `map`/
#     `select` exist): each call's result is bound to a variable before
#     the next call, never `arr.select { }.map { }` in one expression.

nums: Array[Int64] = [1, 2, 3, 4, 5]

doubled: Array[Int64] = nums.map { |x: Int64| x * 2 }
evens: Array[Int64] = nums.select { |x: Int64| x % 2 == 0 }
total: Int64 = nums.reduce(0) { |acc: Int64, x: Int64| acc + x }
above_two: Int64 = nums.count { |x: Int64| x > 2 }
s: Int64 = nums.sum
sorted: Array[Int64] = nums.sort

nums.each_with_index { |x: Int64, i: Int64| puts i }

puts doubled[4]
puts evens.count
puts total
puts above_two
puts s
puts sorted[0]

h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}
h_values: Array[Int64] = h.map { |p: Pair[Int64, Int64]| p.value }
h_total: Int64 = h.reduce(0) { |acc: Int64, p: Pair[Int64, Int64]| acc + p.value }
h_big: Int64 = h.count { |p: Pair[Int64, Int64]| p.value > 15 }

puts h_values[0]
puts h_total
puts h_big
puts h.count
