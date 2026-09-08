class MyError
  code: Int64

  def initialize(code: Int64) -> Void
    @code = code
  end

  def code -> Int64
    @code
  end
end

def risky(x: Int64) -> Int64
  if x > 100
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
