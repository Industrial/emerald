# Plan 147 (Temporary Files & Directories) — `Tempfile.create`/`.path`/
# `.close`, `Tempdir.create`/`.path`/`.close`, wrapping the pure-Rust
# `tempfile` crate. Creation failure (disk full, permissions) raises a
# real, catchable `NativeError` — the same plain-raise convention
# `File`'s own functions and plan 145's `Process.run` already use, not
# a `Result` return.
#
# One real, disclosed adaptation from the plan's own literal Concrete
# Proof text, found only by actually compiling it (the identical class
# of correction `extended_filesystem_proof.em`/`tar_roundtrip.em`/
# `process_spawning_proof.em` already disclose, not new to this plan):
# `puts` accepts only `Int64`/`Float64`/`String` — the plan's own
# `puts Path.exists(p)` (a bare `Boolean`) needs `"#{...}"` string
# interpolation instead, `tar_roundtrip.em`'s own precedent, reused
# verbatim.

tf: Tempfile = Tempfile.create()
path: String = tf.path()
File.write(path, "temporary contents")
back: String = File.read(path)
puts back
puts "#{Path.exists(path)}"

tf.close()
puts "#{Path.exists(path)}"

# Beyond the plan's own minimum Concrete Proof: `Tempdir`, the
# directory counterpart, proving a real, recursive `remove_dir_all` on
# `.close()` — a file nested inside the tempdir exists while open and
# is genuinely gone (not just the top-level directory entry, the
# specific claim this plan's own todo text asks be proven) once
# `.close()` runs.

td: Tempdir = Tempdir.create()
dir_path: String = td.path()
nested_path: String = "#{dir_path}/nested.txt"
File.write(nested_path, "nested contents")
puts "#{Path.exists(nested_path)}"

td.close()
puts "#{Path.exists(nested_path)}"
