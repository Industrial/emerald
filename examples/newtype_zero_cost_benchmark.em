# `newtype` zero-cost proof (`domain-types-and-units`, plan-of-plans row
# 81) — run via `emerald benchmark examples/newtype_zero_cost_benchmark.em`
# (plan 80's mechanism, `examples/benchmark_example.em`'s own established
# idiom). Two structurally IDENTICAL workloads, one over `Array[Float64]`,
# one over `Array[Meters]` (a `newtype Meters: Float64`) — the only
# difference is `Meters.new(...)`/`.value` at the write/read sites, which
# `emerald-codegen` compiles as a pure type-level identity (no allocation,
# no wrapper, no extra instruction). If `newtype` really is zero-cost, both
# benchmarks' elapsed times must be statistically indistinguishable.
#
# Real allocation (`Array.new(1000)`) on every one of 2000 `rep`
# iterations is the same "defeat LLVM's constant-folding" technique
# `examples/benchmark_example.em`'s own second block already uses and
# discloses (`benchmarks/REPORT.md`'s Headline §3: a pure-arithmetic loop
# over compile-time-known data folds to its closed-form result and
# measures near-zero time, which is a mechanism artifact, not a wrong
# number, but not what this benchmark wants to measure either) — a fresh
# heap allocation is a genuine side-effecting runtime call the optimizer
# cannot fold away, so both loops below actually execute all 2,000,000
# element writes and reads.

benchmark "sum over Array[Float64], 1000 elements x 2000 reallocations" do
  total: Float64 = 0.0
  rep: Int64 = 0
  while rep < 2000 do
    arr: Array[Float64] = Array.new(1000)
    i: Int64 = 0
    while i < 1000 do
      arr[i] = 1.0
      i: Int64 = i + 1
    end
    j: Int64 = 0
    while j < 1000 do
      total: Float64 = total + arr[j]
      j: Int64 = j + 1
    end
    rep: Int64 = rep + 1
  end
  puts total
end

newtype Meters: Float64

benchmark "sum over Array[Meters], 1000 elements x 2000 reallocations" do
  total: Float64 = 0.0
  rep: Int64 = 0
  while rep < 2000 do
    arr: Array[Meters] = Array.new(1000)
    i: Int64 = 0
    while i < 1000 do
      arr[i] = Meters.new(1.0)
      i: Int64 = i + 1
    end
    j: Int64 = 0
    while j < 1000 do
      m: Meters = arr[j]
      total: Float64 = total + m.value
      j: Int64 = j + 1
    end
    rep: Int64 = rep + 1
  end
  puts total
end
