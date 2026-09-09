---
name: Compile-Time Execution (comptime)
overview: "A `comptime` marker on a top-level function or on an expression in a const-context, evaluated by a small tree-walking interpreter the compiler runs over its own already-type-checked AST — Zig's comptime model, adapted to Emerald's identity constraints: no macro DSL, no runtime eval, no runtime reflection, a hard step ceiling instead of a termination proof, and exactly one compiler-provided derive routine (`derive Comparable`) rather than a user-facing code-generation API."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-comptime-interpreter-core
    content: "Expr::Comptime + Function.is_comptime in the AST; grammar for the `comptime` prefix keyword on `def` and on an expression; a tree-walking interpreter in emerald-codegen operating over the post-sema AST; sema's static legal/illegal-node check with diagnostics naming the specific disallowed construct; a hard, configurable step ceiling (default 1,000,000) enforced by the interpreter itself"
    status: pending
  - id: leaf-comptime-const-context-integration
    content: "Restrict Expr::Comptime to exactly two legal positions (a top-level constant's initializer, Array.new's size argument); codegen bakes the interpreter's result as an LLVM immediate with zero runtime call; comptime-only functions are never lowered to LLVM IR at all; a white-box instrumentation test proves both claims directly against the emitted module"
    status: pending
  - id: leaf-comptime-query-cache-integration
    content: "A fourth query, comptime_eval_query, added to plan 48's crates/emerald-driver/src/cache.rs QueryCache alongside parse_query/type_check_query/codegen_query, keyed and reported the same way — a comptime evaluation is exactly the kind of pure, deterministic, repeatable computation that cache already targets"
    status: pending
  - id: leaf-derive-comparable
    content: "`class Point derive Comparable ... end` — a post-parse, pre-sema AST rewrite pass (emerald_parser::expand_derives) that walks the class's own flattened field list (reusing the same inheritance-chain-flattening idea plan 08/32's ClassInfo/ClassLayout already establish) and synthesizes an ordinary `==` Function via Spanned::synthetic, indistinguishable downstream from a hand-written one; a real compiled/linked/run comparison proof"
    status: pending
isProject: false
---

# Plan 61 — Compile-Time Execution (comptime)

This is plan 61 of the 58–64 batch: a follow-up debate asked — setting
aside maturity, ecosystem, and stability entirely — what Emerald's
*theoretical* technical ceiling would be, if every remaining gap were
closed without conceding any of the identity constraints every prior
plan (01–57) has already locked in: no `method_missing`/`eval`/`send`/
reflection **at runtime**, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing garbage collector. Every
prior plan has permanently declined macros as an identity non-goal —
Ruby's `eval`/`method_missing` and Lisp/Rust-style macro DSLs both
reintroduce exactly the dynamism this compiler's whole architecture
(static dispatch, `field_ptr` byte-offset layouts computed once at
compile time, no vtables) is built to avoid. The debate's answer was
Zig's `comptime`: ordinary code, executed by the compiler itself, at
compile time, on compile-time-known values — which gets most of what
macros are *for* (deriving code, compile-time validation, generating
constants) without a macro DSL, without `eval`, and without any runtime
dynamism at all. This plan is the first concrete design for that
mechanism in this compiler. Other plans in the 58–64 batch are authored
separately and are not itemized here, since none of them exist yet as
of this plan — the same "each sibling stands alone" posture plans 17,
31, and 50 already established for their own batches. Like every plan
since 17, this is post-v1 scope: it is **not** a row in
[`plan-of-plans`](../../history/2026-09-08T174011Z-plan-of-plans.md),
and this plan does not touch `plan-of-plans.md` or any other plan file.

