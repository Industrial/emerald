# examples/doc_comments.em — plan 77 worked example: `##` doc comments
# (design brief §36) vs. ordinary `#` line comments (plan 20). Run
# `emerald doc examples/doc_comments.em` to see only the `##`-documented
# declarations below extracted; this file also compiles and runs like
# any other example (see examples/README.md's coverage table).

## Adds two integers together.
fn add(a: Int64, b: Int64): Int64 do
  a + b
end

## This doc comment is deliberately separated from `subtract` below by
## a blank line, so `emerald doc` must NOT attach it to `subtract`.

fn subtract(a: Int64, b: Int64): Int64 do
  a - b
end

# An ordinary comment — never extracted as documentation.
fn multiply(a: Int64, b: Int64): Int64 do
  a * b
end

## A point in 2D space, with a Manhattan-distance helper.
class Point
  x: Int64
  y: Int64

  fn initialize(x: Int64, y: Int64): Void do
    @x = x
    @y = y
  end

  ## The sum of the absolute values of both coordinates.
  fn manhattan(): Int64 do
    @x + @y
  end

  fn doubled_manhattan(): Int64 do
    sum: Int64 = @x + @y
    sum * 2
  end
end

## Namespaced integer helpers.
module NumberUtils
  ## Squares an integer.
  fn square(n: Int64): Int64 do
    n * n
  end
end

## How a purchase was paid for.
enum Payment = Cash(Int64) | Card(Int64)

p: Point = Point.new(3, 4)
payment: Payment = Cash(500)

puts add(20, 22)
puts subtract(10, 3)
puts multiply(6, 7)
puts p.manhattan()
puts p.doubled_manhattan()
puts NumberUtils.square(6)
match payment do
  Cash(amount) do
    puts amount
  end
  Card(amount) do
    puts amount
  end
end
