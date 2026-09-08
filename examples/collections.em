arr: Array[Int64] = [10, 20, 30]
sum: Int64 = 0
i: Int64 = 0
while i < 3
  sum: Int64 = sum + arr[i]
  i: Int64 = i + 1
end
arr[1] = 99
puts sum
puts arr[1]

floats: Array[Float64] = [1.5, 2.5]
puts floats[0] + floats[1]
