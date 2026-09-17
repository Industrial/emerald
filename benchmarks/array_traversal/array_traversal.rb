arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
total = 0
rep = 0
while rep < 1_000_000
  i = 0
  while i < 20
    total += arr[i]
    i += 1
  end
  rep += 1
end
puts total
