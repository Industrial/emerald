---
name: Arithmetic & Logical Operators
overview: "Binary `-`/`*`/`/`/`%`, unary `-`/negative literals, and short-circuit `&&`/`||`/`!` — the operators plan 15's benchmarking and plan 11's exceptions work both had to route around because the grammar only ever grew `+` and comparisons."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-operators
    content: "Expr::{Sub,Mul,Div,Rem,Neg,Not,And,Or}; six new precedence tiers (Or > And > Compare > Add/Sub > Mul/Div/Rem > Unary), mirrored across both the Expr and StmtExpr grammar hierarchies"
    status: pending
  - id: leaf-sema-operators
    content: "Numeric-operand rules for Sub/Mul/Div/Rem/Neg (same Int64/Float64-match rule Add already enforces); Boolean-operand rules for And/Or/Not"
    status: pending
  - id: leaf-codegen-operators
    content: "Cranelift lowering: isub/fsub/imul/fmul/sdiv/fdiv/srem/ineg/fneg mirroring Expr::Add's existing F64-vs-int dispatch; real short-circuit branching for And/Or via the same create_block/brif/merge shape Stmt::If already uses"
    status: pending
isProject: false
---

# Plan 18 — Arithmetic & Logical Operators

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md), for
the same reason plan 17 wasn't: that table's Completion note calls
Emerald v1 done as of row 15. This is new, post-v1 language-completeness
scope — not touching `plan-of-plans.md` or any other existing plan file.

Concrete proof this plan targets — two real programs, compiled, linked,
and run, exact stdout asserted:

**Example A (arithmetic):**
```ruby
def factorial(n: Int64) -> Int64
  if n <= 1
    return 1
  end
  return n * factorial(n - 1)
end

puts factorial(5)
puts 17 / 5
puts 17 % 5
puts -3 + 10
```
Expected: `120\n3\n2\n7\n`.

