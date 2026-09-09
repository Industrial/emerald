---
name: Standard Library — Strings and I/O
overview: "A curated `String` method surface (`.length`/`.upcase`/`.downcase`/`.strip`/`.split`+`.split_count`/`.to_i`/`.to_f`/indexing/`.slice`) plus `File.read`/`File.write` and `ARGV`/`ARGC`/`gets()` — every one a compiler-known intrinsic dispatched by receiver storage kind or reserved namespace name, the exact mechanism `puts` already uses, never a general built-in-method-extension system."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-string-basic-intrinsics
    content: "`.length`/`.upcase`/`.downcase`/`.strip`/`.to_i`/`.to_f` — fixed-arity, zero-argument String intrinsics dispatched inside the existing generic `MethodCall` sema/codegen path"
    status: pending
  - id: leaf-string-indexing-slicing
    content: "`str[i]` single-character indexing (extends `Expr::Index`) and `.slice(start, len)` (a method intrinsic, since `Expr::Index` is single-argument only)"
    status: pending
  - id: leaf-string-split
    content: "`.split(sep): Array[String]` paired with `.split_count(sep): Int64` — the array-length-tracking gap this plan cannot avoid, forced by `Array[T]`'s already-documented no-length-metadata representation"
    status: pending
  - id: leaf-file-io
    content: "`File.read(path): String` / `File.write(path, content): Void` — a compiler-provided namespace reusing plan 12's `Name.method(args)` static-dispatch shape, backed by real libc file I/O in `runtime/emerald_runtime.c`"
    status: pending
  - id: leaf-argv-and-gets
    content: "`ARGV: Array[String]` + `ARGC: Int64` populated from the process's real `argv` at the top of generated `main`, and `gets(): String` reading one line from stdin"
    status: pending
isProject: false
---

# Plan 45 — Standard Library: Strings and I/O

This is plan 45 of the 36-47 follow-up batch — twelve independent
sibling plans (string interpolation/heredocs, symbols, this plan, and
nine others not itemized here) whose combined, shared purpose is
closing Emerald's language/stdlib surface from the ~10-15%-of-Ruby
measured once plans 01-35 ship toward the ~45%-of-Ruby ceiling a prior
analysis set as Emerald's strategic maximum: close every gap compatible
with Emerald's identity (static-only dispatch, no reflection, no
mixins, no tracing GC), decline anything that would require dynamism.
Like plans 28-35 before it, this is post-v1 scope and does not touch
[`plan-of-plans`](2026-09-08T174011Z-plan-of-plans.md) — that table's
own Completion note already calls Emerald v1 done at row 15; a separate
pass updates `plan-of-plans.md` once all twelve siblings in this batch
are authored, not this document.

Depends on: plan 19 (`2026-09-08T214500Z-plan-19-string-literals.md` —
`String` literals and the `ValKind::Str`/`emerald_runtime.c` foundation
this plan builds every intrinsic on), plan 12
(`2026-09-08T195103Z-plan-12-modules.md` — the `Name.method(args)`
namespace-dispatch shape `File` reuses), plan 36 (string interpolation
and heredocs — a sibling authored in parallel this session; assumed
contract: `"...#{expr}..."` and `<<~IDENT` on top of plan 19's plain
literals) and plan 44 (symbols — a sibling authored in parallel this
session; assumed contract: a compile-time-interned `Symbol` from `:foo`
literals). See the Decision log for exactly how load-bearing each of
these four actually is to this plan's own acceptance criteria — two are
hard prerequisites, two are soft/compatibility citations.

Concrete proof this plan targets (a fixed literal input, not `ARGV`/
`gets` — see the Decision log for why the combined worked example
needs a compile-time-known input to have a compile-time-known expected
output; `ARGV`/`gets` get their own leaf with their own, separately
deterministic, fixed-invocation proofs):

```ruby
input: String = "hello world foo"
upper: String = input.upcase
File.write("plan45_demo.txt", upper)
readback: String = File.read("plan45_demo.txt")
puts readback
n: Int64 = readback.split_count(" ")
puts n
words: Array[String] = readback.split(" ")
i: Int64 = 0
while i < n
  puts words[i]
  i += 1
end
```

