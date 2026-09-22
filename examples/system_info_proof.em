# Plan 152 (System Information) — a `System` compiler-known namespace
# over `sysinfo`: read-only CPU/memory/disk/process introspection,
# full stop. Every assertion here is relative/derived (`> 0`, `>=`)
# rather than a literal machine-specific number, deliberately — a real
# CPU count/memory total/disk count/process count varies by machine
# and CI environment, and is not itself the property this proof needs
# to establish.
#
# Real, disclosed correction: `puts` accepts only `Int64`/`Float64`/
# `String` — a bare `Boolean` needs string interpolation (`"#{...}"`,
# which does support it) to print.

cpus: Int64 = System.cpu_count
cpus_ok: Boolean = cpus > 0
puts "#{cpus_ok}"

total: Int64 = System.total_memory_bytes
used: Int64 = System.used_memory_bytes
total_ok: Boolean = total >= used
puts "#{total_ok}"
used_ok: Boolean = used >= 0
puts "#{used_ok}"

n: Int64 = System.disk_names_count
n_ok: Boolean = n >= 0
puts "#{n_ok}"

pids: Array[Int64] = System.process_ids
m: Int64 = System.process_ids_count
m_ok: Boolean = m > 0
puts "#{m_ok}"

name: Option[String] = System.process_name(pids[0])
match name do
Some(pname) do
  has_name: Boolean = pname.length > 0
  puts "#{has_name}"
end
None do
  puts "unknown"
end
end
