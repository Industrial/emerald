# Plan 146 (Environment Variables) — `Env.get`/`.set`/`.remove`/
# `.keys_count`, backed entirely by `std::env` (zero third-party
# crate). Every call is serialized behind one process-wide mutex in
# `crates/emerald-rt` — `std::env::set_var`/`remove_var` were
# reclassified `unsafe fn` in the Rust 2024 edition specifically
# because mutating the process environment concurrently with another
# thread reading it is genuine platform-level UB, and Emerald's own
# actor model runs a real multi-threaded worker pool.
#
# Real, disclosed correction: the plan's own Concrete Proof used `T?`
# (`String?`) for `Env.get`'s return type — that nullable sugar was
# removed (plan 73); the real, current annotation is `Option[String]`.
# Second, real, disclosed correction: `puts` accepts only `Int64`/
# `Float64`/`String` — a bare `Boolean` (`puts n > 0`, the plan's own
# Concrete Proof) is rejected outright; string interpolation
# (`"#{...}"`), which does support `Boolean`, is the real, current way
# to print one.

Env.set("EMERALD_PLAN146_PROOF", "hello")
found: Option[String] = Env.get("EMERALD_PLAN146_PROOF")
match found do
Some(v) do
  puts v
end
None do
  puts "unset"
end
end

n: Int64 = Env.keys_count
has_keys: Boolean = n > 0
puts "#{has_keys}"

Env.remove("EMERALD_PLAN146_PROOF")
gone: Option[String] = Env.get("EMERALD_PLAN146_PROOF")
match gone do
Some(v) do
  puts v
end
None do
  puts "unset"
end
end
