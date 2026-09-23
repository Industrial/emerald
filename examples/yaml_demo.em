# Plan 120 (YAML) — `saphyr` wrapped behind plan 118's own `JsonValue`
# tagged-union enum, the identical reuse plan 119 (TOML) already
# establishes. `Yaml.parse` returns a real `Result[JsonValue,
# YamlError]` (this plan lands after plan 195's Typed Domain Errors
# convention, so it ships that shape directly rather than `Toml.
# parse`'s older `Result[JsonValue, String]`); `JsonValue.to_yaml` is
# one more compiler intrinsic, total (`String`, never `Result` — every
# `JsonValue`, including `JsonNull()`, has a direct YAML equivalent).
#
# This grammar has no `when`/`else`/`case` keywords — `match
# <scrutinee> do <Variant>(<binds>) do ... end ... _ do ... end end` is
# the real, current syntax (plan 71); an `Option[T]`-typed local must
# be annotated `Option[T]` directly, never `T?` (plan 73 removed that
# sugar); every YAML integer/float, like every JSON number, widens to
# `Float64` (Emerald has no unified numeric type).
input: String = "name: Ada\nlanguages:\n  - math\n  - cs\nactive: true\n"

parsed: Result[JsonValue, YamlError] = Yaml.parse(input)
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

  langs: Option[JsonValue] = doc.get("languages")
  match langs do
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

  # A real proof YAML's own block-style mapping (`name: Ada`) and block
  # sequence (`languages:` followed by `- math`/`- cs`, indentation-
  # delimited, not `[...]`-bracketed) both lower into the identical
  # `JsonObject`/`JsonArray` shape `Json.parse`/`Toml.parse` already
  # produce from their own, textually unrelated syntaxes — and
  # `.to_yaml` serializes that same tree back into real block-style
  # YAML via `saphyr::YamlEmitter`, beginning with its own `---`
  # document-start marker.
  puts doc.to_yaml
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  EmptyDocument do
    puts "empty document"
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A second, real negative proof: `Yaml.parse("key:\n  - a\n  b")` — a
# real YAML indentation error, mixing a sequence item and a scalar at
# the same nesting level under `key:` — reaches `Err` with `saphyr`'s
# own real parser-error text, landing on `Syntax`, never a crash.
bad: String = "key:\n  - a\n  b"
bad_parsed: Result[JsonValue, YamlError] = Yaml.parse(bad)
match bad_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  EmptyDocument do
    puts "empty document"
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A third, real negative proof: an empty YAML document stream — this
# plan's own disclosed v1 scope cut (`Yaml.parse` narrows `saphyr`'s
# real multi-document `Vec<Yaml>` to its first document and errs
# rather than silently returning `JsonNull()` when that `Vec` is
# empty) — lands on `EmptyDocument` specifically, never `Syntax`.
empty_parsed: Result[JsonValue, YamlError] = Yaml.parse("")
match empty_parsed do
Ok(doc) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  EmptyDocument do
    puts "empty document"
  end
  Other(detail) do
    puts detail
  end
  end
end
end