Expected output, in order:
```
HELLO WORLD FOO
3
HELLO
WORLD
FOO
```
Run from a fresh temporary working directory (so `plan45_demo.txt` is
created, not pre-existing) — a real proof `.upcase` transforms the
string, `File.write` persists it, `File.read` reads the same bytes
back, `.split_count`/`.split` genuinely tokenize the reconstituted
string, and existing `while`/indexing machinery iterates the result
correctly.

## Decision log

- **No user-defined methods on built-in types exist today — verified,
  not assumed.** `crates/emerald-sema/src/lib.rs`'s `resolve_type`
  (L88-99) matches the literal string `"String"` to `Ok(Type::String)`
  *before* it ever checks the `classes` registry, so even if `class
  String ... end` parsed (nothing in `grammar.lalrpop`'s `ClassDef`
  reserves the name `String`, so it would), the resulting `ClassInfo`
  would be permanently unreachable: every value actually typed
  `Type::String` can never become `Type::Class("String")`, and
  `infer_expr_type`'s generic `MethodCall` fallback (L523-539) requires
  exactly `Type::Class(_)`, erroring otherwise. There is no live code
  path from a `String`-typed receiver to a user-declared method, full
  stop. This plan's entire String method surface is therefore built the
  same way `puts` already is — `infer_expr_type`'s own comment at L437
  calls `puts` "a compiler intrinsic, not an overloaded function...
  checked here directly rather than via a `FunctionSig` in `sigs`" — a
  fixed, curated set of names recognized and type-checked directly in
  sema, with a matching fixed dispatch in `emerald-codegen`'s
  `build_method_call`/`build_index`, never a general "look up a method
  on this type" mechanism. Building a real extension mechanism (trait-
  or operator-overload-style static dispatch usable by any built-in
  type, or an "open" intrinsic registry) is explicitly out of scope —
  it's a distinct, larger feature this plan's fixed six-plus-four-plus-
  two intrinsic set doesn't need.
- **String intrinsics are dispatched by checking the receiver's
  *inferred type* inside the existing generic `MethodCall` arm, not by
  adding a new match arm guarded on the method name.** A name-only guard
  (`if method == "upcase"`) would incorrectly intercept a user-defined
  class method that happens to share a name with a String intrinsic —
  a real collision risk since nothing reserves `upcase`/`length`/etc. as
  identifiers anywhere in `grammar.lalrpop`. Gating on `recv_ty ==
  Type::String` *after* `infer_expr_type(recv)` already runs (the same
  point the existing fallback arm inspects `recv_ty` before requiring
  `Type::Class`) avoids the collision entirely — a class instance
  calling its own `.upcase` method still reaches the ordinary
  `Type::Class` path unchanged. Codegen mirrors this: `build_method_call`
  checks `vars.get(recv_name)`'s stored `ValKind` for `ValKind::Str`
  before falling through to `local_classes`' class-name lookup (which
  today errors with "cannot determine the class of `{name}`" for any
  receiver that isn't a registered class — verified at L1610-1612 —
  a String-typed local was always silently doomed to hit exactly that
  error before this plan).
- **`File` reuses plan 12's dispatch *shape* (`Name.method(args)`,
  statically resolved, no receiver value) but is a separate, hard-coded
  arm — not literally plan 12's mechanism.** Verified: `module_info`
  (`crates/emerald-sema/src/lib.rs` L258-271) only ever builds a
  `ClassInfo{is_module: true}` from a real, source-parsed `ModuleDef`
  (`"module" <name:Ident> <methods:FuncDef*> "end"` in
  `grammar.lalrpop`), and `emerald-codegen`'s `Ctx.module_names` (used
  by `build_method_call`'s early-return at L1579-1603) is populated
  the same way, from `program.items`. `File` is never declared in any
  `.em` source — it has no `ModuleDef`, no `ClassInfo`, and would never
  populate either table. So `File.read(path)`/`File.write(path,
  content)` get their own intrinsic arm in `infer_expr_type` (guarded
  on `matches!(recv.as_ref(), Expr::Ident(n) if n == "File")`, checked
  before the real module-dispatch arm purely for arm-ordering hygiene,
  since it can never actually collide — `File` is never in `classes`)
  and their own early-return in `build_method_call`, calling two new
  `emerald_file_read`/`emerald_file_write` runtime functions directly.
  Building a general "declare a compiler-provided namespace without a
  matching `ModuleDef`" mechanism for File's sake alone is not attempted
  — one hard-coded arm is strictly less code than a new registration
  mechanism that would have exactly one user.
