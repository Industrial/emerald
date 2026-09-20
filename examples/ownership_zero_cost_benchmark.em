# `borrow` zero-cost proof (`deterministic-destruction-codegen`,
# plan-of-plans row 84) — run via `emerald benchmark
# examples/ownership_zero_cost_benchmark.em` (plan 80's mechanism,
# `examples/newtype_zero_cost_benchmark.em`'s own established idiom).
#
# Two structurally IDENTICAL workloads, differing only in whether
# `bump`'s parameter is a plain, unannotated `Int64` or a `borrow
# Int64`. If `borrow` really is zero-cost for the common case — the
# argument is already a plain local variable, so the callee's own
# pointer parameter reuses that local's EXISTING `alloca` with no new
# one of its own (`emerald-codegen`'s `build_borrow_arg_ptr` doc
# comment), and the callee does one load-through-pointer where the
# plain version received its value directly — both benchmarks' elapsed
# times must be statistically indistinguishable.
#
# Real, disclosed scope note: this benchmarks `borrow`, not `borrow
# var`. A `borrow var Int64` parameter's own MUTATION path (real
# pointer + writeback on return, proved correct end-to-end in
# `emerald-codegen`'s own test suite — see `bind_params`/`emit_borrow_
# var_writebacks`) can't be exercised from a real, sema-accepted `.em`
# program today: `emerald-sema`'s plan-72 rule makes every parameter's
# own LOCAL BINDING immutable regardless of its declared type ("no
# `var` slot exists on a parameter" — the same reason `examples/
# ownership.em`'s own header comment gives), so there is no legal
# syntax yet for a callee to actually reassign a `borrow var Int64`
# parameter. This benchmark measures what real `.em` source CAN
# exercise today: `borrow`'s read-side pointer-passing cost.
#
# Real allocation (`Array.new(1)`) on every one of 20,000 outer `rep`
# iterations is the same "defeat LLVM's constant folding" technique
# `examples/newtype_zero_cost_benchmark.em`'s own header comment
# documents and `benchmarks/REPORT.md`'s Headline §3 names directly: a
# pure-arithmetic loop over compile-time-known data folds to its
# closed-form result and measures near-zero time — a mechanism
# artifact, not a wrong number, but not what this benchmark wants to
# measure either (an EARLIER version of this file, with the allocation
# hoisted outside the loop, measured exactly that artifact: both
# variants reported ~0 elapsed seconds for the full 20,000,000-call
# workload, since LLVM proved the array cell was never read until after
# the loop and folded the whole thing to one final store). A fresh heap
# allocation is a genuine side-effecting runtime call the optimizer
# cannot fold away, so the inner 1,000-call loop below actually
# executes all 20,000,000 calls in both versions.

fn bump_baseline(x: Int64): Int64 do
  x + 1
end

benchmark "plain Int64 parameter, 20,000 x 1,000 calls" do
  total: Int64 = 0
  rep: Int64 = 0
  while rep < 20000 do
    acc: Array[Int64] = Array.new(1)
    acc[0] = 0
    v: Int64 = 0
    j: Int64 = 0
    while j < 1000 do
      v: Int64 = bump_baseline(v)
      j: Int64 = j + 1
    end
    acc[0] = v
    total: Int64 = total + acc[0]
    rep: Int64 = rep + 1
  end
  puts total
end

fn bump_borrow(x: borrow Int64): Int64 do
  x + 1
end

benchmark "borrow Int64 parameter, 20,000 x 1,000 calls" do
  total: Int64 = 0
  rep: Int64 = 0
  while rep < 20000 do
    acc: Array[Int64] = Array.new(1)
    acc[0] = 0
    v: Int64 = 0
    j: Int64 = 0
    while j < 1000 do
      v: Int64 = bump_borrow(v)
      j: Int64 = j + 1
    end
    acc[0] = v
    total: Int64 = total + acc[0]
    rep: Int64 = rep + 1
  end
  puts total
end
