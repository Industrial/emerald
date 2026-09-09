---
name: REPL and Test Framework
overview: "A real `emerald repl` (incremental compile-and-run against an accumulating session, subprocess-per-line — no LLVM JIT) plus a minimal built-in test framework (`test \"...\" do ... end`, `assert`/`assert_eq` intrinsics raising a real `AssertionError`, `emerald test` reporting pass/fail counts) — both built entirely on the existing static compile pipeline, no dynamic evaluation anywhere."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-repl
    content: "emerald repl / bare `emerald`: subprocess-per-line incremental session over emerald-driver's check()/compile(), declarations-only session persistence, bad lines rejected without corrupting the session"
    status: pending
  - id: leaf-test-intrinsics
    content: "Item::Test (`test \"desc\" do ... end`), assert/assert_eq recognized-call-name intrinsics with parse-time file:line capture, synthetic AssertionError class injection"
    status: pending
  - id: leaf-test-runner
    content: "emerald test <file>: synthesizes a runner main dispatching each Item::Test through a rescue, prints PASS/FAIL lines and a passed/failed count, exits non-zero on any failure"
    status: pending
isProject: false
---

# Plan 47 — REPL and Test Framework

This is plan 47 of the 36-47 batch — twelve independent sibling plans
closing Emerald's language/stdlib surface toward the ~45% Ruby-parity
ceiling a prior analysis set for this project, without conceding any of
its identity constraints (no `method_missing`/`eval`/`send`/reflection,
no mixins/open classes/monkey-patching, no dynamic/virtual dispatch, no
tracing GC). Like plans 17–35 before it, this is post-v1 scope — it does
not touch `plan-of-plans.md`, and that document is updated separately,
once, after all twelve plans in this batch are authored. This plan owns
exactly one gap: Emerald has no interactive shell and no way to write an
automated test for an Emerald program without hand-rolling `if`/`raise`
and reading exit codes — both universally expected of any language
someone is asked to adopt for real work, and neither requires any
dynamism this project has ruled out.

Concrete proof this plan targets — a REPL transcript, and a test file
run through `emerald test`:

```
$ emerald repl
Emerald REPL — each line is compiled and run fresh against the session so far.
emerald> x: Int64 = 10
emerald> x + 5
15
emerald> x + "oops"
error: `+` is not defined for Int64 and String
emerald> puts x
10
emerald> exit
```
The third line is rejected with a diagnostic and produces no output of
its own; the fourth line proves `x` is still `10` — the session survived
the rejected line unmodified, and the second line proves an ordinary
top-level expression auto-prints its value with no explicit `puts`.

```ruby
# math_test.em
test "addition works" do
  assert_eq(2, 1 + 1)
end

test "addition is broken on purpose" do
  assert_eq(3, 1 + 1)
end
```
```
$ emerald test math_test.em
PASS: addition works
expected:
3
but got:
2
FAIL: addition is broken on purpose: math_test.em:6
passed:
1
failed:
1
$ echo $?
1
```

## Decision log

