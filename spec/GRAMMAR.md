# Emerald — GRAMMAR.md

**Status:** v1 grammar inventory, extended in place (this pass) for later
plans' real, currently-parseable additions — actors (§16), interfaces and
bounded generics (§17), `unsafe extern` FFI blocks (§18), the `test`/
`property`/`benchmark` harness forms (§19), `derive`/`read` field-accessor
sugar (§20), and `static` methods (§21) — plus stale-syntax corrections to
§§3-5/9/11 (`&.`→`?.`, `x ||= expr` removed, `case`/`when`/`in`→`match`,
the brace `{ |x| ... }` block form removed). This is a targeted accuracy
pass against `crates/emerald-parser/src/grammar.lalrpop` as it stands
today, not a from-scratch re-derivation of the original Ruby-syntax
inventory below, which is left as first written except where noted.
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
| Compound assignment (`x += 1`, `x -= 1`, `x *= 1`, `x /= 1`, `x %= 1`) | KEEP | Exactly these five forms exist — no `||=`/`&&=`/shift-assign/bitwise-assign spellings. Each desugars to `x = x <op> rhs` under the receiver's statically resolved operator method; same `var` requirement as simple assignment. Restricted to a plain local (`Ident`) target only — unlike plain `=`, a compound form never targets `@field`/`arr[i]`. |
| Multiple assignment (`a, b = 1, 2`) | KEEP | Each target's type is checked against its corresponding source expression's type positionally; no splat-driven arity magic. |
| Splat in multiple assignment (`a, *b = [1, 2, 3]`) | UNDECIDED | Requires `b: Array[T]` typing rules for the captured remainder — deferred to `SEMANTICS.md` §6 (Arrays) once `09 collections` lands; not needed for the v1 milestone in inception §17. |
| Parallel/nested destructuring (`(a, b), c = [[1, 2], 3]`) | REMOVE | High grammar/type-inference cost for a rarely-essential feature; not in inception §5's initial keep list. |
| Conditional assignment (`x ||= expr`) | REMOVED (plan 73) | Superseded outright when `nil`/`T?` were removed, not kept alongside a replacement — see `SEMANTICS.md` §2's own superseded banner. There is no assignment-form nil-coalescing operator; write `x = x ?? default` instead (an ordinary assignment whose RHS uses the `??` expression operator — see §4 Operators). |

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
| Safe navigation (`?.`) | KEEP, respelled (plan 73) | `recv?.method(args)` / `recv?.method` — legal only on an `Option[T]`-typed receiver (not, as in the removed v1 design, any `Nil`-admitting type): short-circuits to `None` if `recv` is `None`, else calls `method` on the wrapped `T` and wraps the result back in `Option[U]`. Respells Ruby's `&.` — `&.` itself is no longer valid syntax; see `SEMANTICS.md` §2 and `TYPE_SYSTEM.md` §4's own superseded banners. |
| Null-coalescing (`a ?? b`) | ADD (plan 73) | `a` must be `Option[T]`; evaluates to `a`'s wrapped value when `Some`, else `b` (`b: T`). The direct replacement for the removed `x ||= expr` assignment form (§3) in the common "provide a default" case — sugar over an `Option[T]` pattern match, not a new runtime nullness check. See `examples/nullable_safe_nav.em`/`examples/c_ffi.em` for real usage. |
| Method-as-operator overloading (defining `+` etc. on a class) | KEEP | Statically dispatched like any other method — see `SEMANTICS.md` §3 (Classes). |

**Example (KEEP: safe navigation, respelled `?.`, plan 73):**
```ruby
greeting: Option[Greeter] = Some(Greeter.new("ada"))
shout: Option[String] = greeting?.shout
puts shout ?? "no greeting"
```

---

