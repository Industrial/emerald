---
name: String Literals
overview: "Real `\"...\"` string literal syntax end to end — lexer, grammar, AST, sema, codegen — closing the gap plan 15 found: `Type::String` exists with no literal syntax able to produce a value of it."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-parser-strings
    content: "A `\"...\"` terminal in grammar.lalrpop's match block (with \\\" and \\n escapes decoded in the parser action) + Expr::StringLit(String)"
    status: pending
  - id: leaf-sema-strings
    content: "Expr::StringLit infers Type::String; Expr::Add and puts's intrinsic check both widen to accept Type::String"
    status: pending
  - id: leaf-codegen-strings
    content: "Object-file data-section string constants, emerald_print_str/emerald_string_concat/emerald_string_eq runtime helpers"
    status: pending
isProject: false
---

# Plan 19 — String Literals

This plan is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
for the same reason [plan 17](../../history/2026-09-08T213000Z-plan-17-ide-integration.md)
isn't: that table's Completion note already treats Emerald v1 as done at
row 15. This plan is new, post-v1 language-completeness scope, chosen
from a list of 10 candidate follow-ups; `plan-of-plans.md` and every
other existing plan file are left untouched.

Concrete proof this plan targets: a real program declaring
`s: String = "hello"`, concatenating two literals with `+`, comparing
two literals with `==`, and using both a `\"` and a `\n` escape,
compiled, linked, and run, prints exactly the expected bytes to stdout —
not a simulated or hand-typed result.

## Decision log

- **The gap this plan closes, precisely**: `emerald_sema::Type::String`
  already exists and `resolve_type` already maps the annotation
  `String` to it (`"String" => Ok(Type::String)`) — a `String`-typed
  *variable declaration* already parses and type-resolves today. But
  nothing can ever produce a `Type::String` *value*: `emerald_parser::
  Expr` has no `StringLit` variant, `grammar.lalrpop`'s `match` block
  only ever skips whitespace (`r"\s*" => { }, _`) with no quoted-string
  pattern, and `infer_expr_type`'s `puts` case explicitly rejects
  anything but `Int64`/`Float64`. This is exactly the gap plan 15's
  Decision log found and disclosed rather than worked around.
