# `own` / `borrow` / `borrow var` (plan 83, `spec/OWNERSHIP.md` §2) — a
# real, lexical-scope-based ownership/borrow checker, enforced entirely
# by `emerald-sema`. `own T` transfers a parameter's binding to the
# callee (the caller may not use it again after the call); `borrow T`
# is a shared, read-only reference; `borrow var T` is an exclusive,
# mutable reference.
#
# A real, disclosed consequence of §8's own lexical-scope (not flow-
# sensitive) model, found this session once top-level statements
# started getting checked at all (see the plan-of-plans's own "Real
# bug fix" note on the top-level message-safety/borrow-checking gap):
# a `borrow` and a `borrow var` of the SAME binding conflict anywhere
# in one flat scope, even used sequentially with no real overlap — the
# model has no notion of "this borrow's use already ended." So `report`
# [borrow] and `increment` [borrow var] below are each demonstrated on
# their OWN, separate `Counter` instance, never the same one — this is
# not a workaround for a bug, it's the real, already-accepted rule
# (plan 83's own doc comment: "never under-rejects, only occasionally
# over-rejects" independent, non-overlapping uses).
#
# Plan 84's real codegen (`spec/OWNERSHIP.md` §9/§10): `own`, and
# `borrow`/`borrow var` of a class instance (like `Counter` below,
# already an LLVM pointer), compile to EXACTLY the plain underlying
# type — genuinely zero-cost, no new machinery, since a class instance
# was already passed by pointer before this plan (see `emerald-codegen`'s
# `bind_params`/`strip_ownership_in_type_expr` doc comments). A
# `borrow`/`borrow var` of a BY-VALUE primitive (`Int64`/`Float64`/
# `Boolean`/`Symbol`) is different: it compiles to a REAL pointer at the
# LLVM level (a genuinely new mechanism this plan builds) — `describe`
# below passes `n` this way (a read-only reference, so this program's
# own output can't distinguish it from a copy, but `emerald-codegen`'s
# own `bind_params` doc comment and test suite prove it really is one).
#
# Real, disclosed limitation this plan found — CLOSED 2026-09-21 (this
# session's "find all bugs" sweep). Mutating a borrowed CLASS instance
# (like `increment` above) works by calling a MUTATING METHOD, which
# writes a FIELD (`@value = ...`) — never by reassigning the local
# binding `c` itself. A primitive has no fields/methods to mutate
# through, so exercising the by-value `borrow var` pointer/writeback
# codegen this plan builds (`emerald-codegen`'s own `bind_params`/
# `emit_borrow_var_writebacks`) needed `emerald-sema` to allow
# reassigning a `borrow var`-typed BY-VALUE parameter itself — which it
# didn't: plan 72's blanket "a parameter is immutable by construction,
# no `var` slot exists on a parameter" rule made `x = ...` a compile
# error for EVERY parameter, `borrow var`-typed or not, so this real,
# already-shipped, zero-cost-proven codegen mechanism was unreachable
# from real `.em` source. `increment_primitive` below is the closed
# version: `emerald-sema` now special-cases exactly this one shape (a
# `borrow var` of `Int64`/`Float64`/`Boolean`/`Symbol`) as mutable,
# leaving every other parameter exactly as immutable as before. This
# stays scoped to TOP-LEVEL functions only, matching plan 84's own
# codegen scope — a class/actor/module METHOD's by-value `borrow var`
# parameter still keeps the original full-erasure passthrough (no real
# pointer, no writeback), so its mutation still would not reach the
# caller; opening the same syntax there would silently accept code that
# doesn't do what it promises, which is why `emerald-sema` deliberately
# does not.

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

readable: Counter = Counter.new(10)
report(readable)

# `mutable.value` is read directly (an ordinary method call, not
# through a `borrow`-typed parameter) rather than via a second call to
# `report` — reusing `report` here would record a `borrow` of
# `mutable` in the same flat scope `increment` already recorded a
# `borrow var` in, exactly the conflict this file's own header comment
# explains.
mutable: Counter = Counter.new(10)
increment(mutable)
puts mutable.value

owned: Counter = Counter.new(10)
finish(owned)

# A `borrow` of a BY-VALUE primitive — real, new plan 84 codegen: `x`
# arrives as an actual pointer into `n`'s own storage, never a copy
# (see this file's own header comment for the full explanation, and
# `emerald-codegen`'s test suite for the end-to-end proof, including
# the `borrow var` mutation case this file itself can't express yet).
fn describe(x: borrow Int64): Void do
  puts x
end

n: Int64 = 21
describe(n)

# A `borrow var` of a BY-VALUE primitive, actually mutated — the gap
# this file's own header comment describes closing 2026-09-21.
# `increment_primitive` reassigns its own parameter; that reassignment
# reaches the caller's `counter_value` through the real pointer/
# writeback mechanism `emerald-codegen` already built, not a local-only
# rebind — the printed value below is `42`, not `41`.
fn increment_primitive(n: borrow var Int64): Void do
  n = n + 1
end

counter_value: Int64 = 41
increment_primitive(counter_value)
puts counter_value