## 5. Control Expressions

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `if` / `elsif` / `else` / `unless` | KEEP | Condition must be `Boolean` (no Ruby-style truthy/falsy coercion of arbitrary objects — see `SEMANTICS.md` §1). |
| `if`/`unless` as expressions (returning a value) | KEEP | Both/all branches unify to one static type, matching inception §17's second milestone slice. |
| Modifier `if`/`unless` (`stmt if cond`) | KEEP | Sugar over the block form; no new semantics. |
| `while` / `until` (+ modifier forms) | KEEP | Same `Boolean`-only condition rule as `if`. |
| `for ... in ...` | KEEP, scope-limited | Two forms only, checked directly against `grammar.lalrpop`'s real `Stmt::For`/`Stmt::ForRange` productions: `for x in [1, 2, 3] do ... end` (a literal array — plan 30's own deliberate restriction) and `for x in a..b do ... end` / `for x in a...b do ... end` (a range over two `Expr` endpoints, inclusive/exclusive — plan 37). **`for x in arr do ... end` over an already-bound `Array[T]`-typed variable is a real parse error, not deferred to sema** — there is no general "iterate anything implementing a protocol" form despite this row's original wording. Use a `while` loop with indexing (`arr[i]`), or an ordinary `.method do |x| ... end` call (§7/§9), to iterate an arbitrary array value. |
| `case` / `when` (value match) | RESPELLED → `match` (plan 71) | `case`/`when`/(bare) `else` is gone outright — Sable's `match <scrutinee> do <arm>... end` replaces it: one `do ... end` per arm instead of a shared body run until the next `when`, and a wildcard default arm spelled `_ do ... end` instead of a trailing bare `else`. See §11 (Pattern Matching) for the full, narrower-than-originally-planned arm-shape inventory. |
| `case` / `in` (pattern match) | RESPELLED → `match`, scope narrower than originally planned | See §11 — the array/binding/guard-clause pattern forms this row originally described were never actually shipped; §11 states precisely what did. |
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
| Ownership-annotated parameters (`own T`, `borrow T`, `borrow var T`) | ADD (plan 83) | Not part of Ruby's own grammar (Ruby has no static ownership/borrow system) — a real, disclosed addition prefixing an ordinary parameter's declared type, per `spec/OWNERSHIP.md` §2. `own T` transfers the argument's binding to the callee (the caller may not use it again — `emerald-sema`'s generalization of plan 56's existing cross-actor consumed-binding check). `borrow T` is a shared, read-only reference; `borrow var T` (reusing plan 72's existing `var` keyword, not a second mutability spelling) is an exclusive, mutable reference. Legal only as a function/method PARAMETER's own type in v1 — `emerald-sema` rejects all three anywhere else (a field, a `Let`, a return type) with a real diagnostic, and additionally rejects `borrow`/`borrow var` specifically as a RETURN type (`spec/OWNERSHIP.md` §2/§10's rule 4: the only region a returned `borrow` could name is the returning function's own call-frame region, destroyed at that exact return). Enforced via lexical-scope liveness checking, not flow-sensitive/NLL analysis (`spec/OWNERSHIP.md` §8) — a `borrow`'s live range is its entire enclosing function/method body. `emerald-codegen` compiles all three exactly like the plain underlying `T` today (a real, disclosed, temporary passthrough — plan 84 owns real zero-cost borrow/own codegen). |
| Default parameter values (`def f(x = 1)`) | KEEP | Default expression's type must match the parameter's declared type. |
| Keyword parameters (`def f(x:, y: 1)`) | KEEP | Same annotation requirement as positional parameters; see `SEMANTICS.md` §3 (Methods) for whether they're retained project-wide. |
| Splat parameters (`def f(*xs)`) | UNDECIDED | Requires deciding `xs`'s static element type and arity checking — tracked in `SEMANTICS.md` §3; not required for inception §17's first three milestones. |
| Double-splat parameters (`def f(**opts)`) | UNDECIDED | Same as above, tracked alongside splat. |
| Block parameter (`def f(&blk)`) | KEEP, scope-limited | Legal only when the block's parameter/return types are statically known at the call site — see `SEMANTICS.md` §5 (Blocks). |
| Method overloading (multiple `def` for one name, different signatures) | UNDECIDED | `SEMANTICS.md` §3 question: "Are methods overloaded?" — answered there. |
| Generic function type parameters (`fn f[T](x: T): T do ... end`) | ADD (plan 41) | `[T]` (bound-less) or `[T: Bound]` (plan 58, one bound) / `[T: Bound1 + Bound2]` (plan 88, `+`-conjunction of several) sits between the function name and its parameter list. A bound-less `[T]` on a top-level generic FUNCTION is a compile error ("generic functions require a bound") — sema-enforced, not a parse restriction (a bound-less `[T]` is fully legal on a `class`/`interface`'s own type-parameter clause instead — §8, §17). Whole-program monomorphized, not erased/boxed. |
| `pure fn f(...): T do ... end` | ADD (plan 63) | An optional leading modifier, before `fn`, asserting the function's whole call graph is side-effect-free — checked, not advisory: a `pure` function calling a non-`pure` function/method is a real, named compile-time diagnostic. Grammatically legal on a top-level function and on a class/actor method (§8); sema additionally rejects `pure` specifically on an actor method (inherently side-effecting — it mutates actor state / sends messages). |
| `comptime fn f(...): T do ... end` / `comptime <call-expr>` | ADD (plan 61) | `comptime` as a function-definition modifier (before `fn`, after `pure` if both appear) marks a top-level function usable from a `comptime`-evaluated call site; `comptime <call-expr>` is a separate, expression-position use (`Array.new(comptime factorial(5))`) forcing that one call to be evaluated entirely at compile time, via a small literal-only constant evaluator, rather than deferred to runtime. Grammatically reachable but sema-rejected as a definition modifier on a module method ("comptime functions must be top-level"). |
| `requires <cond>` / `ensures <cond>` (function-level contracts) | ADD (plan 62) | Zero or more of each, in any order, between the return type and the mandatory `do` — `fn divide(a: Int64, b: Int64): Int64 requires b != 0 do ... end`. Each `requires` type-checks against the parameter list alone; each `ensures` type-checks against the parameters plus a synthesized `result` binding of the declared return type. A violated clause raises a `ContractViolation` exception at runtime (the same `raise`/`rescue` machinery `SEMANTICS.md` §7 documents); a `requires` clause whose every free identifier resolves, at a given call site, to a literal argument is ALSO checked at compile time by a small literal-only evaluator, catching a provably-failing call before it ever runs. Grammatically reachable only on a top-level `fn` — never a class/actor method (a non-empty `requires`/`ensures` registered on a method is a real sema diagnostic) and never on a `type_params`-bearing generic function. |
| `def self.name` (singleton/class methods) | SUPERSEDED, see §21 | Ruby's own `def self.name` spelling was never adopted — the equivalent capability is a real, differently-shaped `static fn` modifier (plan 196, §21 below): no implicit `self` at all, not merely a class-scoped one. |
| Method visibility (`private`, `protected`, `public`) | NOT IMPLEMENTED | Never shipped — `private`/`protected`/`public` are not reserved keywords in `grammar.lalrpop` at all (checked directly, zero matches), despite this row's original "KEEP" status. Every class/actor method is unconditionally, fully visible today. File-level visibility (`export`/`import`, §14) is the only access-control mechanism that actually exists. |
| `define_method` | REMOVE | Explicitly out of scope, inception §5/§20 — runtime method creation. |
| `method_missing` | REMOVE | Explicitly out of scope, inception §5/§20. |
| `alias` / `alias_method` | REMOVE | Runtime aliasing, inception §5/§20. |

**Example (MODIFY: typed method definition, inception §6's own example):**
```ruby
fn add(a: Int64, b: Int64): Int64 do
  a + b
end
```

**Example (ADD: `pure`/`comptime` modifiers and `requires`/`ensures` contracts, plans 61-63):**
```ruby
pure fn square(x: Int64): Int64 do
  x * x
end

fn divide(a: Int64, b: Int64): Int64
  requires b != 0
do
  a / b
end

x: Int64 = comptime factorial(10)
```

**Example (ADD: `own`/`borrow`/`borrow var` parameters, plan 83, `spec/OWNERSHIP.md` §2's own worked examples):**
```ruby
fn process(data: borrow Data): Void do
  puts data.length
end

fn mutate(data: borrow var Data): Void do
  data.append("x")
end

fn consume(data: own Data): Void do
  puts data.length
  # `data` cannot be used again by the caller after this call returns.
end
```

---

## 7. Method Calls

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| Ordinary call (`obj.method(args)`) | KEEP | Receiver's static type must declare the method; no duck typing (inception §9). |
| Parenthesis-less call (`obj.method arg`) | KEEP | Same static resolution; purely a lexical variant. |
| Command call (`puts x`) | KEEP | Resolved as a call on an implicit top-level/kernel receiver with a statically known signature. |
| Block argument (`method do |x| ... end`) | KEEP | The brace form `method { |x| ... }` is gone (plan 87, §9) — `do...end` is the only spelling. See `SEMANTICS.md` §5 (Blocks). |
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

`actor` (an isolated-heap concurrent sibling of `class`) and `interface`
(a nominal method-signature contract) are real, separate declaration kinds
that share `ClassField`/method-definition machinery with `class` below but
are NOT rows of this table — see §16 (Actors) and §17 (Interfaces &
Bounded Generics).

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
| `do |x| ... end` block literal | KEEP, scope-limited | Kept "if they can be represented cleanly" per inception §5; block parameter types must be statically inferable from the call site, or explicitly annotated (`do |x: Int64| ... end`). |
| `{ |x| ... }` block literal (brace form) | REMOVED (plan 87) | Deleted outright, not kept alongside `do...end` — Sable's design brief makes `do...end` the one executable-block spelling across the whole language. The bare `{ }` delimiter survives ONLY for a hash literal (`{k: v}` / `{k => v}`, §1/§8); a `{` immediately followed by `|params|` is a real parse error today, not a second block-literal spelling. |
| `->(x) -> T { ... }` lambda literal | SUPERSEDED | Deleted outright by the Sable-alignment grammar cutover (plan 71) — a lambda is now a bare `do |x: T| ... end` block used directly as an expression, typed by context, with no arrow anywhere; see `history/2026-09-19T110000Z-plan-71-grammar-unification-fn-and-do-end.md`. |
| `Proc.new { ... }` | REMOVE | Redundant with lambda literal syntax once procs are statically typed; one closure literal form is simpler (inception §22 rule 3/10). |
| `proc { ... }` | REMOVE | Same reasoning as `Proc.new`. |
| Block-local variables (`do |x; y| ... end`) | NOT IMPLEMENTED | Never shipped — `DoBlock`'s real production is a plain `"|" Params "|"` list with no `;`-separated block-local-variable extension at all, checked directly against `grammar.lalrpop`, despite this row's original "KEEP" status. |
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

Ruby's `case`/`when`/`in` is gone outright, superseded by Sable's own
`match <scrutinee> do <arm>+ <wildcard>? end` (plan 71's `case`/`when` →
`match`/`do` cutover, layered directly over plan 52's original `case`/
`when` value-and-variant matching mechanism — the AST/codegen underneath
is unchanged; only the keyword and per-arm body delimiter are). The scope
actually shipped is narrower than an early "basic pattern matching" plan
first sketched — no array patterns, no guard clauses, and no bare binding
pattern exist. Every row below is checked directly against
`grammar.lalrpop`'s real `CaseArm`/`MatchWildcard`/`MatchResult`
productions, not the original plan's stated intent.

| Ruby grammar area | Status | Emerald form / reason |
|---|---|---|
| `case expr; when v1, v2; end` (value match) | RESPELLED → `match <scrutinee> do <values> do ... end ... end` (plan 71) | One or more comma-separated INTEGER-literal values per arm (`CaseValues` — `Num` only; a string/symbol/boolean-literal arm is a real parse error). Each arm supplies its own `do ... end`, not a shared body run until the next `when`. |
| Enum-variant destructuring (`Name(a, b) do ... end`) | KEEP (plan 52) | `bindings` are plain, flat identifiers only — never a nested pattern (`Circle(Rectangle(w, h))` is a real parse error). This is the mechanism `Option[T]`'s own `Some(x)` arm uses. |
| Bare nullary-variant arm (`Name do ... end`) | KEEP (plan 73) | No parentheses — needed for `Option[T]`'s zero-field `None` variant; a user-declared `enum` still requires at least one payload field on every variant it declares (§15's own `EnumVariant` cross-reference), so this arm shape is reachable only for a compiler-synthesized variant like `None` today. |
| `Result[T, E]` destructuring (`match ... do Ok(x) do ... end Err(e) do ... end end`) | KEEP (plan 53) | Its own small, dedicated, fixed-order (`Ok` then `Err`), fixed-arity (exactly one of each, both mandatory, no `else`) form — independent of the general arm mechanism above, not a case of it. |
| Wildcard default arm (`_ do ... end`) | KEEP, respelled (plan 71) | Replaces the old bare trailing `else`; `_` is a real reserved token — a user identifier/class literally named `_` cannot shadow it. |
| `case`/`in` — array patterns (`in [a, b]`) | NOT IMPLEMENTED | Never shipped despite the original plan's stated intent — no array-destructuring arm exists on `CaseArm` at all. Use ordinary indexing/`Array` methods instead. |
| `case`/`in` — binding patterns (`in x`) | NOT IMPLEMENTED | No bare-identifier "match anything, bind it" arm exists; the closest available form is the wildcard `_` arm (binds nothing) or an enum-variant arm's own field bindings. |
| `case`/`in` — guard clauses (`in x if x > 0`) | NOT IMPLEMENTED | No `if`-guarded arm exists on any `CaseArm` alternative. |
| Hash/find patterns (`in {a:, b:}`, `in [*, x, *]`) | NOT IMPLEMENTED | Same as above — never reached the grammar. |
| Literal string/symbol/boolean patterns (`when "foo"`, `when :sym`) | NOT IMPLEMENTED | `CaseValues` accepts only `Num` — a string/symbol/boolean arm value is a real parse error. |
| One-line pattern matching (`expr => pattern`, `expr in pattern`) | REMOVE | Never adopted — the original decision stands, restated here now that `case`/`in` itself has been respelled to `match`. |

**Example (KEEP: numeric value match, respelled to `match`):**
```ruby
match x do
  1 do
    puts "one"
  end
  2 do
    puts "two"
  end
  _ do
    puts "other"
  end
end
```

**Example (KEEP: enum-variant destructuring, `Option[T]` — `examples/nullable_safe_nav.em`):**
```ruby
greeting: Option[Greeter] = Some(Greeter.new("ada"))
shout: Option[String] = greeting?.shout
match shout do
  Some(text) do
    puts text
  end
  None do
    puts "nothing"
  end
end
```

**Example (KEEP: `Result[T, E]` destructuring, plan 53):**
```ruby
match risky_call() do
  Ok(value) do
    puts value
  end
  Err(message) do
    puts message
  end
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

## 15. Domain Types (`newtype`)

Not part of Ruby's own grammar (Ruby has no zero-cost nominal-wrapper
construct) — a real, disclosed addition beyond this document's own
"inventory of Ruby's syntax" framing, exactly like `require`/`import`/
`export` in §14 above. Added by `domain-types-and-units` (plan-of-plans
row 81), resolving the OPEN, undecided "type aliases and newtypes"
question the Sable design brief (`history/2026-09-19T100000Z-sable-
design-brief.md`) explicitly left unfinalized ("How can Sable make
domain types easy to create without introducing unnecessary
boilerplate?").

| Form | Status | Emerald form / reason |
|---|---|---|
| `newtype Name: Underlying` | KEEP (plan `domain-types-and-units`) | A one-line declaration, no trailing `end` (the same shape `enum Name = ...` already has) — a real, checked, NOMINALLY DISTINCT wrapper around exactly one existing PRIMITIVE type (`Int64`/`Float64`/`String`/`Boolean`/`Symbol` only — a `Class`/`Enum`/`Array`/... underlying type is a real, named `emerald-sema` registration-time diagnostic, not a parse error). Deliberately its own keyword — not `class`, not `struct` (`class` is heap/arena-allocated, plan 50/51's mechanism; `struct` is reserved by §8 above for a different, not-yet-implemented future feature) — so it reads as visually and conceptually distinct from either at a glance. `Name` and `Underlying` share no implicit conversion in either direction: constructing one is always the explicit `Name.new(<underlying value>)`, and unwrapping is always the explicit `.value` — see `TYPE_SYSTEM.md` §9 for the full typing rule and the zero-cost representation guarantee. |
| Operator overloading on a `newtype` (`Meters + Meters -> Meters`) | DECLINED (this pass) | A real, disclosed decline, not a silent gap: `class`'s existing operator-overloading mechanism (`examples/operator_overloading.em`) compiles every method — including an operator like `fn +`/`fn ==` — with a leading `self` POINTER parameter, because a class instance is always heap/arena-allocated. A `newtype` value has NO pointer at all (that is the entire point of it being zero-cost) — extending operator overloading to it would need a second, genuinely new by-value calling convention for `self`, plus a method-body grammar production `NewtypeDef` doesn't have at all today (`newtype`'s one-line form has no `do ... end` body). That is new machinery, not a small extension of the existing mechanism, so it is explicitly out of scope here rather than attempted half-working. |

**Example (KEEP: a domain type, construct/unwrap round trip):**
```ruby
newtype Meters: Float64

d: Meters = Meters.new(100.0)
puts d.value
```

---

## 16. Actors (concurrency)

Not part of Ruby's own grammar (Ruby has no built-in actor-isolation
model) — a real, disclosed addition, added by plan 54
(`actors-and-message-passing`) as the concurrency half of `SPEC.md`'s
second pillar (isolated-heap actors, `RUNTIME.md` §2).

| Form | Status | Emerald form / reason |
|---|---|---|
| `actor Name ... end` | KEEP (plan 54) | Structurally identical to `class Name ... end` (§8) minus a superclass/`implements` clause — no `actor Dog < Animal ... end` form exists at the grammar level at all (a real parse error, not a semantic rejection). Field declarations and `fn`/`static fn`/`pure fn` method definitions reuse `ClassField`/`MethodDef` verbatim. |
| `ClassName.spawn(args)` | KEEP (plan 54) | The one way to create a running actor instance, symmetric with (and textually disjoint from) `ClassName.new(args)`: `.new` constructs an ordinary heap/region object, `.spawn` schedules an actor onto the runtime's fixed-thread scheduler (`RUNTIME.md` §2). |
| `ClassName.remote(addr, name)` | KEEP (plan 60) | Distributed counterpart of `.spawn` — spawns (or attaches to) a named actor at a remote node address; see `RUNTIME.md` §3. |
| `ClassName.locate(key, args...)` | KEEP (plan 65) | Location-transparent lookup-or-spawn via consistent-hash cluster placement; `RUNTIME.md` §3. |
| `supervise do ... end` | KEEP (plan 57) | A `one_for_one` supervision-tree block; `RUNTIME.md` §2. |

**Example (KEEP: actor declaration, verbatim `examples/counter_actor.em`):**
```ruby
actor Counter
  count: Int64

  fn initialize(start: Int64): Void do
    @count = start
  end

  fn increment: Void do
    @count = @count + 1
  end

  fn report: Void do
    puts @count
  end
end

c: Counter = Counter.spawn(0)
c.increment()
c.report()
```

---

## 17. Interfaces & Bounded Generics

Not part of Ruby's own grammar (Ruby has modules/duck-typing, not nominal
interfaces) — a real, disclosed addition, added by plan 41
(`generics-and-interfaces`) and widened by plans 58 (multi-parameter
bounds), 88 (`+`-conjunction of bounds), and 89 (generic interfaces —
`interface Name[T]`, more than one required method).

| Form | Status | Emerald form / reason |
|---|---|---|
| `interface Name ... end` | KEEP (plan 41/89) | One or more required method signatures (`fn name(params): T`, no body — a body on an interface method is a real parse error). `class Name implements Interface` (below) is checked structurally against every declared method. |
| `interface Name[T] ... end` | KEEP (plan 89) | An optional `TypeParamClause` (the identical `[T]`/`[T: Bound]` clause `class`/`fn` already use) right after the interface name — a generic interface's own type parameter, usable inside its own method signatures. |
| `class Name implements Interface` | KEEP (plan 41) | A single interface name (not a list) — checked only on `class`, never `module`/`actor`; structurally enforces "at least these methods, with these signatures" against the implementing class. |
| `class Name implements Interface[T]` | KEEP (plan 89) | Binds a generic interface's own type parameter to a concrete type at the implementation site (`implements Iterable[Int64]`). |
| `[T: Bound]` type-parameter bound, single | KEEP (plan 58) | On a `class`/`interface`/top-level `fn`: `[T]` is bound-less, `[T: Comparable]` requires `T` to `implement Comparable`. A bound-less `[T]` on a top-level generic FUNCTION is a compile error ("generic functions require a bound") — sema-enforced; classes/interfaces admit a bound-less type parameter (`class Stack[T]`, `examples/generic_classes.em`) with no such requirement. |
| `[T: Bound1 + Bound2]` multi-bound conjunction | KEEP (plan 88) | `+`-separated bound list — `T` must implement every named interface. |
| Generic method resolved against a bound (`fn max[T: Comparable](a: T, b: T): T`) | KEEP (plan 41/89) | The interface's declared method (e.g. `compare_to`) is statically resolved per monomorphized instantiation — no vtable, no dynamic dispatch; every call is compiled directly for its concrete type argument (whole-program monomorphization, `SPEC.md`'s first pillar). |

**Example (KEEP: interface + bounded generic function, verbatim `examples/interfaces_generics.em`):**
```ruby
interface Comparable
  fn compare_to(other: Self): Int64
end

class Money implements Comparable
  read cents: Int64

  fn initialize(cents: Int64): Void do
    @cents = cents
  end

  fn compare_to(other: Money): Int64 do
    @cents - other.cents
  end
end

fn max[T: Comparable](a: T, b: T): T do
  if a.compare_to(b) >= 0 do
    return a
  end
  return b
end
```

**Example (KEEP: generic interface, plan 89):**
```ruby
interface Iterable[T]
  fn map[U](f: Proc[T, U]): Array[U]
end

class Numbers implements Iterable[Int64]
  values: Array[Int64]

  fn initialize(values: Array[Int64]): Void do
    @values = values
  end

  fn map[U](f: Proc[T, U]): Array[U] do
    result: Array[U] = Array.new(0)
    var i: Int64 = 0
    while i < @values.count do
      result = result + [f.call(@values[i])]
      i += 1
    end
    result
  end
end
```

---

## 18. FFI: `unsafe extern` Blocks

Not part of Ruby's own grammar (Ruby has no static FFI declaration block)
— a real, disclosed addition, added by plan 59 (`ffi-and-native-linking`)
and extended by plan 85 (`own`/`borrow` on extern return types — see
[`OWNERSHIP.md`](./OWNERSHIP.md) §7, which already documents this half in
full).

| Form | Status | Emerald form / reason |
|---|---|---|
| `unsafe extern "C" { fn name(params): T ... }` | KEEP (plan 59) | `unsafe` is mandatory at the grammar level, not a sema convention — a bare `extern "C" { ... }` with no `unsafe` is a real parse error. `"C"` is a plain string literal (the ABI), not a restricted keyword set — checked against exactly `"C"` by `emerald-sema`, not the grammar. Each declared `fn` has no body (a one-line signature) and uses the same `":"` return-type spelling as an ordinary `fn`. |
| `own T` / `borrow T` on an extern fn's RETURN type | KEEP (plan 85) | See `OWNERSHIP.md` §7 for the full ownership-transfer rule; extern PARAMETER types stay unannotated, exactly as strict as before that plan. |

**Example (KEEP: verbatim `examples/c_ffi.em`):**
```ruby
unsafe extern "C" {
  fn llabs(x: Int64): Int64
  fn strlen(s: String): Int64
  fn strstr(haystack: String, needle: String): CString
}

x: Int64 = llabs(-42)
puts x
```

**Example (KEEP: `own`/`borrow` extern return types, plan 85, verbatim `examples/ffi_ownership.em`):**
```ruby
unsafe extern "C" {
  fn strdup(s: String): own CString
  fn strstr(haystack: String, needle: String): borrow CString
}
```

---

## 19. Test / Property / Benchmark Harness Blocks

Not part of Ruby's own grammar — a real, disclosed testing-harness
addition, top-level-only (reachable only from `Item`, never nested inside
a function/`if`/`while` body — the same restriction `require`, §14, already
has). Added by plan 47 (`test`), widened by plan 80 (`property`/
`benchmark`) and plan 180 (`property`'s own optional parameter list).

| Form | Status | Emerald form / reason |
|---|---|---|
| `test "description" do ... end` | KEEP (plan 47) | A named, runnable test block. `assert(cond)` / `assert_eq(expected, actual)` are reserved call forms (not ordinary functions — they capture their own source line for the failure message), the two assertion primitives available inside the body. |
| `property "description" do ... end` | KEEP (plan 80) | Zero-parameter form — a fixed-input property check, structurally identical to `test` (shares the same `do...end`/`assert*` mechanism). |
| `property "description" (params) do ... end` | KEEP (plan 180) | An optional parenthesized parameter list (the ordinary `Params`/`Param` production, `x: Type`) between the description and `do` — a property checked over generated/shrunk inputs bound to those parameters, not fixed literals. Omitting the parameter list entirely still parses identically to plan 80's original zero-param form. |
| `benchmark "description" do ... end` | KEEP (plan 15/80) | A named, timed block; run via the `emerald benchmark` subcommand — see `benchmarks/REPORT.md`'s own methodology. |

**Example (KEEP: verbatim `examples/test_framework.em`, `property_test.em`, `property_shrink_proof.em`):**
```ruby
test "addition works" do
  assert_eq(2, 1 + 1)
end

property "addition is commutative for a fixed pair of integers" do
  a: Int64 = 7
  b: Int64 = 35
  assert_eq(a + b, b + a)
end

property "addition is commutative" (a: Int64, b: Int64) do
  assert_eq(a + b, b + a)
end
```

---

## 20. `derive` and `read` Field-Accessor Sugar

Not part of Ruby's own grammar — real, disclosed class-body sugar. `read`
added by plan 33; `derive` added by plan 61 (`Comparable`) and widened by
this session's `derive-serializable` leaf (`Serializable`).

| Form | Status | Emerald form / reason |
|---|---|---|
| `read name: Type` (class/actor field) | KEEP (plan 33) | Marks a field declaration and synthesizes a zero-arg accessor method (`fn name(): Type do @name end`) in the same pass — a pure parse-time desugaring; the field itself is identical to an unmarked `Param` either way. |
| `class Name derive Comparable` | KEEP (plan 61) | Synthesizes comparison methods from the class's own field list. `derive`'s argument is grammatically an arbitrary `Ident` (not a fixed keyword set) — `emerald-sema`'s `expand_derives` rejects any unrecognized name with a real, named diagnostic; the grammar itself stays general. |
| `class Name derive Serializable` | KEEP (`derive-serializable` leaf) | Synthesizes `to_json_value(self): JsonValue` from the class's own field list. Real, disclosed scope limit: only `Int64`/`Float64`/`String`/`Boolean` fields are supported directly (a field of any other type is a `derive`-time compile error naming the field); there is no `derive`-synthesized `from_json_value` (deserialization is deferred — see plan 196's own `leaf-unblock-from-json-value-followup`). |

**Example (KEEP: `read`, verbatim `examples/static_methods.em`):**
```ruby
class Point
  read x: Int64
  read y: Int64

  fn initialize(x: Int64, y: Int64): Void do
    @x = x
    @y = y
  end
end
```

**Example (KEEP: `derive Serializable`, verbatim `examples/derive_serializable.em`):**
```ruby
class Person derive Serializable
  name: String
  age: Int64
  gpa: Float64
  active: Boolean

  fn initialize(name: String, age: Int64, gpa: Float64, active: Boolean): Void do
    @name = name
    @age = age
    @gpa = gpa
    @active = active
  end
end

p: Person = Person.new("Ada", 36, 3.9, true)
puts p.to_json_value.to_s
```

---

## 21. `static` Methods

Not part of Ruby's own grammar (Ruby's own `def self.name` singleton
method — see the now-superseded §6 row above — already covers Ruby's own
instance-vs-class-method distinction) — a real, disclosed, DIFFERENT
mechanism, added by plan 196 (`class-level-static-methods`): a `static fn`
has NO implicit `self` receiver at all, not merely a differently-scoped
one, dispatched by a direct, compile-time-resolved call — no vtable, no
receiver-type polymorphism question to resolve, since there is no
receiver.

| Form | Status | Emerald form / reason |
|---|---|---|
| `static fn name(params): T do ... end` (class method) | ADD (plan 196) | A fourth optional `MethodDef` prefix modifier (alongside `pure`/`comptime`), reachable only inside a `class`'s own method list — never on a top-level `fn`/module method. The body has NO `self`/`@field` access at all (checked, not merely unused-by-convention — referencing `@field` or bare `self` inside a `static fn` body is a real, named compile-time diagnostic, reusing the exact mechanism that already rejects `@field` access in an ordinary top-level function's body). |
| `ClassName.static_method(args)` (call site) | ADD (plan 196) | Grammatically identical to an ordinary `recv.method(args)` call (no new call-site production) — `emerald-sema` resolves it as a static dispatch specifically when `ClassName` is a known class name absent from the local variable environment (a real local variable that happens to share a class's exact name shadows it, same as any other name lookup). |

**Example (KEEP: verbatim `examples/static_methods.em`):**
```ruby
class Point
  read x: Int64
  read y: Int64

  fn initialize(x: Int64, y: Int64): Void do
    @x = x
    @y = y
  end

  static fn origin(): Point do
    Point.new(0, 0)
  end

  static fn midpoint(a: Point, b: Point): Point do
    sx: Int64 = a.x + b.x
    sy: Int64 = a.y + b.y
    Point.new(sx / 2, sy / 2)
  end
end

o: Point = Point.origin()
puts o.x
```

---

## Cross-references

- Every `UNDECIDED` row above is answered, deferred, or locked in
  [`SEMANTICS.md`](./SEMANTICS.md).
- Type-annotation syntax (`x: T`, a function's trailing `: T` return type —
  `-> T` before the Sable-alignment grammar cutover, see plan 71) is defined
  precisely in [`TYPE_SYSTEM.md`](./TYPE_SYSTEM.md).
- Sections 16-21 (actors, interfaces/bounded generics, FFI, the
  test/property/benchmark harness, `derive`/`read`, `static` methods) are
  later, real additions layered on top of this document's original
  Ruby-syntax inventory — each cites its own originating plan, the same
  convention §14/§15 already establish, rather than being retrofitted into
  the inventory framing above as if they were always part of it.
