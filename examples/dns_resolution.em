# Plan 195 (Typed Domain Errors) retrofit, disclosed breaking change:
# `Regex.compile` now returns `Result[Regex, RegexError]`, not
# `Result[Regex, String]` — see `examples/regex_dates.em`'s own header
# comment for the full account. This file's `Err` arm is the disclosed
# migration.
pattern_result: Result[Regex, RegexError] = Regex.compile("^[0-9a-fA-F:.]+$")

match pattern_result do
Ok(addr_pattern) do
  addr: String = Dns.resolve("one.one.one.one")
  puts "#{addr_pattern.is_match(addr)}"

  Dns.configure("cloudflare_tls", "")
  secure_addr: String = Dns.resolve("one.one.one.one")
  puts "#{addr_pattern.is_match(secure_addr)}"
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
