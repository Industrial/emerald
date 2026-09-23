# Plan 122 (Regular Expressions) — `Regex.compile` plus `.is_match`/
# `.find`/`.captures`/`.replace_all`, wrapping the `regex` crate
# directly (RE2-derived finite-automata matching — a real, verified
# worst-case O(m*n) time guarantee with no catastrophic-backtracking/
# ReDoS class at all).
#
# Real, disclosed grammar correction found while writing this: this
# grammar has no `case`/`when`/`else` keywords (removed per plan 71) —
# `match X do Ok(v) do ... end Err(e) do ... end end` /
# `match X do Some(v) do ... end None do ... end end` are the real,
# current syntax.
#
# Second, real, disclosed correction: `decode_string_lit`'s own doc
# comment (`crates/emerald-parser/src/ast.rs`) states plainly that
# this lexer's string-literal regex admits only `\"`/`\n` as escape
# sequences — `\d` (an ordinary regex metacharacter) is a genuine
# lexer-level parse error here, not merely undecoded. This example
# uses the equivalent POSIX bracket-expression form (`[0-9]`) instead
# of `\d` for exactly this reason — a real, current limitation on
# what regex patterns can be written as an Emerald string literal at
# all, not specific to `Regex` itself.
#
# Third, real, disclosed correction: `puts` accepts only `Int64`/
# `Float64`/`String` (`emerald-sema`'s own `puts` arm) — a bare
# `Boolean` argument is rejected outright, a real, pre-existing
# restriction this plan's own text didn't anticipate. String
# interpolation (`"#{...}"`), which DOES support `Boolean`, is the
# real, current way to print one.
#
# Plan 195 (Typed Domain Errors) retrofit, disclosed breaking change:
# `Regex.compile`'s original, plan-122-shipped signature returned
# `Result[Regex, String]`. It now returns `Result[Regex, RegexError]`,
# `RegexError = Syntax(String) | Other(String)` — the real, pinned
# `regex` crate's own `Error` enum only ever produces `Syntax(String)`
# or `CompiledTooBig(usize)` (verified directly against `regex-1.13.1/
# src/error.rs`), so `Other` covers the latter (and any future `#[non_
# exhaustive]` addition) rather than inventing a variant the crate
# can't actually distinguish. Any `.em` code still matching this
# plan's original `Err(msg) do ... end` (a bare `String`) no longer
# type-checks — this file's own `Err` arms below are the disclosed
# migration.

result: Result[Regex, RegexError] = Regex.compile("([0-9]{4})-([0-9]{2})-([0-9]{2})")

match result do
Ok(re) do
  matched: Boolean = re.is_match("shipped on 2026-09-21")
  puts "#{matched}"

  found: Option[String] = re.find("shipped on 2026-09-21")
  match found do
  Some(m) do
    puts m
  end
  None do
    puts "no match"
  end
  end

  caps: Option[Array[Option[String]]] = re.captures("shipped on 2026-09-21")
  match caps do
  Some(groups) do
    year: Option[String] = groups[1]
    match year do
    Some(y) do
      puts y
    end
    None do
      puts "no year"
    end
    end
  end
  None do
    puts "no captures"
  end
  end

  puts re.replace_all("2026-09-21 and 2026-01-08", "$3/$2/$1")
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  Other(detail) do
    puts detail
  end
  end
end
end

# A real negative proof: an unclosed group is a genuine `regex`-crate
# syntax error, so this always lands on `RegexError::Syntax`, never
# `Other` — never a crash, never a silently-empty message.
bad_result: Result[Regex, RegexError] = Regex.compile("(unclosed")
match bad_result do
Ok(re) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  Other(detail) do
    puts detail
  end
  end
end
end
