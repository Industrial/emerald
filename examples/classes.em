class Counter
  value: Int64

  def initialize(start: Int64) -> Void
    @value = start
  end

  def value -> Int64
    @value
  end

  def add(n: Int64) -> Int64
    @value + n
  end
end

c: Counter = Counter.new(10)
puts c.value
puts c.add(5)

class Point
  x: Float64
  y: Float64

  def initialize(x: Float64, y: Float64) -> Void
    @x = x
    @y = y
  end

  def sum -> Float64
    @x + @y
  end
end

p: Point = Point.new(2.0, 3.0)
puts p.sum
