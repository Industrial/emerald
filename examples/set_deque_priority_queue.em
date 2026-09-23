# Plan 193 (`Set[T]`, `Deque[T]`, `PriorityQueue[T]`) — inception-3
# §4.3's own "missing collection primitives" gap, closed. All three
# route through emerald-rt's plan-93 resource-handle registry, backed
# directly by Rust's own `std::collections` (`HashSet`, `VecDeque`,
# `BinaryHeap`) — see this plan's own Decision log for the full
# account of why that differs from `Array[T]`/`Hash[K,V]`'s own
# hand-rolled LLVM memory representation.
#
# `Set[T]`/`PriorityQueue[T]` are `Int64`/`String` only; `Deque[T]`
# covers all four of `derive Serializable`'s own primitive types
# (`Int64`/`Float64`/`String`/`Boolean`) — a real, disclosed narrow v1
# scope, not an oversight (Decision log).
#
# Two real, disclosed grammar corrections found while writing this:
# (1) the plan's own prose Concrete Proof writes bare `Set.new`, but
# this grammar requires explicit `()` on every zero-argument `.new`
# call — `examples/resource_handle_lifetime_proof.em`'s own header
# comment already discloses this exact rule. (2) `puts` accepts only
# `Int64`/`Float64`/`String` (`examples/regex_dates.em`'s own already-
# disclosed finding) — `s.contains(20)`'s `Boolean` result needs
# string interpolation (`"#{...}"`) to print, not a bare `puts`.
# Neither changes this example's own expected output.

s: Set[Int64] = Set.new()
s.add(10)
s.add(20)
s.add(10)
puts s.count
puts "#{s.contains(20)}"

d: Deque[String] = Deque.new()
d.push_back("a")
d.push_front("z")
puts d.pop_front()

pq: PriorityQueue[Int64] = PriorityQueue.new()
pq.push(5)
pq.push(1)
pq.push(9)
puts pq.pop()
