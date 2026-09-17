class Greeter
  name: String

  def initialize(name: String) -> Void
    @name = name
  end

  def shout -> String
    @name + "!"
  end
end

g1: Greeter? = Greeter.new("ada")
m1: String? = g1&.shout
if m1 == nil
  puts 0
else
  puts 1
end

g2: Greeter? = nil
m2: String? = g2&.shout
if m2 == nil
  puts 0
else
  puts 1
end
