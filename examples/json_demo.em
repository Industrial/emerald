# Plan 118 (JSON) — `serde_json` wrapped behind a compiler-synthesized
# `JsonValue` tagged-union enum. `Json.parse` returns a real
# `Result[JsonValue, JsonError]`; `JsonValue.get`/`.to_s` are two more
# compiler intrinsics; the rest is ordinary `match`/pattern matching,
# the same mechanism this language already uses for `Option[T]`.
#
# Plan 195 (Typed Domain Errors) retrofit, disclosed breaking change:
# `Json.parse`'s original, plan-118-shipped signature returned
# `Result[JsonValue, String]` — a caller could pattern-match `Ok`/`Err`
# but never branch on *what kind* of failure occurred short of parsing
# the message string. It now returns `Result[JsonValue, JsonError]`,
# `JsonError = Syntax(String) | UnexpectedEnd | Other(String)` — a real
# compiler-synthesized enum (never declared in this file, exactly like
# `JsonValue` itself), classified from `serde_json::Error::classify()`.
# Any `.em` code still matching this plan's original `Err(msg) do ...
# end` (a bare `String`) no longer type-checks — this file's own `Err`
# arms below are the disclosed migration.
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

parsed: Result[JsonValue, JsonError] = Json.parse(input)
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
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A second, real negative proof: invalid JSON reaches the `Err` arm as
# a real, typed `JsonError` — a stray trailing comma is a genuine
# syntax error, not a truncation, so this always lands on `Syntax`,
# never `Other` (`serde_json::Error::classify()`'s own real
# `Category::Syntax`).
bad: String = "{\"a\":1,}"
bad_parsed: Result[JsonValue, JsonError] = Json.parse(bad)
match bad_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A third, real negative proof: input that is valid so far but cut off
# mid-object lands on `JsonError::UnexpectedEnd` specifically — the
# distinction this plan's own retrofit exists to make possible at all
# (`serde_json::Error::classify()`'s own real `Category::Eof`).
truncated: String = "{\"a\":"
truncated_parsed: Result[JsonValue, JsonError] = Json.parse(truncated)
match truncated_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  UnexpectedEnd do
    puts "unexpected end of input"
  end
  Other(detail) do
    puts detail
  end
  end
end
end
