class Adder
  base: Int64

  def initialize(base: Int64) -> Void
    @base = base
  end

  def add(n: Int64) -> Int64
    @base + n
  end
end

a: Adder = Adder.new(1)
total: Int64 = 0
i: Int64 = 0
while i < 10000000
  total += a.add(i)
  i += 1
end
puts total
