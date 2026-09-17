scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}
puts scores[:bob]
scores[:bob] = 100
puts scores[:bob]

if :foo == :foo
  puts 1
else
  puts 0
end

if :foo == :bar
  puts 1
else
  puts 0
end
