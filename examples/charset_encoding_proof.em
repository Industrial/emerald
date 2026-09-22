# Plan 153 (Character Set / Encoding Conversion) — `Encoding.decode`/
# `.decode_strict`/`.encode`, wrapping `encoding_rs`.
#
# Real, disclosed correction against this plan's own Concrete Proof
# text: it wrote `String?` and `bogus ||= "..."` — that nullable sugar
# and the `||=` operator were both removed outright (plan 73); the
# real, current annotation is `Option[String]`, and `??` (`Expr::
# Coalesce`) is the real, current nil-coalescing operator, applied
# directly at the `puts` call site rather than via a separate
# reassignment statement.

text: String = "café"
latin1: String = Encoding.encode(text, "windows-1252")
roundtrip: String = Encoding.decode(latin1, "windows-1252")
puts roundtrip

bogus: Option[String] = Encoding.decode_strict(latin1, "utf-8")
puts bogus ?? "not valid utf-8"

good: Option[String] = Encoding.decode_strict(latin1, "windows-1252")
puts good ?? "should not happen"
