x: Int64 = 10
add_x: Proc = ->(y: Int64) -> Int64 { y + x }
puts add_x.call(5)

add_one: Proc = ->(y: Int64) -> Int64 { y + 1 }
puts add_one.call(41)
