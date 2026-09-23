class Point
  read x: Int64
  read y: Int64

  fn initialize(x: Int64, y: Int64): Void do
    @x = x
    @y = y
  end

  static fn origin(): Point do
    Point.new(0, 0)
  end

  static fn midpoint(a: Point, b: Point): Point do
    sx: Int64 = a.x + b.x
    sy: Int64 = a.y + b.y
    Point.new(sx / 2, sy / 2)
  end
end

o: Point = Point.origin()
puts o.x
puts o.y

p1: Point = Point.new(2, 4)
p2: Point = Point.new(8, 10)
m: Point = Point.midpoint(p1, p2)
puts m.x
puts m.y
