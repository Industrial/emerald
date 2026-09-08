---
name: Bitwise Operators
overview: "`&`, `|`, `^`, `~`, `<<`, `>>` on `Int64` — spec/GRAMMAR.md §4's own KEEP row, explicitly left aspirational by plan 18's Decision log and Out-of-scope section when it added the rest of arithmetic/logical operators."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-bitwise
    content: "Expr::{BitAnd,BitOr,BitXor,BitNot,Shl,Shr}; three new binary precedence tiers (Shift > BitAnd > BitOr/BitXor) slotted between plan 18's AddExpr and Compare tiers, plus BitNot joining the existing UnaryExpr tier"
    status: pending
  - id: leaf-sema-bitwise
    content: "Int64-only operand rules for all six operators — no Float64, no other integer widths (none exist)"
    status: pending
  - id: leaf-codegen-bitwise
    content: "Cranelift lowering: band/bor/bxor/bnot/ishl/sshr, mirroring Expr::Add's existing dispatch shape"
    status: pending
isProject: false
---

# Plan 28 — Bitwise Operators

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md), for
the same reason plans 17/18 weren't: that table's Completion note calls
Emerald v1 done as of row 15. This is new, post-v1 language-completeness
scope — not touching `plan-of-plans.md` or any other existing plan file.
It is one of a batch of sibling plans (28-35) each covering one distinct
language-completeness gap; the others (control-flow completeness,
`for`-`in`, compound/multiple assignment, class inheritance,
field-access sugar, blocks/`yield`, debug info) are out of scope here.

`spec/GRAMMAR.md` §4 marks "Bitwise operators (`& | ^ ~ << >>`)... KEEP...
Defined on integer types only" — and plan 18 (arithmetic/logical
operators), the most recent plan to touch this precedence table, said so
explicitly in its own Decision log and Out-of-scope section: bitwise
operators "remain exactly as aspirational in `spec/GRAMMAR.md` as before
this plan." This plan is that follow-up.

Concrete proof this plan targets — a real bit-flags program, compiled,
linked, and run, exact stdout asserted:

```ruby
READ: Int64 = 1
WRITE: Int64 = 2
EXEC: Int64 = 4

def has_flag(flags: Int64, flag: Int64) -> Boolean
  return flags & flag == flag
end

perms: Int64 = READ | WRITE
puts perms
if has_flag(perms, READ)
  puts 1
end
if has_flag(perms, EXEC)
  puts 0
end
puts perms ^ WRITE
puts ~0
puts 1 << 4
puts 256 >> 4
```
Expected: `3\n1\n1\n-1\n16\n16\n` — `perms` is `READ | WRITE` = `3`;
`has_flag(perms, READ)` is true (`3 & 1 == 1`); `has_flag(perms, EXEC)`
is false (`3 & 4 == 4` is `0 == 4`) so its `puts 0` never fires;
`perms ^ WRITE` is `1`; `~0` is `-1`; `1 << 4` is `16`; `256 >> 4` is
`16`. `has_flag`'s body (`flags & flag == flag`) also proves `&` binds
tighter than `==` for real, not just that the expression happens to be
parseable.

## Decision log

