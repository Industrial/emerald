# Plan 189 (CBOR Binary Format) — `Cbor.encode`/`.decode` (`ciborium`
# 0.2.2), reusing plan 118's own `JsonValue` as the shared dynamic-
# value representation directly rather than inventing a second one
# (see `cbor.rs`'s own module doc for the Decision log this plan
# makes: CBOR and JSON solve the identical "represent an arbitrary,
# schema-less tree of scalars/arrays/maps" problem, so Emerald needs
# exactly one answer to it, not two).
#
# Real, disclosed adaptations from this plan's own literal Concrete
# Proof text, found only by checking `JsonValue`'s/`Bytes`'s own real
# surface before writing this example — the identical class of
# correction plan 125's own `binary_serialization.em` already
# discloses, not new to this plan: (1) `Json.parse` returns a real
# `Result[JsonValue, JsonError]`, not a bare `JsonValue` directly (plan
# 195's typed-error retrofit, which predates this plan and landed
# before `Json.stringify` — a name this plan's own text guessed at —
# ever existed; the real, already-shipped counterpart is `JsonValue.
# to_s`, used here instead). (2) `JsonValue`-typed values have no `==`
# operator at all (`binary_serialization.em`'s own already-disclosed
# finding) — this example compares each value's own `.to_s` `String`
# instead. (3) `puts` accepts only `Int64`/`Float64`/`String` — the
# `encoded.length > 0` and `decoded_str == original_str` comparisons
# below are both `Boolean`, so each needs string interpolation
# (`"#{...}"`) to print, not a bare `puts`.

input: String = "{\"name\": \"Ada\", \"scores\": [1, 2, 3], \"active\": true}"

parsed: Result[JsonValue, JsonError] = Json.parse(input)
match parsed do
Ok(original) do
  original_str: String = original.to_s

  encoded: Bytes = Cbor.encode(original)
  puts "#{encoded.length > 0}"

  decode_result: Result[JsonValue, CborError] = Cbor.decode(encoded)
  match decode_result do
  Ok(decoded) do
    decoded_str: String = decoded.to_s
    puts decoded_str
    puts "#{decoded_str == original_str}"
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
end
Err(e) do
  puts "unexpected parse error"
end
end
