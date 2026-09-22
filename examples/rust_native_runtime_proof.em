# Plan 91 (Rust-native runtime crate) — proves the whole pipe, not the
# algorithm: a plain `cargo build` from a clean checkout produces a
# working `emerald-cli` that links TWO static archives (the existing
# `runtime/emerald_runtime.c` and this plan's new `crates/emerald-rt`,
# a Rust `staticlib`) into one program, with no manual pre-build step.
# `.fnv1a_hash` is deliberately zero-dependency (public-domain,
# hand-checkable) so the proof is about the build/link mechanism, not
# about trusting any algorithm's correctness.
# A parenless dotted method call requires an `Ident` receiver (grammar's
# own `<recv:Ident> "." <method:CallMethodName>` shape) — a string
# LITERAL has no `.method` form directly, so each value is bound to a
# local first. A real, disclosed correction to this plan's own original
# "Concrete Proof" sketch (`puts "hello".fnv1a_hash`), found only by
# actually running it, not assumed from the plan text.
# Plan 92 (FFI/ABI conventions) extends this same example, per its own
# `leaf-example-and-gate` — the renamed `.fnv1a_hash`, `.fnv1a_hash_
# checked`'s Ok/Err paths, and a real panic caught as `NativeError`.
# Two more real, disclosed corrections to the plan's own sketch, found
# only by running it: (1) a `Result[T, E]` match scrutinee must be a
# plain local, not an inline method-call expression, so each result is
# bound to a `Let` first; (2) the actual grammar production is `match
# <scrutinee> do Ok(<var>) do ... end Err(<var>) do ... end end`, not
# the plan text's own `case`/`when` sketch.
a: String = "hello"
b: String = "hello world"
c: String = ""
puts a.fnv1a_hash
puts b.fnv1a_hash
puts c.fnv1a_hash

checked_ok: Result[Int64, String] = a.fnv1a_hash_checked
match checked_ok do
Ok(v) do
  puts v
end
Err(e) do
  puts e
end
end

checked_err: Result[Int64, String] = c.fnv1a_hash_checked
match checked_err do
Ok(v) do
  puts v
end
Err(e) do
  puts e
end
end

begin
  a.fnv1a_hash_panic_for_test
rescue NativeError => e
  puts e.message
end
