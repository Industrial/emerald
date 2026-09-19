arr: Array[Int64] = [10, 20, 30]
sum: Int64 = 0
i: Int64 = 0
while i < 3 do
  sum: Int64 = sum + arr[i]
  i: Int64 = i + 1
end
arr[1] = 99
puts sum
puts arr[1]

floats: Array[Float64] = [1.5, 2.5]
puts floats[0] + floats[1]

h: Hash[Int64, Int64] = {1 => 10, 2 => 20}
puts h[1]
puts h[2]

flag: Boolean = true
other: Boolean = false
if flag do
  puts 1
end
if other do
  puts 2
else
  puts 3
end

s: Option[String] = None
match s do
  Some(text) do
    puts 0
  end
  None do
    puts 4
  end
end
