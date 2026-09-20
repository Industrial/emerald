# Plan 76's `import-export-module-visibility`: explicit, checked
# `export`/`import` symbol-level visibility, alongside plan 23's
# file-based `require` (still used here for `greeter.em`, which
# exports everything `main.em` actually calls).
require greeter
import mathutils { add }

puts greet("Emerald")
puts add(3, 4)