- **`emerald-driver`'s `check`/`compile` contract, verified against plan
  17's actual text, is precise enough for this plan to build against
  directly — but it does not exist on disk yet.** Verified this session:
  `ls crates/` lists exactly `emerald-lexer`, `emerald-parser`,
  `emerald-codegen`, `emerald-sema`, `emerald-cli` — no `emerald-driver`
  — and root `Cargo.toml`'s `[workspace] members` matches (five crates).
  Plan 17's `leaf-driver-extraction` (still `status: pending` in its own
  frontmatter) specifies exactly `pub fn check(source: &str, name: &str)
  -> Result<(), DriverError>` (parse+sema only) and `pub fn compile(source:
  &str, name: &str, output_path: &Path) -> Result<(), DriverError>` (full
  pipeline through linking, writing a real executable to `output_path`).
  That is precisely the shape a subprocess-per-line REPL needs — `check`
  for a fast reject-without-compiling path, `compile` to produce a real
  runnable binary — so, unlike the task brief's own hedge, there is no
  gap to fill in `emerald-driver`'s *contract*; the gap is only that the
  crate hasn't been built. This plan's leaves depend on plan 17 landing
  first, the same posture plan 23 (multi-file compilation, also verified
  this session to depend on and predate plan 17's actual extraction) already
  disclosed for itself.
- **Session isolation needs no snapshot/rollback of a sema environment,
  because there is no persistent sema environment to snapshot.** Verified
  this session against `crates/emerald-sema/src/lib.rs`: `pub fn
  check_program(program: &Program) -> Result<(), Vec<Diagnostic>>` (line
  ~1270) builds its `env`/`sigs`/`classes` `HashMap`s fresh, internally,
  on every call — they are locals of `check_program`, never fields of a
  struct the REPL could hold onto across lines. This simplifies the
  originally-anticipated design considerably: isolation is achieved for
  free by never committing a rejected line's source text to the session's
  persisted buffer, not by snapshotting and restoring any compiler-internal
  state object (there is none to restore).
- **The session is a growing UTF-8 source buffer of already-accepted
  *declarations*, not a growing AST and not a replay of every line
  verbatim.** Concretely: `emerald-cli/src/repl.rs` holds `prelude:
  String`. A new line is classified, after a first successful parse via
  `emerald_parser::parse_named`, into one of two kinds: a *declaration*
  (`Item::Function`, `Item::Class`, `Item::Module`, or a top-level
  `Item::Stmt` whose `Stmt` is `Let`/`Assign`/`MultiAssign`/`SetField`/
  `SetIndex`) or a *transient expression* (`Item::Stmt(Stmt::Expr(_))`,
  the only remaining shape). The candidate program compiled on every
  line is `prelude + line_text` (declarations) or `prelude +
  wrapped_line_text` (expressions — see below); `emerald_driver::check`
  runs against that full candidate text every time. On success, a
  *declaration* line's text is appended to `prelude` for future lines to
  see; a *transient expression* line's text is **not** appended — it is
  compiled and run once, for its printed output, and then forgotten,
  exactly as `puts` output already forgotten by the time the next line
  runs in a real terminal. On failure, `prelude` is untouched and the
  diagnostic is printed — the concrete proof's third line. This is why
  replaying `prelude` on every subsequent line reproduces no duplicate
  output: declarations (`Let`, `def`, `class`) have no visible side
  effect at the point they're declared, only when later called or
  printed, and a transient expression is never replayed at all.
  **A real, disclosed limitation of this design:** a `Let` whose
  initializer expression itself has an observable side effect (e.g. `x:
  Int64 = f()` where `f` itself calls `puts`) *is* a declaration and
  *is* replayed on every later line, so its side effect repeats each
  time — an honest cost of "recompile-and-rerun the whole prelude from
  scratch," not silently hidden.
- **A bare top-level expression line is auto-printed by literally
  wrapping its source text in `puts(...)` before compiling it — a
  string-level rewrite done by the REPL frontend, not an AST rewrite and
  not a new compiler feature.** No unparser/pretty-printer exists for
  this AST (nothing in `emerald-parser` renders `Expr`/`Stmt` back to
  source), so the REPL cannot re-serialize a parsed-and-modified AST; it
  instead parses the raw line once purely to classify it (declaration vs.
  expression, and — to avoid double-wrapping — whether the parsed
  `Expr` is already `Expr::Call("puts", _)`), then feeds the *original
  text*, optionally prefixed/wrapped as `puts(<text>)`, to `compile`.
  `puts`'s own existing restriction (`build_puts` in
  `crates/emerald-codegen/src/lib.rs`, verified this session: matches
  only `ValKind::Int64 | Float64 | Str`, erroring otherwise) is inherited
  unchanged — auto-printing an array, hash, or class-instance expression
  at the REPL prompt fails with that same, already-existing codegen
  error, not a new "cannot print this value" REPL-specific message.
