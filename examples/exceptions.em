class MyError
  code: Int64

  fn initialize(code: Int64): Void do
    @code = code
  end

  fn code: Int64 do
    @code
  end
end

fn risky(x: Int64): Int64 do
  if x > 100 do
    raise MyError.new(99)
  end
  return x
end

begin
  puts risky(999)
rescue MyError => e
  puts e.code
end

begin
  puts risky(5)
rescue MyError => e
  puts 0
end

attempts: Int64 = 0
begin
  attempts: Int64 = attempts + 1
  if attempts < 2 do
    raise MyError.new(1)
  end
  puts attempts
rescue MyError => e
  if attempts < 2 do
    retry
  end
  puts 999
ensure
  puts 777
end
