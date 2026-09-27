# examples/heredoc.em — plan 36 worked example: `<<~IDENT` squiggly
# heredocs. The body is every line between the opener and a line whose
# trimmed text exactly matches the marker; the minimum leading
# whitespace across all body lines is dedented away (so the marker and
# body can be indented to match surrounding code); `#{expr}`
# interpolation inside the body works exactly like an ordinary
# `"...#{expr}..."` string literal (see strings.em).

name: String = "Emerald"

greeting: String = <<~GREETING
  Hello, #{name}!
  Welcome to squiggly heredocs.
GREETING

puts greeting

ragged: String = <<~RAGGED
    first line, indented four spaces
  second line, indented two spaces
      third line, indented six spaces
RAGGED

puts ragged
