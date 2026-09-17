class Box
  attr_reader :value

  def initialize(v)
    @value = v
  end
end

total = 0
i = 0
while i < 1_000_000
  b = Box.new(i)
  total += b.value
  i += 1
end
puts total
