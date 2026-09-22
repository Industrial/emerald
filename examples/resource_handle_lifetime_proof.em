# Plan 93 (Resource Handle & Lifetime Model for Native Objects) —
# `crates/emerald-rt`'s handle registry, exercised end to end through a
# trivial in-memory counter "resource" (a real I/O-backed resource is a
# later domain plan's own job — this plan's job is the lifecycle
# mechanism, not any particular domain's stdlib surface).
#
# Two real, disclosed corrections to this plan's own original sketch,
# found only by running it: (1) this language has no user-declarable
# class-static-method syntax (`Counter.native_open`-style `ClassName.
# method` dispatch on a plain user class fails to parse — `fn self.foo`
# is not a thing) — `native_open`/`native_bump`/`native_close` are
# ordinary compiler-intrinsic FREE FUNCTIONS instead, dispatched by
# exact name exactly like `puts`/`gets`. (2) A zero-argument `.new`/
# free-function call still needs explicit `()` — `Counter.new` (no
# parens) fails to parse; `Counter.new()` is required.
class Counter
  handle: Int64

  fn initialize: Void do
    @handle = handle_counter_open()
  end

  fn bump: Int64 do
    handle_counter_bump(@handle)
  end

  fn close: Void do
    handle_counter_close(@handle)
  end
end

c: Counter = Counter.new()
puts c.bump
puts c.bump
c.close

begin
  puts c.bump
rescue NativeError => e
  puts e.message
end

c.close
puts "still running"
