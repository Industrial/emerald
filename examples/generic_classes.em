class Stack[T]
  slot0: T
  slot1: T
  slot2: T
  count: Int64
  def initialize() -> Void
    @count = 0
  end
  def push(value: T) -> Void
    if @count == 0
      @slot0 = value
    end
    if @count == 1
      @slot1 = value
    end
    if @count == 2
      @slot2 = value
    end
    @count = @count + 1
  end
  def pop() -> T
    @count = @count - 1
    if @count == 0
      return @slot0
    end
    if @count == 1
      return @slot1
    end
    return @slot2
  end
end

ints: Stack[Int64] = Stack.new()
ints.push(10)
ints.push(20)
ints.push(30)
puts ints.pop()
puts ints.pop()

strs: Stack[String] = Stack.new()
strs.push("first")
strs.push("second")
puts strs.pop()
puts strs.pop()
