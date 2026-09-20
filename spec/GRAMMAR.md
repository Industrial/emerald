# Emerald — GRAMMAR.md

**Status:** v1 grammar inventory
**Derived from:** Ruby's syntax (see inception §2.1 references) as an
inventory, not a wholesale import.

This document lists every grammar area inception §5 puts in scope, tags it
`KEEP`, `MODIFY`, `REMOVE`, or `UNDECIDED`, and gives one Emerald-syntax
example per `KEEP`/`MODIFY` area. `UNDECIDED` entries link to the matching
question in [`SEMANTICS.md`](./SEMANTICS.md) — nothing is left open without a
named place it gets resolved.

Per inception §2.2, nothing here is imported from Rust, Java, or C++ syntax;
every `MODIFY` is the smallest change to Ruby's surface that static typing
requires (mainly: type annotations on parameters/returns/fields/locals).

## Format

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|

---

## 1. Literals

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Integer literals (`42`, `0x2a`, `0b101010`) | KEEP | Same lexical forms. Literal's static type is inferred (see [`TYPE_SYSTEM.md` §6](./TYPE_SYSTEM.md#6-literal-typing)); no suffix required for the common case. |
| Float literals (`3.14`, `1e10`) | KEEP | Same lexical forms; type `Float64` unless annotated otherwise. |
| String literals (`"..."`, `'...'`) | KEEP | Interpolation (`"#{expr}"`) kept — it is static string concatenation, not `eval`. |
| Symbol literals (`:foo`) | KEEP | Kept as a distinct interned `Symbol` type; see `TYPE_SYSTEM.md`. |
| Array literals (`[1, 2, 3]`) | MODIFY | Element type is unified/checked statically at the literal site; a heterogeneous literal is a type error, not a runtime `Array` of mixed objects. |
| Hash literals (`{a: 1}`) | MODIFY | Same static unification as arrays, over key and value types independently. |
| `nil` | REMOVED (Sable) | Was: the single value of type `Nil`; see `SEMANTICS.md` §2's own superseded banner. Plan 73 removes `nil`/`Nil`/`T?` outright in favor of `Option[T]`'s `Some(T)`/`None` — write `None`, not `nil`. |
| `true` / `false` | KEEP | Values of `Boolean`. |
| Range literals (`1..10`, `1...10`) | KEEP | Statically typed as `Range[T]` where `T` is the endpoint type; container type is added to the v1 universe (see `TYPE_SYSTEM.md`). |
| `%w[]`, `%i[]` word/symbol arrays | KEEP | Sugar over `Array[String]` / `Array[Symbol]` literals; no new semantics. |
| Regexp literals (`/.../`) | UNDECIDED | Needs a `Regexp` runtime type decision — deferred to the `09 collections`/stdlib plan; not required for the v1 milestone slices in inception §17. |
| Heredocs (`<<~TEXT`) | KEEP | Same lexical form; produces a `String` literal. |

**Example (KEEP: array literal, statically unified):**
```ruby
xs: Array[Int64] = [1, 2, 3]
```

---

## 2. Identifiers, Variables, Constants

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Local variable identifiers (`foo`) | KEEP | Same lexical rule (lowercase/underscore start). |
| Instance variable identifiers (`@foo`) | KEEP | Must correspond to a field declared on the enclosing class (`SEMANTICS.md` §3 Classes) — undeclared `@foo` is a compile error, not silent `nil`. |
| Class variable identifiers (`@@foo`) | REMOVE | Ruby's `@@` semantics (shared, inheritance-crossing, easy to misuse) add complexity inception §22 rule 9 warns against; a class-level field declared statically covers the legitimate use case. |
| Global variables (`$foo`) | REMOVE | Encourages implicit, non-local state; not part of the static-analysis-friendly core inception §3 requires. |
| Constant identifiers (`Foo`, `FOO`) | KEEP | See `SEMANTICS.md` §1 (Variables) for mutability rules. |
| Local variable declaration with type annotation (`x: Int64 = 0`) | MODIFY | New required-at-first-use surface form; Ruby has no equivalent. This is the one syntax addition inception §6 anticipates. |
| Local variable declaration without annotation (`x = 0`) | REMOVED | Settled, not merely decided: the Sable-alignment grammar cutover (plan 71) made an explicit type annotation mandatory on every local binding — a bare, untyped `x = 0` is a real parse/sema error, not an inference case. |
| Mutable local declaration (`var x: Int64 = 0`) | MODIFY (Sable) | A binding declared without `var` is immutable — reassigning it is a compile error (`"cannot reassign immutable binding..."`). `var` opts a binding into plan 31's existing free-reassignment behavior. Applies to plain locals only: class fields (`@x`) and function/loop parameters have no `var` form and follow their own, separate mutability rules — see `SEMANTICS.md` §1. Added by the Sable-alignment grammar cutover (plan 72), not part of the original inception design. |

