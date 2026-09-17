def greet(name: String, times: Int64 = 1) -> Int64
  return times
end

puts greet(name: "yo")
puts greet(name: "hi", times: 2)

def divmod(a: Int64, b: Int64) -> (Int64, Int64)
  return a / b, a % b
end

q: Int64 = 0
r: Int64 = 0
q, r = divmod(17, 5)
puts q
puts r

def sum_all(*xs: Int64) -> Int64
  total: Int64 = 0
  i: Int64 = 0
  while i < 3
    total += xs[i]
    i += 1
  end
  return total
end

puts sum_all(10, 20, 30)
