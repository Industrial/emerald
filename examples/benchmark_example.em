benchmark "traversing a 20-element array one million times" do
  # The exact accumulation shape `benchmarks/array_traversal/
  # array_traversal.em` uses. Measured against this harness (verified
  # this session): it reports a near-instant elapsed time, the same
  # "sum benchmark measures compile-time constant folding, not loop
  # execution" pitfall `benchmarks/REPORT.md`'s own Headline §3 already
  # discloses for a pure-arithmetic accumulation over compile-time-known
  # literal data — LLVM's `default<O3>` pipeline can reduce this whole
  # loop nest to its closed-form final value even though it reads real
  # array elements, since every input (the literal array, the loop
  # bounds) is compile-time-known. Kept here anyway, disclosed rather
  # than hidden or swapped for a friendlier number: it demonstrates
  # that `benchmark`'s timing mechanism reports whatever LLVM actually
  # executes, honestly, even when that's less work than the source text
  # suggests — see the next `benchmark` block below for a workload
  # (heap allocation) LLVM cannot fold away, which measures real,
  # substantial elapsed time.
  arr: Array[Int64] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
  total: Int64 = 0
  rep: Int64 = 0
  while rep < 1000000 do
    i: Int64 = 0
    while i < 20 do
      total: Int64 = total + arr[i]
      i: Int64 = i + 1
    end
    rep: Int64 = rep + 1
  end
  puts total
end

benchmark "allocating and summing a 20-element array one million times" do
  # Real, measured, non-instant elapsed time (verified this session:
  # tens of milliseconds, not microseconds) — a fresh heap allocation
  # (`Array` literal construction calls the runtime's `emerald_alloc`)
  # on every one of the 1,000,000 iterations is a genuine side-
  # effecting runtime call LLVM cannot fold into a compile-time
  # constant, unlike the previous `benchmark` block above.
  total: Int64 = 0
  rep: Int64 = 0
  while rep < 1000000 do
    arr: Array[Int64] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
    total: Int64 = total + arr[0]
    rep: Int64 = rep + 1
  end
  puts total
end