- **Each REPL line is re-AOT-compiled and run as a subprocess — not
  JIT-executed via LLVM's ORC/MCJIT layer.** Verified this session:
  `crates/emerald-codegen/Cargo.toml` depends on `inkwell 0.10` with
  feature `llvm21-1`; the crate's own import list (checked via its
  compiled dependency graph) pulls `inkwell::module`, `inkwell::targets`,
  `inkwell::types`, `inkwell::basic_block` — `inkwell::execution_engine`
  (the module that exposes `ExecutionEngine`/JIT) is not used anywhere.
  `pub fn compile_to_object(program: &Program, out_path: &Path) ->
  Result<(), String>` (verified, line ~3523) is the *only* public
  entry point this crate exposes, and it targets `ObjectModule`-style
  object emission, then `emerald-cli` links with a plain `cc -no-pie`
  subprocess call (verified in `crates/emerald-cli/src/main.rs`'s
  `link_stage`). Building real ORC JIT support would mean: standing up
  an `inkwell::execution_engine::ExecutionEngine`, resolving symbols
  across successive incremental modules (each REPL line would otherwise
  be its own freshly-JITted module), and keeping already-JITted
  machine code alive across lines to preserve state — a materially
  larger, currently-nonexistent codegen surface, disproportionate to one
  plan in a batch of twelve independent, similarly-scoped siblings.
  Subprocess-per-line instead reuses the *exact*, already-tested
  `compile_to_object` + `cc` + run pipeline verbatim, with zero new
  `emerald-codegen` surface — only new orchestration in `emerald-cli`.
  **The real, disclosed tradeoff:** every line pays a full LLVM `-O3`
  compile and a linker invocation (measured informally elsewhere in this
  project at low tens of milliseconds for `hello.em`-sized programs, but
  not benchmarked here) instead of a warm in-process JIT's near-zero
  per-line cost — a real, felt latency cost for an interactive tool,
  accepted for a first REPL and not a blocker to a future "REPL v2: real
  ORC JIT" plan once demand for a snappier prompt materializes.
- **`test "..." do ... end` gets its own narrow `"do" Stmt* "end"`
  grammar production, not a general block-literal alternative.** Task
  framing asks this plan to "reuse plan 34's blocks-and-yield block
  syntax" for `test` — checked directly against plan 34's own Decision
  log this session: plan 34 explicitly chose `{ |params| body }` only,
  stating "no `do...end` form" as a deliberate scope cut. Reopening that
  general grammar decision is not this plan's call to make. What this
  plan actually reuses from plan 34/10 is narrower and real: the
  "compile a bare `Vec<Stmt>` body into its own synthesized top-level
  function" codegen technique (`define_lambda`'s shape, verified in
  `crates/emerald-codegen/src/lib.rs`) — not the `&blk`/`yield`
  call-site-specialization machinery itself, since a `test` block takes
  zero parameters, is never attached to a user-called method, and is
  never `yield`ed into. A new, narrow `Item::Test { description: String,
  body: Vec<Stmt> }` top-level AST node, parsed via `"test"
  <description:StringLitTok> "do" <body:Stmt*> "end"`, is a small,
  self-contained grammar addition that reuses `end` (already massively
  overloaded across `if`/`while`/`def`/`class`/`module`/`begin`/`for`/
  `case`, confirmed in `grammar.lalrpop`) as its own closer, and adds
  exactly one new terminal, `"do"`, which does not collide with anything
  since it is not currently used as a keyword or reachable at any other
  position in the grammar.
