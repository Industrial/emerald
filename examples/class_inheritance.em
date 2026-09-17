class Animal
  read age: Int64

  def initialize(age: Int64) -> Void
    @age = age
  end

  def describe -> Int64
    @age
  end
end

class Dog < Animal
  breed_code: Int64

  def initialize(age: Int64, breed_code: Int64) -> Void
    @age = age
    @breed_code = breed_code
  end

  def describe -> Int64
    @age + @breed_code
  end
end

a: Animal = Animal.new(5)
d: Dog = Dog.new(3, 100)
puts a.describe
puts d.age
puts d.describe