- **Ground truth for today's gap is `crates/emerald-parser/src/
  grammar.lalrpop`, re-verified this session directly**: it has zero
  `&`/`|`/`^`/`~`/`<<`/`>>` terminals or productions of any kind, and
  `crates/emerald-sema/src/lib.rs`'s `Type` enum has exactly `Int64`,
  `Float64`, `String`, `Boolean`, `Void`, `Class`, `Array`, `Proc` — no
  `Int32`/`UInt64`/other integer width. Bitwise operators land on
  `Int64` only; `Float64` bitwise semantics don't exist in any language
  this compiler's spec is modeled on either, so there's nothing to
  approximate there — see Out of scope.
- **Codegen target is described against the last documented
  implementation, not today's on-disk state.** As of this session,
  `crates/emerald-codegen/src/lib.rs` is a large Cranelift-based file
  (`build_expr`/`build_stmt`/`Ctx`/etc. — read in full earlier this
  session) that plan 18 also targeted. Separately, and unrelated to this
  plan, a different concurrent agent is mid-refactor implementing plan
  16's Cranelift-vs-LLVM bake-off outcome directly inside
  `crates/emerald-codegen/`, `crates/emerald-codegen-llvm/`, and the
  workspace `Cargo.toml` — at the time of writing, `crates/
  emerald-codegen/src/lib.rs` is transiently absent from the working
  tree mid-edit. This plan's codegen leaf targets the Cranelift API
  shape documented above (the last stable, fully-read implementation),
  not whatever the file's exact bytes are at this instant; if plan 16's
  outcome has landed a different backend crate by the time this plan is
  executed, the codegen leaf's *target instruction set* translates
  directly (every backend candidate under consideration — Cranelift,
  LLVM/inkwell — has native bitwise-AND/OR/XOR/NOT/shift instructions;
  only the specific Rust API calls differ), so this plan isn't actually
  blocked on which backend wins, only its exact code samples assume
  Cranelift's API.
- **Precedence: three new binary tiers between plan 18's `AddExpr` and
  its `Compare` tier, plus `~` joining plan 18's existing `UnaryExpr`
  tier.** Ruby's real precedence table (tightest to loosest, the
  relevant slice): unary `! ~ +` > `**` > unary `-` > `* / %` >
  `+ -` > `<< >>` > `&` > `| ^` > relational (`> >= < <=`) > equality
  (`== != <=>`) > `&&` > `||`. Plan 18 already collapsed Ruby's
  relational and equality levels into one non-associative `Compare`
  tier (its own Decision log: "six real tiers, not Ruby's full table")
  — this plan follows the same honesty standard rather than
  retroactively re-splitting `Compare`, which is plan 18's leaf, not
  this one's. So, tightest to loosest, this plan inserts: `UnaryExpr`
  (now `-`/`!`/`~`, tightest — unchanged tier, one more operator) →
  `MulExpr` (unchanged) → `AddExpr` (unchanged) → new `ShiftExpr`
  (`<<`/`>>`, left-assoc) → new `BitAndExpr` (`&`, left-assoc) → new
  `BitOrExpr` (`|`/`^`, left-assoc, both at one precedence level —
  Ruby's real table actually gives `|` and `^` the same precedence as
  each other, so merging them costs nothing, unlike plan 18's
  relational/equality merge which was a real simplification) →
  `Compare` (plan 18's tier, unchanged) → `AndExpr`/`OrExpr` (plan 18's
  tiers, unchanged).
- **Tokenization: `&`/`|` do not collide with plan 18's `&&`/`||`.**
  LALRPOP's built-in lexer (there is no separate hand-rolled tokenizer —
  verified: `grammar.lalrpop`'s own `match { }` block generates it)
  already prioritizes longer literal-string terminals over shorter ones
  that share a prefix wherever both are declared, the same way `->`
  already coexists with no bare `-`-prefixed collision risk today
  (plan 18 introduces bare `-` for subtraction/negation without
  conflicting with `->`'s existing use). Declaring `"&"`/`"|"` alongside
  the already-declared `"&&"`/`"||"` needs no special handling beyond
  that existing, already-relied-upon longest-match behavior — flagged
  here as a real tokenization fact to verify empirically during
  implementation (a genuine LALRPOP build-time conflict would surface
  immediately as a build error), not asserted as risk-free by
  assumption alone.
- **`~` (unary bitwise-NOT) joins `UnaryExpr` alongside plan 18's unary
  `-`/`!` without colliding**, because each is its own distinct
  single-character prefix terminal — the grammar accepts one
  unary-prefix token then recurses into the next-tighter tier, the same
  shape plan 18 already established for `-`/`!`; adding a third
  alternative to that same production is additive, not restructuring.
- **This plan does not implement `**` (exponentiation)** even though
  it sits in the same Ruby precedence neighborhood — `**` is arithmetic,
  not bitwise, and plan 18 already explicitly deferred it as
  aspirational; picking it up here would blur this plan's own scope
  line. See Out of scope.

## Leaf: leaf-ast-bitwise

### 1. Context
- Why: no AST shape or grammar production exists for any bitwise
  operator (verified this session against `crates/emerald-parser/src/
  grammar.lalrpop` and `crates/emerald-parser/src/ast.rs` — zero
  matches for `&`/`|`/`^`/`~`/`<<`/`>>` as operator terminals).
- Target state: `Expr::{BitAnd(Box<Expr>, Box<Expr>), BitOr(..),
  BitXor(..), BitNot(Box<Expr>), Shl(..), Shr(..)}` — one variant per
  operator, matching `Expr::Add`'s/plan 18's existing per-operator-variant
  shape rather than a generic opcode-field enum. Three new `Expr`
  precedence tiers (`ShiftExpr`, `BitAndExpr`, `BitOrExpr`, per the
  Decision log's ordering) inserted between `AddExpr` and the existing
  `Compare`/`Expr` level; `BitNot` added as a third alternative on the
  existing `UnaryExpr` production. Per plan 18's own established
  pattern (and the same reason: a bare `[...]` array literal must stay
  excluded from `FOLLOW(CallExpr)` in statement-initial position), each
  new tier gets a parallel `Stmt`-initial mirror (`StmtShiftExpr`,
  `StmtBitAndExpr`, `StmtBitOrExpr`) — three more tiers on top of the
  three "real" ones, the same doubling plan 18 already flagged as a
  structural cost of plan 04/09's earlier decisions, not new complexity
  this plan invents.

### 2. Acceptance Criteria
1. Precedence is real, not just parseable: `1 | 2 & 3` parses as
   `BitOr(Int(1), BitAnd(Int(2), Int(3)))` (`&` binds tighter than `|`);
   `1 << 2 & 3` parses as `BitAnd(Shl(Int(1), Int(2)), Int(3))` (`<<`
   binds tighter than `&`); `flags & flag == flag` (this plan's own
   worked example) parses as `Compare(BitAnd(flags, flag), Eq, flag)`,
   not `BitAnd(flags, Compare(flag, Eq, flag))`.
2. `~0` parses as `BitNot(Int(0))`; `~x + 1` parses as
   `Add(BitNot(Ident(x)), Int(1))` (`~` binds at the same tight tier as
   `-`/`!`, tighter than `+`).
3. This plan's full worked example parses without error into the
   expected shapes above.
4. Regression: every prior plan's example (`hello.em` through plan 18's
   two worked examples) parses identically to before.

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

## Leaf: leaf-sema-bitwise

### 1. Context
- Why: `infer_expr_type`/`check_stmt` have no rules for any of the six
  new `Expr` variants — they don't exist yet (leaf 1's job) and neither
  does their type-checking.
- Target state: `BitAnd`/`BitOr`/`BitXor`/`Shl`/`Shr` each require both
  operands to resolve to `Type::Int64` exactly — no `Float64`, unlike
  `Add`/`Sub`/`Mul`/`Div` (plan 18), which accept either numeric type as
  long as both operands match. `BitNot` requires a single `Int64`
  operand. All six return `Type::Int64`.

### 2. Acceptance Criteria
1. The worked example type-checks `Ok(())`.
2. `2.0 & 1` (a `Float64` operand to a bitwise operator) is rejected
   with a diagnostic naming the operator and the offending type — not
   silently truncated or coerced.
3. `~2.5` is rejected the same way.
4. `1 << 2` and `1 & 2` and `~1` each type-check as `Int64`, verified
   directly (not just "no error").

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-bitwise

### 1. Context
- Why: no codegen case exists for any of the six new `Expr` variants.
  Cranelift (the last fully-documented codegen backend this session —
  see Decision log on today's transient, unrelated in-flight
  migration) has direct native instructions for exactly this operator
  set: `band`, `bor`, `bxor`, `bnot`, `ishl` (left shift), and `sshr`
  (arithmetic/signed right shift — the correct choice for a signed
  `Int64`, as opposed to `ushr`, logical right shift, which would be
  wrong for a negative operand).
- Target state: `BitAnd`/`BitOr`/`BitXor`/`BitNot`/`Shl`/`Shr` each
  lower to exactly one Cranelift instruction on their operand(s)'
  already-`Int64` Cranelift values (no F64-vs-int dispatch needed,
  unlike `Add`/`Sub`/`Mul` — sema (leaf 2) has already rejected any
  non-`Int64` operand by the time codegen sees these nodes).

### 2. Acceptance Criteria
1. The worked example, compiled, linked, and run, prints exactly
   `3\n1\n1\n-1\n16\n16\n` — real executed proof of `|`, `&` (via
   `has_flag`, also proving `&`'s precedence against `==` at runtime,
   not just at parse time), `^`, `~`, `<<`, and `>>` all working
   together and printing a genuine negative value (`~0` → `-1`)
   correctly through the existing `puts` runtime path.
2. `1 << 63` (a shift that sets `Int64`'s sign bit) round-trips through
   `puts` as the correct negative two's-complement value, not garbage —
   real proof `ishl`'s bit pattern and the runtime's signed-print path
   agree with each other.
3. An unsupported shape defensively returns a descriptive `Err`, not a
   panic — same AC standard as every prior codegen plan.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run worked-example output and the `1 << 63` case | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- Bitwise operators on anything but `Int64` — `Float64` has no bitwise
  semantics in any language this spec is modeled on; no other integer
  width exists in this compiler. See Decision log.
- Shift-count validation (e.g. rejecting or defining behavior for
  `1 << 64` / `1 << -1`) — Cranelift's `ishl`/`sshr` semantics for an
  out-of-range shift amount are taken as-is, undisclosed further than
  this line; a real, narrow follow-up if it turns out to matter in
  practice.
- `**` (exponentiation) — arithmetic, not bitwise; plan 18 already
  deferred it, and picking it up here would blur this plan's scope line.
  See Decision log.
- `<<`/`>>` as string-append/stream-shovel operator overloads (Ruby
  itself overloads `<<` this way for `String`/`Array`/`IO`) — purely the
  bitwise-integer meaning is in scope here.
- Compound bitwise assignment (`&=`, `|=`, `^=`, `<<=`, `>>=`) — compound
  assignment in general is a separate sibling plan in this same batch
  (control-flow/assignment completeness); not duplicated here.
- Ternary, spaceship (`<=>`), range operators, exponentiation (`**`),
  the `and`/`or`/`not` keyword spellings, method-operator overloading —
  all remain exactly as aspirational in `spec/GRAMMAR.md` as before this
  plan, per plan 18's own already-established Out-of-scope line.