- **`assert`/`assert_eq` are recognized by literal call name, exactly
  the mechanism `puts` already uses — not new `Stmt` variants.** Verified
  this session: `puts <expr>` is not a real user-callable function (no
  `def puts` exists anywhere); it is matched by the literal string
  `"puts"` inside `build_stmt`/`build_expr`'s `Expr::Call` handling and
  given its own codegen path (`build_puts`). `assert`/`assert_eq` are
  added as two more such recognized names — `Expr::Call("assert",
  [cond, loc])` / `Expr::Call("assert_eq", [expected, actual, loc])` —
  checked in `emerald-sema` and lowered in `emerald-codegen` the same
  way, not modeled as `Stmt::Assert`/`Stmt::AssertEq`.
- **`loc`'s `file:line` value is computed once, at parse time, via a
  targeted use of LALRPOP's already-available `@L` marker plus a small
  post-parse AST rewrite — not plan 22's general `Spanned<T>` overhaul.**
  Plan 22 (sema diagnostic spans — verified this session: `status:
  pending`, no `Spanned<T>` exists anywhere in `emerald-parser::ast`
  today) is the right owner of *general* per-`Expr`/`Stmt` position
  data; redoing that here for two call names would be exactly the
  "invasive, whole-AST change disproportionate to one plan" plan 13 and
  17 already declined for the same reason. This plan's grammar
  production for `assert(...)`/`assert_eq(...)` captures `<l:@L>` (a
  bare byte offset — `@L`/`@R` are real and already unused in
  `grammar.lalrpop`, confirmed this session) and stores it, unconverted,
  as an extra `Expr::Int(offset)` argument — the same "desugar at parse
  time" pattern plan 31 already established for `+=`. `emerald_parser::
  parse_named(source, name)`, which already owns the full source text as
  its own parameter, then runs one small, targeted post-parse pass (not
  a generic `Spanned` walk) over just the freshly-built `Program`
  replacing that raw-offset `Expr::Int` with a real `Expr::StringLit(
  format!("{name}:{line}"))`, `line` computed by counting `\n` bytes in
  `source[..offset]`. Net new surface: one helper function and one
  targeted tree walk, not a change to `Expr`/`Stmt`'s general shape. If
  plan 22 lands first, this plan's targeted mechanism keeps working
  unchanged (it never touches `Spanned<T>`); a natural, undone-here
  follow-up would let it reuse `Spanned<Expr>.span` directly instead of
  its own `@L` capture.
- **A synthetic `AssertionError` class (`message: String`, one
  `initialize`) is injected into the parsed `Program` by the compiler
  whenever it contains a `test`/`assert`/`assert_eq` usage — the user
  never declares it.** `raise <expr>` already requires `expr` to
  evaluate to a class instance (plan 11's Decision log, unchanged) — the
  smallest way to give `assert`'s failure a real, catchable value is an
  ordinary class using the exact field/`initialize`/`raise`/`rescue`
  machinery the `Point` example already proves works, not a new runtime
  primitive. This mirrors an existing pattern in this codebase:
  `define_main` already synthesizes structure (`main`) around user code
  that never declares it itself; `AssertionError` injection is the same
  idea applied one level earlier, in the AST.
- **`assert_eq`'s type-checking is not new sema logic — it is the
  existing `==`-comparison checker, invoked on a synthetic `Expr::
  Compare(expected, CompareOp::Eq, actual)` node.** `emerald-sema`
  already type-checks `Expr::Compare` for exactly this pair-of-operands
  shape (`infer_expr_type`, verified this session at line ~353);
  `assert_eq(2, 1 + 1)` is checked by constructing that same node
  internally and discarding its `Type::Boolean` result, keeping only
  `Ok(())`/`Err(Diagnostic)` — so `assert_eq("s", 1)` is rejected with
  the *existing* Compare type-mismatch diagnostic, not a new one. This
  also fixes `assert_eq`'s real scope precisely: it works for exactly
  the operand types `==` already supports (`Int64`, `Float64`, `String`,
  `Bool` per the existing Compare rules) — not classes, arrays, or
  hashes, since `==` doesn't support those today either and this plan
  doesn't add operator-overload dispatch to get there (that would need
  the same virtual-dispatch machinery the identity ceiling rules out).
- **`emerald test`'s failure-reporting concatenates only `String +
  String`, and prints `Int64` counts on their own separate `puts` lines
  — deliberately never interpolating a count into a message string.**
  Verified this session: `rejects_string_plus_int` is an existing,
  passing `emerald-sema` test — `String + Int64` is a real, standing
  type error in this language, and no `to_s`/format primitive exists
  anywhere in the runtime (`runtime/emerald_runtime.c` only exposes
  `emerald_print_i64`/`_f64`/(string print), no int-to-string
  conversion). `"FAIL: " + description + ": " + e.message` is legal
  (four `String`s); a passed/failed *count* cannot be spliced into that
  same string without a runtime string-formatting primitive this plan
  does not add — so the summary is `puts "passed:"` / `puts passed`
  (an `Int64`, which `puts` already prints) on their own lines. A tidier
  one-line `"3 passed, 1 failed"` summary is real, disclosed future
  work, gated on a stdlib `Int64#to_s`, not this plan's job.
