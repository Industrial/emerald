x: Int64 = 10
add_x: Proc = do |y: Int64| y + x end
puts add_x.call(5)

add_one: Proc = do |y: Int64| y + 1 end
puts add_one.call(41)

fn repeat(n: Int64, &blk): Void do
  i: Int64 = 0
  while i < n do
    yield i
    i: Int64 = i + 1
  end
end

repeat(3) do |i: Int64| puts i end
