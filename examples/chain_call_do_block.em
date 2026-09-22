# Plan 194's leaf 1: `ChainCallExpr` do-block chaining, end to end,
# real compile-and-run proof. Plan 192's own repro for what it called a
# "real, previously-undisclosed grammar bug" used UNTYPED block params
# (`|x|`) — syntax this language has never supported (`Param` requires
# `name: Type`, already documented in `enumerable.em`'s own header
# comment, pre-dating that investigation). Once corrected to typed
# params, the exact same do...end chain parses, type-checks, compiles,
# and runs correctly, exactly as `ChainCallExpr`'s own doc comment
# (`grammar.lalrpop`) always claimed — no grammar change was needed.
# See `history/2026-09-22T224000Z-plan-194-iterable-iterator-protocol.md`
# for the full diagnosis, including a falsified LALR-state-merging
# hypothesis.
#
# `Array[T]` has no `.to_s`/`.join` method (a separate, real,
# pre-existing gap this plan's own leaf 1 is not scoped to close), so
# the result is printed via `.count` then `.each` instead of a single
# bracketed string.

nums: Array[Int64] = [1, 2, 3, 4, 5, 6]
result: Array[Int64] = nums.select do |x: Int64| x % 2 == 0 end.map do |x: Int64| x * 10 end

puts result.count
result.each do |v: Int64| puts v end