- **The core `emerald test` mechanism needs only plan 11's existing
  single-typed `rescue`, not plan 38's bare catch-all** — a narrower,
  more honest dependency than the task's own framing suggests. Each
  `test` body is wrapped `begin ... rescue AssertionError => e ...
  end`, a shape plan 11 already ships today (verified: `Stmt::Begin`
  with one `rescue_type`/`rescue_var`/`rescue_body`, already compiling
  and running per its own existing test suite) — sufficient for every
  `assert`/`assert_eq` failure, which is this framework's designed
  failure mode. **Plan 38's bare catch-all is what this plan actually
  needs it for:** isolating a test whose body raises something *other*
  than `AssertionError` (a genuine bug in the code under test) so one
  bad test doesn't abort the whole `emerald test` run and lose the
  summary for every test after it. Verified this session: `history/`
  contains no plan 38 file yet (highest present is plan 35) — this
  plan proceeds on the assumed contract given in its own brief (`ensure`,
  multiple typed `rescue`, a bare catch-all, `retry`), and additionally
  assumes the bare-catch value gets *some* statically checkable common
  type. If plan 38 lands without one usable enough to read a `.message`
  field off of (a real risk under this project's no-reflection identity
  constraint — a bare-caught value needs a real static type to have any
  member at all), this leaf's bare-catch branch degrades to a fixed
  string, `"FAIL: " + description + ": (non-assertion failure)"`,
  disclosed explicitly in that leaf's acceptance criteria rather than
  silently assumed away.
- **Out of scope, permanently: a mocking/stubbing framework
  (RSpec-style test doubles).** A test double works by intercepting a
  method call at runtime and substituting different behavior for
  it — in Ruby, `allow(obj).to receive(:method).and_return(x)`
  monkey-patches the object's singleton class. Every method call in
  Emerald resolves statically, from the receiver's declared type, at
  compile time (this project's Decision-log-level identity constraint,
  restated in this batch's own charter) — there is no dispatch point
  left at runtime to intercept. Building mocking here would mean
  building virtual dispatch first, which is explicitly, permanently
  ruled out — this is a real, disclosed, permanent gap against Ruby's
  mature testing ecosystem (RSpec, Mocha, Minitest's `Mock`), not a
  scope cut this plan or any later one is expected to revisit.
- **Out of scope: `emerald test <directory>` / multi-file test-project
  discovery.** `emerald-cli` today (verified in `main.rs`) takes exactly
  one source-file argument; `emerald test` in this plan mirrors that —
  one entry file, which may itself `require` other files (plan 23,
  confirmed implemented via this repo's git history, commit
  `123ebe3`). Globbing a directory for `*_test.em` files and aggregating
  counts across a whole project is real, valuable, and a natural
  follow-up once a real multi-test-file Emerald project exists to prove
  it against — not invented speculatively here.

## Leaf: leaf-repl

### 1. Context
- Why: there is no interactive entry point at all today —
  `emerald-cli`'s `main` (verified, `crates/emerald-cli/src/main.rs`,
  129 lines) requires exactly one source-file argument and only ever
  compiles+links, never runs anything itself.
- Target state: `crates/emerald-cli/src/repl.rs` (new) implements a
  read-loop reading one line at a time from stdin, holding a `prelude:
  String` session buffer (empty initially), classifying and compiling
  each line per the Decision log's algorithm via `emerald_driver::check`/
  `compile`, running the produced temp executable via
  `std::process::Command`, and printing its captured stdout verbatim.
  `main.rs` gains subcommand dispatch: zero args or `emerald repl`
  enters this loop; an explicit source-file argument keeps today's
  exact existing behavior unchanged. The loop exits on a line that is
  exactly `exit` or `quit`, or on EOF (Ctrl-D).
- Depends on: plan 17's `emerald-driver` (`check`/`compile` as specified
  in its Decision log) actually existing — if plan 17 hasn't executed
  yet when this leaf starts, extracting the driver is a real prerequisite
  this leaf does not redo, the same posture plan 23 already disclosed
  for itself against the same dependency.

