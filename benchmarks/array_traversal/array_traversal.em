arr: Array[Int64] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
total: Int64 = 0
rep: Int64 = 0
while rep < 1000000
  i: Int64 = 0
  while i < 20
    total: Int64 = total + arr[i]
    i: Int64 = i + 1
  end
  rep: Int64 = rep + 1
end
puts total
