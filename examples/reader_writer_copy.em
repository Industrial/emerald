# Plan 194's leaf 2: a `Reader`/`Writer` interface pair (Go's
# `io.Reader`/`io.Writer`, Rust's `std::io::Read`/`Write` as named
# precedent), built entirely with the EXISTING, plain user-declarable
# interface mechanism (plan 41) plus the existing single-type-parameter
# generic-function mechanism (plan 41/88/89) — no compiler change of
# any kind. `read_chunk` returns an EMPTY array to signal EOF (Go's own
# documented convention — a zero-length read is a valid, non-error
# outcome — rather than Rust's own ambiguous 0-byte-read overload,
# which this language's fully-synchronous runtime has no need to
# import). `write_chunk` returns the number of bytes actually written,
# for short-write handling.
#
# Several real, disclosed adaptations this session found necessary,
# none specific to Reader/Writer themselves — each confirmed directly
# against the current build, not assumed from any plan record's text:
#
# 1. This language has no `UInt8`/`Byte`/`Bytes` type at all (checked
#    directly against `emerald-sema`'s `Type` enum — the same gap
#    `base64_hex_encoding.em`'s own header comment already discloses
#    for a different plan). Every "byte" here is a plain `Int64`
#    holding a value in 0..255, so `Array[UInt8]` throughout this
#    plan's own text is written as `Array[Int64]` instead.
# 2. A plain, non-generic interface-typed parameter does not exist —
#    `fn f(x: Reader): ...` fails to resolve with `unknown type
#    Reader` (`resolve_named_type` has no interface branch at all;
#    only `Type::Class`/`Type::Enum`/.../`Type::Generic` exist, and
#    `Generic` is reachable only inside a bounded type parameter's own
#    body). So `copy` below is generic over its SOURCE only
#    (`copy[R: Reader](src: R, ...)`), not over both source and
#    destination the way this plan's own original sketch shows —
#    `emerald-sema` rejects a SECOND type parameter on any one
#    function or method outright ("declares 2 type parameters —
#    multiple type parameters are not supported",
#    `crates/emerald-sema/src/lib.rs` lines 1094/13419, confirmed
#    directly by trying `fn copy[R: Reader, W: Writer](...)`). `dst`
#    is the concrete `BufferWriter` class instead — still a real,
#    `implements Writer`-checked conformance, just not itself a bound
#    generic parameter in this one signature.
# 3. Calling a method directly on an instance variable
#    (`@data.count`), or indexing/index-assigning one directly
#    (`@data[i]`, `@data[i] = v`), both fail at codegen ("method calls
#    are only supported on a plain local-variable receiver" / "indexed
#    assignment is only supported on a plain local-variable receiver")
#    — every method/index in `FixedChunkReader`/`BufferWriter` below
#    first copies the field to a local, operates on the local, and (for
#    a write) reassigns the field from the local afterward.
# 4. `copy`'s own `chunk_size: Int64 = 65536` default parameter value
#    is declared (matching this plan's own API sketch) but not actually
#    honored when `copy` is called with only 2 arguments — a real,
#    disclosed gap in how default parameter values interact with a
#    GENERIC function specifically (`examples/function_signatures.em`'s
#    own ordinary, non-generic functions do honor theirs). Both calls
#    below pass `chunk_size` explicitly rather than relying on it.
# 5. A generic method's `Let`-bound, `?`-unwrapped `Result[Array[T],
#    E]` return value did not correctly track its own declared element
#    type for a later `.count` call ("cannot determine the class of
#    `chunk` for `.count`") until rebound to a second, freshly-declared
#    local (`chunk0` then `chunk` in `copy` below) — a narrow, real
#    gap in generic-call type-tracking, worked around rather than
#    fixed (out of this plan's own scope; see plan 194's own history
#    record for the full account).

class IoError
  read message: String

  fn initialize(message: String): Void do
    @message = message
  end
end

interface Reader
  fn read_chunk(max_bytes: Int64): Result[Array[Int64], IoError]
end

interface Writer
  fn write_chunk(bytes: Array[Int64]): Result[Int64, IoError]
end

class FixedChunkReader
  implements Reader

  data: Array[Int64]
  pos: Int64

  fn initialize(data: Array[Int64]): Void do
    @data = data
    @pos = 0
  end

  fn read_chunk(max_bytes: Int64): Result[Array[Int64], IoError] do
    d: Array[Int64] = @data
    if @pos >= d.count do
      empty: Array[Int64] = Array.new(0)
      return Ok(empty)
    end
    remaining: Int64 = d.count - @pos
    var n: Int64 = max_bytes
    if remaining < n do
      n = remaining
    end
    chunk: Array[Int64] = Array.new(n)
    var i: Int64 = 0
    while i < n do
      chunk[i] = d[@pos + i]
      i = i + 1
    end
    @pos = @pos + n
    return Ok(chunk)
  end
end

class BufferWriter
  implements Writer

  buffer: Array[Int64]
  pos: Int64

  fn initialize(capacity: Int64): Void do
    buf: Array[Int64] = Array.new(capacity)
    @buffer = buf
    @pos = 0
  end

  fn write_chunk(bytes: Array[Int64]): Result[Int64, IoError] do
    buf: Array[Int64] = @buffer
    var i: Int64 = 0
    while i < bytes.count do
      buf[@pos] = bytes[i]
      @pos = @pos + 1
      i = i + 1
    end
    @buffer = buf
    return Ok(bytes.count)
  end

  fn written_count(): Int64 do
    @pos
  end

  fn byte_at(idx: Int64): Int64 do
    b: Array[Int64] = @buffer
    b[idx]
  end
end

fn copy[R: Reader](src: R, dst: BufferWriter, chunk_size: Int64 = 65536): Result[Int64, IoError] do
  var total: Int64 = 0
  var done: Boolean = false
  while !done do
    chunk0: Array[Int64] = src.read_chunk(chunk_size)?
    chunk: Array[Int64] = chunk0
    if chunk.count == 0 do
      done = true
    else
      written: Int64 = dst.write_chunk(chunk)?
      total = total + written
    end
  end
  return Ok(total)
end

# "Hello" as bytes, copied 2 at a time — forces `copy` through three
# real `read_chunk`/`write_chunk` round trips (2 + 2 + 1 bytes), not
# one single pass, and the destination buffer's exact contents are
# checked byte-for-byte afterward.
data: Array[Int64] = [72, 101, 108, 108, 111]
src: FixedChunkReader = FixedChunkReader.new(data)
dst: BufferWriter = BufferWriter.new(5)

copied: Result[Int64, IoError] = copy(src, dst, 2)
match copied do
Ok(n) do
  puts n
  var i: Int64 = 0
  while i < dst.written_count() do
    puts dst.byte_at(i)
    i = i + 1
  end
end
Err(e) do
  puts e.message
end
end

# A second call with a chunk_size large enough that one real chunk
# covers all 5 bytes in a single `read_chunk`/`write_chunk` round trip,
# unlike the forced-multi-chunk proof above.
src2: FixedChunkReader = FixedChunkReader.new(data)
dst2: BufferWriter = BufferWriter.new(5)
copied2: Result[Int64, IoError] = copy(src2, dst2, 65536)
match copied2 do
Ok(n) do
  puts n
end
Err(e) do
  puts e.message
end
end
