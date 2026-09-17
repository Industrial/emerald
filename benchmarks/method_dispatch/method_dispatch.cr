class Adder
  def initialize(@base : Int64)
  end

  def add(n : Int64) : Int64
    @base + n
  end
end

a = Adder.new(1_i64)
total : Int64 = 0_i64
i : Int64 = 0_i64
while i < 10_000_000_i64
  total += a.add(i)
  i += 1_i64
end
puts total
