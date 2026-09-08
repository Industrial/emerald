# grammar/

Emerald's formal grammar file lives at
[`crates/emerald-parser/src/grammar.lalrpop`](../crates/emerald-parser/src/grammar.lalrpop)
— LALRPOP was chosen over Chumsky in plan `02 toolchain-prototype` (see
[`spec/COMPILER.md`](../spec/COMPILER.md) for the decision record), and
LALRPOP grammar files conventionally live alongside the crate that consumes
them rather than in a separate top-level directory.

This directory exists as the documented, discoverable pointer to that file
rather than duplicating or relocating it. See
[`spec/GRAMMAR.md`](../spec/GRAMMAR.md) for the prose grammar inventory the
`.lalrpop` file implements a growing subset of.
