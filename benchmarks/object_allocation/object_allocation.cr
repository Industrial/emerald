class Holder
  getter value : Int64

  def initialize(@value : Int64)
  end
end

total : Int64 = 0_i64
i : Int64 = 0_i64
while i < 1_000_000_i64
  b = Holder.new(i)
  total += b.value
  i += 1_i64
end
puts total
