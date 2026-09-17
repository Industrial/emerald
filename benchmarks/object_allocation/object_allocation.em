class Box
  value: Int64

  def initialize(v: Int64) -> Void
    @value = v
  end

  def value -> Int64
    @value
  end
end

total: Int64 = 0
i: Int64 = 0
while i < 1000000
  b: Box = Box.new(i)
  total += b.value
  i += 1
end
puts total
