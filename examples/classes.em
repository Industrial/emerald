class Counter
  value: Int64

  fn initialize(start: Int64): Void do
    @value = start
  end

  fn value: Int64 do
    @value
  end

  fn add(n: Int64): Int64 do
    @value + n
  end
end

c: Counter = Counter.new(10)
puts c.value
puts c.add(5)

class Point
  x: Float64
  y: Float64

  fn initialize(x: Float64, y: Float64): Void do
    @x = x
    @y = y
  end

  fn sum: Float64 do
    @x + @y
  end
end

p: Point = Point.new(2.0, 3.0)
puts p.sum
