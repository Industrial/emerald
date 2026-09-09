actor Counter
  count: Int64

  def initialize(start: Int64) -> Void
    @count = start
  end

  def increment -> Void
    @count = @count + 1
  end

  def report -> Void
    puts @count
  end
end