**Example (MODIFY: typed local declaration, immutable by default):**
```ruby
total: Int64 = 0
var count: Int64 = 0
count += 1
```

---

## 3. Assignment

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Simple assignment (`x = expr`) | KEEP | Requires `x` already declared with a compatible type, or is the declaring occurrence when annotated; also requires `x` to have been declared `var` (plan 72) — see `SEMANTICS.md` §1. |
| Compound assignment (`x += 1`, etc.) | KEEP | Desugars to `x = x + 1` under the receiver's statically resolved `+` method; same `var` requirement as simple assignment. |
| Multiple assignment (`a, b = 1, 2`) | KEEP | Each target's type is checked against its corresponding source expression's type positionally; no splat-driven arity magic. |
| Splat in multiple assignment (`a, *b = [1, 2, 3]`) | UNDECIDED | Requires `b: Array[T]` typing rules for the captured remainder — deferred to `SEMANTICS.md` §6 (Arrays) once `09 collections` lands; not needed for the v1 milestone in inception §17. |
| Parallel/nested destructuring (`(a, b), c = [[1, 2], 3]`) | REMOVE | High grammar/type-inference cost for a rarely-essential feature; not in inception §5's initial keep list. |
| Conditional assignment (`x ||= expr`) | KEEP | Requires `x`'s type to already admit `Nil` at the point of use — ties to `SEMANTICS.md` §2 (Nil). |

**Example (KEEP: compound assignment):**
```ruby
total += x * x
```

---

## 4. Operators & Precedence

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Arithmetic operators (`+ - * / %`) | KEEP | Statically resolved to the receiver's declared operator method; see `SEMANTICS.md` §8 (Numeric types) for conversion rules. |
| Comparison operators (`== != < > <= >=`) | KEEP | Same static-resolution rule. |
| Logical operators (`&& \|\| !`, `and or not`) | KEEP | `and`/`or`/`not` kept for readability parity with Ruby; same short-circuit semantics. |
| Ternary (`cond ? a : b`) | KEEP | Both branches must unify to one static type. |
| Spaceship (`<=>`) | KEEP | Statically resolved like other operators; return type fixed to a three-way ordering type. |
| Range operators (`.. ...`) | KEEP | See Literals above. |
| Bitwise operators (`& \| ^ ~ << >>`) | KEEP | Defined on integer types only. |
| Operator precedence table | KEEP | Ruby's precedence table is adopted unchanged — no motivation to diverge (inception §2.2). |
| Safe navigation (`&.`) | KEEP | Only legal on a receiver whose type admits `Nil`; result type is the wrapped method's return type unioned with `Nil`. Directly serves `SEMANTICS.md` §2. |
| Method-as-operator overloading (defining `+` etc. on a class) | KEEP | Statically dispatched like any other method — see `SEMANTICS.md` §3 (Classes). |

**Example (KEEP: safe navigation, typed):**
```ruby
name: String? = user&.name
```

---

## 5. Control Expressions

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `if` / `elsif` / `else` / `unless` | KEEP | Condition must be `Boolean` (no Ruby-style truthy/falsy coercion of arbitrary objects — see `SEMANTICS.md` §1). |
| `if`/`unless` as expressions (returning a value) | KEEP | Both/all branches unify to one static type, matching inception §17's second milestone slice. |
| Modifier `if`/`unless` (`stmt if cond`) | KEEP | Sugar over the block form; no new semantics. |
| `while` / `until` (+ modifier forms) | KEEP | Same `Boolean`-only condition rule as `if`. |
| `for ... in ...` | KEEP | Iterates a value of a type implementing the static iteration protocol (`Array[T]`, `Range[T]`, ...); no duck-typed `each`. |
| `case` / `when` (value match) | KEEP | `when` clauses are compared via the statically resolved `===`/`==` on the scrutinee's type. |
| `case` / `in` (pattern match) | KEEP, scope-limited | Basic pattern matching only (literal, array, and simple binding patterns) per inception §5; deconstruction protocols (`deconstruct`/`deconstruct_keys`) are `UNDECIDED`, tracked in `SEMANTICS.md` §3. |
| `begin ... end while` (do-while) | KEEP | Same condition-typing rule. |
| `return` | KEEP | Return expression's type must match the enclosing method's declared return type. |
| `break` / `next` | KEEP | `break value` must unify with the loop's static result type when the loop is used as an expression. |
| `redo` | REMOVE | Rarely used, adds control-flow analysis complexity disproportionate to value; not in inception §5's keep list. |
| `throw` / `catch` (non-exception control transfer) | REMOVE | Ruby's `throw`/`catch` is a second, parallel non-local-exit mechanism alongside exceptions; `SEMANTICS.md` §7 covers exceptions as the one mechanism Emerald keeps. |

