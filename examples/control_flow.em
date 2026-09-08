def classify(n: Int64) -> Int64
  if n == 0
    return 0
  end
  if n < 10
    return 1
  end
  return 2
end

a: Int64 = 5
b: Int64 = 10

if a < b
  puts 1
else
  puts 0
end

if b > a
  puts 1
end

if a <= 5
  puts 1
end

if b >= 10
  puts 1
end

if a != b
  puts 1
end

if a == 5
  puts 1
end

i: Int64 = 0
while i < 5
  if i == 2
    i: Int64 = i + 1
    next
  end
  if i == 4
    break
  end
  puts i
  i: Int64 = i + 1
end

puts classify(0)
puts classify(5)
puts classify(20)
