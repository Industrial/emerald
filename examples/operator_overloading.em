class Vector2
  read x: Float64
  read y: Float64

  def initialize(x: Float64, y: Float64) -> Void
    @x = x
    @y = y
  end

  def +(other: Vector2) -> Vector2
    Vector2.new(@x + other.x, @y + other.y)
  end

  def ==(other: Vector2) -> Boolean
    @x == other.x && @y == other.y
  end
end

v1: Vector2 = Vector2.new(1.0, 2.0)
v2: Vector2 = Vector2.new(3.0, 4.0)
v3: Vector2 = v1 + v2
puts v3.x
puts v3.y
if v1 == v2
  puts 1
else
  puts 0
end
if v1 == v1
  puts 1
else
  puts 0
end