### 2. Acceptance Criteria
1. Driving `emerald repl` over stdin with exactly this plan's worked
   transcript's five lines (`x: Int64 = 10`, `x + 5`, `x + "oops"`,
   `puts x`, `exit`) produces stdout matching the transcript exactly:
   no output for line 1, `15` for line 2, a diagnostic (not a crash, not
   a hang) for line 3, `10` for line 4 — real, executed, subprocess-based
   proof, not a simulated/mocked compile.
2. After line 3's rejection, `prelude` is byte-identical to its value
   before line 3 was attempted — verified directly against the session
   struct's own held string, not just inferred from line 4's output.
3. Negative case: a line that fails to *parse* at all (e.g. an
   unterminated `if` block never closed) is rejected the same way a
   sema-rejected line is — a diagnostic printed, `prelude` untouched,
   loop continues — proving the isolation holds for parse failures too,
   not only type errors.
4. Regression: `emerald-cli`'s three existing `hello_em.rs` tests
   (compile `examples/hello.em`, run it, assert `42\n`) pass unmodified —
   proof this leaf's subcommand dispatch doesn't disturb the existing
   single-file compile path.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/src/repl.rs`,
  `crates/emerald-cli/tests/repl.rs` (drives the binary over a piped
  stdin, asserts on captured stdout)
- **Modify:** `crates/emerald-cli/src/main.rs` (subcommand dispatch),
  `crates/emerald-driver/src/lib.rs` (consumed as-is per plan 17; no
  change expected unless plan 17's actual landed shape differs from its
  own spec, in which case this leaf's own build failure is the real
  signal, not a silent workaround)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-cli` | clean | agent-claimed-locally |
| REPL transcript | `cargo test -p emerald-cli --test repl` | matches worked transcript exactly | agent-claimed-locally |
| Regression | `cargo test -p emerald-cli --test hello_em` | 3/3 pass unmodified | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-test-intrinsics

### 1. Context
- Why: no grammar/AST shape exists for `test`/`assert`/`assert_eq`
  today (verified: a check against `grammar.lalrpop` this session found
  no such terminals), and there is no `AssertionError` or any
  compiler-synthesized class anywhere in this codebase.
