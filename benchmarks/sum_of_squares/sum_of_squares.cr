def square(n : Int64) : Int64
  n * n
end

total : Int64 = 0_i64
i : Int64 = 0_i64
while i < 1_000_000_i64
  total += square(i)
  i += 1_i64
end
puts total
