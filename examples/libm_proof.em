# Plan 164 (Portable Math Functions) — `Math.sqrt`/`.pow`/`.sin`/`.cos`,
# a thin Float64-in-Float64-out wrapping of `libm` (the exact pure-Rust
# fallback `core`'s own float math already uses on targets with no
# OS-provided math library — portable to `wasm32-wasip1` with zero
# linker flag, unlike calling libc math via `extern "C"`).
#
# Real, disclosed prerequisite this plan's own text calls for directly:
# `Float64#is_nan` didn't exist in this compiler before this plan added
# it (verified directly — no other plan defines it), since `Math.sqrt`'s
# own domain-error convention (a real IEEE 754 `NaN`, matching libc's
# own `sqrt`, never a crash/`Result`) needs a way to check for it.

puts Math.sqrt(2.0)
puts Math.pow(2.0, 10.0)

nan_result: Float64 = Math.sqrt(-1.0)
is_nan: Boolean = nan_result.is_nan
puts "#{is_nan}"

identity: Float64 = Math.sin(1.0) * Math.sin(1.0) + Math.cos(1.0) * Math.cos(1.0)
puts identity
