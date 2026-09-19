class Animal
  read age: Int64

  fn initialize(age: Int64): Void do
    @age = age
  end

  fn describe: Int64 do
    @age
  end
end

class Dog < Animal
  breed_code: Int64

  fn initialize(age: Int64, breed_code: Int64): Void do
    @age = age
    @breed_code = breed_code
  end

  fn describe: Int64 do
    @age + @breed_code
  end
end

a: Animal = Animal.new(5)
d: Dog = Dog.new(3, 100)
puts a.describe
puts d.age
puts d.describe
