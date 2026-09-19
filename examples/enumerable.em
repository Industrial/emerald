# Plan 70 (enumerable stdlib completion): `map`/`select`/`filter`/
# `reduce`/`inject`/`each_with_index`/`count`/`sum`/`sort` on
# `Array[T]`, plus the K,V-appropriate subset (`map`/`reduce`/
# `each_with_index`/`count`) on `Hash[K,V]` — using this plan's own new
# block-attached-call syntax (`recv.method do |params| ... end`, plan
# 87's exclusive `do...end` spelling; braces are gone), not the
# pre-existing plan 42 workaround of binding a named `Proc` first.
#
# Two real, disclosed narrowings from this plan's own concrete-proof
# sketch, found implementing it against the real grammar/type-checker
# (see `examples/README.md`'s "Real bugs found" section for the full
# writeup):
#   - A block's parameters need an explicit type (`|x: Int64|`, not a
#     bare `|x|`) — this language has no call-site-driven type
#     inference for an unannotated block parameter.
#   - No chaining across the calls below (inherited from plan 42's own
#     original scope: each call's result is still bound to a variable
#     before the next call in this file). Plan 87 does add real chain-
#     local `do...end` chaining at the grammar level (`arr.select do |x|
#     ... end.map do |x| ... end`), but this example predates that plan
#     and isn't rewritten to use it.

nums: Array[Int64] = [1, 2, 3, 4, 5]

doubled: Array[Int64] = nums.map do |x: Int64| x * 2 end
evens: Array[Int64] = nums.select do |x: Int64| x % 2 == 0 end
total: Int64 = nums.reduce(0) do |acc: Int64, x: Int64| acc + x end
above_two: Int64 = nums.count do |x: Int64| x > 2 end
s: Int64 = nums.sum
sorted: Array[Int64] = nums.sort

nums.each_with_index do |x: Int64, i: Int64| puts i end

puts doubled[4]
puts evens.count
puts total
puts above_two
puts s
puts sorted[0]

h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}
h_values: Array[Int64] = h.map do |p: Pair[Int64, Int64]| p.value end
h_total: Int64 = h.reduce(0) do |acc: Int64, p: Pair[Int64, Int64]| acc + p.value end
h_big: Int64 = h.count do |p: Pair[Int64, Int64]| p.value > 15 end

puts h_values[0]
puts h_total
puts h_big
puts h.count
