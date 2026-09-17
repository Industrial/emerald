def square(n)
  n * n
end

total = 0
i = 0
while i < 1_000_000
  total += square(i)
  i += 1
end
puts total
