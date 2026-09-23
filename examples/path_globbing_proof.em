# Plan 150 (Path Globbing) — `Glob.match`/`.match_count`, wrapping the
# `rust-lang`-owned `glob` crate for real shell-style `*`/`?`/`[...]`/
# `**` pattern matching directly against the filesystem, combined with
# plan 45's `File.write` to create the fixture files this proof reads
# back.
#
# Two real, disclosed findings from actually running this file (not
# assumed from this plan's own original text):
#
# (1) exactly `examples/extended_filesystem_proof.em`'s own finding
# (1) for plan 144 — `File.write("plan150_demo/nested/d.rs", ...)` is
# unrunnable exactly as the plan's own Concrete Proof writes it, since
# neither `plan150_demo/` nor `plan150_demo/nested/` is ever created
# first, and `File.write`'s own C implementation (`runtime/emerald_
# runtime.c`, plan 45, a bare `fopen(path, "wb")`) does not create
# missing parent directories. This example's own fixture directories
# are therefore created by the test harness itself (`crates/emerald-
# cli/tests/examples.rs`), the identical accommodation plan 144's own
# proof already established.
#
# (2) The plan's own literal method name, `Glob.match`/`.match_count`,
# is unparseable as written — `match` is a grammar-reserved keyword
# (`"match" <scrutinee:CondExpr> "do" ... "end"`, `grammar.lalrpop`'s
# own pattern-matching `Stmt::Case` production), confirmed directly by
# actually compiling this plan's own worked example, not assumed —
# the exact same category of collision `examples/crypto_hashing_
# proof.em`'s own finding (2) already discloses for plan 109's own
# `Sha256.new()` (also unparseable, `new` also reserved), resolved
# there by renaming to `Sha256.hasher()` rather than widening the
# grammar's own `CallMethodName` production (only ever widened once,
# for plan 45's own `.read`, and never again since — the repo's own
# real, established precedent for every subsequent collision is to
# rename, not to keep widening the grammar). This plan follows that
# same, more-established precedent: `Glob.match`/`.match_count` are
# `Glob.glob`/`.glob_count` here instead, mirroring the `glob` crate's
# own `glob::glob` function name directly.
#
# Otherwise this is the plan's own Concrete Proof verbatim: `b.txt` is
# written before `a.txt` (proving `Glob.glob`'s own sort-before-
# returning behavior, not an accident of directory-read order), a
# single `*` in `plan150_demo/*.txt` reaches only the two top-level
# `.txt` files (never `nested/c.txt` — one directory too deep — and
# never `nested/d.rs` regardless, wrong extension), and `**` in
# `plan150_demo/**/*.rs` reaches into `nested/` a single `*` could not.

File.write("plan150_demo/b.txt", "b")
File.write("plan150_demo/a.txt", "a")
File.write("plan150_demo/nested/c.txt", "c")
File.write("plan150_demo/nested/d.rs", "d")

top: Array[String] = Glob.glob("plan150_demo/*.txt")
n: Int64 = Glob.glob_count("plan150_demo/*.txt")
puts n
puts top[0]
puts top[1]

deep: Array[String] = Glob.glob("plan150_demo/**/*.rs")
m: Int64 = Glob.glob_count("plan150_demo/**/*.rs")
puts m
puts deep[0]
