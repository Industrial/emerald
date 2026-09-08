---
name: Compound and Multiple Assignment
overview: "`+= -= *= /= %=` and fixed-arity multiple assignment (`a, b = b, a`) — spec/GRAMMAR.md §3's two remaining unimplemented KEEP rows, plus the bare-local-reassignment statement shape neither can exist without."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-bare-assignment
    content: "Stmt::Assign { name, value } — reassign an already-declared plain local with no type annotation restated, the missing prerequisite both compound and multiple assignment need as their target shape"
    status: pending
  - id: leaf-compound-assignment
    content: "+= parses/desugars now; -= *= /= %= are grammar-ready but blocked on plan 18's operators landing first"
    status: pending
  - id: leaf-multiple-assignment
    content: "Fixed-arity `a, b = <expr>, <expr>` over already-declared plain locals, RHS evaluated into temporaries before any target is written (real swap semantics)"
    status: pending
isProject: false
---

# Plan 31 — Compound and Multiple Assignment

This is plan 31 of the 28-35 follow-up batch (bitwise operators,
control-flow completeness, `for`-`in`, this plan, class inheritance,
field-access sugar, blocks/`yield`, debug info) — independent siblings,
each owning one distinct language-completeness gap. It is **not** a row
in [`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
(that table's own Completion note calls Emerald v1 done at row 15; this
is post-v1 scope, same posture as plan 17 — this plan does not touch
`plan-of-plans.md` or any other plan file).

Concrete proof this plan targets:
```ruby
total: Int64 = 0
i: Int64 = 0
while i < 5
  total += i
  i += 1
end
puts total

a: Int64 = 1
b: Int64 = 2
a, b = b, a
puts a
puts b
```
Expected output: `10`, `2`, `1` — a real accumulator loop using `+=`, and
a real swap proving multiple assignment evaluates both right-hand sides
before writing either target (a left-to-right, unbuffered implementation
would print `2`, `2`, silently failing the swap).

## Decision log

- **Neither compound nor multiple assignment can exist without a
  statement shape this compiler doesn't have yet: reassigning an
  already-declared plain local with no type annotation.** Verified this
  session against `crates/emerald-parser/src/grammar.lalrpop`'s `Stmt`
  production: the only assignment-shaped statements today are `Ident ":"
  TypeName "=" Expr` (`Stmt::Let` — a type annotation is mandatory every
  time) and the `=>?`-guarded `StmtExpr "=" Expr` rule, whose match arm
  only accepts `Expr::InstanceVar` (→ `SetField`) or `Expr::Index` (→
  `SetIndex`); a bare `Expr::Ident` on the left falls through to that
  same arm's `other => Err("invalid assignment target")` case. This is
  not a shortcut — it's why every existing example that reassigns a
  local (e.g. plan 09's `sum: Int64 = sum + arr[i]` inside a `while`
  loop) *restates the type annotation on every reassignment*, because
  that's the only grammar shape available. `x += 1` and `a, b = b, a`
  carry no type annotation at all, so they cannot desugar into `Let` —
  this plan adds a genuine new AST node, `Stmt::Assign { name: String,
  value: Expr }`, valid only when `name` is already bound in scope (sema
  looks it up in `env`; an undefined name is a diagnostic, the same
  "undefined variable" standard every other plan already uses — not a
  panic, not a silent fresh declaration). `leaf-bare-assignment` builds
  this as its own leaf because both remaining leaves depend on it as
  their target shape, not because reassignment-without-restating-the-
  type is this plan's headline feature — it's a real, disclosed
  side-effect: this plan also finally gives Emerald bare local
  reassignment syntax (`x = 5`, no `+=`/multi-assign involved), which
  didn't exist before.
- **Codegen for `Stmt::Assign` reuses the exact update-in-place
  mechanism `Stmt::Let` already uses for a loop counter's repeated
  reassignment** (Cranelift's SSA `Variable` type requires exactly this
  "declare once, `def_var` on every update" pattern — a `while` loop
  incrementing a counter, e.g. `i: Int64 = i + 1`, already works today,
  which is only possible if `Let`'s codegen already reuses an existing
  `Variable` by name rather than redeclaring one every time). `leaf-
  bare-assignment`'s acceptance criteria require verifying this directly
  against `crates/emerald-codegen/src/lib.rs`'s actual `build_stmt`
  `Let` arm at execution time (this plan's Decision log states the
  expected mechanism; the leaf itself confirms it against real source,
  same discipline as every prior plan) — if `Let`'s codegen turns out
  not to already share this path, `Stmt::Assign`'s codegen still only
  needs the "existing-variable, `def_var`" half of whatever `Let`
  already does for reassignment, which is a strictly smaller surface.
- **Compound assignment desugars to `Stmt::Assign { name, value:
  Expr::Add(Ident(name), rhs) }`** (per `spec/GRAMMAR.md` §3's own
  description: "Desugars to `x = x + 1` under the receiver's statically
  resolved `+` method") — a parse-time desugaring into the new bare-
  assignment shape, not a new runtime mechanism or a new `Stmt` variant
  of its own.
- **Only `+=` is actually buildable by this plan; `-= *= /= %=` are
  grammar-ready but functionally blocked on plan 18** (arithmetic/
  logical operators — `2026-09-08T213500Z-plan-18-arithmetic-and-
  logical-operators.md`, already written) **landing first**, since
  `Expr::Sub`/`Mul`/`Div`/`Mod` don't exist in the AST yet (only
  `Expr::Add` does, verified against `crates/emerald-parser/src/ast.rs`
  this session). `leaf-compound-assignment` adds all five tokens and all
  five desugaring rules at the grammar level in one pass (cheap, and
  avoids a second grammar-touching leaf later), but its *acceptance
  criteria* only require a real compiled-and-run proof for `+=`; the
  other four are structurally present and reference `Expr::Sub`/etc. by
  name, so this plan compiles cleanly only once plan 18 lands — this
  plan's own quality gate does not depend on plan 18, but the described
  target state's `-=`/`*=`/`/=`/`%=` desugaring rules do, and that
  dependency is stated here rather than silently assumed.
- **Multiple assignment is scoped to fixed-arity, already-declared plain
  locals only** (`a, b = b, a`) — not a fresh multi-declaration
  (`a, b: Int64, Int64 = 1, 2`, an awkward grammar shape this language's
  mandatory-per-declaration type annotation doesn't lend itself to
  naturally), not mixed instance-var/index targets (`@x, arr[i] = ...`
  — a real possible future extension, deliberately deferred to keep this
  plan's `Stmt::MultiAssign` variant a flat `Vec<String>` of plain names
  rather than a `Vec<Expr>` of arbitrary assignable shapes), and no
  splat (`a, *b = [1, 2, 3]`) — `spec/GRAMMAR.md` §3 itself already
  marks splat UNDECIDED, deferred pending collections; collections now
  exist (plan 09), but splat's variable-length capture into an
  `Array[T]` remainder is a distinct, larger feature this plan doesn't
  need to prove fixed-arity multiple assignment works.
- **RHS expressions are all evaluated into temporaries before any target
  is written** — the concrete reason `a, b = b, a` is worth proving at
  all: Ruby's multiple assignment evaluates every right-hand side first,
  which is what makes it a real swap. An implementation that assigns
  left-to-right without buffering (`a = b` then `b = a`) silently
  corrupts the swap (`b`'s original value is gone by the second
  assignment) while still type-checking and compiling cleanly — a real,
  easy-to-get-wrong correctness pitfall, not just an implementation
  detail, which is why this plan's own worked example is a swap and not
  two independent-looking assignments.
- **`||=`/`&&=` conditional compound assignment are out of scope.**
  `spec/GRAMMAR.md` §3 itself ties `||=` to "`x`'s type already admitting
  `Nil`" — a separate sibling plan (25, standard library expansion)
  already made its own narrow, disclosed scope decision about `nil`
  (a bare sentinel value, explicitly *not* a full `T?` optional-type
  system). Building `||=` correctly needs that fuller optional-type
  system, not just a `nil` literal — this plan doesn't redo plan 25's
  scope call or attempt `||=`/`&&=` against its narrower `nil`.
- **Parallel/nested destructuring (`(a, b), c = [[1, 2], 3]`) is out of
  scope** — `spec/GRAMMAR.md` §3 itself marks this REMOVE, not merely
  deferred; there is nothing to build toward here.

## Leaf: leaf-bare-assignment

### 1. Context
- Why: neither of this plan's two headline features has a target
  statement shape to desugar into — see Decision log. This is a genuine
  prerequisite leaf, not scope creep.
- Target state: `Stmt::Assign { name: String, value: Expr }` in
  `crates/emerald-parser/src/ast.rs`; a new grammar alternative `<name:
  Ident> "=" <value:Expr> => Stmt::Assign { name, value }` in
  `crates/emerald-parser/src/grammar.lalrpop` (verify it doesn't collide
  with the existing `StmtExpr "=" Expr` guarded rule's own bare-`Ident`
  fallthrough — the cleanest fix is likely folding this case directly
  into that rule's `=>?` match arm as a third accepted shape,
  `Expr::Ident(name) => Ok(Stmt::Assign { name, value })`, rather than
  adding a second, competing production); `emerald-sema` requires `name`
  already present in `env` (undefined-variable diagnostic otherwise,
  matching every other plan's existing standard) and checks `value`'s
  type against the existing binding; `emerald-codegen` reuses the
  existing-`Variable`/`def_var` update mechanism (see Decision log).

### 2. Acceptance Criteria
1. `x: Int64 = 1` followed by `x = 2` (bare reassignment, no type
   annotation) parses to `Stmt::Let` then `Stmt::Assign`, type-checks
   `Ok(())`, and — compiled, linked, run — a program reassigning and
   `puts`-ing a local this way prints the reassigned value, not the
   original.
2. `y = 5` where `y` was never declared is rejected with an
   undefined-variable diagnostic, not a panic and not a silent fresh
   declaration.
3. `x = "mismatched"` where `x` is declared `Int64` is rejected with a
   type-mismatch diagnostic, using `x`'s already-declared type as the
   expected type — proving the reassignment is genuinely checked against
   the existing binding, not accepted unconditionally.
4. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, the collections/exceptions/module examples) still
   parses and type-checks identically — this leaf only adds a new
   alternative, it must not change how any existing `Let`/`SetField`/
   `SetIndex` shape is parsed or checked.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-sema/src/lib.rs`, `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. new bare-reassignment compiled-and-run test | agent-claimed-locally |

---

## Leaf: leaf-compound-assignment

### 1. Context
- Why: no `+=`-family token exists in the grammar at all today
  (verified this session — `grammar.lalrpop` has no `+=`/`-=`/etc.
  terminal).
- Target state: grammar gains `"+=" "-=" "*=" "/=" "%="` tokens and five
  parallel `Stmt` alternatives, each desugaring at parse time to
  `Stmt::Assign { name, value: Expr::<Op>(Box::new(Expr::Ident(name)),
  Box::new(rhs)) }` per the Decision log. `+=` is real and buildable now
  (`Expr::Add` exists); `-=`/`*=`/`/=`/`%=` reference `Expr::Sub`/`Mul`/
  `Div`/`Mod`, which don't exist until plan 18 lands — added here at the
  grammar level regardless (cheap, avoids revisiting the grammar twice),
  but only `+=` is exercised by this leaf's own acceptance criteria.

### 2. Acceptance Criteria
1. `total += i` desugars to (and is verified, via the parsed AST
   directly, to equal) `Stmt::Assign { name: "total", value:
   Expr::Add(Ident("total"), Ident("i")) }`.
2. This plan's own worked accumulator-loop example, compiled, linked,
   and run, prints `10` — real executed proof `+=` works as a real
   accumulator across five loop iterations (0+1+2+3+4), not just that it
   parses.
3. `-=`/`*=`/`/=`/`%=` parse to the analogous `Expr::Sub`/`Mul`/`Div`/
   `Mod`-shaped `Stmt::Assign` (a parser-level AST-shape test only —
   this leaf does not compile-and-run them, since their operators don't
   exist in codegen yet; that's plan 18's job, not a re-litigation of it
   here).
4. Regression: every prior plan's example still parses identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |
| Workspace (real `+=` run) | `cargo test --workspace` | all pass, incl. accumulator-loop prints `10` | agent-claimed-locally |

---

## Leaf: leaf-multiple-assignment

### 1. Context
- Why: no comma-separated multi-target assignment shape exists at all
  (verified this session).
- Target state: `Stmt::MultiAssign { names: Vec<String>, values:
  Vec<Expr> }`; grammar rule `<names:(<Ident> ",")+> <last:Ident> "="
  <vals:(<Expr> ",")+> <last_val:Expr> => ...` (fixed-arity — sema
  rejects a `names.len() != values.len()` mismatch as a real diagnostic,
  not a panic, not silent truncation/padding) requiring every named
  target already declared (same `env`-lookup rule as `leaf-bare-
  assignment`'s `Stmt::Assign`, generalized to N targets). Codegen
  evaluates every value in `values` into a temporary Cranelift value
  *before* writing any target via `def_var` — seeDecision log for why
  this ordering is the entire point of this leaf.

### 2. Acceptance Criteria
1. `a, b = b, a` (both already declared) parses to `Stmt::MultiAssign`,
   type-checks `Ok(())` when both sides' types line up positionally.
2. This plan's own worked swap example, compiled, linked, and run,
   prints `2` then `1` (the swapped values) — real executed proof the
   evaluate-then-assign ordering is correct, not a left-to-right
   corruption (which would print `2`, `2`).
3. A positional type mismatch (e.g. `a, b = "s", 1` where `a: Int64`) is
   rejected with a diagnostic naming which target/position mismatched.
4. An arity mismatch (`a, b = 1, 2, 3` or `a, b, c = 1, 2`) is rejected
   with a diagnostic, not a panic.
5. An undefined target name is rejected the same way `leaf-bare-
   assignment`'s single-target case is.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-sema/src/lib.rs`, `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real linked-and-run swap printing `2\n1\n` | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
(`-=`/`*=`/`/=`/`%=`'s own compiled-and-run proof is explicitly plan
18's quality gate, not this one's — see Decision log.)

## Out of scope / deferred
- `-=`/`*=`/`/=`/`%=` compiled-and-run proof — grammar-ready here,
  functionally blocked on plan 18's operators; see Decision log.
- Splat in multiple assignment (`a, *b = [...]`) — `spec/GRAMMAR.md`
  itself marks this UNDECIDED; a distinct, larger feature.
- Parallel/nested destructuring — `spec/GRAMMAR.md` marks this REMOVE
  outright.
- Fresh multi-declaration with type annotations (`a, b: Int64, Int64 =
  1, 2`) — see Decision log's grammar-awkwardness argument.
- Mixed instance-var/index targets in multiple assignment (`@x, arr[i]
  = ...`) — see Decision log; `Stmt::MultiAssign` is plain-local-only.
- `||=`/`&&=` conditional compound assignment — ties to a full optional-
  type system beyond plan 25's narrow, already-scoped `nil`; see
  Decision log.
