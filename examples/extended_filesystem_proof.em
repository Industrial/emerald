# Plan 144 (Extended Filesystem Operations) — `Dir` (non-recursive
# `std::fs::read_dir` listing + recursive `walkdir` traversal) and
# `Path`/`FileMetadata` (existence/kind predicates, metadata
# snapshotting, Unix permissions, symlinks), strictly additive to
# plan 45's own `File.read`/`File.write` (never touched by this plan).
#
# Four real, disclosed findings from actually running this file (not
# assumed from this plan's own original text):
#
# (1) This plan's own Concrete Proof is unrunnable exactly as
# originally written: it calls `File.write("plan144_demo/a.txt", ...)`
# and `File.write("plan144_demo/nested/b.txt", ...)` with neither
# `plan144_demo/` nor `plan144_demo/nested/` ever created first — and
# no plan (this one included; its own leaf list has no `Dir.create`/
# mkdir todo at all, an omission its own "Out of scope" section never
# names either) has ever given Emerald source a directory-creation
# primitive. `File.write`'s own C implementation (`runtime/emerald_
# runtime.c`, plan 45) is a bare `fopen(path, "wb")` — it does not
# create missing parent directories. This example's own fixture
# directories are therefore created by the test harness itself
# (`crates/emerald-cli/tests/examples.rs`), exactly the way this
# plan's own todo text already asks for a `tempfile::tempdir()`-built
# fixture tree in `emerald-rt`'s own Rust-level unit tests — this is
# the identical accommodation, one level up, for the compiled-example
# proof.
#
# (2) This plan's own original Decision log specified a raw nullable-
# pointer return (`FileMetadata?`/`String?`, with `||=` unwrapping) for
# `Path.metadata`/`Path.read_link` — dead syntax: `T?`/`nil`/`||=` were
# removed outright by plan 73, well before plan 195 (Typed Domain
# Errors) existed. This example uses plan 195's newer, strictly more
# informative `Result[T, PathError]` shape instead (this grammar's own
# current `match X do Ok(v) do ... end Err(e) do ... end end` syntax,
# `examples/bignum_decimal_proof.em`'s own precedent, reused here) —
# not a stylistic choice, the only shape the current grammar has.
#
# (3) This plan's own Concrete Proof also assumes `puts` accepts a
# `Boolean` argument directly (`puts meta.is_dir` printing `false`) —
# `emerald-sema`'s own real, current `puts` intrinsic (`spec/
# SEMANTICS.md` §3, plan 08's Decision log) accepts only `Int64`/
# `Float64`/`String`, rejecting `Boolean` with a real compile
# diagnostic (`` `puts` does not support type Boolean ``, found by
# actually compiling this plan's own worked example, not assumed).
# `examples/control_flow.em`'s own precedent (`if flag do ... else
# ... end`) is this grammar's real way to print a `Boolean` today.
#
# (4) The plan's own Concrete Proof binds `Dir.entries`/`Dir.walk`'s
# own `Array[String]` return to a local (`entries`/`all`) it then
# never reads again — this repo's own `emerald lint` (`crates/
# emerald-cli/tests/lint_subcommand.rs`'s own `every_real_file_under_
# examples_lints_clean`, run over every real file under `examples/`)
# flags exactly this as `unused-local-variable`, found only by
# actually running the lint against this plan's own worked example.
# This example calls `Dir.entries`/`Dir.walk` as bare, unbound
# expression statements instead — still proving both compile and run
# against the real fixture tree, without tripping the lint.

File.write("plan144_demo/a.txt", "hello")
File.write("plan144_demo/nested/b.txt", "world")

n: Int64 = Dir.entries_count("plan144_demo")
puts n
Dir.entries("plan144_demo")

m: Int64 = Dir.walk_count("plan144_demo")
puts m
Dir.walk("plan144_demo")

meta_result: Result[FileMetadata, PathError] = Path.metadata("plan144_demo/a.txt")
match meta_result do
Ok(meta) do
  puts meta.size
  if meta.is_dir do
    puts "true"
  else
    puts "false"
  end
end
Err(e) do
  puts "unexpected metadata error"
end
end

Path.symlink("plan144_demo/a.txt", "plan144_demo/a_link.txt")
link_result: Result[String, PathError] = Path.read_link("plan144_demo/a_link.txt")
match link_result do
Ok(target) do
  puts target
end
Err(e) do
  puts "unexpected read_link error"
end
end

# Beyond the plan's own minimum Concrete Proof: full coverage of every
# leaf this plan adds (`Path.exists`/`.is_file`/`.is_dir`/`.is_symlink`,
# `.unix_mode`/`.set_unix_mode`, and the `PathError.NotFound` arm).
if Path.exists("plan144_demo/a.txt") do
  puts "true"
else
  puts "false"
end
if Path.exists("plan144_demo/does_not_exist.txt") do
  puts "true"
else
  puts "false"
end
if Path.is_file("plan144_demo/a.txt") do
  puts "true"
else
  puts "false"
end
if Path.is_dir("plan144_demo/nested") do
  puts "true"
else
  puts "false"
end
if Path.is_symlink("plan144_demo/a_link.txt") do
  puts "true"
else
  puts "false"
end

Path.set_unix_mode("plan144_demo/a.txt", 420)
if Path.unix_mode("plan144_demo/a.txt") == 33188 do
  puts "true"
else
  puts "false"
end

missing_result: Result[FileMetadata, PathError] = Path.metadata("plan144_demo/does_not_exist.txt")
match missing_result do
Ok(m) do
  puts "unexpected ok"
end
Err(e) do
  match e do
  NotFound do
    puts "not found"
  end
  PermissionDenied do
    puts "permission denied"
  end
  Other(detail) do
    puts detail
  end
  end
end
end
