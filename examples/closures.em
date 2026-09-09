x: Int64 = 10
add_x: Proc = ->(y: Int64) -> Int64 { y + x }
puts add_x.call(5)

add_one: Proc = ->(y: Int64) -> Int64 { y + 1 }
puts add_one.call(41)

def repeat(n: Int64, &blk) -> Void
  i: Int64 = 0
  while i < n
    yield i
    i: Int64 = i + 1
  end
end

repeat(3) { |i: Int64| puts i }
