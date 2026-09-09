---
name: Function Signature Completeness
overview: "Default parameter values, compile-time-resolved keyword arguments, statically-typed homogeneous splat capture, and fixed-arity multiple return values via an anonymous tuple — spec/GRAMMAR.md §6/§7's four still-open function-signature rows (default params KEEP, keyword params KEEP, splat params UNDECIDED, splat call args UNDECIDED), closed the identity-preserving way: every one of the four is resolved entirely against the callee's declared static signature at compile time, with no runtime dispatch, no Hash-backed `**kwargs`, and no dynamically-typed splat."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-default-params
    content: "`def f(x: Int64 = 0)` — a literal-only default value, filled in at the call site by both sema and codegen when a trailing argument is omitted"
    status: pending
  - id: leaf-keyword-args
    content: "`f(x: 1, y: 2)` — compile-time name-to-position reordering against the callee's declared parameter names, via a new `Expr::CallKw` node"
    status: pending
  - id: leaf-splat-params
    content: "`def f(*args: Int64)` — trailing positional call-site arguments packed into a real `Array[Int64]`, reusing plan 09's actual array representation, at a fixed LLVM arity"
    status: pending
  - id: leaf-multi-return
    content: "`return a, b` / `x, y = f()` — a real fixed-arity anonymous tuple return type (LLVM struct return), consumed on the receiving side by extending plan 31's `Stmt::MultiAssign`"
    status: pending
isProject: false
---

# Plan 39 — Function Signature Completeness

This is plan 39 of the 36-47 follow-up batch — twelve independent sibling
plans, each owning one distinct, identity-preserving gap between Emerald's
post-plan-35 surface (~10-15% of standard Ruby) and its analyzed strategic
ceiling of ~45%. None of the twelve concedes any of Emerald's four identity
constraints: no `method_missing`/`eval`/`send`/reflection, no mixins/open
classes/monkey-patching, no dynamic/virtual dispatch (every call still
resolves from the receiver's *declared* type), no tracing GC, no runtime
reflection. It is post-v1, post-plans-28-35 scope — it does not touch
`plan-of-plans.md`, which will be updated separately once all twelve
36-47 plans are authored.

This plan bundles four related, additive completions to Emerald's function
*signatures* and *call sites* — the same "bundle related grammar-adjacent
gaps into one plan" posture plan 29 used for `elsif`/`unless`/`until`:
default parameter values, compile-time keyword arguments, statically-typed
homogeneous splat capture, and fixed-arity multiple return values. All four
are resolved by the compiler against the callee's *declared, static*
signature — never by a runtime name/hash lookup, never by inspecting an
object's actual shape at a call site.

Concrete proof this plan targets:
```ruby
def greet(name: String, times: Int64 = 1) -> Void
  i: Int64 = 0
  while i < times
    puts name
    i += 1
  end
end

greet(name: "yo")
greet(name: "hi", times: 2)

def divmod(a: Int64, b: Int64) -> (Int64, Int64)
  return a / b, a % b
end

q: Int64 = 0
r: Int64 = 0
q, r = divmod(17, 5)
puts q
puts r
```
Expected output, compiled, linked, and run: `yo`, `hi`, `hi`, `3`, `2` (one
per line). `greet(name: "yo")` proves keyword-argument resolution *and*
`times`'s default (`1`) being filled in when the call omits it;
`greet(name: "hi", times: 2)` proves an explicitly supplied keyword
argument overrides the default; `divmod`'s `(Int64, Int64)` return type and
`return a / b, a % b` prove a real two-value return, and `q, r =
divmod(17, 5)` proves the receiving side unpacks it correctly (`17 / 5 =
3`, `17 % 5 = 2`, integer division, matching every other plan's `Int64`
semantics).

## Decision log

- **Scoped to plain top-level `def` functions only — not class methods, not
  module methods.** Verified this session: `emerald-sema/src/lib.rs`'s
  `check_function_body` (L1188-1216) and `check_method_body` (L1218-1248)
  are already two separate functions, and `emerald-codegen/src/lib.rs`'s
  `define_user_function` (L3206-3247) and `define_method` (L3254-3305) are
  likewise separate — extending all four of this plan's features onto both
  code paths at once would double the surface for no proof-of-concept
  value. This plan touches only the top-level-function path; a method
  declared with a default parameter, a splat, a keyword-argument call
  target, or a tuple return type is rejected with an explicit "not
  supported on methods yet" diagnostic — not silently ignored (silently
  treating a method's `default`/`splat_param` as inert would leave a
  parsed-but-dead AST field, a worse outcome than a clear compile error). A
  later, disclosed follow-up can extend this onto `check_method_body`/
  `define_method` once the mechanism is proven here.