- **`Array[T]` has no runtime length metadata at all — a real,
  already-documented representation limit this plan runs straight into
  and works around, not a new discovery it gets to redesign.** Verified
  two independent places: `build_array_lit`'s doc comment
  (`crates/emerald-codegen/src/lib.rs` L1657-1662) states array
  literals produce "no length prefix, no bounds checking", and
  `Stmt::For`'s doc comment (`crates/emerald-parser/src/ast.rs`) states
  `for`-`in` is grammar-restricted to a literal array specifically
  "since `Array[T]`'s own representation carries no runtime length to
  iterate against." A `.split(sep): Array[String]` whose element count
  is only known at runtime therefore cannot be iterated by anything
  this compiler has today — not `for`-`in`, not any `.length` (no such
  method exists on `Array` at all). The smallest fix that doesn't
  redesign `Array[T]` is a companion scalar: `.split_count(sep): Int64`,
  computed independently (a second full scan of `sep`'s occurrences —
  a disclosed, accepted inefficiency; every runtime call in
  `emerald_runtime.c` returns exactly one value, and inventing a
  struct-returning ABI to share one scan between `.split`/
  `.split_count` is strictly more machinery than this plan needs).
  `ARGV`/`ARGC` hit the identical wall for the identical reason and get
  the identical fix (see the `leaf-argv-and-gets` entry below). A real,
  general dynamic-length `Array[T]` (a stored length header, a real
  `.length`, `for x in <arbitrary Array[T] expression>`) is explicitly
  out of scope — a distinct, larger feature this plan's two narrow,
  disclosed workarounds don't attempt to generalize into.
