# `own` / `borrow` / `borrow var` (plan 83, `spec/OWNERSHIP.md` §2) — a
# real, lexical-scope-based ownership/borrow checker, enforced entirely
# by `emerald-sema`. `own T` transfers a parameter's binding to the
# callee (the caller may not use it again after the call); `borrow T`
# is a shared, read-only reference; `borrow var T` is an exclusive,
# mutable reference. `emerald-codegen` compiles all three exactly like
# the plain underlying `T` today — a real, disclosed, temporary
# passthrough (see `value_kind_for_type`'s own doc comment in that
# crate) — plan 84 owns real zero-cost borrow/own codegen.

class Counter
  value: Int64

  fn initialize(start: Int64): Void do
    @value = start
  end

  fn value: Int64 do
    @value
  end

  fn bump: Void do
    @value = @value + 1
  end
end

# A shared, read-only reference — `report` may read `c` but never
# mutate it (enforced structurally today: `Counter` has no method
# `report` could call that isn't itself borrow-checked the same way).
fn report(c: borrow Counter): Void do
  puts c.value
end

# An exclusive, mutable reference — legal to call a mutating method
# through it.
fn increment(c: borrow var Counter): Void do
  c.bump
end

# Ownership transfer — `finish`'s caller may not use its argument again
# after this call returns.
fn finish(c: own Counter): Void do
  puts c.value
end

counter: Counter = Counter.new(10)
report(counter)
increment(counter)
report(counter)
finish(counter)
