property "sorting an array preserves its element sum" do
  arr: Array[Int64] = [5, 3, 8, 1, 9, 2]
  sorted: Array[Int64] = arr.sort()
  assert_eq(arr.sum(), sorted.sum())
end

property "addition is commutative for a fixed pair of integers" do
  a: Int64 = 7
  b: Int64 = 35
  assert_eq(a + b, b + a)
end
