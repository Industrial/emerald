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
a: String = "hello"
b: String = "hello world"
c: String = ""
puts a.fnv1a_hash
puts b.fnv1a_hash
puts c.fnv1a_hash
