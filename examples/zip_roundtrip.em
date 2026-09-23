# Plan 133 (Zip Archives) — `Zip.create`/`.extract`, `ZipReader.open`/
# `.entry_count`/`.entry_name`/`.entry_size`/`.read_entry_data`/
# `.close`, wrapping the pure-Rust-only `zip` crate (`deflate-flate2-
# zlib-rs` feature — see this plan's own Decision log).
#
# Real, disclosed adaptations from the plan's own literal Concrete
# Proof text, found only by checking this session's own real, already-
# shipped surface before writing this example (the same class of
# correction plan 132's own `tar_roundtrip.em` already discloses, not
# new to this plan):
#
# 1. `Dir.mkdir` does not exist at this plan's own EXECUTE time — plan
#    144 (Extended Filesystem), landed earlier this session, added
#    only `.entries`/`.entries_count`/`.walk`/`.walk_count`, never a
#    `.mkdir` (real-checked directly against `dir.rs` before writing
#    this example, the identical correction `tar_roundtrip.em` already
#    makes). This example archives two plain files written directly in
#    the working directory instead of inside a subdirectory first —
#    `Zip.create`'s own real behavior (archive member paths exactly as
#    given, member order exactly caller order) is identical either way.
# 2. `Bytes` (plan 109) exposes exactly one instance method today,
#    `.to_hex(): String` — no `.to_string()`. Every `reader.read_
#    entry_data(i)` below is therefore printed via `.to_hex()`, the
#    identical substitution `gzip_roundtrip.em`/`tar_roundtrip.em`
#    already establish (real content is still proven: two files with
#    different content hex-encode to different strings, and the
#    restored file's own content is separately verified byte-for-byte
#    via `File.read` below, never just re-hashed).
# 3. `puts` accepts only `Int64`/`Float64`/`String` — the final
#    `Boolean` comparison needs `"#{...}"` string interpolation to
#    print, not a bare `puts`.
# 4. `ZipReader` is index-based (`entry_count`/`entry_name(i)`/`.entry_
#    size(i)`/`.read_entry_data(i)`), not iterator-advance-only like
#    `TarReader` — no `Option[String]`/`match` needed at all here, a
#    real, format-driven API difference (see this plan's own Decision
#    log), not an adaptation forced by a missing feature.

File.write("zip_demo_a.txt", "hello")
File.write("zip_demo_b.txt", "world, a longer second file that compresses well well well well")

paths: Array[String] = ["zip_demo_a.txt", "zip_demo_b.txt"]
Zip.create("demo.zip", paths)

reader: ZipReader = ZipReader.open("demo.zip")
n: Int64 = reader.entry_count()
puts n
var i: Int64 = 0
while i < n do
  name: String = reader.entry_name(i)
  puts name
  data: Bytes = reader.read_entry_data(i)
  puts data.to_hex()
  i += 1
end
reader.close()

Zip.extract("demo.zip", "zip_demo_out")
restored: String = File.read("zip_demo_out/zip_demo_a.txt")
expected: String = "hello"
puts "#{restored == expected}"
