# Plan 123 (Base64 & Hex Encoding) — `Base64.encode`/`.decode` and its
# four-variant-pair siblings, `Hex.encode`/`.encode_upper`/`.decode`,
# wrapping the `base64`/`hex` crates directly from `crates/emerald-rt`.
#
# Real, disclosed scope correction, found only by checking the actual
# compiler source before writing this: the plan's own original text
# specified a `Bytes` (ptr, len) type on both sides of every function
# here — verified directly against `emerald-sema`'s own `Type` enum
# (and plan 92's own "not yet decided" section, which names this exact
# gap) that no `Bytes` type exists anywhere in this compiler. Every
# function here operates on `String` instead: real and useful for the
# base64url/JWT and hex/digest-display shapes this plan's own Decision
# log names (printable text, not arbitrary embedded-NUL binary), but
# narrower than originally scoped — round-tripping genuinely arbitrary
# binary data is deferred until a real `Bytes` type lands.
#
# Second, real correction: this grammar has no `case`/`when`/`else`
# keywords (removed per plan 71) — `match X do Ok(v) do ... end
# Err(e) do ... end end` is the real, current syntax for a
# `Result[T, E]` scrutinee.

raw: String = "hello world"

encoded: String = Base64.encode(raw)
puts encoded

decoded: Result[String, String] = Base64.decode(encoded)
match decoded do
Ok(s) do
  puts s
end
Err(msg) do
  puts msg
end
end

url_form: String = Base64.encode_url_safe(raw)
puts url_form

hex_form: String = Hex.encode(raw)
puts hex_form

hex_back: Result[String, String] = Hex.decode(hex_form)
match hex_back do
Ok(s) do
  puts s
end
Err(msg) do
  puts msg
end
end

# A second, real negative proof: malformed input reaches Err with a
# real, non-empty library error message, never a crash.
bad_decoded: Result[String, String] = Base64.decode("not valid base64!!!")
match bad_decoded do
Ok(s) do
  puts "unexpected ok"
end
Err(msg) do
  puts msg
end
end
