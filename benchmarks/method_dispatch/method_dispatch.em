class Adder
  base: Int64

  fn initialize(base: Int64): Void do
    @base = base
  end

  fn add(n: Int64): Int64 do
    @base + n
  end
end

a: Adder = Adder.new(1)
total: Int64 = 0
i: Int64 = 0
while i < 10000000 do
  total += a.add(i)
  i += 1
end
puts total
