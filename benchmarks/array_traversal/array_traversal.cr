arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20] of Int64
total : Int64 = 0_i64
rep : Int64 = 0_i64
while rep < 1_000_000_i64
  i = 0
  while i < 20
    total += arr[i]
    i += 1
  end
  rep += 1_i64
end
puts total