- Target state: `Item::Test { description: String, body: Vec<Stmt> }` in
  `crates/emerald-parser/src/ast.rs`; grammar production `"test"
  <description:StringLitTok> "do" <body:Stmt*> "end"`; `assert`/
  `assert_eq` recognized via the `<l:@L> "assert" "(" <cond:Expr> ")"`/
  `<l:@L> "assert_eq" "(" <expected:Expr> "," <actual:Expr> ")"`
  productions, desugaring to `Expr::Call("assert", [cond,
  Expr::Int(l as i64)])` / `Expr::Call("assert_eq", [expected, actual,
  Expr::Int(l as i64)])` per the Decision log. `emerald_parser::
  parse_named` gains a post-parse pass rewriting each such `Expr::Int`
  placeholder into `Expr::StringLit("{name}:{line}")`. `emerald-sema`
  recognizes `Item::Test` (type-checks `body` as a fresh, `Void`-return,
  no-`self`-fields scope, same shape `check_function_body` already
  uses) and the two call names (`assert`: one `Boolean`-typed argument;
  `assert_eq`: reuses the `Compare(Eq)` checker per the Decision log).
  `emerald-codegen` injects the synthetic `AssertionError` class into
  `Program.items` before layout/codegen whenever any `Item::Test` or
  `assert`/`assert_eq` call is present, and lowers `assert`/`assert_eq`
  via `build_expr`/`build_stmt`'s existing name-recognition mechanism
  (alongside `build_puts`) into the `if !(cond) { raise
  AssertionError.new(loc) }` / mismatch-then-raise shape from the
  Decision log.

### 2. Acceptance Criteria
1. A function body containing `assert(true)` compiles, links, and runs
   with no output and exit code `0` — the happy path raises nothing.
2. A function body containing `assert(false)` on line 3 of a file named
   `t.em`, wrapped in `begin ... rescue AssertionError => e ... end`,
   compiled/linked/run, prints `e.message` equal to exactly `t.em:3` —
   real, executed proof the file:line capture is correct, not merely
   that *some* exception fires.
3. `assert_eq(2, 1 + 1)` raises nothing; `assert_eq(3, 1 + 1)`, run the
   same way as AC2, prints `expected:`, `3`, `but got:`, `2` (in that
   order) before the exception propagates to its `rescue`.
4. Negative case: `assert_eq("s", 1)` is rejected at `emerald-sema` time
   with the existing `Compare`-family type-mismatch diagnostic (naming
   `String`/`Int64`), not a runtime failure and not a panic.
5. Regression: `cargo test --workspace`'s full existing suite (every
   prior plan's fixture) passes unmodified — `AssertionError` injection
   only fires for programs that actually use `test`/`assert`/
   `assert_eq`, verified by asserting a plain `hello.em`-style compile
   produces an object file with no `AssertionError` symbol present.

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (the post-parse rewrite pass +
  tests), `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Parser tests | `cargo test -p emerald-parser` | all pass, incl. `t.em:3`-style location assertions | agent-claimed-locally |
| Sema tests | `cargo test -p emerald-sema` | all pass, incl. `assert_eq` type-mismatch rejection | agent-claimed-locally |
| Codegen (real compiled-and-run proof) | `cargo test -p emerald-codegen` | all pass, incl. AC1–AC3 linked-and-run | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass, no prior fixture changes | agent-claimed-locally |

---

## Leaf: leaf-test-runner

### 1. Context
- Why: nothing today collects `Item::Test` entries into a runnable
  program or reports pass/fail — `leaf-test-intrinsics` makes `test`/
  `assert`/`assert_eq` compile and raise correctly, but only inside a
  hand-written `begin`/`rescue`; there is no `emerald test` subcommand.
- Target state: `emerald_codegen::compile_test_harness(program: &Program,
  out_path: &Path) -> Result<usize, String>` (new; returns the number of
  tests found), a sibling of `compile_to_object` that, instead of
  synthesizing `main` from top-level `Item::Stmt`/`Item::Function`
  bodies, synthesizes it from every `Item::Test` in source order: for
  each, wraps a call into that test's synthesized function in `begin
  ... rescue AssertionError => e ... end` (plan 11, unchanged), printing
  `puts("PASS: " + description)` on success or `puts("FAIL: " +
  description + ": " + e.message)` on catch, incrementing an `Int64`
  `passed`/`failed` local; at the end, `puts "passed:"`/`puts
  passed`/`puts "failed:"`/`puts failed`, then returns `1` from `main`
  if `failed > 0` else `0` (the same non-zero-exit-on-failure convention
  `emerald-cli` already uses for parse/sema/link failures). Compiling a
  file containing `Item::Test` via the *ordinary* (non-`test`)
  `emerald <file>` path is a rejected, described error (`"top-level
  test block only valid under emerald test"`), not silently ignored
  or silently run — consistent with the existing "unsupported top-level
  shape errors, not panics" precedent already proven in this crate's own
  test suite. `crates/emerald-cli/src/test_runner.rs` (new) wires
  `emerald test <file>` through `emerald_driver`'s `check` (rejecting
  parse/sema errors exactly as `emerald-cli`'s normal path already does,
  before ever calling `compile_test_harness`), then
  `compile_test_harness` + the existing `cc -no-pie` link step, then
  runs the produced binary and propagates its exit code.
