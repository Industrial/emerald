# Plan 145 (Process Spawning & Control) — `Process.run(cmd: String,
# args: Array[String], argc: Int64, stdin_data: String): ProcessResult`,
# backed by plain `std::process::Command` (zero third-party crate). A
# spawn failure (the executable doesn't exist) raises a real, catchable
# `NativeError`; a nonzero exit code is a plain field on the returned
# `ProcessResult`, never an error — `grep` finding no match is a
# routine, expected outcome, not a failure this plan's design treats as
# exceptional.
#
# Three real, disclosed deviations from this plan's own literal
# Concrete Proof text, found only by actually compiling and running
# this file:
#
# (1) `puts` does not accept a `Boolean` argument (`emerald-sema`'s own
# real, current `puts` intrinsic accepts only `Int64`/`Float64`/
# `String`) — `examples/control_flow.em`'s own precedent (`if flag do
# ... else ... end`) is this grammar's real way to print a `Boolean`,
# reused here for every `.success` this file prints.
#
# (2) `rescue => e` (a bare, classless rescue clause) is not this
# grammar's real syntax — every `rescue` names the class it catches
# (`examples/exceptions.em`'s own precedent); a spawn failure raises a
# real `NativeError` (plan 92's own mechanism, `crate::raise_native_
# error`), so this file catches it as `rescue NativeError => e`,
# `examples/resource_handle_lifetime_proof.em`'s own precedent.
#
# (3) `[]` (an empty array literal) is rejected outright by
# `emerald-sema` with a real compile diagnostic ("empty array literals
# are not supported — the element type can't be inferred") even in
# this call-argument position, where the callee's own declared
# `Array[String]` parameter type is already known — array-literal type
# inference here only ever looks at the literal's own elements, never
# an enclosing call's expected parameter type. This file's own third
# `Process.run` call passes a real, single-element placeholder array
# (`["unused"]`) with `argc` still `0`, so `emerald-rt`'s own `argc`-
# driven loop never actually reads it — proving the exact same spawn-
# failure path the plan's own `[]` text intended, without depending on
# an empty-array-literal inference this grammar does not have.

r: ProcessResult = Process.run("echo", ["hello", "from", "emerald"], 3, "")
puts r.stdout
puts r.exit_code
if r.success do
  puts "true"
else
  puts "false"
end

grep_result: ProcessResult = Process.run("grep", ["missing"], 1, "one\ntwo\nthree\n")
puts grep_result.exit_code
if grep_result.success do
  puts "true"
else
  puts "false"
end

begin
  bad: ProcessResult = Process.run("definitely-not-a-real-binary-xyz", ["unused"], 0, "")
  puts bad.exit_code
rescue NativeError => e
  puts "spawn failed"
end
