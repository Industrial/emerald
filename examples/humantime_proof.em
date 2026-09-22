# Plan 162 (Human-Readable Duration/Time Formatting) — `Duration.
# humanize`/`.parse_human`, `Timestamp.to_rfc3339`/`.parse_rfc3339`,
# a narrow, four-function wrapping of `humantime`. No new `Type`/
# `Class` anywhere — every value is a plain `Int64` (whole seconds) or
# `String`, this batch's cleanest example of a numeric concept
# round-tripping entirely through Emerald's existing type system.
#
# Real, disclosed correction: this grammar has no `case`/`when`/`else`
# keywords (removed per plan 71) — `match X do Ok(v) do ... end Err(e)
# do ... end end` is the real, current syntax.

puts Duration.humanize(266400)

parsed_result: Result[Int64, String] = Duration.parse_human("3d 2h")
match parsed_result do
Ok(seconds) do
  puts seconds
end
Err(e) do
  puts e
end
end

bad_result: Result[Int64, String] = Duration.parse_human("not a duration")
match bad_result do
Ok(seconds) do
  puts seconds
end
Err(e) do
  puts e
end
end

ts: String = Timestamp.to_rfc3339(1780000000)
puts ts
roundtrip: Result[Int64, String] = Timestamp.parse_rfc3339(ts)
match roundtrip do
Ok(secs) do
  puts secs
end
Err(e) do
  puts e
end
end