- **Plain double-quoted literals only — no interpolation, no
  single-quote variant, no heredocs.** `spec/GRAMMAR.md` §1 marks both
  `"..."`/`'...'` and `"#{expr}"` interpolation KEEP ("static string
  concatenation, not `eval`"), but interpolation needs a real
  expression-embedding desugaring pass this plan doesn't need in order
  to prove the core literal/representation/print/concat/compare path —
  the same shape of cut plan 09 made for `Hash[K, V]` ("proving one
  collection's... claim is this plan's job; [the rest] gets its own
  attention when a concrete program needs it").
- **Escape sequences: only `\"` and `\n` are decoded**, in the parser
  action, into the `String` value `Expr::StringLit` actually carries
  (so codegen and sema never see raw escape sequences — a literal's AST
  value is already the real string). `\\`, `\t`, `\u{...}`, etc. are
  real, disclosed gaps, not silently mis-handled: an unrecognized
  backslash sequence is a real parse error, not a raw passthrough.
- **Representation: a single pointer to a null-terminated UTF-8 byte
  buffer** (`spec/TYPE_SYSTEM.md` §9: "UTF-8 encoded... reference type...
  byte-level access goes through an explicit conversion, not through
  `String` itself pretending to be a byte array" — a bare pointer with
  no exposed length satisfies this directly). No length header, no
  small-string optimization. This mirrors `Array[T]`'s own established
  precedent exactly (plan 09's Decision log: "a raw pointer to a
  contiguous buffer... nothing more... a real, disclosed scope cut, not
  an oversight") — the same representation-simplicity call, made for
  the same reason, applied to the second reference type this compiler
  gets. Null-termination (rather than a length prefix) is chosen
  specifically because it lets `emerald_print_str` reuse libc's own
  `"%s"` formatting and `strcmp`/string-concatenation reuse ordinary C
  string functions in the runtime, with zero new conventions to
  hand-roll.
- **String literal bytes live in the object file's data section** — a
  genuinely new codegen capability. Every value this compiler has ever
  emitted before this plan is either a runtime-computed Cranelift SSA
  value or a heap allocation built at runtime (`emerald_alloc`, plans
  08/09/11); nothing has ever needed a compile-time-constant byte
  sequence embedded directly into the compiled object before. This
  plan is the first to call `ObjectModule::declare_data`/`define_data`
  and materialize a pointer to it via `global_value` at each literal's
  use site.
- **`+` on two `String` operands concatenates** (allocating a new
  null-terminated buffer via `emerald_alloc` + a new
  `emerald_string_concat` runtime helper) — reusing the existing
  `Expr::Add` AST node and `CompareOp`/type-checking dispatch rather
  than adding a new operator. This plan owns *all* of `Expr::Add`'s
  `Type::String` case; the separate, independently-authored "operators"
  plan (new binary `-`/`*`/`/`/`%`, unary `-`, logical `&&`/`||`/`!`) is
  scoped to numeric/boolean operators only and does not touch `Add` or
  `String` at all — no overlap between the two.
- **`==`/`!=` on two `String` operands work for free, with a caveat
  disclosed here rather than silently relied upon.** `infer_expr_type`'s
  `Expr::Compare` case already only requires `lt == rt` structurally and
  returns `Type::Boolean` regardless of *which* type that is — it never
  special-cased `Int64`/`Float64`. Once `Expr::StringLit` exists, `"a" ==
  "b"` already type-checks under that existing, unmodified logic. The
  real, disclosed side effect: `<`, `>`, `<=`, `>=` also silently
  "type-check" on two strings today under that same generic rule, with
  no lexicographic ordering defined anywhere — a latent gap in
  `Compare`'s pre-existing generic design that this plan is the first to
  actually expose (numbers are totally ordered, so it was never visible
  before). Fixing `Compare` to restrict `<`/`>`/`<=`/`>=` to orderable
  types is real, valuable follow-up work, explicitly not this plan's
  job — `leaf-codegen-strings` only implements codegen for `==`/`!=` on
  strings and leaves the other four `CompareOp` variants to fail loudly
  (a descriptive codegen `Err`, not silently-wrong machine code) if ever
  reached with two `String` operands.

## Leaf: leaf-parser-strings

### 1. Context
- Why: no lexical or grammar support for `"..."` exists at all — LALRPOP's
  own built-in tokenizer (configured via `grammar.lalrpop`'s `match`
  block) has no quoted-string pattern, and `Expr` has no variant to hold
  one.
- Target state: `match { r"\s*" => { }, _ }` gains a quoted-string regex
  terminal (matching `"` followed by any run of non-`"`/non-`\` bytes or
  a recognized `\"`/`\n` escape pair, up to a closing `"`); a new
  `Expr::StringLit(String)` AST variant; a grammar action that decodes
  the two supported escapes into the literal's real `String` value
  (never leaking a raw `\"`/`\n` byte pair into the AST); new
  `PrimaryExpr`/`StmtPrimaryExpr` alternatives producing `Expr::
  StringLit`.

### 2. Acceptance Criteria
1. `"hello"` parses to `Expr::StringLit("hello".to_string())`.
2. `"a\"b"` parses to `Expr::StringLit("a\"b".to_string())` — the escape
   is decoded, the AST value contains a real `"` byte, not the two-byte
   `\"` sequence.
3. `"line1\nline2"` parses to `Expr::StringLit("line1\nline2".to_string())`
   — a real newline byte, not the two-byte `\n` sequence.
4. An unterminated string literal (no closing `"`) is a real parse
   error, not a panic or a silent swallow of the rest of the file.
5. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, etc.) still parses identically — none of them contain a
   `"` today, so this is a pure grammar addition, not a modification of
   any existing production.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass | agent-claimed-locally |

---

## Leaf: leaf-sema-strings

### 1. Context
- Why: `infer_expr_type` has no case for `Expr::StringLit` yet, and its
  `puts`/`Expr::Add` cases explicitly reject any type but `Int64`/
  `Float64`.
- Target state: `Expr::StringLit(_) => Ok(Type::String)`;
  `Expr::Add`'s existing same-type check widens its "both operands must
  be `Int64` or `Float64`" rule to also accept `Type::String` (producing
  `Type::String`, i.e. concatenation); the `puts` intrinsic's type
  check widens the same way. `Expr::Compare` needs no code change at
  all (see Decision log) — its existing generic `lt == rt` rule already
  covers `String`.

### 2. Acceptance Criteria
1. `s: String = "hello"` type-checks `Ok(())`.
2. `a: String = "foo" + "bar"` type-checks `Ok(())` with the `Let`'s
   declared type; `"foo" + 1` (mismatched operand types) is rejected
   with the existing type-mismatch diagnostic, unchanged wording style.
3. `puts "hello"` type-checks `Ok(())`.
4. `if "abc" == "abc" ... end` type-checks `Ok(())` (proving the "free"
   `Compare` behavior described in the Decision log is real, not
   assumed).
5. Regression: `cargo test -p emerald-sema` — every existing numeric
   `Add`/`puts`/`Compare` test still passes unmodified (the widened
   checks are strictly additive, not a replacement of the numeric
   rule).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-strings

### 1. Context
- Why: nothing compiles `Expr::StringLit`, string `+`, string `puts`, or
  string `==`/`!=` to machine code; no object-file data-section
  machinery exists in this codegen at all yet (see Decision log).
- Target state: `runtime/emerald_runtime.c` gains `emerald_print_str
  (const char*)` (`printf("%s\n", s)`, the same one-line shape as the
  existing `emerald_print_i64`/`emerald_print_f64`), `emerald_string_
  concat(const char*, const char*) -> char*` (measures both with
  `strlen`, allocates `len_a + len_b + 1` bytes via the existing
  `emerald_alloc`, `memcpy`s both plus a null terminator), and
  `emerald_string_eq(const char*, const char*) -> long long` (`strcmp
  (...) == 0`, returned as `0`/`1`). Codegen's `build_expr` gains an
  `Expr::StringLit` case that declares (or reuses, per literal value) an
  `ObjectModule` data object holding the literal's bytes plus a trailing
  NUL, and materializes a pointer to it via `global_value` at the use
  site; `build_puts` dispatches to `emerald_print_str` when the built
  value's static type is `String`; `Expr::Add`'s codegen dispatches to
  `emerald_string_concat` the same way; `Expr::Compare`'s codegen
  dispatches `Eq`/`Ne` to `emerald_string_eq` (comparing its `i64`
  result against `0` via the existing `IntCC` machinery) when both
  operands are `String`, and returns a descriptive `Err` (not a panic,
  not silently-wrong code) for the other four `CompareOp` variants on
  `String` operands, per the Decision log's disclosed caveat.

### 2. Acceptance Criteria
1. A real program —
   ```ruby
   s: String = "hello"
   puts s
   a: String = "foo" + "bar"
   puts a
   if "abc" == "abc"
     puts "equal"
   end
   puts "line1\nline2"
   puts "a\"b"
   ```
   — compiled, linked, and run, prints exactly:
   ```
   hello
   foobar
   equal
   line1
   line2
   a"b
   ```
   real executed proof, not simulated or hand-typed.
2. Two distinct `Expr::StringLit`s with the same text each produce a
   correct, independently-valid pointer (no requirement that they alias
   the same data object — deduplication is a real, disclosed, low-risk
   optimization left for later, not attempted here).
3. An unsupported shape (`<`/`>`/`<=`/`>=` reached with two `String`
   operands) defensively returns a descriptive `Err`, not a panic and
   not silently-wrong generated code — same AC discipline as every
   prior codegen plan's "unsupported shape" acceptance criterion.

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`,
  `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. the real linked-and-run program in AC1 | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```

## Out of scope / deferred
- **String interpolation (`"#{expr}"`)** — `spec/GRAMMAR.md` §1 marks it
  KEEP, but it needs a real expression-embedding desugaring pass; see
  Decision log.
- **Single-quoted literals, heredocs (`<<~TEXT`)** — both separately
  KEEP in `spec/GRAMMAR.md` §1, both deferred alongside interpolation.
- **Escape sequences beyond `\"`/`\n`** (`\\`, `\t`, `\u{...}`, etc.) —
  see Decision log; an unrecognized escape is a real parse error, not a
  silent pass-through.
- **Symbol literals (`:foo`)** — `spec/GRAMMAR.md` §1 marks these KEEP
  as a *distinct* interned `Symbol` type, not a `String` — genuinely
  separate scope from this plan.
- **Regexp literals (`/.../`)** — `spec/GRAMMAR.md` §1 itself marks this
  UNDECIDED, deferred to a stdlib plan; not this plan's job either way.
- **`Compare`'s `<`/`>`/`<=`/`>=` restricted to orderable types** — the
  latent pre-existing gap this plan's Decision log surfaces (not
  introduces); real follow-up work, not fixed here.
- **String deduplication in the object file's data section, string
  interning, small-string optimization** — see AC2; a real, disclosed,
  low-risk future optimization.
- **Any `String` instance methods** (`.length`, `.upcase`, `.split`,
  indexing into a string's characters, etc.) — only the literal/print/
  concat/compare path this plan's concrete proof needs is implemented;
  the rest is stdlib-expansion scope, not this plan's.
