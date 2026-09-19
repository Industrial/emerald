fn greet(name: String, times: Int64 = 1): Int64 do
  return times
end

puts greet(name: "yo")
puts greet(name: "hi", times: 2)

fn divmod(a: Int64, b: Int64): (Int64, Int64) do
  return a / b, a % b
end

var q: Int64 = 0
var r: Int64 = 0
q, r = divmod(17, 5)
puts q
puts r

fn sum_all(*xs: Int64): Int64 do
  var total: Int64 = 0
  var i: Int64 = 0
  while i < 3 do
    total += xs[i]
    i += 1
  end
  return total
end

puts sum_all(10, 20, 30)
