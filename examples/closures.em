x: Int64 = 10
add_x: Proc = do |y: Int64| y + x end
puts add_x.call(5)

add_one: Proc = do |y: Int64| y + 1 end
puts add_one.call(41)

fn repeat(n: Int64, &blk): Void do
  i: Int64 = 0
  while i < n do
    yield i
    i: Int64 = i + 1
  end
end

repeat(3) do |i: Int64| puts i end

# A real bug fix (found post-plan-89, this session): a `Proc`-typed
# CLASS FIELD's `.call` was disclosed as unsupported — `@op.call(x)`
# from inside the declaring class's own method body now works, the
# same genuine indirect call a Proc-typed parameter already got.
class Adder
  op: Proc[Int64, Int64]

  fn initialize(op: Proc[Int64, Int64]): Void do
    @op = op
  end

  fn apply(x: Int64): Int64 do
    @op.call(x)
  end
end

adder: Adder = Adder.new(add_one)
puts adder.apply(41)
