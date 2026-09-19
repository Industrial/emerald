fn classify(n: Int64): Int64 do
  if n == 0 do
    return 0
  end
  if n < 10 do
    return 1
  end
  return 2
end

a: Int64 = 5
b: Int64 = 10

if a < b do
  puts 1
else
  puts 0
end

if b > a do
  puts 1
end

if a <= 5 do
  puts 1
end

if b >= 10 do
  puts 1
end

if a != b do
  puts 1
end

if a == 5 do
  puts 1
end

i: Int64 = 0
while i < 5 do
  if i == 2 do
    i: Int64 = i + 1
    next
  end
  if i == 4 do
    break
  end
  puts i
  i: Int64 = i + 1
end

puts classify(0)
puts classify(5)
puts classify(20)

c: Int64 = 7
if c == 1 do
  puts 100
elsif c == 7 do
  puts 101
else
  puts 102
end

unless c == 1 do
  puts 103
end

u: Int64 = 0
until u == 2 do
  puts u
  u: Int64 = u + 1
end

for x in [5, 6, 7]
  puts x
end

match c do
  7 do
    puts 200
  end
  8 do
    puts 201
  end
end
