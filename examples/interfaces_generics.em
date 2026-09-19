interface Comparable
  fn compare_to(other: Self): Int64
end

class Money implements Comparable
  read cents: Int64

  fn initialize(cents: Int64): Void do
    @cents = cents
  end

  fn compare_to(other: Money): Int64 do
    @cents - other.cents
  end
end

class Distance implements Comparable
  read meters: Int64

  fn initialize(meters: Int64): Void do
    @meters = meters
  end

  fn compare_to(other: Distance): Int64 do
    @meters - other.meters
  end
end

fn max[T: Comparable](a: T, b: T): T do
  if a.compare_to(b) >= 0 do
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
