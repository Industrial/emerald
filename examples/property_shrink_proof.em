property "addition is commutative" (a: Int64, b: Int64) do
  assert_eq(a + b, b + a)
end

property "subtraction finds a real bug" (a: Int64, b: Int64) do
  assert_eq(a - b, b - a)
end
