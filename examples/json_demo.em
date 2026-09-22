# Plan 118 (JSON) — `serde_json` wrapped behind a compiler-synthesized
# `JsonValue` tagged-union enum. `Json.parse` returns a real
# `Result[JsonValue, String]`; `JsonValue.get`/`.to_s` are two more
# compiler intrinsics; the rest is ordinary `match`/pattern matching,
# the same mechanism this language already uses for `Option[T]`.
#
# Several real, disclosed corrections to this plan's own original
# sketch, found only by running it: (1) this grammar has no `when`/
# `else`/`case` keywords — `match <scrutinee> do <Variant>(<binds>) do
# ... end ... _ do ... end end` is the real, current syntax (plan 71).
# (2) `T?` nullable sugar was removed (plan 73) — an `Option[T]`-typed
# local must be annotated `Option[T]` directly, never `T?`. (3) Every
# JSON number becomes `Float64` (Emerald has no unified numeric type) —
# `36` round-trips as `36.0`, not `36`.
input: String = "{\"name\": \"Ada\", \"age\": 36, \"active\": true, \"tags\": [\"math\", \"cs\"]}"

parsed: Result[JsonValue, String] = Json.parse(input)
match parsed do
Ok(doc) do
  name: Option[JsonValue] = doc.get("name")
  match name do
  Some(v) do
    match v do
    JsonString(s) do
      puts s
    end
    _ do
      puts "missing"
    end
    end
  end
  None do
    puts "missing"
  end
  end

  nickname: Option[JsonValue] = doc.get("nickname")
  match nickname do
  Some(v) do
    match v do
    JsonString(s) do
      puts s
    end
    _ do
      puts "missing"
    end
    end
  end
  None do
    puts "missing"
  end
  end

  tags: Option[JsonValue] = doc.get("tags")
  match tags do
  Some(v) do
    match v do
    JsonArray(items) do
      puts items.count
    end
    _ do
      puts "missing"
    end
    end
  end
  None do
    puts "missing"
  end
  end

  puts doc.to_s
end
Err(msg) do
  puts msg
end
end

# A second, real negative proof: invalid JSON reaches the `Err` arm
# with `serde_json::Error`'s own real, non-empty parser error text —
# never a crash, never a silently-empty string.
bad: String = "{not json"
bad_parsed: Result[JsonValue, String] = Json.parse(bad)
match bad_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(msg) do
  puts msg
end
end