**Example (KEEP: `if` as a typed expression):**
```ruby
label: String = if x > 5 do
  "big"
else
  "small"
end
```

---

## 6. Method Definitions

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `fn name(params): T do ... end` | MODIFY | Every parameter requires a type annotation; the method requires an explicit return-type annotation, a trailing `: T` (superseded from `-> T` by the Sable-alignment grammar cutover, `history/2026-09-19T110000Z-plan-71-grammar-unification-fn-and-do-end.md`). This is inception §6's headline example, in its current spelling. |
| Default parameter values (`def f(x = 1)`) | KEEP | Default expression's type must match the parameter's declared type. |
| Keyword parameters (`def f(x:, y: 1)`) | KEEP | Same annotation requirement as positional parameters; see `SEMANTICS.md` §3 (Methods) for whether they're retained project-wide. |
| Splat parameters (`def f(*xs)`) | UNDECIDED | Requires deciding `xs`'s static element type and arity checking — tracked in `SEMANTICS.md` §3; not required for inception §17's first three milestones. |
| Double-splat parameters (`def f(**opts)`) | UNDECIDED | Same as above, tracked alongside splat. |
| Block parameter (`def f(&blk)`) | KEEP, scope-limited | Legal only when the block's parameter/return types are statically known at the call site — see `SEMANTICS.md` §5 (Blocks). |
| Method overloading (multiple `def` for one name, different signatures) | UNDECIDED | `SEMANTICS.md` §3 question: "Are methods overloaded?" — answered there. |
| `def self.name` (singleton/class methods) | KEEP | Statically resolved against the class's metatype. |
| Method visibility (`private`, `protected`, `public`) | KEEP | Enforced at compile time as a static access check, not a runtime `NoMethodError`. |
| `define_method` | REMOVE | Explicitly out of scope, inception §5/§20 — runtime method creation. |
| `method_missing` | REMOVE | Explicitly out of scope, inception §5/§20. |
| `alias` / `alias_method` | REMOVE | Runtime aliasing, inception §5/§20. |

