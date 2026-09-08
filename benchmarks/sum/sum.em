total: Int64 = 0
i: Int64 = 0
while i < 10000000
  total: Int64 = total + i
  i: Int64 = i + 1
end
puts total
