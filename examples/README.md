# Examples

Every file here is real, working Emerald source, verified this session by
compiling it with `emerald-cli`, running the resulting binary, and
checking its exact stdout. Together they exercise every grammar
construct and AST node this compiler currently supports. Each file
compiles and runs standalone:

```bash
cargo run -p emerald-cli -- examples/<file>.em -o /tmp/out && /tmp/out
```

| File | Covers | Verified output |
|---|---|---|
| `hello.em` | `def`/return type, `Call`, `puts`, `Add` | `42` |
| `control_flow.em` | all 6 `CompareOp`s (`<` `>` `<=` `>=` `==` `!=`), `if`/`else`, `while`, `break`, `next`, `return`, nested `if` inside `while` | `1×6`, `0`,`1`,`3`, `0`,`1`,`2` |
| `classes.em` | `class`, fields, `initialize`, `@field` read/write (`InstanceVar`/`SetField`), `.new`, zero-arg and one-arg method calls, `Float64` fields | `10`, `15`, `5` |
| `collections.em` | `Array[Int64]`/`Array[Float64]` literals (`ArrayLit`), indexed read/write (`Index`/`SetIndex`) | `60`, `99`, `4` |
| `closures.em` | `Proc` type, lambda literals (`Lambda`) with and without a captured variable, `.call` | `15`, `42` |
| `exceptions.em` | `raise`, `begin`/`rescue` (`Raise`/`Begin`) on both the exception and non-exception path | `99`, `5` |
| `modules.em` | `module`, namespaced static method calls (`Name.method(args)`) | `42` |

## AST coverage

Every variant of `emerald_parser::ast::{Expr, Stmt, Item}` is exercised
somewhere above:

- `Expr`: `Ident`, `Int`, `Float`, `Add`, `Compare`, `Call`, `New`,
  `MethodCall` (zero-arg and multi-arg, on a class instance, a module,
  and a `Proc`), `InstanceVar`, `ArrayLit`, `Index`, `Lambda`.
- `Stmt`: `Let`, `SetField`, `SetIndex`, `If` (with and without `else`),
  `While`, `Return`, `Break`, `Next`, `Expr`, `Raise`, `Begin`.
- `Item`: `Function`, `Class`, `Module`, `Stmt`.

## What isn't covered, and why

These aren't missing example coverage — they're constructs that don't
exist in the language yet, confirmed while writing `benchmarks/` (plan
15's Decision log):

- **No `-` or `*` operator at all** (not even unary minus) — only `+`
  and comparisons exist. No example uses a negative number or
  multiplication.
- **No string literal syntax** — `Type::String` exists in the type
  system, but nothing can construct a `String` value, so no example
  declares one.
- **A `Boolean`-typed `Let` binding** (e.g. `flag: Boolean = a < b`,
  storing a comparison's result instead of using it directly in an
  `if`/`while` condition) is grammar-legal but was never exercised by
  any prior plan's tests and isn't included here — untested, not
  confirmed working, and out of scope to newly verify without doing
  actual codegen work.