**Example (MODIFY: typed method definition, inception §6's own example):**
```ruby
fn add(a: Int64, b: Int64): Int64 do
  a + b
end
```

---

## 7. Method Calls

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Ordinary call (`obj.method(args)`) | KEEP | Receiver's static type must declare the method; no duck typing (inception §9). |
| Parenthesis-less call (`obj.method arg`) | KEEP | Same static resolution; purely a lexical variant. |
| Command call (`puts x`) | KEEP | Resolved as a call on an implicit top-level/kernel receiver with a statically known signature. |
| Block argument (`method { |x| ... }`, `method do |x| ... end`) | KEEP | See `SEMANTICS.md` §5 (Blocks). |
| `yield` | KEEP | Legal only inside a method whose block parameter type is statically known. |
| Splat call arguments (`method(*args)`) | UNDECIDED | Tied to the splat-parameter decision above; tracked together in `SEMANTICS.md` §3. |
| `send` / `public_send` | REMOVE | Reflective/dynamic dispatch, inception §5/§20. |
| `method(:name)` (Method objects) | REMOVE | Reflection surface, inception §20. |
| Operator-call sugar (`a + b` as `a.+(b)`) | KEEP | Same statically resolved rule as any other call; see Operators above. |

**Example (KEEP: block argument call):**
```ruby
xs.each do |x: Int64| puts x end
```

---

## 8. Classes & Modules

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `class Name ... end` | KEEP | Matches inception §8's `Point`/`Animal` examples exactly. |
| Single inheritance (`class Dog < Animal`) | KEEP | Statically known ancestor chain, per inception §8. |
| Multiple inheritance | REMOVE | `SEMANTICS.md` §3 records this as a locked decision (not merely "undecided"), consistent with inception §19's Classes question. |
| Instance field declarations (`x: Float64` inside a class body) | MODIFY | New required surface form — Ruby has no static field declaration; matches inception §6's `Point` example. |
| `initialize` | KEEP | Ordinary method with the typed-parameter rules from §6 above; still called via `.new`. |
| `module Name ... end` | KEEP, scope-limited | Kept only where its static semantics are straightforward (inception §5); `SEMANTICS.md` §10 fixes exactly which Ruby module behaviors survive. |
| `include` (mixin) | UNDECIDED | `SEMANTICS.md` §10 question, directly from inception §19 (Modules). |
| `extend` | UNDECIDED | Same as `include`, tracked together. |
| Refinements (`using`, `refine`) | REMOVE | Scoped monkey patching; excluded by inception §20. |
| Open classes / reopening (`class String; ...; end` on a builtin) | REMOVE | Monkey patching, inception §5/§20. |
| `class_eval` / `instance_eval` | REMOVE | Explicitly out of scope, inception §5/§20. |
| Struct-style value types (`struct Foo`) | KEEP | Named in inception §7's user-defined type list alongside `class`; exact field/mutability rules land in `TYPE_SYSTEM.md`. |

**Example (MODIFY: typed class body, inception §6's own example):**
```ruby
class Point
  x: Float64
  y: Float64

  fn initialize(x: Float64, y: Float64): Void do
    @x = x
    @y = y
  end
end
```

---

## 9. Blocks, Procs, Lambdas

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `{ |x| ... }` / `do |x| ... end` block literal | KEEP, scope-limited | Kept "if they can be represented cleanly" per inception §5; block parameter types must be statically inferable from the call site. |
| `->(x) -> T { ... }` lambda literal | SUPERSEDED | Deleted outright by the Sable-alignment grammar cutover (plan 71) — a lambda is now a bare `do |x: T| ... end` block used directly as an expression, typed by context, with no arrow anywhere; see `history/2026-09-19T110000Z-plan-71-grammar-unification-fn-and-do-end.md`. |
| `Proc.new { ... }` | REMOVE | Redundant with lambda literal syntax once procs are statically typed; one closure literal form is simpler (inception §22 rule 3/10). |
| `proc { ... }` | REMOVE | Same reasoning as `Proc.new`. |
| Block-local variables (`{ |x; y| ... }`) | KEEP | No new semantics beyond ordinary scoping. |
| Closures capturing outer locals | KEEP | See `SEMANTICS.md` §5 for escape-analysis and heap-allocation-when-necessary rules (inception §12, §19). |

**Example (KEEP: typed block):**
```ruby
total: Int64 = xs.reduce(0) do |acc: Int64, x: Int64| acc + x end
```

---

## 10. Exceptions

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `begin ... rescue ... else ... ensure ... end` | KEEP | Full clause set retained; matches inception §5's keep list. |
| `rescue ExceptionType => e` | MODIFY | `e`'s static type is the declared `ExceptionType` (or its narrowest common ancestor across multiple `rescue` clauses), not `StandardError`-typed dynamically. |
| Bare `rescue` (implicit `StandardError`) | KEEP | Same implicit-class convention as Ruby. |
| `raise` / `raise ExceptionClass, "msg"` | KEEP | See `SEMANTICS.md` §7 for whether raised types must be statically declared on the method signature. |
| `retry` | KEEP | No new semantics. |
| Modifier `rescue` (`expr rescue default`) | KEEP | `default`'s type must unify with `expr`'s type. |
| Custom exception classes (`class MyError < StandardError`) | KEEP | Ordinary class inheritance, per §8 above. |

**Example (MODIFY: typed rescue clause):**
```ruby
begin
  risky_call
rescue DivisionError => e
  puts e.message
end
```

---

## 11. Pattern Matching

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `case expr; in pattern; end` | KEEP, scope-limited | "Basic pattern matching" per inception §5; see §5 Control Expressions above for the exact scope line. |
| Literal patterns (`in 1`, `in "foo"`) | KEEP | Compared via the scrutinee's static type. |
| Array patterns (`in [a, b]`) | KEEP | Element types must unify with the scrutinee's declared element type. |
| Binding patterns (`in x`) | KEEP | Binds `x` with the statically inferred type from the matched position. |
| Guard clauses (`in x if x > 0`) | KEEP | Guard must be `Boolean`. |
| Hash/find patterns (`in {a:, b:}`, `in [*, x, *]`) | UNDECIDED | Beyond "basic" per inception §5; tracked in `SEMANTICS.md` §3 alongside `deconstruct`/`deconstruct_keys`. |
| One-line pattern matching (`expr => pattern`, `expr in pattern`) | REMOVE | Redundant surface form once `case/in` exists; trims grammar surface per inception §22 rule 10. |

**Example (KEEP: basic array pattern):**
```ruby
case point
in [x, y]
  puts x + y
end
```

---

## 12. Comments

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `# line comment` | KEEP | Unchanged. |
| `=begin ... =end` block comment | KEEP | Unchanged. |

---

## 13. Explicitly Removed (inception §5/§20, verbatim inventory)

Every item inception §5 lists under "Remove from the initial language" is
accounted for above except the following, which have no distinct grammar
production of their own (they are call-site uses of already-`REMOVE`d
methods, not separate syntax):

| Ruby feature | Status | Reason |
|---|---|---|
| `eval` | REMOVE | Arbitrary runtime code execution — incompatible with inception §3's static-analyzability requirement. |
| `instance_eval` | REMOVE | See Classes & Modules §8 above. |
| `class_eval` | REMOVE | See Classes & Modules §8 above. |
| `define_method` | REMOVE | See Method Definitions §6 above. |
| `method_missing` | REMOVE | See Method Definitions §6 above. |
| `send` / `public_send` | REMOVE | See Method Calls §7 above. |
| Monkey patching (reopening builtin/library classes) | REMOVE | See Classes & Modules §8 above. |
| Runtime class modification | REMOVE | See Classes & Modules §8 above (`class_eval`, reopening). |
| Runtime method definition | REMOVE | See Method Definitions §6 above (`define_method`). |
| Runtime aliasing (`alias`/`alias_method`) | REMOVE | See Method Definitions §6 above. |
| Dynamic constant definition (`const_set`) | REMOVE | Reflective mutation of program structure — out of scope per inception §2.3/§20. |
| Dynamic instance-variable definition (`instance_variable_set`) | REMOVE | Same reasoning; instance variables must correspond to declared fields (§2 above). |
| Arbitrary reflection (`instance_variables`, `methods`, `ObjectSpace`, etc.) | REMOVE | Out of scope per inception §20. |

---

## 14. Modules: `require` / `import` / `export`

Not part of Ruby's own grammar (Ruby's `require`/`require_relative` load
files at runtime; Emerald's is a compile-time AST splice) — a real,
disclosed addition beyond this document's own "inventory of Ruby's syntax"
framing, added by plan 23 (`require`) and plan 76 (`import`/`export`,
`import-export-module-visibility`).

| Form | Status | Emerald form / reason |
|---|---|---|
| `require <path>` | KEEP (plan 23) | A bare, unquoted, `/`-separated path (`require deps/mathutils/lib`), `.em` implied, always relative to the containing file. Top-level only — a `require` inside a function/`if`/`while` body is a real parse error. Splices the target file's top-level items in place at compile time (`emerald-cli`'s `require.rs`/`emerald-driver`'s `require_graph.rs`); see `SEMANTICS.md` §11 for what it makes visible. |
| `export` (declaration modifier) | KEEP (plan 76) | A leading keyword directly before a top-level `fn`/`class`/`interface`/`module`/`enum`/`actor` declaration — `export fn greet(): Void do ... end`, `export class Point ... end`. Not usable on a bare top-level statement, `require`/`import` itself, or `test`/`property`/`benchmark`/`unsafe extern` (none of those declares a nameable symbol `export` could attach visibility to). |
| `import <path> { Name, Name2, ... }` | KEEP (plan 76) | An explicit-import-list alternative/complement to a bare `require` — pulls in only the named symbols' *visibility* (their definitions are still compiled in, same as `require`; only which names this file may reference differs — see `SEMANTICS.md` §11). `path` reuses `require`'s own bare path syntax verbatim. The name list requires at least one entry — `import path {}` is a real parse error, not a silent no-op. |

**Example (KEEP: `export`/`import`, plan 76):**
```ruby
# mathutils.em
export fn add(a: Int64, b: Int64): Int64 do
  a + b
end

fn internal_helper(): Int64 do  # not exported — invisible to other files
  0
end
```
```ruby
# main.em
import mathutils { add }

puts add(3, 5)
```

---

## Cross-references

- Every `UNDECIDED` row above is answered, deferred, or locked in
  [`SEMANTICS.md`](./SEMANTICS.md).
- Type-annotation syntax (`x: T`, a function's trailing `: T` return type —
  `-> T` before the Sable-alignment grammar cutover, see plan 71) is defined
  precisely in [`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md).
