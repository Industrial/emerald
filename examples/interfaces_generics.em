interface Comparable
  def compare_to(other: Self) -> Int64
end

class Money implements Comparable
  read cents: Int64

  def initialize(cents: Int64) -> Void
    @cents = cents
  end

  def compare_to(other: Money) -> Int64
    @cents - other.cents
  end
end

class Distance implements Comparable
  read meters: Int64

  def initialize(meters: Int64) -> Void
    @meters = meters
  end

  def compare_to(other: Distance) -> Int64
    @meters - other.meters
  end
end

def max[T: Comparable](a: T, b: T) -> T
  if a.compare_to(b) >= 0
    return a
  end
  return b
end

m1: Money = Money.new(500)
m2: Money = Money.new(750)
winner_money: Money = max(m1, m2)
puts winner_money.cents

d1: Distance = Distance.new(100)
d2: Distance = Distance.new(42)
winner_distance: Distance = max(d1, d2)
puts winner_distance.meters
