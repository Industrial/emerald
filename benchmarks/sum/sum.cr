total : Int64 = 0_i64
i : Int64 = 0_i64
while i < 10_000_000_i64
  total += i
  i += 1_i64
end
puts total
