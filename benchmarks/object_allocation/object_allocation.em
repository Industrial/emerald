class Box
  value: Int64

  fn initialize(v: Int64): Void do
    @value = v
  end

  fn value: Int64 do
    @value
  end
end

total: Int64 = 0
i: Int64 = 0
while i < 1000000 do
  b: Box = Box.new(i)
  total += b.value
  i += 1
end
puts total
