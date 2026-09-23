# Plan 163 (Arbitrary-Precision Integers & Decimals) — `BigInt`
# (`num-bigint`, a `crate::handle`-registry opaque `Int64` handle — its
# own internal `Vec<u32>` digit storage is genuinely unbounded, so no
# fixed number of scalar fields could ever hold it) and `Decimal`
# (`rust_decimal`, a packed two-`Int64`-field class reconstructing the
# real 16-byte value on every native call — no heap allocation, no
# handle-registry leak). Two genuinely different representations for
# two genuinely different problems: `Int64`'s 64-bit range cannot hold
# an arbitrary-precision integer like `25!`; `Float64` silently loses
# precision on ordinary decimal arithmetic (`0.1 + 0.2 !=  0.3` in IEEE
# 754 double precision).
#
# Plan 195 (Typed Domain Errors) convention, applied at this plan's own
# EXECUTE time rather than shipping a bare `Result[T, String]`:
# `BigInt.from_s` returns `Result[BigInt, BigIntError]`, `BigIntError =
# Other(String)` (`num_bigint::BigInt::parse_bytes` returns a bare
# `Option`, no real Rust error object of its own, so a single
# `Other(String)` escape hatch is this domain's entire, honest v1
# scope — see `crates/emerald-rt/src/bignum.rs`'s own module doc).
# `Decimal.from_s`/`.div` return `Result[Decimal, DecimalError]`,
# classifying the real, six-variant `rust_decimal::Error` enum plus one
# synthetic `DivisionByZero` tag (see `crates/emerald-rt/src/
# decimal.rs`'s own module doc).
#
# This grammar has no `case`/`when`/`else` keywords (plan 71) — `match
# X do Variant(binds) do ... end ... end` is the real, current syntax
# (`examples/json_demo.em`/`regex_dates.em` precedent, reused here).
#
# Two real, disclosed findings from actually running this file (not
# assumed from this plan's own original text): (1) `puts float_sum`
# (`0.1 + 0.2` as a bare `Float64`) prints `0.3`, not the classic
# `0.30000000000000004` this plan's own Concrete Proof predicted — this
# compiler's `Float64`-to-`String` conversion evidently formats with
# fewer significant digits than Rust's own default shortest-round-trip
# `f64::to_string()`, which hides the classic IEEE 754 rounding
# artifact at the PRINTED-output level even though the underlying
# hardware `+` still computes the same imprecise bit pattern — the
# `Decimal` arithmetic immediately above it (also printing `0.3`, via
# an exact base-10 mantissa+scale, never a binary approximation) is
# still the real, structurally different computation this plan exists
# to demonstrate, just not visually distinguishable from the `Float64`
# line's own output in this particular case. (2) `rust_decimal`'s own
# real `Error::ErrorString` message text for a wholly non-numeric input
# is `"Invalid decimal: unknown character"` — this plan's own
# `DecimalError::Syntax` classification is exercised correctly (this
# is real `rust_decimal` 1.43.0 crate text, not synthesized).

twenty_factorial: BigInt = BigInt.factorial(20)
puts twenty_factorial.to_s

twenty_five_factorial: BigInt = BigInt.factorial(25)
puts twenty_five_factorial.to_s

bad_bigint: Result[BigInt, BigIntError] = BigInt.from_s("not a number")
match bad_bigint do
Ok(n) do
  puts n.to_s
end
Err(e) do
  match e do
  Other(detail) do
    puts detail
  end
  end
end
end

a: BigInt = BigInt.from_i64(20)
b: BigInt = BigInt.from_i64(22)
sum_bigint: BigInt = a.add(b)
puts sum_bigint.to_s
prod_bigint: BigInt = a.mul(b)
puts prod_bigint.to_s

point_one_result: Result[Decimal, DecimalError] = Decimal.from_s("0.1")
point_two_result: Result[Decimal, DecimalError] = Decimal.from_s("0.2")
match point_one_result do
Ok(point_one) do
  match point_two_result do
  Ok(point_two) do
    sum: Decimal = point_one.add(point_two)
    puts sum.to_s

    diff: Decimal = point_two.sub(point_one)
    puts diff.to_s

    prod: Decimal = point_one.mul(point_two)
    puts prod.to_s

    zero_result: Result[Decimal, DecimalError] = Decimal.from_s("0")
    match zero_result do
    Ok(zero) do
      div_result: Result[Decimal, DecimalError] = point_one.div(zero)
      match div_result do
      Ok(q) do
        puts q.to_s
      end
      Err(e) do
        match e do
        Syntax(detail) do
          puts detail
        end
        ExceedsMax do
          puts "exceeds max"
        end
        BelowMin do
          puts "below min"
        end
        Underflow do
          puts "underflow"
        end
        ScaleExceeded(detail) do
          puts detail
        end
        DivisionByZero do
          puts "division by zero"
        end
        Other(detail) do
          puts detail
        end
        end
      end
      end
    end
    Err(e) do
      puts "unexpected zero parse error"
    end
    end
  end
  Err(e) do
    puts "unexpected point_two parse error"
  end
  end
end
Err(e) do
  puts "unexpected point_one parse error"
end
end

float_sum: Float64 = 0.1 + 0.2
puts float_sum

bad_decimal: Result[Decimal, DecimalError] = Decimal.from_s("not a decimal")
match bad_decimal do
Ok(d) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  Syntax(detail) do
    puts detail
  end
  ExceedsMax do
    puts "exceeds max"
  end
  BelowMin do
    puts "below min"
  end
  Underflow do
    puts "underflow"
  end
  ScaleExceeded(detail) do
    puts detail
  end
  DivisionByZero do
    puts "division by zero"
  end
  Other(detail) do
    puts detail
  end
  end
end
end
