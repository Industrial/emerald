class Adder
  def initialize(base)
    @base = base
  end

  def add(n)
    @base + n
  end
end

a = Adder.new(1)
total = 0
i = 0
while i < 10_000_000
  total += a.add(i)
  i += 1
end
puts total
