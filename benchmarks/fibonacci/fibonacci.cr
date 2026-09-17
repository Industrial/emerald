def fib(n : Int64) : Int64
  return n if n < 2
  fib(n - 1) + fib(n - 2)
end

puts fib(30_i64)
