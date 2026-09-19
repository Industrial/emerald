class Greeter
  name: String

  fn initialize(name: String): Void do
    @name = name
  end

  fn shout: String do
    @name + "!"
  end
end

g1: Option[Greeter] = Some(Greeter.new("ada"))
m1: Option[String] = g1?.shout
match m1 do
  Some(text) do
    puts 1
  end
  None do
    puts 0
  end
end

g2: Option[Greeter] = None
m2: Option[String] = g2?.shout
match m2 do
  Some(text) do
    puts 1
  end
  None do
    puts 0
  end
end
