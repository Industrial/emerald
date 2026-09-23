# Plan 182 (Structured CLI Flag Parsing) — `CliParser`/`CliParseResult`,
# a builder-sequence wrapper over `clap`'s non-derive builder API
# (`clap::Command`/`clap::Arg`/`clap::ArgAction`), consuming plan 45's
# own `ARGV: Array[String]`/`ARGC: Int64` pair directly at `.parse` —
# no second, parallel argument-collection mechanism. A missing
# required `--name` is a real, queryable `.error_message()` field, not
# a raised exception (see this plan's own Decision log); `--help`
# never calls `std::process::exit` on Emerald's behalf — it surfaces
# as `.help_requested()`/`.help_text()`, ordinary fields this file
# branches on like any other.
#
# Four real, disclosed deviations from this plan's own literal
# Concrete Proof text, found only by actually compiling this file:
#
# (1) `case X when Some(v) ... when None ... end` is not this
# grammar's real `Option[T]` pattern-matching syntax — plan 73's own
# actual construct (`examples/nullable_safe_nav.em`'s own precedent)
# is `match X do Some(v) do ... end None do ... end end`, used here
# in place of every `case`/`when` in the plan's own literal text.
#
# (2) every `if` needs this grammar's real `do` keyword immediately
# after its condition (`examples/control_flow.em`'s own precedent) —
# the plan's own literal `if result.help_requested()` (no `do`) does
# not parse.
#
# (3) `match`'s own scrutinee must be a plain `Let`-bound local, never
# a direct method-call expression — `emerald-codegen`'s own
# `build_case` doc comment states this outright: an enum-typed
# scrutinee is detected via `local_classes`, which "only a plain
# `Expr::Ident` scrutinee can carry a known static type" for, "the
# same receiver restriction `build_method_call` already imposes
# elsewhere in this backend." `match result.error_message() do ... end`
# (the plan's own literal shape, direct on the method call) fails with
# "codegen: `case` scrutinee must be Int64"; every `match` here first
# `Let`-binds its own `Option[String]` to a local, `examples/nullable_
# safe_nav.em`'s own precedent (`m1: Option[String] = g1?.shout`
# before `match m1 do ... end`), never matching a call expression
# directly.
#
# (4) `??`'s own left operand has the identical restriction — a real,
# disclosed compiler diagnostic states it outright: "`??`'s left
# operand must be a plain local variable or a `?.` chain." `result.
# value("name") ?? "world"` (the plan's own literal shape, direct on
# the method call) is rejected the same way; `name_val: Option[String]
# = result.value("name")` is `Let`-bound first, then `name_val ??
# "world"` matches a plain local.

parser: CliParser = CliParser.new("greet", "1.0.0")
parser.flag("verbose", "v", "print extra detail")
parser.option("name", "n", "who to greet", true)
parser.positional("suffix", "a trailing word", false)

result: CliParseResult = parser.parse(ARGV, ARGC)

if result.help_requested() do
  puts result.help_text()
else
  err_msg: Option[String] = result.error_message()
  match err_msg do
    Some(msg) do
      puts msg
    end
    None do
      name_val: Option[String] = result.value("name")
      name: String = name_val ?? "world"
      puts "Hello, " + name
      if result.flag("verbose") do
        puts "(verbose mode on)"
      end
      suffix_val: Option[String] = result.positional_value("suffix")
      match suffix_val do
        Some(s) do
          puts s
        end
        None do
          puts "no suffix"
        end
      end
    end
  end
end

parser.close()
result.close()
