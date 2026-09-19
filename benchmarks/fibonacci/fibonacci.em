fn fib(n: Int64): Int64 do
  if n < 2 do
    return n
  end
  return fib(n - 1) + fib(n - 2)
end

puts fib(30)
