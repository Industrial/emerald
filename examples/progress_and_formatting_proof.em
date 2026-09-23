bar: ProgressBar = ProgressBar.new(100)
i: Int64 = 0
while i < 100 do
  bar.increment(1)
  i: Int64 = i + 1
end
bar.finish()
puts Console.styled("done", "green")
if Console.is_terminal do
  puts "true"
else
  puts "false"
end
