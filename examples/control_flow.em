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

c: Int64 = 7
if c == 1
  puts 100
elsif c == 7
  puts 101
else
  puts 102
end

unless c == 1
  puts 103
end

u: Int64 = 0
until u == 2
  puts u
  u: Int64 = u + 1
end

for x in [5, 6, 7]
  puts x
end

case c
when 7
  puts 200
when 8
  puts 201
end
