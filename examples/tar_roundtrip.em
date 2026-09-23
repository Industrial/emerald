# Plan 132 (Tar Archives) — `Tar.create`/`.extract`, `TarReader.open`/
# `.next_entry`/`.entry_size`/`.read_entry_data`/`.close`, wrapping the
# pure-Rust `tar` crate.
#
# Real, disclosed adaptations from the plan's own literal Concrete
# Proof text, found only by checking this session's own real, already-
# shipped surface before writing this example (the same class of
# correction plan 130's own `gzip_roundtrip.em`/plan 146's own
# `environment_variables_proof.em` already disclose, not new to this
# plan):
#
# 1. `Dir.mkdir` does not exist at this plan's own EXECUTE time — plan
#    144 (Extended Filesystem), which would add it, is a separate,
#    still-unmerged plan this session, not a real prerequisite this
#    plan's own text could actually rely on despite assuming it. This
#    example archives two plain files written directly in the working
#    directory instead of inside a subdirectory first — `Tar.create`'s
#    own real behavior (archive member paths exactly as given, member
#    order exactly caller order) is identical either way.
# 2. `match`/`Option[T]` pattern syntax is `match expr do / Variant(
#    binding) do ... end / end`, not the plan's own Ruby-flavored
#    `when` keyword (plan 73's real, shipped grammar — the identical
#    correction `environment_variables_proof.em`/`nullable_safe_nav.em`
#    already establish). `match`'s own scrutinee must itself be a
#    plain, already-bound local (`emerald-codegen`'s own enum-case fast
#    path only recognizes `Expr::Ident`), not `reader.next_entry()`
#    called inline — every result below is bound to a local first, the
#    identical shape `environment_variables_proof.em`'s own `found`/
#    `gone` locals already establish.
# 3. `Bytes` (plan 109) exposes exactly one instance method today,
#    `.to_hex(): String` — no `.to_string()`. Every `reader.read_
#    entry_data()` below is therefore printed via `.to_hex()`, the
#    identical substitution `gzip_roundtrip.em` already establishes
#    (real content is still proven: two files with different content
#    hex-encode to different strings, and the restored file's own
#    content is separately verified byte-for-byte via `File.read`
#    below, never just re-hashed).
# 4. `puts` accepts only `Int64`/`Float64`/`String` — the final
#    `Boolean` comparison needs `"#{...}"` string interpolation to
#    print, not a bare `puts`.

File.write("tar_demo_a.txt", "hello")
File.write("tar_demo_b.txt", "world, a longer second file")

paths: Array[String] = ["tar_demo_a.txt", "tar_demo_b.txt"]
Tar.create("demo.tar", paths)

reader: TarReader = TarReader.open("demo.tar")

entry1: Option[String] = reader.next_entry()
match entry1 do
  Some(name1) do
    puts name1
    data1: Bytes = reader.read_entry_data()
    puts data1.to_hex()
  end
  None do
    puts "unexpected end"
  end
end

entry2: Option[String] = reader.next_entry()
match entry2 do
  Some(name2) do
    puts name2
    data2: Bytes = reader.read_entry_data()
    puts data2.to_hex()
  end
  None do
    puts "unexpected end"
  end
end

entry3: Option[String] = reader.next_entry()
match entry3 do
  Some(name3) do
    puts name3
  end
  None do
    puts "end"
  end
end
reader.close()

Tar.extract("demo.tar", "tar_demo_out")
restored: String = File.read("tar_demo_out/tar_demo_a.txt")
expected: String = "hello"
puts "#{restored == expected}"
