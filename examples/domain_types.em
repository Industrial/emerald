# `newtype` — domain types and units (`domain-types-and-units`, plan-of-
# plans row 81). Resolves the Sable design brief's own "type aliases and
# newtypes" section, explicitly left open/undecided there — this is the
# concrete design decision: a zero-cost, nominally distinct wrapper
# around exactly one primitive type. `Meters`/`Seconds`/`MetersPerSecond`
# mirror the brief's own illustrative example.
#
# `Meters` and `Seconds` are mutually non-interchangeable even though
# neither can be confused with a bare `Float64` either — construction is
# always the explicit `Name.new(value)`, unwrapping is always the
# explicit `.value`. See `spec/TYPE_SYSTEM.md` §11 and `spec/GRAMMAR.md`
# §15 for the full rule, and `examples/newtype_zero_cost_benchmark.em`
# for the proof that this costs nothing at runtime.

newtype Meters: Float64
newtype Seconds: Float64
newtype MetersPerSecond: Float64

fn speed(distance: Meters, time: Seconds): MetersPerSecond do
  return MetersPerSecond.new(distance.value / time.value)
end

race_distance: Meters = Meters.new(100.0)
race_time: Seconds = Seconds.new(9.58)
top_speed: MetersPerSecond = speed(race_distance, race_time)
puts top_speed.value

# Two `Meters` values compare structurally equal via `==` directly (no
# `.value` unwrap needed) — nominal distinctness never gets in the way of
# ordinary structural equality between two values of the SAME domain type.
lap_one: Meters = Meters.new(400.0)
lap_two: Meters = Meters.new(400.0)

if lap_one == lap_two do
  puts 1
else
  puts 0
end

# `Array[Meters]` has the identical packed, unboxed representation
# `Array[Float64]` does (see the benchmark) — indexing, summing, and
# accumulating over it costs nothing extra.
splits: Array[Meters] = [Meters.new(25.0), Meters.new(25.0), Meters.new(25.0), Meters.new(25.0)]
total: Float64 = 0.0
i: Int64 = 0
while i < splits.count do
  leg: Meters = splits[i]
  total: Float64 = total + leg.value
  i: Int64 = i + 1
end
puts total
