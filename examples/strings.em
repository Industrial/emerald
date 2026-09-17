name: String = "World"
age: Int64 = 30
puts "Hello, #{name}! You are #{age} years old."

phrase: String = "  Hello World  "
puts phrase.strip
puts phrase.upcase
puts phrase.downcase
puts phrase.length
puts phrase.split_count(" ")

raw: String = "42"
puts raw.to_i
fraw: String = "3.5"
puts fraw.to_f
