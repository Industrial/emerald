# Plan 183 (Layered Configuration Loading) — `ConfigBuilder`/
# `ConfigValue`, layering a real TOML config file, a real environment
# variable, and a real parsed CLI flag into one precedence-ordered
# view: CLI flag > environment variable > config file > defaults file,
# per this plan's own Decision log. Run from a fixed temporary
# directory (see `crates/emerald-cli/tests/examples.rs`'s own
# dedicated test) with a real `app.toml` fixture, a real
# `APP__SERVER__PORT` environment variable, and a real `--port 7070`
# argv — the CLI flag wins for `server.port` (both the env var and the
# file also set it), the file's own value survives unshadowed for
# `server.host` (neither the env var nor the CLI flag touches it) — a
# genuine three-layer precedence proof, not a two-source toy case that
# could pass by accident.
#
# Two real, disclosed deviations from this plan's own literal Concrete
# Proof text, found only by actually compiling this file (the same
# class of gap plan 182's own `examples/cli_flag_parsing.em` already
# disclosed):
#
# (1) `case X when Some(v) ... when None ... end` is not this
# grammar's real `Option[T]` pattern-matching syntax — the actual
# construct (`examples/nullable_safe_nav.em`'s own precedent) is
# `match X do Some(v) do ... end None do ... end end`.
#
# (2) `match`'s own scrutinee must be a plain `Let`-bound local, never
# a direct method-call expression (`emerald-codegen`'s own
# `build_case` doc comment: an enum-typed scrutinee is detected via
# `local_classes`, which only a plain `Expr::Ident` scrutinee carries
# a known static type for) — `match cfg.get_string("server.port") do
# ... end` (the plan's own literal shape, direct on the method call)
# is rejected; `port_val: Option[String] = cfg.get_string("server.
# port")` is `Let`-bound first, then `match port_val do ... end`
# matches a plain local.

parser: CliParser = CliParser.new("app", "1.0.0")
parser.option("port", "p", "server port override", false)
cli_result: CliParseResult = parser.parse(ARGV, ARGC)

builder: ConfigBuilder = ConfigBuilder.new()
builder.add_config_file("app")
builder.add_env_prefix("APP")
override_keys: Array[String] = ["server.port"]
builder.add_cli_overrides(cli_result, override_keys, 1)

cfg: ConfigValue = builder.build()

port_val: Option[String] = cfg.get_string("server.port")
match port_val do
  Some(port) do
    puts port
  end
  None do
    puts "no port configured"
  end
end

host_val: Option[String] = cfg.get_string("server.host")
match host_val do
  Some(host) do
    puts host
  end
  None do
    puts "no host configured"
  end
end

parser.close()
cli_result.close()
