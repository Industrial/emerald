---
name: String Interpolation and Heredocs
overview: "`\"...#{expr}...\"` string interpolation (compile-time-parsed into `Expr::Interpolate(Vec<StringPart>)`, never a runtime eval, over a compiler-known Int64/Float64/String/Boolean stringification set) and `<<~IDENT ... IDENT` squiggly heredocs (a lexer-level textual pre-pass, since LALRPOP's regex-based tokenizer can't express the heredoc terminator's backreference), both extending plan 19's plain `\"...\"` string literals — plan 36 of the 36-47 gap-closing batch."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-string-interpolation-parsing
    content: "Expr::Interpolate(Vec<StringPart>) + a new `crate::interpolate` module that splits a decoded string body on `#{...}` and recursively parses each span via a new `pub Expr` grammar entry point — not eval, a second compile-time parse"
    status: pending
  - id: leaf-string-interpolation-sema-codegen
    content: "Sema restricts interpolated expressions to the compiler-known Int64/Float64/String/Boolean set; codegen concatenates parts via emerald_string_concat plus three new emerald_*_to_string runtime helpers, dispatched by each part's static ValKind"
    status: pending
  - id: leaf-heredocs
    content: "<<~IDENT ... IDENT squiggly heredocs as a source-text pre-pass (Ident-shaped placeholder substitution + minimum-indent dedent + span-offset remap), feeding the dedented body through the same #{...} splitter leaf-string-interpolation-parsing built"
    status: pending
isProject: false
---

# Plan 36 — String Interpolation and Heredocs

This is plan 36, the first of a new batch — plans 36-47, twelve
independent sibling plans, each owning one distinct gap — whose shared
purpose is closing Emerald's language/stdlib surface to its analyzed
~45% Ruby-parity ceiling without conceding any identity constraint (no
`method_missing`/`eval`/`send`/reflection, no mixins/open classes, no
dynamic dispatch or vtables, no tracing GC). It is post-v1 scope, the
same posture as plans 17/19/28-35: not a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md)
(whose Completion note already treats v1 as done at row 15), and
`plan-of-plans.md` is left untouched here — it gets updated separately,
once, after all twelve of plans 36-47 exist.