- Depends on: `leaf-test-intrinsics` (this leaf only orchestrates
  already-compiling `Item::Test`/`assert`/`assert_eq` shapes). Plan 38's
  bare catch-all is *not* required for this leaf's core AC — see
  Decision log; it is required only for the bonus AC below covering a
  non-`AssertionError` failure inside a test body.

### 2. Acceptance Criteria
1. This plan's own worked `math_test.em` example, run via `emerald test
   math_test.em`, produces stdout exactly matching the worked example
   (including line/counting order) and exits with code `1`.
2. A test file where every test passes exits `0`, with `passed:`/`1`/
   `failed:`/`0`-shaped output (arity matching however many `test`
   blocks the file declares) — a real, distinct, all-green run, not
   just the negative case above.
3. Negative case: a file that fails to parse or type-check (e.g. an
   `assert_eq` operand type mismatch) run via `emerald test` prints the
   same diagnostic `emerald <file>` would have printed, exits non-zero,
   and never attempts `compile_test_harness`/produces no test-count
   output at all — proof the existing check-first pipeline is reused,
   not bypassed.
4. Regression: compiling that same `math_test.em` file via the ordinary
   `emerald math_test.em` (not `emerald test`) is rejected with the
   `"only valid under emerald test"`-style diagnostic from a *specific*,
   asserted message substring — not silently compiled into a no-op
   binary and not a panic.
5. **Contingent on plan 38's bare catch-all landing with a usable common
   caught-value type:** a test body that raises some class other than
   `AssertionError` is caught, reported as `FAIL: <description>: (non-
   assertion failure)` (or, if plan 38's bare-catch exposes a usable
   message field, that real message), and every test *after* it in the
   same file still runs and is reported — proof one broken test doesn't
   silently swallow the rest of the suite. If plan 38 has not landed by
   the time this leaf executes, this AC is deferred (not silently
   dropped) and the leaf ships with the plan-11-only mechanism from AC1–4
   intact.

### 3. File & Module Structure
- **Create:** `crates/emerald-cli/src/test_runner.rs`,
  `crates/emerald-cli/tests/test_subcommand.rs`
- **Modify:** `crates/emerald-codegen/src/lib.rs` (adds
  `compile_test_harness`), `crates/emerald-cli/src/main.rs` (`emerald
  test <file>` dispatch), `crates/emerald-driver/src/lib.rs` (thin
  passthrough to `compile_test_harness`, mirroring how `compile` already
  wraps `compile_to_object`)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build -p emerald-codegen -p emerald-cli` | clean | agent-claimed-locally |
| Test-runner integration | `cargo test -p emerald-cli --test test_subcommand` | AC1–4 pass; AC5 pass or explicitly-deferred per plan-38 availability | agent-claimed-locally |
| Codegen unit tests | `cargo test -p emerald-codegen` | all pass, incl. `compile_test_harness`'s reject-under-`emerald`-path case | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