**Example B (short-circuit `&&`/`||`):**
```ruby
def noisy(n: Int64) -> Boolean
  puts n
  return n > 0
end

x: Int64 = -5
if x > 0 && noisy(1)
  puts 100
end
if x > -10 && noisy(3)
  puts 300
end
if x < 0 || noisy(2)
  puts 200
end
if x > 0 || noisy(4)
  puts 400
end
```
Expected: `3\n300\n200\n4\n400\n` — `1` and `2` never print (both `&&`/
`||` genuinely short-circuit `noisy(1)`/`noisy(2)` away), while `3` and
`4` do (the operator falls through to evaluating its right side when
short-circuiting can't apply), proving this isn't just a non-branching
bitwise AND/OR that happens to work when both operands are cheap.

## Decision log

- **Unary minus and negative literals are in scope here, not split into
  their own plan.** Plan 11's own Decision log already found the gap and
  named it exactly: "this grammar has no unary minus or negative-literal
  syntax at all, only `[0-9]+`" — plan 11 worked around it by choosing
  only positive test values (`risky(999)`, not `risky(-1)`). Binary `-`
  alone is of limited use without it (no way to write a negative
  constant, no way to decrement below zero in a loop guard), and both
  are naturally the same grammar-level addition (a tightest-precedence
  prefix `UnaryExpr` tier), so this plan does both together.
- **Precedence scope is six real tiers, not Ruby's full table.**
  `spec/GRAMMAR.md` §4 marks Ruby's *entire* precedence table KEEP
  ("no motivation to diverge") — that's an aspirational blanket
  statement already known to outrun the implementation (the same table
  also KEEPs `<=>`, ternary, range operators, bitwise operators, `**`,
  and the `and`/`or`/`not` keyword spellings, none of which exist in
  `grammar.lalrpop` today). This plan implements exactly: unary `-`/`!`
  (tightest) → `*`/`/`/`%` → binary `+`/`-` → comparisons (unchanged,
  still non-associative) → `&&` → `||` (loosest). Ternary, `<=>`, range
  operators, bitwise operators, `**`, `and`/`or`/`not`, compound
  assignment (`+=` etc.), and method-operator overloading all remain
  exactly as aspirational after this plan as they were before it — see
  Out of scope.
- **The `Stmt`/`Expr` grammar mirror doubles this plan's grammar
  surface.** Plan 04's "no significant newlines" simplification and
  plan 09's array-literal-statement fix together mean every `Expr`
  precedence tier has a second, near-identical `Stmt`-initial-position
  copy (`AddExpr`/`StmtAddExpr`, etc.) so a bare `[...]` can't collide
  with `CallExpr`'s indexing suffix in `FOLLOW`. Six new tiers therefore
  means roughly twelve new nonterminals, not six — a real complexity
  cost of that original decision, not new complexity this plan invents.
- **No `true`/`false`/`nil` literals.** `Type::Boolean` already exists in
  `emerald-sema` and is already the required type of every `if`/`while`
  condition (`spec/GRAMMAR.md` §5, enforced today) — but `Expr::Compare`
  is the *only* thing that currently produces one. `&&`/`||`/`!` operate
  directly on Boolean-typed expressions (comparisons, and other
  `&&`/`||`/`!` expressions) — fully useful without a literal, as
  Example B shows. Literal `true`/`false`/`nil` are stdlib-expansion
  scope, deliberately left for a separate future plan.
- **`&&`/`||` compile to real short-circuit branches**, using the same
  `create_block`/`brif`/jump-to-merge shape `Stmt::If` codegen already
  uses for its then/else split — not an eager `band`/`bor` over two
  unconditionally-evaluated operands. Short-circuit is Ruby's actual
  `&&`/`||` semantics (inception keeps them unchanged) and matters for
  real programs the moment a side-effecting call sits on the right side,
  exactly what Example B is built to prove.
- **Division/modulo by a runtime-zero divisor traps; there is no
  catchable `ZeroDivisionError`.** Cranelift's `sdiv`/`srem` are defined
  to trap on division by zero (and on the `MIN / -1` overflow case) — the
  resulting binary aborts rather than continuing with a garbage value.
  This is a real, disclosed behavior inherited from Cranelift's own
  contract, in the same risk family as plan 09's already-accepted
  unchecked array indexing (a wild read is UB there; a trap is the
  analogous outcome here) — not a new safety regression this plan
  introduces. A catchable exception would mean wiring division into
  plan 11's setjmp/longjmp handler stack, itself already disclosed as a
  non-native-unwinding stand-in; that's real follow-up work, not this
  plan's job.
- **`Float64` `%` is out of scope; `Int64` `%` is fully in scope.**
  Cranelift has no native floating-point-remainder instruction (unlike
  `fadd`/`fsub`/`fmul`/`fdiv`, which map straight to hardware), and this
  codegen has no general libm-call plumbing yet — every existing runtime
  call (`emerald_alloc`, the exception-handling functions) is a
  hand-declared import for one specific purpose, not a mechanism for
  calling arbitrary C library functions like `fmod`. `Int64` `%` lowers
  directly to Cranelift's `srem`, no such gap.
- **This unblocks, but does not itself re-run, three of plan 15's
  explicitly-deferred benchmark programs** (sum of squares, Fibonacci,
  matrix multiplication — all blocked on "this compiler has neither
  `-` nor `*`", per plan 15's own Decision log). Re-benchmarking is a
  separate, later exercise: consistent with inception §21/§22's own
  "do not optimize the compiler before there is a working compiler"
  posture, this plan's job is making the operators exist and be
  correct, not re-measuring performance.

## Leaf: leaf-ast-operators

### 1. Context
- Why: `Expr` has exactly one arithmetic variant (`Add`) and one
  structural comparison variant (`Compare`); `grammar.lalrpop`'s only
  precedence tiers above `CallExpr` are `AddExpr` and `Expr` (comparison)
  — verified this session directly against the grammar file. `Num` is
  `r"[0-9]+"` with no sign; there is no unary-prefix production anywhere
  in the grammar.
- Target state: `Expr::{Sub(Box<Expr>, Box<Expr>), Mul(..), Div(..),
  Rem(..), Neg(Box<Expr>), Not(Box<Expr>), And(Box<Expr>, Box<Expr>),
  Or(Box<Expr>, Box<Expr>)}` — one variant per operator, matching
  `Expr::Add`'s existing shape rather than introducing a generic
  `BinOp`/opcode-field enum. Six new precedence tiers, loosest to
  tightest: `OrExpr` (`||`, left-assoc) → `AndExpr` (`&&`, left-assoc) →
  the existing comparison level (renamed/kept non-associative) →
  `AddExpr` (now `+` *and* `-`) → a new `MulExpr` (`*`/`/`/`%`) →
  a new `UnaryExpr` (prefix `-`/`!`, right-assoc, binding tighter than
  `MulExpr`, sitting directly above `CallExpr`). The identical tier
  structure is mirrored on the `Stmt`-initial side (`StmtOrExpr` through
  `StmtUnaryExpr`), preserving the existing exclusion of a bare `[...]`
  array literal at `StmtPrimaryExpr` (plan 09's fix — still needed, still
  correct, just now sitting under more tiers). A negative literal
  (`-5`, `-2.0`) is `Expr::Neg` wrapping `Expr::Int`/`Expr::Float` — no
  separate signed-literal token; the unary tier already covers it.

### 2. Acceptance Criteria
1. Precedence is real, not just parseable: `2 + 3 * 4` parses as
   `Add(Int(2), Mul(Int(3), Int(4)))`, not `Mul(Add(2,3), 4)`; `-3 + 10`
   parses as `Add(Neg(Int(3)), Int(10))`; `a > 0 && b > 0 || c > 0`
   parses as `Or(And(Compare(a,Gt,0), Compare(b,Gt,0)), Compare(c,Gt,0))`
   (`&&` binds tighter than `||`).
2. Both of this plan's worked examples (Example A, Example B) parse
   without error into the expected shapes above.
3. `!x > 0` parses as `Not(Compare(x, Gt, 0))` is explicitly **not**
   required to hold — `!` binds at the unary tier, tighter than
   comparison, so `!x > 0` actually parses as `Compare(Not(x), Gt, 0)`;
   the acceptance criterion is that this is what the grammar produces
   (verified by a real parser test), documented as real, Ruby-divergent
   precedence behavior rather than silently left unspecified.
4. Regression: every prior plan's example (`hello.em` through the
   plan 12 module example) parses identically to before.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. new precedence tests | agent-claimed-locally |

---

## Leaf: leaf-sema-operators

### 1. Context
- Why: `infer_expr_type`/`check_stmt` have no rules for `Sub`/`Mul`/
  `Div`/`Rem`/`Neg`/`Not`/`And`/`Or` — those variants don't exist yet
  (leaf 1's job) and neither does their type-checking.
- Target state: `Sub`/`Mul`/`Div`/`Rem` each apply the exact rule
  `Expr::Add` already enforces — both operands must resolve to the same
  numeric type (`Int64` or `Float64`), result is that type, no implicit
  conversion. `Neg` requires a numeric operand, returns the same type.
  `Not` requires a `Boolean` operand, returns `Boolean`. `And`/`Or`
  require both operands `Boolean`, return `Boolean`.

### 2. Acceptance Criteria
1. Both worked examples type-check `Ok(())`.
2. `5 - 2.0` (mismatched numeric types, mirroring the existing rejected
   `5 + 2.0` case for `Add`) is rejected with a diagnostic naming both
   types — same standard as every existing type-mismatch diagnostic.
3. `!5` (non-Boolean operand to `!`) and `5 && (3 > 1)` (non-Boolean left
   operand to `&&`) are each rejected with a diagnostic, not a panic.
4. `0 - 5` and `-5` both type-check as `Int64`; `0.0 - 5.0` and `-5.0`
   both type-check as `Float64`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-operators

### 1. Context
- Why: `build_expr`'s only arithmetic case is `Expr::Add`, dispatching
  on whether the operand's Cranelift value type is `F64` (`fadd`) or not
  (`iadd`) — verified this session directly against
  `crates/emerald-codegen/src/lib.rs`. `Stmt::If` codegen already
  branches via `create_block`/`brif` into a then-block and a merge block
  — the shape `&&`/`||` need to reuse for real short-circuiting.
- Target state: `Sub`/`Mul`/`Div`/`Rem` extend `Add`'s exact F64-vs-int
  dispatch (`isub`/`fsub`, `imul`/`fmul`, `sdiv`/`fdiv`, `srem` for
  `Int64` only — see Decision log on `Float64 %`). `Neg` lowers to
  `ineg`/`fneg` on its single operand's Cranelift type. `Not` flips the
  boolean value `Expr::Compare`'s `icmp` already produces. `And`/`Or`
  each build a real two-block short-circuit: evaluate the left operand;
  branch — for `&&`, a false left operand jumps straight to a merge
  block carrying `false` without ever building the right operand's code;
  a true left operand falls into a block that evaluates the right
  operand and jumps to the same merge block carrying that result
  (mirrored, inverted, for `||`).

### 2. Acceptance Criteria
1. Example A, compiled, linked, and run, prints exactly `120\n3\n2\n7\n`
   — real executed proof of `Mul`+`Sub`+recursion+comparison working
   together, plus `Div`/`Rem`/`Neg` each independently.
2. Example B, compiled, linked, and run, prints exactly
   `3\n300\n200\n4\n400\n` — real executed proof that `noisy(1)`/
   `noisy(2)` are genuinely never invoked (their `puts` never fires),
   not just that the final boolean result happens to be correct.
3. Integer division by a runtime-zero divisor (e.g. `10 / (5 - 5)`)
   traps at runtime rather than silently producing a wrong value or
   corrupting subsequent state — verified by asserting the compiled
   binary exits via a signal/trap, not code `0`, matching the Decision
   log's disclosed semantics.
4. An unsupported shape defensively returns a descriptive `Err`, not a
   panic — same AC standard as every prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run Example A/B output and the trap case | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- `true`/`false`/`nil` literals — `Type::Boolean` already exists;
  `&&`/`||`/`!` work fully without literals (Example B). See Decision
  log; deferred to a future stdlib-expansion plan.
- Ternary (`cond ? a : b`), spaceship (`<=>`), range operators (`..`/
  `...`), bitwise operators (`& | ^ ~ << >>`), exponentiation (`**`),
  the `and`/`or`/`not` keyword spellings, compound assignment (`+=` etc.
  — `SEMANTICS.md`/`GRAMMAR.md` both mark it KEEP, but it needs a
  desugaring pass this plan doesn't add), and method-operator overloading
  (defining `+`/`-`/etc. on a user class) — all remain exactly as
  aspirational in `spec/GRAMMAR.md` as before this plan; see Decision
  log's precedence-scope note.
- `Float64` `%` — see Decision log (no native Cranelift `frem`, no libm
  call plumbing).
- A catchable `ZeroDivisionError`/runtime exception for division by zero
  — see Decision log; the current behavior is a hard trap.
- Constant folding / compile-time evaluation of arithmetic on literals
  (e.g. folding `2 + 3 * 4` to `14` at compile time) — a real,
  reasonable optimization, but inception §21/§22 explicitly gate
  optimization work behind a working, correct compiler first; this plan
  is squarely in the "correct" half.
- Re-running plan 15's benchmark suite now that `-`/`*` exist — see
  Decision log; a separate, later exercise.
