actor Counter
  count: Int64

  fn initialize(start: Int64): Void do
    @count = start
  end

  fn increment: Void do
    @count = @count + 1
  end

  fn report: Void do
    puts @count
  end
end