- **Default parameter values are restricted to compile-time-evaluable
  literals only (`Int`/`Float`/`StringLit`/`Bool`/`Nil` tokens) — never an
  arbitrary expression, and never a reference to another parameter.** Ruby
  allows `def f(x, y = x + 1)`, where `y`'s default reads an
  already-bound parameter; this plan declines that shape outright, at the
  *grammar* level, not just by sema convention: `Param { name: String, ty:
  String }` (verified in `crates/emerald-parser/src/ast.rs`) gains a new
  `default: Option<Expr>` field, but the *only* grammar path that can ever
  produce `Some(_)` is a dedicated `DefaultLit` nonterminal (literal
  tokens only) inside a new `FuncParams` list used exclusively by
  `ParenParams` (`crates/emerald-parser/src/grammar.lalrpop`) — the
  existing, shared `Param`/`Params` productions (verified: reused verbatim
  today by `ClassField`'s field declarations and by lambda/block parameter
  lists) are left untouched and can never carry a default. This is a real,
  disclosed restriction, not an oversight: evaluation order and
  cross-parameter dependency analysis for arbitrary defaults is a strictly
  larger feature this plan doesn't need to prove "default parameter values
  exist and work."
- **Keyword arguments are resolved entirely at compile time by name-to-
  position matching against the callee's declared parameter names — a new
  `Expr::CallKw(String, Vec<(String, Expr)>)` AST node, not a runtime
  hash/dispatch mechanism.** Verified this session: `Expr::Call(String,
  Vec<Expr>)` (ast.rs) is positional-only today, and
  `emerald-codegen/src/lib.rs`'s `Expr::Call` arm (L1312-1339) builds its
  `arg_vals` by walking `args: &[Expr]` in source order against
  `ctx.user_func_ids: HashMap<String, (FunctionValue, ValKind)>` — no
  parameter *names* are available to codegen at that point at all.
  `FunctionSig` (`emerald-sema/src/lib.rs` L54-57, today just `{ params:
  Vec<Type>, return_type: Type }`) gains `param_names: Vec<String>` and
  `defaults: Vec<Option<Expr>>`, populated in `function_signature()`
  (L129-143) from the (now-extended) `Function.params`. Codegen needs its
  own equivalent lookup (a name-indexed view of the same `Function.params`
  it already has via the full `Program` it's handed) — this is a real,
  disclosed *duplication* of bookkeeping between the two passes, but it
  matches the codebase's own established pattern of independent,
  unsynchronized sema/codegen registries built from the same AST (e.g.
  `ClassInfo` in `emerald-sema`, L65-78, vs `ClassLayout` in
  `emerald-codegen`, L131-134 — both separately derived from the same
  `ClassDef`, verified this session). An unknown keyword name or a missing
  required (no-default) keyword is a compile-time diagnostic naming the
  offending keyword/parameter, produced identically to every other
  diagnostic in this compiler — never a panic, never a runtime `NoMethod`/
  `KeyError`-style failure.
- **Keyword-argument calls are scoped to plain function calls only (not
  `.method(...)` / `.new(...)`), and to all-keyword-or-all-positional call
  sites (no mixing the two in one call).** Ruby allows both mixing
  (`f(1, y: 2)`) and keyword syntax on any call form; this plan declines
  both narrowings for a first cut — mixing would require threading a
  `Vec<Arg>` (tagged positional-or-named) through every one of `Expr::New`
  and `Expr::MethodCall`'s existing codegen paths (`build_method_call`,
  L1562-1655, and the `Expr::New` arm, L1340-1378), a substantially larger
  surface than proving keyword resolution works at all. `Expr::Call`
  itself is untouched and keeps its purely positional meaning; a call
  written with any `name:` argument parses as the new, disjoint
  `Expr::CallKw` instead.
