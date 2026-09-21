# Regression fixture for the plan 69/76 gap: a file using ONLY
# `import` (zero bare `require`) must still route through the real
# multi-file resolver via the plain `emerald <file>.em -o out` CLI
# form (no `--jobs`). Before the fix, `run_legacy`'s `requires_present`
# check in `crates/emerald-cli/src/main.rs` only matched
# `Item::Require`, so an import-only file like this one silently
# skipped multi-file resolution and failed with a confusing
# "undefined function" error instead of correctly resolving `add`/
# `mul` from `mathutils.em`.
import mathutils { add, mul }

puts add(3, 4)
puts mul(3, 4)
