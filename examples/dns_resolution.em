pattern_result: Result[Regex, String] = Regex.compile("^[0-9a-fA-F:.]+$")

match pattern_result do
Ok(addr_pattern) do
  addr: String = Dns.resolve("one.one.one.one")
  puts "#{addr_pattern.is_match(addr)}"

  Dns.configure("cloudflare_tls", "")
  secure_addr: String = Dns.resolve("one.one.one.one")
  puts "#{addr_pattern.is_match(secure_addr)}"
end
Err(msg) do
  puts msg
end
end
