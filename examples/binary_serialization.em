# Plan 125 (Binary Serialization: bincode/msgpack) — `Bincode.encode`/
# `.decode` (`bincode` 2.0.1, pinned exactly — see `Cargo.toml`'s own
# disclosed "the crate is formally unmaintained" finding),
# `MessagePack.encode`/`.decode` (`rmp-serde`), both operating on plan
# 118's own `JsonValue` as the shared dynamic-value representation.
#
# Real, disclosed adaptations from this plan's own literal Concrete
# Proof text, found only by checking this grammar's/`JsonValue`'s/
# `Bytes`'s own real surface before writing this example (the same
# class of correction plan 109's `crypto_hashing_proof.em`/plan 130's
# `gzip_roundtrip.em` already disclose, not new to this plan):
#
# 1. `JsonValue`-typed values have no `==` operator at all — the
#    compiler's own `Expr::Compare` codegen only handles Int64/
#    Float64/String/Symbol operands, or a real user-declared class's
#    own `==` method (`emerald-codegen`'s own `build_expr`, the
#    `Expr::Compare` arm's final `_ => Err(...)` case). This plan's
#    own Concrete Proof text assumed `v == value` worked directly; the
#    real, provable-today equivalent compares each value's own `.to_s`
#    `String` instead (`JsonValue.to_s` already exists, plan 118) —
#    two structurally-equal `JsonValue` trees always print identically
#    (`serde_json`'s own `preserve_order` feature, already enabled in
#    `Cargo.toml`, keeps object key order stable through the round
#    trip), so this is a faithful stand-in for the plan's own `v ==
#    value`.
# 2. `puts` accepts only `Int64`/`Float64`/`String` (`examples/
#    regex_dates.em`'s own already-disclosed finding, reused verbatim
#    by plan 130's `gzip_roundtrip.em`) — every `==` comparison below
#    is a `Boolean`, so it needs string interpolation (`"#{...}"`) to
#    print, not a bare `puts`. `Bytes.length` (this plan's own real,
#    disclosed addition — no existing `Bytes` method exposed a byte
#    count before this plan; see `bytes.rs`'s own doc comment) returns
#    a plain `Int64`, so `puts wire.length` needs no such wrapping.

value: JsonValue = JsonObject({
  "name" => JsonString("emerald"),
  "version" => JsonNumber(0.1),
  "tags" => JsonArray([JsonString("compiler"), JsonString("rust")]),
})
value_str: String = value.to_s

packed: Bytes = Bincode.encode(value)
restored: Result[JsonValue, BincodeError] = Bincode.decode(packed)
match restored do
Ok(v) do
  v_str: String = v.to_s
  puts "#{v_str == value_str}"
end
Err(e) do
  match e do
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(msg) do
    puts msg
  end
  end
end
end

wire: Bytes = MessagePack.encode(value)
puts wire.length

back: Result[JsonValue, MessagePackError] = MessagePack.decode(wire)
match back do
Ok(v) do
  v_str: String = v.to_s
  puts "#{v_str == value_str}"
end
Err(e) do
  match e do
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(msg) do
    puts msg
  end
  end
end
end

# A second, real negative proof (the same style `toml_demo.em`'s own
# second example already establishes): a real, ordinary `String`'s own
# bytes (`String.to_bytes`, plan 109) are not a valid `Bincode`
# encoding of anything, so decoding them lands on the typed
# `BincodeError` this plan's own `Result[JsonValue, BincodeError]`
# retrofit exists to make possible at all — a real, non-panicking
# `Err`, not a silently-wrong decode.
garbage_str: String = "not a valid bincode encoding at all"
garbage: Bytes = garbage_str.to_bytes()
garbage_result: Result[JsonValue, BincodeError] = Bincode.decode(garbage)
match garbage_result do
Ok(v) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(msg) do
    puts "other error"
  end
  end
end
end
puts packed.length
