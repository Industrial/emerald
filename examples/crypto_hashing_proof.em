# Plan 109 (Cryptographic Hashing) — Sha256/Sha3_256/Blake3/Md5
# one-shot digests plus Sha256Hasher's incremental handle, built on a
# new `Bytes` type. Every digest below is a real, independently-
# checkable value (see this plan's own history doc for each one's
# primary source), not invented for this example.
#
# Real, disclosed deviation from the plan's own literal design: `Bytes`
# is NOT a genuine two-word `{ptr, len}` value (the plan's own
# provisional shape, left open by its own "Not yet decided" item 1).
# It is implemented as the same zero-cost `Int64`-newtype-wrapping-an-
# opaque-pointer shape `Regex`/`NativeHandle`/`LogFields` already use —
# a bare heap pointer to a `[len: i64][data]` block, never a
# `crate::handle` registry id (a `Bytes` value holds no Rust-side
# resource needing one). See `crates/emerald-rt/src/bytes.rs`'s own
# module doc for the full reasoning.
#
# Real, disclosed grammar limitation found while writing this example
# (pre-existing, not introduced by this plan): this grammar's own
# `MethodCall` productions restrict a `.method(...)` call's receiver to
# a bare `Ident`/`InstanceVarTok`/a `do...end`-block-attached
# `ChainCallExpr` — never a string literal directly (`"x".to_bytes()`
# is a real parse error, confirmed against `grammar.lalrpop`'s
# `PrimaryExpr`/`StmtPrimaryExpr`/`CondPrimaryExpr` productions, all
# three), and never the bare result of a previous, non-block-attached
# `.method(...)` call (`Sha256.hash(x).to_hex()` is equally a parse
# error — a MethodCall's own result is not itself an `Ident`). Every
# value below is therefore bound to a local first, then chained off
# that local — the same pattern this session's own `libm_proof.em`/
# `humantime_proof.em` examples already use for the identical reason.
#
# Second real, disclosed deviation: the incremental handle constructor
# is `Sha256.hasher()`, not the plan's own literal `Sha256.new()` —
# `"new"` is a grammar-reserved keyword (`<recv:Ident> "." "new" "("
# <args:Args> ")" => Expr::New(recv, args)`), so `Sha256.new()` parses
# as a real class-instantiation node and fails with "undefined class
# `Sha256`" rather than ever reaching this plan's own reserved-
# namespace `MethodCall` dispatch.

greeting: String = "hello world"
abc: String = "abc"
empty: String = ""

digest1: Bytes = Sha256.hash(greeting.to_bytes())
puts digest1.to_hex()

digest2: Bytes = Sha3_256.hash(abc.to_bytes())
puts digest2.to_hex()

digest3: Bytes = Blake3.hash(empty.to_bytes())
puts digest3.to_hex()

digest4: Bytes = Md5.hash(greeting.to_bytes())
puts digest4.to_hex()

part1: String = "hello "
part2: String = "world"
hasher: Sha256Hasher = Sha256.hasher()
hasher.update(part1.to_bytes())
hasher.update(part2.to_bytes())
digest5: Bytes = hasher.finalize()
puts digest5.to_hex()