Concrete proof this plan targets:
```ruby
name: String = "World"
age: Int64 = 30
puts "Hello, #{name}! You are #{age} years old."

report: String = <<~TEXT
  Name: #{name}
  Age: #{age}
TEXT
puts report
```
Compiled, linked, and run, this prints exactly (four lines, the last
one empty — see `leaf-heredocs`'s Decision log entry on why, disclosed
rather than fudged):
```
Hello, World! You are 30 years old.
Name: World
Age: 30

```
The trailing blank line is real, not a typo: the heredoc's own content
ends with a newline (every heredoc body line, including the last one
before the terminator, contributes its own `\n` — the same as real
Ruby), and `puts` (via `emerald_print_str`'s `printf("%s\n", s)`) adds
one more on top. An implementation that silently trimmed the heredoc's
trailing newline to make the output "look nicer" would be hiding real
behavior, not proving it.

## Decision log

- **Interpolation is a second compile-time parse, never a runtime
  `eval`** — `spec/GRAMMAR.md` §1 says exactly this: interpolation is
  "kept — it is static string concatenation, not `eval`." Concretely:
  `ast.rs` gains `Expr::Interpolate(Vec<StringPart>)` where `StringPart`
  is `Literal(String) | Expr(Box<Expr>)`; the parser splits a string
  body on `#{...}` boundaries and, for each span, *re-invokes the same
  LALRPOP-generated expression grammar recursively* on that substring —
  the exact same static grammar that parses the rest of the program,
  just re-entered — rather than deferring anything to runtime. This is
  the plan's core identity-preserving move: interpolation looks dynamic
  in Ruby but costs nothing beyond ordinary AST-level string
  concatenation here.
- **A generic, user-extensible `to_s`/`Display` protocol is explicitly
  declined — not because it would require dynamic dispatch (a future
  interface could still resolve statically from the receiver's declared
  type, matching this project's no-vtable rule), but because Emerald has
  no interface/protocol mechanism at all yet to type-check "the
  receiver's declared type has a method named `to_s` returning
  `String`" against arbitrary future classes.** Verified this session:
  `crates/emerald-parser/src/ast.rs`'s `ClassDef`/`Function` carry no
  notion of an implemented interface, and `emerald-sema`'s
  `method_owners`/`ClassInfo` machinery resolves every method call
  nominally, by the receiver's own declaration or its single
  superclass chain (plan 32) — there is no vocabulary yet for "any type
  that provides method X." Building that vocabulary is a later plan in
  this same batch (interfaces, plan 41) — named here as a real forward
  dependency this plan does **not** take on. Instead, `Expr::Interpolate`
  stringifies only a fixed, compiler-known set: `Int64`, `Float64`,
  `String`, `Boolean` — chosen because these are exactly the four
  `ValKind` storage kinds `crates/emerald-codegen/src/lib.rs` already
  gives a first-class representation (`ValKind::Int64/Float64/Str/Bool`,
  verified at L51-62), so no new type-representation work is needed,
  only three new stringification functions.
- **No lexer/terminal regex change is needed to let `"..."` contain
  `#{...}` at all — only the parser *action* changes.** Verified this
  session against `crates/emerald-parser/src/grammar.lalrpop:709`:
  `StringLitTok: String = <s:r#""([^"\\]|\\[n"])*""#> => decode_string_lit(&s);`
  already matches any run of non-`"`/non-`\` bytes, which includes `#`,
  `{`, and `}` — a string containing `#{name}` already lexes today as an
  ordinary `StringLitTok`, it just isn't *interpreted* as anything
  special. This plan's entire parsing job is replacing the two call
  sites of `decode_string_lit` (`grammar.lalrpop:415` and `:628`, both
  `<s:StringLitTok> => Expr::StringLit(s)`) with a call into a new
  splitting/recursive-parse function, not touching `StringLitTok`'s
  regex.
- **The splitting/recursive-parse function needs a second `pub` grammar
  entry point, `pub Expr: Expr = { OrExpr };` — verified this session
  that `grammar.lalrpop` exposes exactly one today, `pub Program`
  (`grammar.lalrpop:34`; `crates/emerald-parser/src/lib.rs`'s
  `parse_named` is the only caller of the generated parser, and it only
  ever constructs `grammar::grammar::ProgramParser`).** Since `Expr`
  itself is already a plain (non-`pub`) rule at `grammar.lalrpop:517-519`
  (`Expr: Expr = { OrExpr };`), adding `pub` to it is a one-line, fully
  additive change — LALRPOP generates a second, independent
  `ExprParser` alongside the existing `ProgramParser` with no
  interaction between them.
- **The new splitting logic lives in a new module,
  `crates/emerald-parser/src/interpolate.rs`, not in `ast.rs`.**
  `ast.rs`'s existing `decode_string_lit` (plan 19) is a pure function
  with zero dependency on the generated `grammar` module — its own doc
  comment frames its placement purely as "LALRPOP's grammar-file format
  doesn't support a bare top-level `fn`." The new function needs to call
  `grammar::grammar::ExprParser::new().parse(...)` to recursively parse
  each `#{...}` span, which is a real, new kind of dependency `ast.rs`
  doesn't have today; keeping `ast.rs` a pure-data/AST module and
  putting the grammar-recursion call in its own sibling module (declared
  in `lib.rs` next to `mod grammar`) is this plan's own disclosed
  module-layout call, not a hard technical necessity (nothing stops one
  crate's modules from calling each other regardless of declaration
  order — the separation is for clarity, matching the existing
  ast.rs/grammar split).
- **Backward compatibility: a string with zero `#{...}` spans still
  produces `Expr::StringLit(String)`, unchanged, not
  `Expr::Interpolate(vec![Literal(s)])`.** This is the plan's own
  regression-safety guarantee, not an incidental detail: plan 19's
  codegen already compiles `Expr::StringLit` straight to a data-section
  global constant (`build_global_string_ptr`, `grammar.lalrpop`
  unaffected). Routing every plain string through the new runtime
  concatenation path instead would be strictly correct but a real,
  needless codegen regression for the overwhelmingly common
  no-interpolation case — this plan's parser action checks whether any
  `#{` was found and only emits `Interpolate` when it actually needs to.
- **Heredocs cannot be a LALRPOP grammar terminal at all — verified
  this session that LALRPOP's built-in tokenizer (generated from
  `grammar.lalrpop`'s `match {}` block plus its inline `r"..."`
  terminals, e.g. `StringLitTok`) compiles every pattern through the
  `regex` crate, whose engine is explicitly non-backtracking and
  supports no backreferences.** A single token matching `<<~IDENT ...
  IDENT` needs exactly that — the closing identifier must equal the
  opening one — so it cannot be expressed as one `match{}` pattern.
  Verified this session that no custom hand-written `Lexer` exists
  anywhere in this crate today (`crates/emerald-parser/src/` holds only
  `ast.rs`, `grammar.lalrpop`, `lib.rs` — every token this compiler has
  ever needed, across plans 01-35, has been regular). Writing a full
  custom `Lexer` (implementing LALRPOP's
  `Iterator<Item = Result<(usize, Tok, usize), Error>>` trait, replacing
  the declarative `match{}` block wholesale) would be a strictly larger
  surface than this plan needs and would put every *other* token's
  lexing on the hook for this one feature's sake. This plan instead adds
  a **source-text pre-pass** run in `parse_named`/`parse` before the
  string ever reaches LALRPOP's tokenizer — see `leaf-heredocs`.
- **`<<~` collides lexically with the already-existing `<<` left-shift
  operator (plan 28) — verified this session at `grammar.lalrpop:362`/
  `:553` (`"<<" ` is a real binary operator token, `Expr::Shl`).** `a <<
  ~b` (shift `a` left by the bitwise-complement of `b`) and a heredoc
  opener `<<~b` (nonsensical as a shift, since `~b` alone isn't followed
  by a value) are textually indistinguishable to a naive scanner. This
  plan resolves it exactly the way Ruby's own lexer does: `<<~IDENT`
  is recognized as a heredoc opener only when `IDENT` is the last
  token on its source line (immediately followed by only whitespace
  then a newline) — a real, disclosed narrowing (a shift-by-
  bitwise-complement expression that happens to end its line
  immediately after the identifier is mis-lexed as a heredoc open), but
  the same one Ruby itself accepts, and not a shape any of plans 01-35's
  existing examples use (no example anywhere in `history/` shifts by a
  bitwise complement).
- **Heredoc identifiers are matched via a plain identifier-shaped
  placeholder substitution, not a re-escaped string splice.** The
  pre-pass replaces each `<<~IDENT\n...\nIDENT` span in the *source
  text* with a single synthesized identifier token (e.g.
  `__emerald_heredoc_0__`) — always syntactically legal at that
  position, since `PrimaryExpr`/`StmtPrimaryExpr` both already have a
  bare `<name:Ident> => Expr::Ident(name)` alternative
  (`grammar.lalrpop:421` and its `StmtPrimaryExpr` mirror) — and
  records the heredoc's already-fully-parsed replacement expression
  (an `Expr::Interpolate`/`Expr::StringLit`, built by feeding the
  dedented body through the *same* `#{...}`-splitting function
  `leaf-string-interpolation-parsing` built) in a side table keyed by
  that placeholder name. After `ProgramParser::parse` succeeds, a small
  AST walk replaces every `Expr::Ident(name)` matching an entry in that
  table with its recorded expression. This avoids re-deriving any
  escaping convention for heredoc bodies (they need none — a heredoc
  never needs a `\"` escape, since a raw `"` isn't special inside
  `<<~`) and keeps the grammar itself completely unaware heredocs exist.
  The one disclosed, real gap: a user variable literally named
  `__emerald_heredoc_0__` would collide — a low-probability, low-risk
  edge case in the same spirit as plan 19's disclosed no-deduplication
  gap, not fixed here.
- **Diagnostic span accuracy is only partially preserved, and this is
  disclosed rather than silently regressed.** Errors *inside* a
  heredoc's own `#{...}` expressions get a fully correct byte offset,
  because the pre-pass locates and parses those spans directly out of
  the untouched original source text before any substitution happens.
  Errors *elsewhere* in the file, after a heredoc, see a shifted byte
  offset (the placeholder's length differs from the original heredoc
  span's length) unless corrected — this plan's pre-pass returns the
  list of `(original_span, placeholder_len)` substitutions it made
  specifically so `parse_named` can remap any resulting LALRPOP error
  offset by the accumulated length delta of every heredoc entirely
  before it in the file. This is a real, bounded, disclosed
  approximation (whole-heredoc granularity, not per-byte), not a claim
  of perfect fidelity.
- **Out of scope: single-quoted `'...'` string literals.**
  `spec/GRAMMAR.md` §1 lists `'...'` alongside `"..."` under the same
  KEEP row, and it would be a natural companion to add alongside
  interpolation work — declined anyway. A non-interpolating,
  non-escaping `'...'` literal would just duplicate `String`'s existing
  representation with different (in fact, *fewer*) escape semantics; it
  adds a second lexical string form for zero new expressive power this
  plan's worked example needs, and plan 19 already made this same cut
  once. Re-affirmed here rather than silently re-litigated, since this
  plan is the direct interpolation follow-up and is the most natural
  place someone would be tempted to add it.
- **Out of scope: `%w[]`/`%i[]` percent-literal arrays.**
  `spec/GRAMMAR.md` §1 marks these KEEP as "sugar over `Array[String]` /
  `Array[Symbol]` literals; no new semantics" — but `%i[]` needs a
  `Symbol` type this compiler doesn't have (verified: no `Symbol`
  variant anywhere in `emerald-sema::Type`, L12-38), and `%w[]` alone is
  pure sugar over `Array[String]` literals that already work (plan 09) —
  neither is needed to prove string interpolation or heredocs work, and
  both are a distinct, separately-scoped stdlib-sugar gap.

## Leaf: leaf-string-interpolation-parsing

### 1. Context
- Why: `Expr` has no way to represent an embedded expression inside a
  string today (verified: no `Interpolate`/`StringPart` anywhere in
  `crates/emerald-parser/src/ast.rs`), and `decode_string_lit` only ever
  produces a flat, fully-literal `String`.
- Target state: `ast.rs` gains `pub enum StringPart { Literal(String),
  Expr(Box<Expr>) }` and `Expr::Interpolate(Vec<StringPart>)`.
  `grammar.lalrpop` gains `pub Expr: Expr = { OrExpr };` (see Decision
  log) alongside the existing non-`pub` `Expr` rule it wraps. A new
  `crates/emerald-parser/src/interpolate.rs` module (declared via `mod
  interpolate;` in `lib.rs`) exposes a function that: strips the
  surrounding quotes, decodes `\"`/`\n` exactly as `decode_string_lit`
  does today, then scans the decoded text for `#{`, tracking `{`/`}`
  brace depth (so a nested hash literal like `#{ {1 => 2}[1] }` inside
  an interpolation span is handled correctly, not broken by naive
  first-`}` matching) to find each span's end, and recursively invokes
  `grammar::grammar::ExprParser::new().parse(...)` on that span's text.
  An unterminated `#{` (no matching `}` before the string's own closing
  `"`) is a real error, propagated through a new `=>?` fallible action
  at both call sites (`grammar.lalrpop:415`, `:628`), not a panic.
  Zero-interpolation strings still produce `Expr::StringLit` (Decision
  log's backward-compatibility guarantee).

### 2. Acceptance Criteria
1. `"Hello, #{name}!"` parses to `Expr::Interpolate(vec![
   StringPart::Literal("Hello, ".into()),
   StringPart::Expr(Box::new(Expr::Ident("name".into()))),
   StringPart::Literal("!".into())])`.
2. `"hello"` (no `#{` at all) still parses to
   `Expr::StringLit("hello".into())`, byte-for-byte the same AST plan 19
   already produces — a direct regression check, not just "it still
   parses."
3. `"#{ {1 => 2}[1] }"` (a nested hash literal, containing its own
   `{`/`}`, inside the interpolation span) parses successfully, proving
   the brace-depth scan (not naive first-`}` matching, which would cut
   the span short at the hash literal's own inner `}`) is real.
4. `"unterminated #{name"` (a `#{` with no closing `}` before the
   literal's own closing `"`) is a real parse error, not a panic and not
   a silently-truncated interpolation.
5. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, the collections/exceptions/module/string examples) still
   parses identically — none of them contain `#{`, so this is additive
   only.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs`
- **Create:** `crates/emerald-parser/src/interpolate.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts, two `pub` parsers coexist) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. AC1-5 | agent-claimed-locally |

---

## Leaf: leaf-string-interpolation-sema-codegen

### 1. Context
- Why: `infer_expr_type` (`crates/emerald-sema/src/lib.rs:353-613`,
  verified this session) has no case for `Expr::Interpolate`; codegen's
  `build_expr` (`crates/emerald-codegen/src/lib.rs:921-1457`) has no
  case for it either, and the runtime (`runtime/emerald_runtime.c`) has
  no non-`String` stringification helpers at all — only
  `emerald_print_str`/`emerald_string_concat`/`emerald_string_eq`
  (plan 19).
- Target state: `infer_expr_type`'s new `Expr::Interpolate(parts)` arm
  requires every embedded `StringPart::Expr(e)`'s inferred type be one
  of `Type::Int64 | Type::Float64 | Type::String | Type::Boolean`
  (Decision log's compiler-known set); any other type is a `Diagnostic`
  naming the offending type, using the same construction path every
  other type-mismatch case in this function already uses. The overall
  expression's type is always `Type::String`. Codegen's `Ctx` (struct
  at `crates/emerald-codegen/src/lib.rs:644-682`) gains three more
  `FunctionValue<'ctx>` fields — `int64_to_string`, `float64_to_string`,
  `bool_to_string` — declared in `compile_to_object` exactly like
  `print_str`/`string_concat`/`string_eq` are today (L3564-3579). A new
  `build_expr` arm for `Expr::Interpolate(parts)` builds each part's
  `char*` (a `Literal` reuses `StringLit`'s own
  `build_global_string_ptr`; an `Expr` part is built via `build_expr`
  then dispatched by its returned `ValKind` — `Str` passes through
  unchanged, `Int64`/`Float64`/`Bool` each call the matching new
  `to_string` helper, any other `ValKind` is a descriptive `Err` since
  sema has already ruled it out) and folds them left-to-right through
  repeated calls to the existing `ctx.string_concat`, returning
  `ValKind::Str`. `runtime/emerald_runtime.c` gains
  `emerald_int64_to_string`/`emerald_float64_to_string` (each
  `emerald_alloc`s a small fixed buffer and `snprintf`s into it — `%lld`
  and `%g` respectively, `%g` matching `emerald_print_f64`'s own
  existing format for consistency) and `emerald_bool_to_string` (returns
  a pointer to a static `"true"`/`"false"` constant — no allocation
  needed, matching how `Type::Boolean`/`ValKind::Bool` values need no
  heap representation anywhere else in this compiler either).

### 2. Acceptance Criteria
1. This plan's own worked example's interpolation line — `name:
   String = "World"`, `age: Int64 = 30`, `puts "Hello, #{name}! You are
   #{age} years old."` — compiled, linked, and run, prints exactly
   `Hello, World! You are 30 years old.`
2. `puts "#{true} and #{3.5}"` compiled, linked, and run prints exactly
   `true and 3.5` — proving `Boolean` and `Float64` interpolation both
   dispatch to their correct new helper, not just `Int64`/`String`.
3. An interpolated expression of an unsupported type (e.g. `arr:
   Array[Int64] = [1]` then `"#{arr}"`) is rejected by sema with a
   diagnostic naming the unsupported type — proving the decline of
   generic `to_s` (Decision log) is actually enforced, not merely
   documented.
4. Regression: `cargo test -p emerald-sema` and `cargo test -p
   emerald-codegen` — every existing `String`-related test from plan 19
   (literal, concat, `==`) and every numeric/boolean test from plans
   18/25/28 still pass unmodified; the three new `Ctx` fields and
   runtime helpers are additive only.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass | agent-claimed-locally |
| Test (real linked-and-run interpolation) | `cargo test -p emerald-codegen` | all pass, incl. AC1-2 | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-heredocs

### 1. Context
- Why: no heredoc syntax exists anywhere in this compiler today
  (verified: no `<<~` anywhere in `grammar.lalrpop`), and — per the
  Decision log — it cannot be added as an ordinary LALRPOP terminal at
  all, since matching a heredoc's terminator against its specific
  opening identifier needs a regex backreference the `regex` crate
  doesn't support.
- Target state: a new `crates/emerald-parser/src/heredoc.rs` module
  (declared via `mod heredoc;` in `lib.rs`), called from `parse_named`
  before `ProgramParser::parse` runs. It scans the raw source for
  `<<~IDENT` occurrences where `IDENT` is the last token on its line
  (Decision log's `<<`-shift disambiguation rule), captures every
  following line up to (not including) the first line whose trimmed
  content exactly equals `IDENT`, computes the minimum leading-
  whitespace-run length across all non-blank captured lines, strips
  exactly that many leading characters from every captured line
  (squiggly dedent), and feeds the resulting dedented text through
  `leaf-string-interpolation-parsing`'s `#{...}`-splitting function
  (no `\"`/`\n` escape-decoding step — heredoc bodies need no `\"`
  escape at all, since a raw `"` isn't special inside `<<~`; see
  Decision log's disclosed simplification) to produce an `Expr`. Each
  heredoc span in the source is replaced with a synthesized
  identifier-shaped placeholder (`__emerald_heredoc_{n}__`); `parse_named`
  runs `ProgramParser::parse` against the substituted source, remaps any
  resulting `ParseError`'s byte offset using the recorded
  `(original_span, placeholder_len)` list (Decision log), and — on
  success — walks the resulting `Program`'s every `Expr`/`Stmt` to
  replace each placeholder `Expr::Ident` with its recorded heredoc
  expression.

### 2. Acceptance Criteria
1. This plan's own worked example's heredoc — `report: String =
   <<~TEXT\n  Name: #{name}\n  Age: #{age}\nTEXT` with `name = "World"`,
   `age = 30` — compiled, linked, and run, prints exactly `Name:
   World\nAge: 30\n` followed by `puts`'s own trailing newline (the
   full four-line output block described at the top of this plan).
2. A heredoc with unevenly indented body lines (one line indented 4
   spaces, another indented 2) dedents by the *minimum* (2), leaving the
   first line with 2 residual leading spaces in the resulting string —
   verified directly against the parsed `Expr`'s value, not just visual
   output.
3. An unterminated heredoc (an opener with no line before EOF exactly
   matching its identifier) is a real, descriptive parse error — with a
   byte offset pointing at the original opener (Decision log: this
   specific error is detected during the pre-pass itself, against
   unmodified source, so its span is exact) — not a panic and not a
   silent absorption of the rest of the file.
4. Regression: a source file containing zero `<<~` occurrences is
   passed through `heredoc`'s pre-pass byte-for-byte unchanged (a direct,
   testable identity-function property) — every prior plan's example
   still parses identically. A source file with a heredoc *and* a real
   syntax error later in the file still reports that later error at a
   plausible, remapped location, not a byte offset inside the (shorter)
   substituted source with no correction at all.

### 3. File & Module Structure
- **Create:** `crates/emerald-parser/src/heredoc.rs`
- **Modify:** `crates/emerald-parser/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. AC1-4 | agent-claimed-locally |
| Workspace (real linked-and-run heredoc + interpolation proof) | `cargo test --workspace` | all pass, incl. this plan's full worked example printing the exact four-line block | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