Depends on **plan 06** (the codegen pipeline this plugs into — parse →
sema → codegen → link), **plan 48** (incremental, query-based
compilation — verified this session, real and landed:
`crates/emerald-driver/src/cache.rs` exists with a working `QueryCache`
and passing tests, not just a proposal), and **plan 52** (algebraic
data types — this plan's derive example reuses the exact field-layout
representation plan 52's own Decision log generalized from `Hash[K,V]`).
Also touches, by citation only (nothing here modifies them): **plan 08/
32** (object model / inheritance — the flattened-field-list machinery
this plan's derive routine reuses), **plan 33** (field-access sugar —
`read`, needed for a derived method to read another instance's field),
**plan 40** (operator overloading — `==` as a literally-named class
method, the mechanism that makes a derived `==` reachable via `a == b`
at all), **plan 45** (I/O intrinsics — `puts`/`File.*`/`gets`/`ARGV`,
named explicitly below as illegal inside `comptime`), and **plan 54/55**
(actor `.spawn`/message sends — also named explicitly below as illegal).

Concrete proof this plan targets — two independent worked examples,
since a comptime-baked constant is invisible in ordinary stdout on its
own:

```ruby
def comptime factorial(n: Int64) -> Int64
  result: Int64 = 1
  i: Int64 = 1
  while i <= n
    result = result * i
    i = i + 1
  end
  result
end

FACT10: Int64 = comptime factorial(10)
puts FACT10

class Point derive Comparable
  x: Int64
  y: Int64

  def initialize(x: Int64, y: Int64) -> Void
    @x = x
    @y = y
  end
end

p1: Point = Point.new(3, 4)
p2: Point = Point.new(3, 4)
p3: Point = Point.new(3, 4)
p4: Point = Point.new(5, 6)
puts p1 == p2
puts p3 == p4
```

Expected output: `3628800`, `true`, `false`. `FACT10` is computed
entirely by the compiler before `main` is ever emitted — the payoff
(proof (a)) is that no `factorial` call, and no `factorial` function at
all, exists anywhere in the linked binary; `leaf-comptime-const-
context-integration`'s own acceptance criteria prove this directly
against the emitted LLVM module, not by inference from stdout. `p1 ==
p2` / `p3 == p4` (proof (b)) exercise a real, compiled, field-by-field
`==` method that no one wrote by hand — `derive Comparable` on `Point`
produced it, and it prints `true`/`false` correctly for an equal and an
unequal pair.

## Decision log

- **Why `comptime` does not violate the no-`eval`/no-reflection
  identity rule — stated explicitly, since this is the load-bearing
  claim of the whole plan.** `eval` and runtime reflection are properties
  of a *running program*: a compiled Emerald binary that can parse and
  execute text handed to it at runtime, or introspect its own types/
  methods while executing, is dynamic in exactly the way this compiler's
  entire architecture (static dispatch resolved once at compile time,
  `field_ptr` byte-offset layouts computed once at compile time, no
  vtables anywhere) is built to rule out. `comptime` evaluation never
  runs inside the compiled binary — the tree-walking interpreter this
  plan adds is compiler-internal Rust code, invoked only by
  `emerald-cli`/`emerald-driver` during compilation, over the same
  `Spanned<Expr>`/`Spanned<Stmt>` AST every other pass in this compiler
  already walks, on values that are already known and already fully
  type-checked before interpretation starts. There is no runtime
  string-to-code execution (`comptime`'s legal subset — see below — has
  no notion of "parse this string and run it"; `Expr::Interpolate`'s own
  `#{...}` re-parse, cited in `ast.rs`'s own doc comment as "a
  *compile-time* parse, never a runtime `eval`," is the closest existing
  precedent, and this plan's interpreter doesn't even do that much: it
  interprets already-parsed AST, never raw text). There is no runtime
  reflection API exposed to a running program — by the time the linked
  binary exists, every `comptime`-marked function/expression has already
  been fully reduced to either a baked LLVM immediate or, in the common
  case (see `leaf-comptime-const-context-integration`), simply doesn't
  exist in the object file at all. A compiled Emerald binary contains
  **zero** comptime machinery, exactly as it contains zero macro-
  expansion machinery today — the difference between this and a macro
  system is not a matter of degree, it's categorical: a macro DSL adds a
  second language-inside-the-language for users to write generative code
  in; `comptime` adds no new language at all, just a restricted
  execution *mode* for the one language Emerald already has, run once,
  by the compiler, before the "runtime" concept even exists.
- **Mechanism: a tree-walking interpreter over the post-sema AST, not a
  compile-time JIT and not a macro DSL — verified against this
  session's real source, not assumed.** `crates/emerald-codegen/
  Cargo.toml` depends on `inkwell = { version = "0.10", features =
  ["llvm21-1"] }` with no `execution_engine`-adjacent feature enabled,
  and `emerald-codegen/src/lib.rs`'s own dependency list (`inkwell::
  types`, `inkwell::targets`, `inkwell::module`, `inkwell::basic_block`,
  ...) never imports `inkwell::execution_engine` anywhere — `compile_to_
  object` emits object files exclusively through `inkwell::targets::
  TargetMachine`; nothing in this codebase has ever invoked LLVM's
  JIT/ORC machinery, or the generated code it produces, from inside the
  compiler process. Standing up that machinery for `comptime` alone
  would be a materially larger, riskier dependency surface — real
  ORC-based execution needs native target initialization, an execution
  engine's own memory manager, and running arbitrary just-compiled
  machine code inside the compiler's own address space — for a feature
  whose entire legal subset (below) is bounded arithmetic, control flow,
  and struct/enum construction over values already representable as
  plain Rust enums. A tree-walking interpreter needs none of that: it
  operates directly on `&Spanned<Expr>`/`&Spanned<Stmt>`, the same data
  `emerald-sema`'s own `infer_expr_type`/`check_stmt` already walk, using
  ordinary Rust `i64`/`f64`/`bool` arithmetic and a small `ComptimeValue`
  enum (`Int(i64)`, `Float(f64)`, `Bool(bool)`, `Struct{class_name,
  fields: HashMap<String, ComptimeValue>}`, `Variant{enum_name,
  variant_name, fields: Vec<ComptimeValue>}`) — no LLVM context, no
  target machine, no cross-architecture concern at all, since it never
  produces machine code, only Rust-native values that `leaf-comptime-
  const-context-integration` later lowers to an LLVM immediate.
- **Where it lives: `emerald-codegen`, invoked once per `comptime`-legal
  call site, mirroring this codebase's own established "no shared
  sema→codegen structure" architecture.** Verified this session:
  `emerald-codegen/src/lib.rs`'s own comment above `build_class_layout`
  states plainly "this backend has no typed IR / no shared sema→codegen
  data structure anywhere" — `ClassLayout`/`EnumLayout` are codegen's own
  independent re-derivations of `ClassDef`/`EnumDef`, duplicating (not
  reusing) `emerald-sema`'s own `ClassInfo`/`resolve_chain`/
  `build_flattened_class_info`. This plan's interpreter follows the same
  precedent: sema (see next bullet) performs a purely *structural*,
  always-terminating legality check (which `Expr`/`Stmt` node kinds
  appear inside a `comptime` body — a plain AST walk, no evaluation);
  codegen performs the actual *evaluation* (which can fail to terminate,
  which is exactly why the step ceiling below lives here, not in sema).
  This split is forced, not a style choice: sema's `check_program(
  program: &Program)` takes an immutable reference and produces
  diagnostics, never a computed value — the interpreter's job (producing
  an actual `ComptimeValue` to bake into LLVM IR) has nowhere to live
  except codegen, the pass that already turns AST into concrete,
  finished output.
- **Legal AST node kinds inside a `comptime`-marked function body or a
  `comptime` expression — a concrete, enumerated allow-list, not an
  implicit "everything except the ban-list."** `Expr`: `Ident`, `Int`,
  `Float`, `Bool`, `Add`/`Sub`/`Mul`/`Rem`/`Div`, `Neg`, `Not`, `And`/
  `Or`, `Compare`, `BitAnd`/`BitOr`/`BitXor`/`BitNot`/`Shl`/`Shr` (plan
  28), `Call` (only to another `comptime`-marked function — see below),
  `New` (constructs a `ComptimeValue::Struct`/`Variant` using the same
  field metadata plan 08/32/52 already establish, never touching
  `emerald_alloc` or any heap), `InstanceVar` (reads a field off a
  `New`-constructed value bound to a local — the "struct/enum field
  access and construction" the brief calls for). `Stmt`: `Let`,
  `Assign`, `MultiAssign`, `If`, `While`, `For`, `ForRange`, `Case`
  (both `CasePattern::Values` and `CasePattern::Variant` — plan 52's
  closed, exhaustive enum match is exactly the sort of decidable,
  terminating control flow this subset wants), `Return`, `Expr`.
- **Illegal AST node kinds — named explicitly, each with the concrete
  diagnostic wording sema must produce, matching this codebase's
  standing "name the specific gap" discipline (plan 52's own
  exhaustiveness diagnostic names the missing variant; this plan's
  rejection names the specific disallowed construct).**
  - I/O: `puts`/`File.read`/`File.write`/`gets`/`ARGV`/`ARGC` (plan 45)
    are ordinary `Expr::Call`/`Expr::MethodCall` nodes dispatched by
    reserved name, not a distinct AST variant (plan 45's own Decision
    log: "every one a compiler-known intrinsic dispatched by receiver
    storage kind or reserved namespace name, the exact mechanism `puts`
    already uses") — so the legality check doesn't ban `Call` itself
    (calling another `comptime` function is legal), it bans these
    specific *names* appearing as a callee. Diagnostic: `` comptime
    evaluation may not call `puts` — I/O is not available at compile
    time ``.
  - Actors: `Expr::Spawn` and `Stmt::Supervise` (plan 54/55/57) —
    `` comptime evaluation may not `.spawn` — actor isolation and
    message delivery are runtime concepts ``.
  - `extern "C"` FFI: no such construct exists anywhere in this
    codebase as of plan 57 — verified this session, nothing beyond
    plans 01–57 is written. Declined here explicitly and pre-emptively
    regardless: executing arbitrary host-machine code chosen by a
    to-be-compiled program, during that same program's own compilation,
    is unsound no matter how a future FFI plan words its signature —
    the compiler cannot verify an external C function is pure or
    terminating, which is exactly what this whole subset is built to
    guarantee about everything else in it.
  - Nondeterminism: covered entirely by the I/O ban above (`ARGV`/
    `ARGC`/`gets` are this codebase's only process-environment-dependent
    constructs; no clock/random intrinsic exists anywhere in the
    current stdlib surface).
  - Exceptions: `Stmt::Begin`/`Stmt::Raise`/`Stmt::Retry` — `` comptime
    evaluation may not `raise`/`rescue` — exception unwinding is not
    modeled by the compile-time interpreter ``. A real, disclosed v1
    scope cut, not a fundamental impossibility — an exception model for
    `comptime` is legitimate future work this plan doesn't attempt.
  - Closures/blocks: `Expr::Lambda`, `Stmt::Yield` — `` comptime
    evaluation may not construct a lambda — closures capturing runtime
    state have no compile-time meaning ``.
  - `Result[T,E]`: `Expr::Try`/`Ok`/`Err`, `Stmt::MatchResult` (plan
    53) — declined for the same "not modeled" reason as exceptions.
  - Calling a non-`comptime` function: `` comptime evaluation may not
    call `helper` — mark it `def comptime helper(...)` if its body is a
    legal comptime subset ``. This is what makes the legality check
    compositional: a `comptime` function may call another `comptime`
    function (itself checked, recursively, against this same allow-
    list), never an ordinary one whose body sema hasn't verified is
    safe to interpret.
- **Termination: a hard, disclosed step ceiling — not a termination
  proof, because general termination is undecidable (Rice's theorem)
  and this plan doesn't pretend otherwise.** The interpreter increments
  a step counter on every statement executed and every loop-condition
  re-check; exceeding the ceiling aborts evaluation with `` comptime
  evaluation of `fib` exceeded 1,000,000 steps (possible infinite
  loop); pass --comptime-step-limit=N to raise it ``, a real compile
  failure, never a hang, never a panic. Default `1,000,000`,
  configurable via a new `--comptime-step-limit` flag on `emerald-cli`
  (mirroring `--comptime-step-limit`'s own test-time override — a small
  ceiling like `100` lets `leaf-comptime-interpreter-core`'s own
  runaway-loop acceptance criterion run fast without waiting out a
  million real iterations). This is the same "state the pragmatic
  cutoff, don't prove it" discipline plan 48 already used for its own
  compiler-fingerprint cache-key salting and plan 31 used for its
  swap-ordering pitfall — a real, disclosed engineering compromise, not
  a claim of soundness this plan can't back.
- **`derive Comparable` is a compiler-provided routine, not user-
  authorable comptime code — a real, smaller v1 scope than full Zig
  comptime, stated here rather than overclaimed.** Zig's own comptime
  lets *user* code generate other code (a comptime function returning a
  `type`, iterated over with `inline for`). This plan does not build
  that. Users write `comptime` **expressions that produce compile-time
  VALUES** (`comptime factorial(10)` → `3628800`) — they do not, in
  this plan, author their own comptime **code generators**. `derive
  Comparable` is the one, single, hardcoded exception: a Rust routine
  living inside the compiler itself (`emerald_parser::expand_derives`),
  authored once by this project, not exposed as a general facility a
  user's own `comptime` code could invoke or extend. It embodies the
  same governing idea comptime generalizes — ordinary code, run by the
  compiler, over compile-time-known structure (a class's own field
  list), producing ordinary generated code — but the "ordinary code"
  doing the generating is Rust inside the compiler, not Emerald inside
  a user's `comptime` block. That gap is real and disclosed, not an
  oversight: a general `derive`-authoring facility (a real, legitimate
  future direction) needs users to write comptime code that itself
  emits AST nodes, which is a materially larger, riskier surface (an
  actual code-generation API) than this plan's single, compiler-
  reviewed, compiler-shipped routine.
- **`derive Comparable`'s synthesis must run before `emerald-sema`, not
  inside `emerald-codegen` — a real architectural constraint, not a
  style preference.** `a == b` needs to already type-check as `Boolean`
  via plan 40's `resolve_class_operator` (verified this session:
  `crates/emerald-sema/src/lib.rs` L759–789), which runs during
  `check_program` — strictly before `emerald-codegen` ever sees the
  `Program`. Synthesizing the `==` method only in codegen would leave
  sema unaware such a method exists at all, and `p1 == p2` would fail
  to type-check. `check_program(program: &Program)` also takes an
  *immutable* reference, so no sema-internal pass can inject a new
  method into the AST either. `expand_derives` therefore runs as its
  own new pipeline stage in `emerald-parser`, between parsing/`require`
  splicing and `check_program` — a genuinely new stage, not slotted
  into an existing one, and (see next bullet) it needs the program's
  full, cross-file class table to resolve inheritance, which isn't
  available until after multi-file merging (plan 23/46) either way.
- **`derive Comparable` reuses the field-layout *idea* plan 08/32
  already established (a class's fields, flattened across its
  inheritance chain), independently re-derived a third time — a real,
  disclosed instance of this codebase's own repeated pattern, not a new
  one.** `emerald-sema`'s `ClassInfo.fields: HashMap<String, Type>` is
  already flattened across `superclass` (verified this session, L170–
  211: "Flattened — includes every ancestor's fields too"), built by
  `build_flattened_class_info`/`resolve_chain`; `emerald-codegen`'s
  `ClassLayout.fields: HashMap<String, FieldInfo>` is codegen's own
  independent flattening of the same chain via `resolve_class_chain`.
  Neither is `pub`, and `expand_derives` runs *before* either exists (it
  runs before sema, and long before codegen) — so it performs its own
  small, disclosed third flattening: walk `ClassDef.superclass` across
  the already-`require`-merged `Program.items`, collecting `{field_name
  -> declared TypeName string}` bottom-up from the root ancestor down.
  This mirrors, rather than reuses, `build_flattened_class_info`'s own
  shape — the same "duplicate bookkeeping between passes" precedent
  plan 52's Decision log already names directly (`ClassInfo` vs.
  `ClassLayout`) and codegen's own `find_variant_layout` mirroring
  sema's `find_variant` already establishes a second time.
  **Determinism pitfall, stated explicitly:** both `ClassInfo.fields`
  and `ClassLayout.fields` are `HashMap`s with no guaranteed iteration
  order; `expand_derives`'s own flattened map inherits the same
  property. Generating the `&&`-chain in arbitrary hash order would
  make the synthesized method's IR non-deterministic across compiler
  runs — a real correctness-adjacent problem for `leaf-comptime-query-
  cache-integration`'s cache keys (a cache key computed from
  non-deterministic content can never stabilize into a HIT) even though
  `==`'s own *result* wouldn't care about field order. `expand_derives`
  therefore sorts field names alphabetically before emitting the
  `&&`-chain — a small, disclosed fix, the same "name the pitfall, fix
  it, don't silently avoid it" discipline plan 31's swap-ordering
  bullet and plan 48's fingerprint-salting bullet already established.
- **A derived `==` needs `read` access to `other`'s fields, and this
  compiler has no privileged/friend-access path around that — a real,
  disclosed consequence, not a workaround.** Verified this session,
  directly from plan 33's own shipped example: even a *hand-written*
  `==` (plan 40's own `Vector2` worked proof, L40–55) needs `read x`/
  `read y` declared, because `obj.field` is *always* a method call in
  this language (plan 33's Decision log: "Ruby-idiomatic dot syntax is
  always a method call here, never implicit public field access") —
  there is no bare external-field-read AST node at all, and no
  self/friend-only access modifier this compiler could grant a
  synthesized method that a hand-written one couldn't also have.
  `expand_derives` therefore also synthesizes a zero-arg `read`
  accessor (plan 33's exact mechanism, reused) for any field that
  doesn't already declare one, as part of processing `derive
  Comparable`. The disclosed side effect: opting a class into `derive
  Comparable` makes every one of its fields externally readable — the
  honest cost of keeping the generated method's body **actually
  expressible via the existing grammar** (ordinary `@field`/`other.
  field` nodes) rather than inventing a privileged AST shape no surface
  syntax could ever produce, which is what "indistinguishable from a
  hand-written one" requires in the first place.
- **A class that already hand-writes `==` and also declares `derive
  Comparable` is rejected, not silently overwritten.** `expand_derives`
  checks `ClassDef.methods` for an existing method literally named
  `"=="` before synthesizing one; a collision is a real diagnostic
  (`` class `Point` already defines `==`; remove it or drop `derive
  Comparable` ``), never a silent replacement of user code — the same
  "never discard what the user wrote" posture this codebase has taken
  everywhere else a synthesis mechanism exists (plan 33's `read` only
  ever *adds* a method whose name — the field name — can't already
  collide with a hand-written method for an unrelated reason).
- **Const-contexts, checked against this codebase's real grammar rather
  than assumed from the task brief's own list.** The brief names three
  candidate positions: global constant initializers, array sizes, and
  type-parameter defaults. Verified this session: **type-parameter
  defaults don't exist** — plan 41 restricts `TypeParam` to `{name,
  bound}` with no default-value clause anywhere in `ast.rs` — so this
  plan doesn't invent one; **global constant initializers** are exactly
  a top-level `Item::Stmt(Spanned<Stmt::Let{..}>)` (there is no
  separate "constant" declaration form in this AST — a top-level `Let`
  already is one, the same way every plan since `plan 06`'s `hello.em`
  has used it); **array sizes** means, concretely, `Expr::ArrayNew`'s
  one size argument (`Array.new(size)`, plan 25) — this language has no
  fixed-size array *type* (`[T; N]`) for a size to appear in a type
  position, so "array sizes" is honestly scoped down to exactly this
  one expression position, not invented beyond it.

## Leaf: leaf-comptime-interpreter-core

### 1. Context
- Why: nothing in this compiler can evaluate Emerald code at compile
  time today — every AST node this compiler has ever built is either
  type-checked (sema) or lowered to LLVM IR for later, runtime,
  execution (codegen). This leaf is the interpreter itself, checked in
  isolation from where its results get used (the next leaf).
- Target state: `Expr::Comptime(Box<Spanned<Expr>>)` in
  `crates/emerald-parser/src/ast.rs`; `Function.is_comptime: bool`
  (new field, `false` for every pre-existing declaration — additive,
  source-compatible); grammar gains the `comptime` reserved keyword
  (verified this session: not used as an identifier or existing token
  anywhere in `grammar.lalrpop`, so it collides with nothing), legal as
  an optional prefix on `"def"` (restricted, like plan 41's
  `type_params`, to top-level functions only — sema rejects a
  `comptime`-marked class method or module method with a real
  diagnostic) and as a general expression-position prefix
  (`"comptime" <e:Expr> => Expr::Comptime(Box::new(e))`, deliberately
  unrestricted at the grammar level — `leaf-comptime-const-context-
  integration` is what narrows its legal *positions*). `emerald-sema`
  gains a purely structural `check_comptime_legal(body: &[Spanned
  <Stmt>]) -> Result<(), Diagnostic>` walk (the enumerated allow-/
  ban-list above), invoked once per `is_comptime: true` function at
  registration time — recursively re-checked for every `comptime`
  function such a body calls. `emerald-codegen` gains the actual
  interpreter: `ComptimeValue` (the small value enum above), `struct
  ComptimeInterpreter { steps: u64, limit: u64 }`, and `fn eval(&mut
  self, expr: &Spanned<Expr>, ...) -> Result<ComptimeValue, String>` /
  its `Stmt` counterpart, both incrementing `self.steps` and failing
  past `self.limit`.

### 2. Acceptance Criteria
1. `def comptime fib(n: Int64) -> Int64 ... end`, body built entirely
   from the legal-subset list, registers with `is_comptime: true` and
   passes `check_comptime_legal`.
2. A `comptime`-marked function whose body calls `puts` is rejected
   with the exact diagnostic wording named in the Decision log — not a
   generic "invalid comptime construct" message, not a panic.
3. The same, for a body containing `.spawn` — diagnostic names
   `.spawn` specifically.
4. A `comptime`-marked function declared as a class method (not
   top-level) is rejected with a diagnostic naming that restriction,
   mirroring plan 41's `type_params` precedent exactly.
5. A `comptime` function containing `while true ... end` (no
   terminating condition), evaluated directly against the interpreter
   with an injected small ceiling (e.g. `limit: 100`, the same
   "injectable seam in tests, not a full million-iteration wait" style
   plan 48 already used for its own fingerprint-override tests), fails
   with the step-ceiling diagnostic after exactly `limit` steps — not a
   hang, not a panic.
6. Regression: every prior plan's example still parses and type-checks
   identically; `Function.is_comptime` defaults `false` everywhere else.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-sema/src/lib.rs`, `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-sema -p emerald-codegen` | all pass, incl. the illegal-node diagnostics and the injected-ceiling test | agent-claimed-locally |

---

## Leaf: leaf-comptime-const-context-integration

### 1. Context
- Why: an interpreter that can evaluate `comptime` expressions is
  useless until something actually asks it to, and bakes the result
  into the emitted binary — this leaf is the wiring, and the payoff
  proof (zero runtime computation) instrumentation needs.
- Target state: `Expr::Comptime` is legal in exactly two positions —
  a top-level `Stmt::Let`'s direct `value`, and `Expr::ArrayNew`'s
  direct size argument — sema rejects it anywhere else with a
  diagnostic naming the restriction (`` `comptime` may only appear as
  a top-level constant's initializer or `Array.new`'s size argument ``),
  the same "grammar stays general, sema narrows" discipline plan 31
  already established for `Stmt::MultiAssign`. Codegen, on reaching
  either legal position, runs `ComptimeInterpreter::eval` and emits the
  resulting `ComptimeValue` as a literal LLVM constant (`i64`/`double`
  immediate) instead of building any call — a top-level `FACT10: Int64
  = comptime factorial(10)` becomes `@FACT10 = global i64 3628800`,
  full stop. **A `comptime`-marked function that is never referenced
  outside a `comptime` expression anywhere in the program is never
  emitted as an LLVM function at all** — codegen's function-declaration
  pass skips any `Function` with `is_comptime: true` that has no
  ordinary (non-`comptime`) call site, since nothing at runtime could
  ever reach it.

### 2. Acceptance Criteria
1. This plan's own `factorial`/`FACT10` worked example, compiled,
   linked, and run, prints `3628800`.
2. White-box instrumentation (inspecting the emitted LLVM module
   directly, the same "prove it against real output, not stdout
   inference" discipline plan 50/51 already used for `EscapeStats`):
   `FACT10`'s global initializer is a literal constant, containing no
   `call` instruction; the module's function list contains **no**
   `factorial` symbol at all.
3. `comptime <expr>` appearing outside the two legal positions (e.g.
   as a `while` condition, or a bare statement) is rejected with the
   exact diagnostic named above.
4. `Array.new(comptime factorial(5))`, compiled and run, produces a
   real, runtime-allocated array whose observed size is `120` (proven
   by filling and reading back every index up to `119` successfully and
   failing/erroring past it, or via plan 45's `.length`-equivalent if
   landed) — the size argument was computed at compile time, but the
   allocation itself still happens at runtime, unchanged.
5. A `comptime` evaluation that exceeds `--comptime-step-limit=100`
   (set below the default specifically to keep this test fast) fails
   the whole compilation with the step-ceiling diagnostic, not a hang.
6. Regression: every prior plan's example still compiles and runs
   identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `crates/emerald-cli/src/main.rs`
  (new `--comptime-step-limit` flag)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test (real linked-and-run + white-box IR proof) | `cargo test --workspace` | all pass, incl. `factorial` absent from the emitted module | agent-claimed-locally |

---

## Leaf: leaf-comptime-query-cache-integration

### 1. Context
- Why: `comptime` evaluation is a pure, deterministic function of
  already-type-checked AST — exactly the shape of computation plan 48's
  `QueryCache` already exists to memoize, and re-interpreting an
  unchanged `comptime` function on every single compile (this plan's
  other two leaves, on their own, would do exactly that) wastes work
  the same way an unmemoized `type_check`/`codegen` call would.
- Target state: `crates/emerald-driver/src/cache.rs`'s `QueryCache`
  (verified this session, real and shipped: `parse_query`,
  `type_check_query`, `codegen_query`, each keyed via `key_for`/
  `key_for_many` over `combine(fingerprint, hashes)`/`raw_hash`) gains
  a fourth method, `comptime_eval_query(&self, key: CacheKey, expr:
  &Spanned<Expr>, program: &Program, label: &str, reporter: &dyn
  CacheReporter) -> Result<ComptimeValue, String>`, mirroring `codegen_
  query`'s own MISS/fallback/persist shape exactly: a cache MISS always
  falls back to calling `ComptimeInterpreter::eval` verbatim, a HIT
  deserializes a small persisted value file (`comptime_value_path(key)`,
  the direct analogue of `codegen_query`'s `obj_path`/`obj_hash_path`
  pair) instead of re-interpreting. The cache key combines the compiler
  fingerprint with a content hash of the `comptime`-marked function's
  own source span, the same "content hash of the bytes that produced
  this value" precedent plan 48's own Decision log already established
  for `type_check`/`codegen`.

### 2. Acceptance Criteria
1. Two identical builds of this plan's `factorial`/`FACT10` example
   report `[cache] comptime FACT10 MISS` then `HIT` under
   `--verbose-cache`, with byte-identical linked-binary output both
   times — plan 48's own "prove it, don't just assert it" standard for
   cache correctness.
2. Editing `factorial`'s own body forces a MISS on the very next build;
   an unrelated edit elsewhere in the same file (one that changes that
   file's content hash but not `factorial`'s referenced span) still
   forces a MISS too, matching plan 48's already-disclosed whole-file
   granularity — this leaf doesn't claim finer-grained invalidation
   than plan 48 itself provides anywhere else.
3. Corrupting or deleting the persisted comptime-value cache entry
   forces a real re-interpretation on the next build, not an error —
   mirroring `cache.rs`'s own existing
   `corrupting_the_cached_object_file_forces_a_real_recompile_not_an_
   error` test precedent, applied to this fourth query.
4. Overriding the compiler fingerprint forces a MISS on `comptime_eval_
   query` the same way it already does for `type_check_query`/
   `codegen_query` — a rebuilt compiler binary never silently reuses a
   stale compile-time-computed constant.

### 3. File & Module Structure
- **Modify:** `crates/emerald-driver/src/cache.rs`
- **Modify:** `crates/emerald-cli/src/main.rs` (or `emerald-driver`'s
  own pipeline entry point, whichever already owns the `--verbose-cache`
  wiring from plan 48) to route `comptime` evaluation through
  `comptime_eval_query` instead of calling the interpreter directly.

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-driver` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-driver` | all pass, incl. HIT/MISS + corruption + fingerprint-override tests for the new query | agent-claimed-locally |

---

## Leaf: leaf-derive-comparable

### 1. Context
- Why: this is the plan's flagship "macro power without macros"
  proof — deriving a real, correct, field-aware method body from a
  class's own shape, with zero runtime reflection and zero user-facing
  code-generation surface.
- Target state: `ClassDef.derive: Option<String>` (new field, mirroring
  `implements: Option<String>`'s exact shape) in `ast.rs`; grammar gains
  an optional `"derive" <name:Ident>` clause on the class header,
  reachable the same way `implements`'s clause already is. A new public
  function, `emerald_parser::expand_derives(program: &mut Program) ->
  Result<(), String>`, runs once between `require`-splicing and
  `emerald_sema::check_program`. For each `Item::Class` with `derive:
  Some(name)`: reject `name != "Comparable"` with a diagnostic naming
  `name`; walk `superclass` across `program.items` to build this
  class's own flattened `{field_name -> TypeName}` map (see Decision
  log — a real, disclosed third re-derivation of the inheritance-
  flattening idea plan 08/32 already established); reject if `"=="`
  already exists in `ClassDef.methods` (see Decision log); synthesize
  a zero-arg `read`-style accessor (plan 33's mechanism) for any field
  lacking one; sort the flattened field names alphabetically; build
  `Expr::And`-chained `Expr::Compare(.., CompareOp::Eq, ..)` over
  `Expr::InstanceVar(field)` (self side) and `Expr::MethodCall(other,
  field, [])` (other side) via `Spanned::synthetic` — the exact
  mechanism this codebase already uses for compiler-synthesized AST
  (`ast.rs`'s own doc comment: "used only where codegen/rewrite passes
  construct a brand-new `Expr`/`Stmt` that was never actually written
  in source"); wrap in a `Function{name: "==".into(), params: [Param{
  name: "other", ty: class_name, default: None}], return_type:
  "Boolean".into(), body: [Return(Some(chain))], ..}`; push into
  `ClassDef.methods`.

### 2. Acceptance Criteria
1. `class Point derive Comparable ... end` (two `Int64` fields, no
   pre-existing `read`/`==`) — after `expand_derives`, `ClassDef.
   methods` contains a synthesized `read x`/`read y` pair (only for
   fields that lacked one) and a synthesized `==` method whose body is
   verified directly against the exact expected AST shape (`@x ==
   other.x && @y == other.y`, alphabetically ordered) — the same
   "verify the desugared AST directly, not just its behavior" standard
   plan 31's `leaf-compound-assignment` already established.
2. `derive Unknown` (any name other than `Comparable`) is rejected with
   a diagnostic naming `Unknown` as an unsupported derive target — not
   a panic, not a silent no-op.
3. A class that already hand-writes its own `==` **and** declares
   `derive Comparable` is rejected with the diagnostic named in the
   Decision log, proving the collision check runs before overwriting.
4. This plan's own worked proof, compiled, linked, and run: `p1 == p2`
   (equal instances) prints `true`; `p3 == p4` (unequal instances)
   prints `false` — real, executed proof the generated method is
   correct, not just present.
5. A subclass `Point3D < Point` (plan 32) declaring its own additional
   `z: Int64` field and its own `derive Comparable` clause synthesizes
   an `==` covering `x`, `y`, **and** `z` — verified directly against
   the synthesized AST, proving the flattened-field walk actually
   crosses the inheritance boundary, not just a single class's own
   `fields` list.
6. Regression: every prior plan's example (including plan 40's own
   hand-written `Vector2.==` proof) still parses, type-checks, and
   compiles identically — `expand_derives` is a strict no-op for any
   `ClassDef` with `derive: None`.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`
- **New:** a small `expand_derives` module inside `emerald-parser`
  (colocated with the AST it rewrites, the same place
  `decode_string_lit` already lives as an AST-adjacent free function)
- **Modify:** `crates/emerald-cli/src/main.rs` (pipeline wiring: call
  `expand_derives` after `require` resolution, before `check_program`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test (AST-shape + real linked-and-run) | `cargo test --workspace` | all pass, incl. `true`/`false` printed for the equal/unequal `Point` pair | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
