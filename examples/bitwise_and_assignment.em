READ: Int64 = 1
WRITE: Int64 = 2
EXEC: Int64 = 4

fn has_flag(flags: Int64, flag: Int64): Boolean do
  return flags & flag == flag
end

perms: Int64 = READ | WRITE
puts perms
if has_flag(perms, READ) do
  puts 1
end
if has_flag(perms, EXEC) do
  puts 0
end
puts perms ^ WRITE
puts ~0
puts 1 << 4
puts 256 >> 4

var total: Int64 = 0
var i: Int64 = 0
while i < 5 do
  total += i
  i += 1
end
puts total

var a: Int64 = 1
var b: Int64 = 2
a, b = b, a
puts a
puts b
