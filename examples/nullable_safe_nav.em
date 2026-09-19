class Greeter
  name: String

  fn initialize(name: String): Void do
    @name = name
  end

  fn shout: String do
    @name + "!"
  end
end

g1: Greeter? = Greeter.new("ada")
m1: String? = g1&.shout
if m1 == nil do
  puts 0
else
  puts 1
end

g2: Greeter? = nil
m2: String? = g2&.shout
if m2 == nil do
  puts 0
else
  puts 1
end
