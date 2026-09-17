READ: Int64 = 1
WRITE: Int64 = 2
EXEC: Int64 = 4

def has_flag(flags: Int64, flag: Int64) -> Boolean
  return flags & flag == flag
end

perms: Int64 = READ | WRITE
puts perms
if has_flag(perms, READ)
  puts 1
end
if has_flag(perms, EXEC)
  puts 0
end
puts perms ^ WRITE
puts ~0
puts 1 << 4
puts 256 >> 4

total: Int64 = 0
i: Int64 = 0
while i < 5
  total += i
  i += 1
end
puts total

a: Int64 = 1
b: Int64 = 2
a, b = b, a
puts a
puts b