- **`gets()` requires explicit parentheses — `gets` bare is not
  recognized.** Verified: `grammar.lalrpop`'s only zero-argument-call
  shape is `<name:Ident> "(" <args:Args> ")"` with `Args` allowing zero
  elements (`Args: Vec<Expr> = { <mut v:(<Expr> ",")*> <last:Expr?> =>
  ...}`, confirmed empty when both are absent); there is no "bare
  `Ident` used as a zero-arg call" production anywhere — a bare `Ident`
  is always `Expr::Ident`, i.e. a variable lookup. `puts` sidesteps this
  by being a dedicated *statement* keyword (`"puts" <arg:Expr>`,
  `Stmt`-level, per plan 07's Decision log), which works because `puts`
  is never needed in expression position. `gets` must be usable as an
  expression (`line: String = gets()`), so it doesn't get that same
  carve-out — Ruby's own paren-optional `gets` is a real, disclosed,
  minor deviation here, forced by grammar mechanics rather than chosen
  for its own sake.
- **This entire plan needs zero grammar changes.** Verified directly
  against `grammar.lalrpop`: `<recv:Ident> "." <method:Ident> "("
  <args:Args> ")"` (generic method call) already covers `s.upcase`,
  `s.split(sep)`, `s.slice(start, len)`, and `File.read(path)`/
  `File.write(path, content)` with no new terminal or production;
  `<base:CallExpr> "[" <idx:Expr> "]"` (generic indexing) already
  covers `s[i]`; `<name:Ident> "(" <args:Args> ")"` already covers
  `gets()`. Every leaf below only touches `emerald-sema`,
  `emerald-codegen`, and `runtime/emerald_runtime.c` — a notable,
  verified fact for a plan whose surface looks this broad on paper.
- **String indexing/slicing are unchecked, matching `Array`'s own
  existing precedent exactly.** `emerald_alloc`'s doc comment already
  discloses "no free... no lifetime tracking", and `build_array_lit`
  already discloses "no bounds checking" for arrays — `str[i]`/
  `.slice(start, len)` inherit the identical no-bounds-check contract
  (an out-of-range access is real, disclosed undefined behavior, not a
  new UB surface this plan introduces). `.upcase`/`.downcase` are
  byte-wise ASCII case conversion only (`toupper`/`tolower` per byte) —
  `spec/TYPE_SYSTEM.md` §9 declares `String` UTF-8 encoded, and a
  byte-wise conversion can corrupt a multi-byte UTF-8 sequence; full
  Unicode-aware case folding needs a real Unicode library dependency
  this plan declines to add for two case-conversion methods.
- **A `Regexp` type/engine is explicitly out of scope.** `.split`,
  `.strip`, and every other method in this plan operate on literal
  separator/whitespace bytes only — no pattern matching anywhere. A
  real `Regexp` needs its own literal syntax, an NFA/DFA engine or an
  FFI binding to a C regex library (e.g. PCRE2), and capture-group
  typing (what type does a capture group even have in a statically-
  typed language with no `nil`-friendly optional system? — a real,
  separate design question). This is a substantial standalone feature
  deserving its own future plan beyond this 36-47 batch, not an
  intrinsic this plan can absorb as a side effect of `.split`.
- **`.split(sep)` declines two real pieces of Ruby's default `String
  #split` behavior: it does not collapse runs of whitespace when
  `sep == " "` (Ruby's awk-like special case for the default/space
  separator), and it does not trim trailing empty strings.** Both are
  genuine Ruby-idiomatic behaviors this plan is deliberately not
  reproducing — always literal-substring splitting, always every
  segment including trailing empties — because either one would need
  `.split_count`'s independent count-only scan to agree exactly with
  `.split`'s element-producing scan on which segments get dropped, a
  synchronization surface this plan's two-independent-scans design
  (see above) doesn't want to carry. A single, simple, literal
  strstr-based split is the smaller, correct-by-construction choice.
- **File I/O errors are a disclosed runtime abort
  (`fprintf(stderr, ...); exit(1)`), not wired into the class-based
  `raise`/`rescue` exception system (plan 11).** This mirrors
  `emerald_hash_key_not_found`'s existing precedent exactly (a
  `Hash[K,V]` miss aborts the same way, per plan 25's Decision log) —
  a real, controlled failure path, not silent UB. A recoverable
  `File.read` (an `IOError`/`Errno`-mapped exception hierarchy a
  program could `rescue`) is a distinct, larger feature: it needs a
  real errno-to-class mapping and for every File call site to become a
  `raise`-capable one, neither of which this plan's narrow read/write
  pair needs to prove File I/O works at all.
- **Mutating bang-methods (`.upcase!`, `.strip!`, etc.) are out of
  scope.** `spec/TYPE_SYSTEM.md` §9 calls `String` "mutable in v1", but
  no method on *any* type in this compiler mutates its receiver in
  place today — the only mutation mechanisms that exist are
  `Stmt::SetField` (`@x = ...`) and `Stmt::SetIndex` (`arr[i] = ...`),
  both plain assignment statements, not method calls. A `.upcase!`
  would need a genuinely new capability (a method call that writes back
  through its receiver) with no existing precedent anywhere in
  `emerald-codegen` to extend; this plan's methods are all the
  Ruby non-bang, return-a-new-`String` shape, which fits the existing
  "a method call produces a value" mechanism with zero extension.
- **Relationship to the batch's two parallel siblings.** Plan 36
  (interpolation/heredocs) is a *soft* dependency: every intrinsic this
  plan adds dispatches purely on `ValKind::Str` — it cannot tell, and
  does not need to tell, whether a given `String` value came from a
  plain `"..."` literal (plan 19), an interpolated `"...#{expr}..."`,
  or a `<<~IDENT` heredoc (both plan 36); nothing here special-cases
  literal syntax, so this plan's whole method surface is retroactively
  available to plan 36's output with zero additional work once plan 36
  lands, and this plan's own worked examples don't require plan 36 to
  exist (verified above — the combined proof uses only plain literals).
  Plan 44 (symbols) is cited for architectural precedent, not runtime
  coupling: this plan's dispatch-by-receiver-storage-kind pattern
  (checked inside the shared generic `MethodCall` arm, not a
  proliferation of new name-guarded arms) is the template plan 44's own
  future `Symbol` intrinsics (if any, e.g. a `.to_s` conversion) should
  reuse rather than inventing a third parallel dispatch mechanism
  alongside `puts`'s and this plan's.

## Leaf: leaf-string-basic-intrinsics

### 1. Context
- Why: `String` has real values (plan 19) but zero methods — every
  program that wants to inspect or transform one has no vocabulary at
  all beyond `+` (concatenation) and `==`/`!=` (plan 19's own scope).
- Target state: `.length -> Int64`, `.upcase -> String`, `.downcase ->
  String`, `.strip -> String`, `.to_i -> Int64`, `.to_f -> Float64`, all
  zero-argument, all dispatched inside the existing generic
  `Expr::MethodCall` arm of `infer_expr_type`
  (`crates/emerald-sema/src/lib.rs`, currently L523-539) once `recv_ty
  == Type::String` is detected (see Decision log for why this is a
  branch inside the existing arm, not a new guarded one), and inside
  `build_method_call` (`crates/emerald-codegen/src/lib.rs`, currently
  L1562-1655) once `vars.get(recv_name)`'s `ValKind` is `Str`. New
  `Ctx<'ctx>` fields (`string_length`, `string_upcase`,
  `string_downcase`, `string_strip`, `string_to_i`, `string_to_f`) hold
  the declared runtime `FunctionValue`s, declared in `compile_to_object`
  alongside `print_str`/`string_concat`/`string_eq` (L3564-3579). New
  `runtime/emerald_runtime.c` functions: `emerald_string_length`
  (`strlen`), `emerald_string_upcase`/`emerald_string_downcase`
  (allocate a same-length copy via `emerald_alloc`, `toupper`/
  `tolower` per byte), `emerald_string_strip` (trims ASCII
  space/tab/newline/carriage-return from both ends into a fresh
  allocation), `emerald_string_to_i`/`emerald_string_to_f` (`atoll`/
  `atof`).

### 2. Acceptance Criteria
1. `name: String = "chicago"` then `puts name.length` — compiled,
   linked, run — prints `7`.
2. `name: String = "chicago"` then `puts name.upcase` prints `CHICAGO`;
   a separate `shout: String = "CHICAGO"` then `puts shout.downcase`
   prints `chicago`.
3. `raw: String = "  padded  "` then `puts raw.strip` prints exactly
   `padded` with no leading/trailing spaces in the printed line.
4. `digits: String = "42abc"` then `puts digits.to_i` prints `42`;
   `decimal: String = "3.5"` then `puts decimal.to_f` prints `3.5`.
5. Negative: `x: Int64 = 5` then `x.length` is rejected with a
   diagnostic naming `x`'s actual type — a String-only intrinsic name
   is never silently accepted on a non-String receiver, and never
   panics.
6. Negative: `s: String = "hi"` then `s.upcase(1)` (wrong arity — this
   intrinsic takes zero arguments) is rejected with a diagnostic, not
   a panic.
7. Regression: every prior plan's example — in particular plan 19's own
   `STRING_EXAMPLE` (concatenation/equality) and every non-String
   example (`POINT_EXAMPLE`, `ARRAY_EXAMPLE`, etc.) — still parses,
   type-checks, and (where already compiled-and-run) still compiles and
   runs identically.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Sema unit tests | `cargo test -p emerald-sema` | all pass, incl. new negative-diagnostic cases | agent-claimed-locally |
| Workspace (real compiled+run proof) | `cargo test --workspace` | all pass, incl. `length`/`upcase`/`downcase`/`strip`/`to_i`/`to_f` printed-output assertions | agent-claimed-locally |

---

## Leaf: leaf-string-indexing-slicing

### 1. Context
- Why: Ruby's `str[i]`/`str[start, len]` have no equivalent yet;
  `Expr::Index(Box<Expr>, Box<Expr>)` (`crates/emerald-parser/src/
  ast.rs`) is single-argument only, so a two-argument bracket slice
  (`str[1, 3]`) has no grammar shape to land in without a new,
  broader multi-arg-bracket production — declined here in favor of a
  `.slice(start, len)` method intrinsic, matching Ruby's own less-common
  but equivalent `str.slice(1, 3)` method form.
- Target state: `Expr::Index` gains a `Type::String` case in
  `infer_expr_type` (`crates/emerald-sema/src/lib.rs`, currently
  L557-581, alongside the existing `Type::Array`/`Type::Hash` arms) —
  index must be `Int64`, result is `Type::String`. `build_index`
  (`crates/emerald-codegen/src/lib.rs`, currently L1822-1900) gains a
  branch checking `vars.get(arr_name)`'s `ValKind` for `Str` (alongside
  the existing `local_array_elem_types`/`local_classes` checks),
  calling a new `emerald_string_char_at(const char*, long long) ->
  char*` runtime function (allocates a 2-byte buffer: the one
  character plus a NUL terminator). `.slice(start, len)` is a new
  `Type::String`-gated branch in the same generic `MethodCall` arm
  `leaf-string-basic-intrinsics` extends, checking both arguments are
  `Int64`, backed by a new `emerald_string_slice(const char*, long
  long, long long) -> char*` runtime function (allocates `len + 1`
  bytes, `memcpy`s from `s + start`, NUL-terminates).

### 2. Acceptance Criteria
1. `s: String = "hello"` then `puts s[1]` prints `e` (a real one-
   character `String`, not an `Int64` byte value).
2. `s: String = "hello"` then `puts s.slice(1, 3)` prints `ell`.
3. Negative: `idx: Float64 = 1.0` then `s[idx]` is rejected with a
   diagnostic ("array/string index must be Int64" or equivalent) —
   `Float64` is never silently truncated to an index.
4. Negative: `s.slice(1)` (wrong arity — `.slice` takes exactly two
   arguments) is rejected with a diagnostic.
5. Regression: `Array`/`Hash` indexing (plan 09's `ARRAY_EXAMPLE`, plan
   25's `HASH_EXAMPLE`) are unaffected — `build_index`'s new `String`
   branch is checked independently of, and does not shadow, the
   existing `local_array_elem_types`/`local_classes` checks.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real compiled+run `s[1]`/`s.slice(1,3)` output assertions | agent-claimed-locally |

---

## Leaf: leaf-string-split

### 1. Context
- Why: `.split(sep)` is the one intrinsic in this plan that produces a
  *collection*, and `Array[T]` carries no runtime length (see the
  Decision log's dedicated bullet) — so `.split` alone would produce a
  value with no way to know how many elements it holds. This leaf
  therefore ships `.split`/`.split_count` together; neither is useful
  as a solo acceptance criterion.
- Target state: two new `Type::String`-gated branches in the same
  generic `MethodCall` arm: `.split(sep: String) -> Array[String]` and
  `.split_count(sep: String) -> Int64`. Verified this session: the
  *existing*, unmodified generic `Stmt::Let` arm in
  `crates/emerald-codegen/src/lib.rs` (currently L2423-2450) already
  populates `local_array_elem_types` for *any* `Array[Elem]`-typed
  `Let`, regardless of which `Expr` variant produced the value (it
  reads the element type from the `Let`'s own declared annotation, not
  from the RHS expression) — so `words: Array[String] = s.split(sep)`
  needs no codegen change beyond `build_method_call` itself returning
  the right pointer value; indexing `words[i]` afterward already works
  through the existing, unmodified `build_index`. New runtime
  functions: `emerald_string_split_count(const char*, const char*) ->
  long long` and `emerald_string_split(const char*, const char*) ->
  void**` (each independently scans for `sep` via `strstr`; `.split`
  allocates one `emerald_alloc`'d copy per segment plus one
  `emerald_alloc`'d pointer buffer for the segment pointers themselves
  — see Decision log for why two independent scans, not one shared
  one). Both reject an empty `sep` with a disclosed runtime abort,
  matching `emerald_hash_key_not_found`'s existing precedent.

### 2. Acceptance Criteria
1. `s: String = "a b c"` then `puts s.split_count(" ")` prints `3`.
2. The same `s`, `words: Array[String] = s.split(" ")`, then
   `puts words[0]`, `puts words[1]`, `puts words[2]` print `a`, `b`,
   `c` in order — compiled, linked, run.
3. A `while i < s.split_count(" ")` loop indexing `s.split(" ")[i]`... 
   is not required to re-derive per iteration; this plan's own
   combined worked example (top of this document) is the canonical
   compiled-and-run proof, printing exactly `HELLO`, `WORLD`, `FOO` on
   three separate lines after the loop.
4. Negative: `s.split("")` (empty separator) is a disclosed runtime
   abort (non-zero exit, a diagnostic on stderr) — documented as
   expected behavior in the leaf's own test, not silently looping
   forever or producing garbage.
5. Regression: `.length`/`.upcase`/etc. from `leaf-string-basic-
   intrinsics` and indexing/`.slice` from `leaf-string-indexing-
   slicing` are unaffected — the new branches are additive to the same
   `match method.as_str()`-shaped dispatch, not a restructuring of it.

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real compiled+run `.split`/`.split_count` indexed-loop output | agent-claimed-locally |

---

## Leaf: leaf-file-io

### 1. Context
- Why: Emerald has no I/O beyond `puts` — no program can read or write
  a file. `File.read`/`File.write` are the minimum real filesystem
  surface, deliberately narrow (no `File.open`/block-based handle,
  no append mode, no directory listing).
- Target state: `File.read(path: String) -> String` and
  `File.write(path: String, content: String) -> Void`, dispatched via
  a new, hard-coded `Expr::MethodCall` arm in `infer_expr_type`
  (guarded on `recv.as_ref() == &Expr::Ident("File".to_string())`,
  placed before the real module-dispatch arm at L492-509 purely for
  arm-ordering clarity) and a matching early-return in
  `build_method_call` (checked immediately after the existing
  `ctx.module_names` check at L1579-1603, since `File` is never a
  member of that set — see Decision log). New `Ctx<'ctx>` fields
  `file_read`/`file_write`. New `runtime/emerald_runtime.c` functions
  `emerald_file_read(const char* path) -> char*` (`fopen`/`fseek`/
  `ftell`/`fread` into an `emerald_alloc`'d NUL-terminated buffer) and
  `emerald_file_write(const char* path, const char* content) -> void`
  (`fopen`/`fwrite`/`fclose`); both abort with a disclosed
  `fprintf(stderr, ...); exit(1)` on an `fopen` failure (see Decision
  log — not wired into `raise`/`rescue`).

### 2. Acceptance Criteria
1. `File.write("plan45_write_test.txt", "hello file")` then
   `File.read("plan45_write_test.txt")` — compiled, linked, run in a
   fresh temporary working directory — round-trips exactly: `puts
   File.read("plan45_write_test.txt")` prints `hello file`.
2. Negative (sema): `File.read(42)` (an `Int64` argument where `String`
   is required) is rejected with a diagnostic, not accepted or
   silently coerced.
3. Negative (sema): `File.write("only-one-arg.txt")` (arity mismatch —
   `File.write` requires exactly two arguments) is rejected with a
   diagnostic.
4. Negative (runtime, disclosed): `File.read("/nonexistent/path")`
   compiled and run against a guaranteed-missing path aborts with a
   non-zero exit code and a stderr message — documented as the
   expected, disclosed failure mode, not a silent empty-string return.
5. Regression: plan 12's own module examples (`MODULE_EXAMPLE`) still
   parse, type-check, compile, and run identically — the new `File`
   arm is checked independently of, and never shadows, real user-
   declared `module`s (verified: no `.em` source can ever name a
   module `File` and have it collide, since the sema arm matches on
   the literal receiver name `"File"` regardless of what `classes`
   contains, and a real `module File ... end` would still register
   into `classes` as before — a genuine naming collision a user could
   create is out of scope to defend against here, matching this
   compiler's existing "no shadowing checks for built-in names" posture
   elsewhere, e.g. nothing stops a user class from being named
   `Int64`-adjacent things either).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real compiled+run `File.write`-then-`File.read` round-trip in a temp dir | agent-claimed-locally |

---

## Leaf: leaf-argv-and-gets

### 1. Context
- Why: no program can currently see its own command-line arguments or
  read a line of stdin. Verified: `compile_to_object`
  (`crates/emerald-codegen/src/lib.rs`, currently L3700-3701) declares
  `main` as `fn() -> i32` — no parameters at all — so there is
  currently no path from the OS's real `argv` into any generated
  program.
- Target state: `main`'s LLVM signature becomes `fn(i32, ptr) -> i32`
  (`argc`, `argv`, the ordinary C `main` ABI `cc`'s own linked `_start`
  already expects, so no change to `emerald-cli`'s link step is
  needed). `define_main` (currently L3385-3431) allocates two extra
  storage slots — `"ARGV"` (`ValKind::Ptr`) and `"ARGC"`
  (`ValKind::Int64`) — before `build_block` runs, populated via a new
  `emerald_build_argv(int argc, char** argv) -> void**` runtime
  function (skips `argv[0]`, the program name, matching Ruby's own
  `ARGV`; reuses the OS-owned `argv` string pointers directly with no
  copy, since they already live for the whole process — the same
  no-ownership assumption every allocation in this runtime already
  makes) and a plain `argc - 1` subtraction for `ARGC`. `emerald-sema`
  needs no new AST-visiting code at all: `check_program`'s `top_env`
  (currently seeded empty at L1699) is pre-seeded with `"ARGV" ->
  Type::Array(Box::new(Type::String))` and `"ARGC" -> Type::Int64`
  before the top-level statement loop runs — `infer_expr_type`'s
  existing `Expr::Ident` arm (an ordinary `env.get(name)` lookup)
  resolves both with zero special-casing beyond the initial seed.
  `gets(): String` is a new `Expr::Call(name, args) if name == "gets"`
  arm in both `infer_expr_type` (checked the same way `puts` is, at
  L437-455) and `build_expr`'s general `Expr::Call` handling (currently
  L1312-1327 — inserted as a new arm above it, since `gets` returns a
  real, non-`Void` value and must be usable as an expression, unlike
  `puts`), backed by a new `emerald_gets(void) -> char*` runtime
  function (`getline` from `stdin`, copied into an `emerald_alloc`'d
  buffer so the returned `String` came from this runtime's one
  allocator; returns an empty `""` string at EOF rather than `nil` —
  a disclosed deviation forced by `Type::Nil`'s already-narrow, non-
  optional scope, per plan 25's Decision log).

### 2. Acceptance Criteria
1. Compiled once, then run twice with different fixed arguments in the
   test itself (e.g. invoked as `./a.out foo bar`): `puts ARGC` prints
   `2`; `puts ARGV[0]` prints `foo`; `puts ARGV[1]` prints `bar` — a
   real, deterministic proof (the test controls the exact argv the
   compiled binary receives, even though `ARGV`'s *content* is not
   compile-time-known in general).
2. Run with zero extra arguments (`./a.out`): `puts ARGC` prints `0`.
3. `line: String = gets()` fed fixed stdin content `"hello\n"`, then
   `puts line.strip` prints `hello` — a real compiled-and-run proof
   composing this leaf's `gets()` with `leaf-string-basic-intrinsics`'s
   `.strip` (stdin's trailing newline is real content `gets()` keeps,
   matching Ruby's own un-chomped `gets`; `.strip` is what removes it
   here, not a hidden `gets`-side behavior).
4. Fed no stdin at all (EOF immediately): `gets()` returns `""`, not a
   panic and not a hang.
5. Regression: every prior compiled-and-run example in
   `emerald-codegen`'s own test suite (`POINT_EXAMPLE` through
   `INHERITANCE_EXAMPLE`) still compiles and runs identically —
   changing `main`'s LLVM signature from zero-arg to `(i32, ptr)` must
   not change any existing program's observable behavior (`cc`'s
   linked C runtime calls `main(argc, argv)` either way; a program that
   never references `ARGV`/`ARGC`/`gets()` is unaffected).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`,
  `crates/emerald-codegen/src/lib.rs`, `runtime/emerald_runtime.c`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build | `cargo build --workspace` | clean | agent-claimed-locally |
| Test | `cargo test --workspace` | all pass, incl. real fixed-argv `ARGV`/`ARGC` runs and a fixed-stdin `gets()` run | agent-claimed-locally |
| Regression (main signature change) | `cargo test -p emerald-codegen` | every pre-existing compiled-and-run example (`POINT_EXAMPLE`, `ARRAY_EXAMPLE`, `EXCEPTION_EXAMPLE`, `INHERITANCE_EXAMPLE`, etc.) still passes unchanged | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
