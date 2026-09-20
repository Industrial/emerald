export fn add(a: Int64, b: Int64): Int64 do
  a + b
end

export fn mul(a: Int64, b: Int64): Int64 do
  a * b
end

# Not exported — invisible to any other file, whether it plainly
# `require`s this file or `import`s specific names from it (plan 76's
# `import-export-module-visibility`). `main.em` never references it.
fn internal_double(x: Int64): Int64 do
  x * 2
end
