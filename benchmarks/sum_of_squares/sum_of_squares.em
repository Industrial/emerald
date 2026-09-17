def square(n: Int64) -> Int64
  n * n
end

total: Int64 = 0
i: Int64 = 0
while i < 1000000
  total += square(i)
  i += 1
end
puts total