- **Splat capture (`def f(*args: Int64)`) reuses plan 09's actual
  `Array[T]` runtime representation wholesale — packed at the *call site*,
  never as a variadic LLVM function signature.** Verified this session
  against `emerald-codegen/src/lib.rs`: `build_array_lit` (L1664-1696)
  allocates a flat, contiguous, `ctx.alloc`-backed buffer with no stored
  runtime length (every element a fixed 8 bytes), and `bind_params`
  (L3169-3204) already tracks an `Array[Elem]`-typed parameter's element
  kind in the `local_array_elem_types` side table. `Function` (ast.rs)
  gains `splat_param: Option<Param>` (the *element* type only, e.g.
  `Int64` for `*args: Int64`) — mirroring how `block_param: Option<String>`
  already sits alongside `params` rather than inside it (plan 34's
  precedent). The compiled LLVM function itself stays fixed-arity: one
  extra trailing pointer parameter, bound exactly like an ordinary
  `Array[Elem]` parameter via the existing `bind_params` path. The actual
  variable-length packing happens once, at each call site, using the exact
  same alloc-and-store loop `build_array_lit` already runs for a literal
  `[1, 2, 3]` — not a second, parallel array representation.
- **Double-splat (`**opts`) is explicitly declined, staying `UNDECIDED`.**
  `spec/GRAMMAR.md` §6's "Double-splat parameters" row and §7's "Splat call
  arguments" row are both `UNDECIDED`, tracked together; this plan
  resolves ordinary splat's status but deliberately leaves double-splat
  where it was. Verified this session: `resolve_type`'s `Hash[K, V]`
  branch (`emerald-sema/src/lib.rs` L105-117) and codegen's
  `parse_hash_type`/`value_kind_for_type` accept only `Int64` keys (plan
  25's Decision log) — there is no `Hash[String, T]` today at all, which
  `**opts`'s natural representation would need at minimum, and Ruby's real
  `**opts` semantics (an open-ended, dynamically-typed bag) are exactly
  the kind of dynamism this project's identity constraints rule out
  regardless. A statically-typed double-splat would need every call site
  to supply the exact same *closed* set of keyword names as a struct-like
  type — a distinct, larger feature than this plan's four items.
- **Multiple return values are a real, fixed-arity anonymous tuple — an
  LLVM struct return — never plan 09's boxed `Array[T]`.** New `Type::
  Tuple(Vec<Type>)` (sema) and `ValKind::Tuple(Vec<ValKind>)` (codegen).
  Verified this session: `ValKind` (`emerald-codegen/src/lib.rs` L50-62)
  today derives `Clone, Copy, PartialEq, Eq, Debug` and is passed *by
  value* as a single `ret_kind: ValKind` argument through every one of
  `build_stmt`/`build_block`/`build_case`/`build_begin`/`build_for`/
  `build_function_body` — adding a `Vec`-carrying variant forces dropping
  `Copy` from `ValKind`, a real, disclosed mechanical cost touching every
  existing call site that currently relies on an implicit copy across this
  4316-line file (each becomes an explicit `.clone()`), not a free
  addition. The tuple is valid **only** as a function's declared return
  type — never a parameter type, a field type, a `Let`'s local type, an
  array element type, or nested inside another tuple; there is no
  first-class tuple *value* that can be stored in a variable or passed as
  an argument.
- **The tuple return-type annotation reuses the exact layered-resolution
  precedent this codebase already established for `Proc`.** `TypeName`
  (`grammar.lalrpop` L704-709) gains a `"(" T1 "," T2 [, ...] ")"`
  alternative (mirroring `Array[Elem]`/`Hash[K, V]`'s existing
  compound-string convention, formatted as `"(Int64, Int64)"`), added as a
  grammar-wide alternative — *not* gated to only the return-type position
  at the grammar level. The actual restriction lives in sema: a new
  `resolve_return_type` function (used only by `function_signature()`,
  L129-143) checks for the `"(...)"` prefix first and builds a real
  `Type::Tuple`; the existing `resolve_type` (used for every param, field,
  and `Let` annotation) gains no such branch and falls straight through to
  its pre-existing `other => Err("unknown type")` catch-all — the same
  "grammar permits it everywhere, only one specific resolution path gives
  it real meaning" shape `resolve_type`'s own `"Proc"` case already uses
  (verified: a bare `Proc` annotation used outside `Let`'s special-cased
  lambda binding resolves to an inert placeholder today, L118-124) . `x:
  (Int64, Int64) = ...` therefore still fails, cleanly, as an unknown
  type — no separate enforcement pass is needed.
- **`return a, b` is real, additive comma-list syntax — a new
  `Expr::TupleLit(Vec<Expr>)`, legal only as `Stmt::Return`'s direct
  argument.** The existing `"return" <e:Expr> => Stmt::Return(Some(e))`
  rule generalizes to a comma list the same mechanical way plan 34's
  `YieldArgs` and plan 31's `MultiAssignNames`/`MultiAssignValues` already
  do (verified: both are mandatory-first-element, right-recursive
  productions in `grammar.lalrpop`, L275-303) — `Stmt::Return(Some(e))`
  when the list has one element (unchanged, existing behavior), or
  `Stmt::Return(Some(Expr::TupleLit(es)))` when it has more than one. Sema
  accepts `Expr::TupleLit` only when the enclosing function's declared
  return type is `Type::Tuple` of matching arity and per-position types
  (an ordinary function whose return type isn't a tuple, given `return a,
  b`, is a real type-mismatch diagnostic against its declared return
  type — reusing the exact comparison `check_stmt`'s existing `Stmt::
  Return(Some(e))` arm already performs).
- **`x, y = f()` extends plan 31's actual `Stmt::MultiAssign`, not a new
  AST node — but plan 31's own arity rule doesn't fit this shape and needs
  a genuine special case.** Verified this session against
  `emerald-parser/src/ast.rs`: `Stmt::MultiAssign { names: Vec<String>,
  values: Vec<Expr> }` is exactly the receiving-side shape needed. But
  `check_multi_assign` (`emerald-sema/src/lib.rs` L795-827) and its
  codegen counterpart (`emerald-codegen/src/lib.rs` L2350-2371) both
  hard-require `names.len() == values.len()`, checking/evaluating each
  `values[i]` independently and zipping it against `names[i]` — plan 31
  never anticipated a *single* value expression producing more than one
  result. `x, y = f()` parses today as `names.len() == 2`, `values.len()
  == 1`, which plan 31's existing arity check would reject as a mismatch.
  This leaf adds one special case *ahead of* that generic check: when
  `values.len() == 1` and that sole value is a call whose resolved return
  type is `Type::Tuple(ts)` with `ts.len() == names.len()`, match `names`
  positionally against `ts` instead of the per-value path; codegen's
  mirror builds the call once, then unpacks the returned LLVM struct via
  one `build_extract_value` per name instead of the existing "evaluate
  each value, then zip" loop. Every pre-existing multi-assign shape
  (`a, b = b, a`, `values.len() == names.len()`, no call involved) is
  untouched — this is a genuinely additive special case, not a rewrite of
  plan 31's mechanism, and plan 31's own swap example is this leaf's
  regression proof.
- **Out of scope: splat combined with keyword arguments, or an ordinary
  parameter declared after a splat.** Ruby allows `def f(*xs, y:
  Int64)`; this plan requires a function's splat, if present, to be its
  last positional-ish parameter (ordinary params, then an optional
  splat, then an optional `&blk`) — a real, disclosed narrowing, not
  a grammar oversight, that keeps `FuncParams`' shape a simple
  three-segment sequence instead of an arbitrarily-interleaved one.

## Leaf: leaf-default-params

### 1. Context
- Why: `spec/GRAMMAR.md` §6 marks default parameter values `KEEP`
  ("Default expression's type must match the parameter's declared type"),
  and no such mechanism exists today — verified this session:
  `crates/emerald-parser/src/ast.rs`'s `Param { name: String, ty: String
  }` carries no default field, and `grammar.lalrpop`'s `Param`/`Params`
  productions (used by function params, class fields, and lambda/block
  params alike) have no `"=" Expr` alternative anywhere.
- Target state: `Param` gains `default: Option<Expr>`; a new `DefaultLit`
  nonterminal (literal tokens only — `Num`/`FloatLit`/`StringLitTok`/
  `"true"`/`"false"`/`"nil"`, each wrapped in its matching `Expr`
  variant) and a new `FuncParams` list (used only by `ParenParams`) that
  allows a trailing `"=" DefaultLit` on any parameter; every other
  existing use of `Param`/`Params` (`ClassField`, lambda literals, block
  literals) is untouched and always produces `default: None`.
  `FunctionSig` gains `defaults: Vec<Option<Expr>>`
  (`emerald-sema/src/lib.rs`, populated in `function_signature()`); a
  call site with `args.len() < sig.params.len()` has the missing trailing
  arguments filled from `defaults` before `check_args` runs (a genuine
  "or diagnostic if the missing param has no default" branch, not a
  silent `None`-as-zero substitution). Codegen's `Expr::Call` arm
  (`emerald-codegen/src/lib.rs` L1312-1339) mirrors the same fill logic
  against its own name-indexed view of the callee's `Function.params`
  before building `arg_vals`.

### 2. Acceptance Criteria
1. `def inc(n: Int64, step: Int64 = 1) -> Int64` returning `n + step`,
   called as `inc(5)` (one argument, `step` omitted) — compiled, linked,
   and run, prints `6`, proving the default is actually filled and used,
   not merely parsed.
2. The same function called as `inc(5, 10)` (both arguments supplied) —
   compiled, linked, and run, prints `15`, proving an explicit argument
   overrides the default rather than the default always winning.
3. `def bad(n: Int64, step: Int64 = n) -> Int64 ... end` (a default
   referencing another parameter) is a parse error — `DefaultLit`
   admits no `Expr::Ident`, so this is rejected at the grammar level,
   verified via a parser-unit test expecting `Err`, not a sema
   diagnostic and not a panic.
4. `inc()` (omitting `n`, which has no default) is a compile-time arity
   diagnostic naming the missing required parameter — never silently
   defaulted to `0`/`Nil`.
5. A class method declared with a default parameter (e.g. inside a
   `class ... def m(x: Int64 = 0) -> Void ... end ... end`) is rejected
   with an explicit "default parameter values are not supported on
   methods yet" diagnostic from `check_method_body` — not silently
   accepted and not silently ignored.
6. Regression: every prior plan's example that supplies every argument
   explicitly (the full existing test suite) still parses, type-checks,
   and compiles/links/runs identically — `Param.default` being `None`
   for every pre-existing declaration must produce byte-identical
   codegen to before this leaf.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Param`),
  `crates/emerald-parser/src/grammar.lalrpop` (`ParenParams`, new
  `FuncParams`/`DefaultLit`), `crates/emerald-sema/src/lib.rs`
  (`FunctionSig`, `function_signature`, `check_args`, `check_method_body`
  diagnostic), `crates/emerald-codegen/src/lib.rs` (`Expr::Call` arm,
  `define_method`'s rejection path)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. `inc(5)` → `6` and `inc(5, 10)` → `15` linked-and-run | agent-claimed-locally |

---

## Leaf: leaf-keyword-args

### 1. Context
- Why: `spec/GRAMMAR.md` §6 marks keyword parameters `KEEP`; no keyword
  call syntax exists today — verified this session: `Expr::Call(String,
  Vec<Expr>)` is positional-only (ast.rs), and codegen's `Expr::Call` arm
  (`emerald-codegen/src/lib.rs` L1312-1339) has no access to parameter
  names at all, only `ctx.user_func_ids: HashMap<String, (FunctionValue,
  ValKind)>`.
- Target state: a new `Expr::CallKw(String, Vec<(String, Expr)>)` node,
  populated by a new `CallArgs`/`Arg` grammar path used only by the bare
  function-call productions in `StmtPrimaryExpr`/`PrimaryExpr` (`<name:
  Ident> "(" <args:Args> ")" => Expr::Call(...)` gains a sibling
  alternative for a keyword-only argument list — `.method()`/`.new()`
  call sites are untouched). `FunctionSig` gains `param_names:
  Vec<String>` (see Decision log). Sema resolves `Expr::CallKw` by
  matching each supplied name against `param_names`, filling any
  remaining defaulted parameter per `leaf-default-params`, and erroring
  on an unknown name, a duplicate name, or a missing required parameter.
  Codegen performs the identical reorder against its own view of the
  callee's `Function.params` before building `arg_vals`.

### 2. Acceptance Criteria
1. This plan's own worked example — `greet(name: "yo")` and `greet(name:
   "hi", times: 2)` — compiled, linked, and run, prints `yo`, `hi`, `hi`
   (three lines), proving keyword resolution and default-filling compose
   correctly together.
2. `greet(nam: "hi")` (a misspelled keyword) is a compile-time
   diagnostic naming `nam` as an unrecognized keyword for `greet`, not a
   panic.
3. `greet()` (omitting `name`, which has no default) is a compile-time
   diagnostic naming `name` as a required, missing keyword.
4. `greet(name: "hi", name: "yo")` (a duplicate keyword) is a
   compile-time diagnostic, not last-write-wins.
5. Regression: every prior plan's `Expr::Call`-shaped example (positional
   calls, zero keyword arguments) parses to the same `Expr::Call` node
   and produces byte-identical codegen — `CallArgs`'s new alternative
   must not change how a purely positional argument list parses.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Expr::CallKw`),
  `crates/emerald-parser/src/grammar.lalrpop` (new `CallArgs`/`Arg`,
  wired into `StmtPrimaryExpr`/`PrimaryExpr`'s bare-call productions),
  `crates/emerald-sema/src/lib.rs` (`FunctionSig::param_names`,
  `infer_expr_type`'s new `Expr::CallKw` arm), `crates/emerald-codegen/
  src/lib.rs` (`build_expr`'s new `Expr::CallKw` arm)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. the `greet` worked example linked-and-run | agent-claimed-locally |
| Clippy | `cargo clippy --workspace --all-targets` | no new warnings | agent-claimed-locally |

---

## Leaf: leaf-splat-params

### 1. Context
- Why: `spec/GRAMMAR.md` §6/§7 mark splat parameters and splat call
  arguments both `UNDECIDED`, explicitly deferred "pending `09
  collections`" — plan 09 has since landed (verified: `Type::Array` /
  `ValKind::Ptr` + `local_array_elem_types` side table are real, working
  machinery today, per `build_array_lit`, `emerald-codegen/src/lib.rs`
  L1664-1696).
- Target state: `Function` (ast.rs) gains `splat_param: Option<Param>`
  (element type only, e.g. `Int64` for `*xs: Int64`); grammar's
  `FuncParams` gains an optional trailing `"*" <name:Ident> ":"
  <ty:TypeName>` marker, ordered after ordinary/defaulted parameters and
  before an optional `&blk` (see Decision log's scope cut). Sema treats
  a call to a splat-declared function as `args.len() >= ordinary_count`,
  type-checking every trailing argument against the splat's declared
  element type. Codegen keeps the compiled function fixed-arity (ordinary
  params, then one trailing `Array[Elem]`-typed pointer parameter, bound
  via the existing `bind_params` path) and, at each call site, packs the
  trailing arguments into a freshly `ctx.alloc`'d buffer using the exact
  loop `build_array_lit` already runs for an array literal.

### 2. Acceptance Criteria
1. `def sum_all(*xs: Int64) -> Int64` (summing `xs` via a `while`/index
   loop over a compile-time-known call-site count) called as
   `puts sum_all(1, 2, 3, 4)` — compiled, linked, and run, prints `10`.
2. `sum_all()` (zero trailing arguments) — compiled, linked, and run,
   prints `0`, proving a zero-length capture is legal and not a special-
   cased arity error.
3. `sum_all(1, "x")` (a trailing argument of the wrong element type) is
   a compile-time diagnostic naming the mismatched trailing argument's
   position, not a runtime crash.
4. Regression: an ordinary (non-splat) function's declaration, call, and
   compiled output are byte-identical to before this leaf —
   `Function.splat_param` is `None` for every function this plan didn't
   touch, and `param_kinds`/`bind_params`/`declare_user_functions`
   (`emerald-codegen/src/lib.rs`) treat such functions exactly as they
   did before this leaf.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Function::
  splat_param`), `crates/emerald-parser/src/grammar.lalrpop`
  (`FuncParams`'s trailing `*name: Type` marker), `crates/emerald-sema/
  src/lib.rs` (`function_signature`, `check_args`'s variadic-tail case),
  `crates/emerald-codegen/src/lib.rs` (`param_kinds`, `bind_params`,
  `declare_user_functions`, `Expr::Call`'s call-site packing)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. `sum_all(1,2,3,4)` → `10` and `sum_all()` → `0` linked-and-run | agent-claimed-locally |

---

## Leaf: leaf-multi-return

### 1. Context
- Why: no function can return more than one value today — verified this
  session: `emerald-codegen/src/lib.rs`'s `define_user_function`
  (L3206-3247) derives a single `ret_kind: ValKind` from
  `value_kind_for_type(&f.return_type)`, threaded by value through
  `build_stmt`/`build_block`/`build_case`/`build_begin`/`build_for`, and
  `Stmt::Return(Some(e))`'s codegen arm (L2431-2443) builds exactly one
  LLVM value and returns it. Plan 31's `Stmt::MultiAssign` (the intended
  receiving side) hard-requires `names.len() == values.len()`
  (`check_multi_assign`, `emerald-sema/src/lib.rs` L795-827, and its
  codegen mirror, `emerald-codegen/src/lib.rs` L2350-2371) — a real gap
  for `x, y = f()`'s `values.len() == 1` shape (see Decision log).
- Target state: `Type::Tuple(Vec<Type>)` / `ValKind::Tuple(Vec<ValKind>)`
  (dropping `ValKind`'s `Copy` derive — see Decision log for the
  disclosed cost); `TypeName` gains a `"(" T1 "," T2 ")"` compound-string
  alternative resolved only by a new `resolve_return_type`; a new
  `Expr::TupleLit(Vec<Expr>)`, legal only as `Stmt::Return`'s argument
  when the enclosing function's return type is `Type::Tuple`; `Stmt::
  MultiAssign`'s sema/codegen both gain the single-call-tuple special
  case described in the Decision log, ahead of the pre-existing
  positional-list path.

### 2. Acceptance Criteria
1. This plan's own worked example — `def divmod(a: Int64, b: Int64) ->
   (Int64, Int64) ... return a / b, a % b end` and `q, r = divmod(17, 5)`
   — compiled, linked, and run, prints `3` then `2`.
2. `return a, b` inside a function declared `-> Int64` (not a tuple) is
   a compile-time type-mismatch diagnostic against the declared return
   type, not silently accepted or truncated to `a`.
3. `x, y = f()` where `f`'s declared return type is `(Int64, Int64,
   Int64)` (a 3-tuple, arity mismatch against 2 targets) is a
   compile-time diagnostic, reusing plan 31's existing arity-mismatch
   diagnostic shape and wording style.
4. Regression: plan 31's own swap example, `a, b = b, a` (`values.len()
   == names.len() == 2`, no function call involved), still compiles,
   links, and runs, printing `2` then `1` — this leaf's single-call
   special case must sit strictly ahead of, and never shadow, plan 31's
   pre-existing per-value path.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs` (`Expr::TupleLit`),
  `crates/emerald-parser/src/grammar.lalrpop` (`TypeName`'s tuple
  alternative, `Stmt`'s `"return"` rule generalized to a comma list),
  `crates/emerald-sema/src/lib.rs` (`Type::Tuple`, `resolve_return_type`,
  `infer_expr_type`'s `Expr::TupleLit` arm, `check_multi_assign`'s
  special case), `crates/emerald-codegen/src/lib.rs` (`ValKind::Tuple`,
  every `ret_kind: ValKind` call site's `Copy` → `.clone()` mechanical
  update, `Stmt::Return`'s struct-building codegen, `Stmt::MultiAssign`'s
  `build_extract_value` special case)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. `divmod(17, 5)` → `3`, `2` and plan 31's swap regression | agent-claimed-locally |
| Clippy | `cargo clippy --workspace --all-targets` | no new warnings from the `ValKind` `Copy`-removal fallout | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
