---
name: Generic Types (Monomorphized Structs and Classes)
overview: "User-declarable generic classes — `class Box[T]`, `class Box[T: Comparable]` — monomorphized into one specialized LLVM struct layout and method set per distinct concrete type argument actually instantiated, by generalizing plan 41's (function, concrete type) specialization cache and `$$`-mangling convention to (class, concrete type args), reusing plan 41's own bound-checking machinery for `T: Comparable`, and adding the one genuinely new correctness mechanism plan 41 never needed: a bounded instantiation-depth guard against a generic type recursively instantiating itself forever."
maestro:
  mission_id: null
  spec_path: null
  execution_overlay: null
todos:
  - id: leaf-ast-parser-generic-classes
    content: "ClassDef gains type_params: Vec<TypeParam>; TypeParam.bound widens String -> Option<String> (a real, disclosed change to plan 41's already-shipped AST node); grammar gains a general <Ident> \"[\" TypeName-list \"]\" TypeName alternative, distinct from Array[Elem]/Hash[K,V]'s hardcoded-keyword forms"
    status: pending
  - id: leaf-sema-generic-class-templates
    content: "A generic ClassDef is registered as a raw, unresolved template (mirroring GenericFunctionSig's params_raw/return_type_raw) instead of an ordinary ClassInfo; class-body method checking reuses plan 41's Type::Generic/GenericsCtx machinery, generalized from one function's parameter list to a class's fields + every method"
    status: pending
  - id: leaf-sema-instantiation-depth-guard
    content: "A whole-program collection pass finds every concrete Box[Int64]-shaped instantiation actually written; a bounded in-progress instantiation stack (depth 32) detects and rejects unbounded recursive self-instantiation (Box[Box[Box[...]]], or a two-class instantiation cycle) with a real diagnostic, not a hang or a stack overflow"
    status: pending
  - id: leaf-codegen-monomorphized-classes
    content: "One synthesized, fully concrete ClassDef per distinct (class, type args) pair collected above, fed through the existing, unmodified build_class_layout/declare_user_functions/compile_to_object_impl class-codegen path; mangled Stack$$Int64/Stack$$String symbols, zero vtables, zero runtime type tags"
    status: pending
isProject: false
---

# Plan 58 — Generic Types (Monomorphized Structs and Classes)

This is plan 58 of the 58–64 batch — seven independent sibling plans
implementing the answer to a follow-up debate that asked, ignoring
maturity/ecosystem/stability entirely, what Emerald's theoretical
technical ceiling looks like: what would let it out-do the union of
C/C++/Rust/Python/Ruby/TypeScript/Go/Haskell/Elixir combined, without
conceding any of Emerald's identity constraints (no `method_missing`/
`eval`/`send`/reflection, no mixins/open classes/monkey-patching, no
dynamic/virtual dispatch or vtables, no tracing garbage collector, no
runtime reflection). Like plans 17–57 before it, this is post-v1 scope
— it does not touch `plan-of-plans.md`, and that document (plus this
batch's own eventual summary) is updated separately, once, after all
seven of plans 58–64 are authored.

This plan closes the single largest, most-cited gap in plan 41
(`2026-09-09T107000Z-plan-41-interfaces-and-generics.md`)'s own
Decision log, quoted directly: *"Type parameters are legal on top-level
functions only — never on classes/structs... `class Box[T] ... end` is
out of scope: a generic class needs a storage-layout strategy per
instantiation... materially larger than monomorphizing one function
body, and this plan's job is proving the call-site-substitution +
monomorphization mechanism works at all via the smallest complete
example, not building out its full surface."* This plan is that
follow-up: it builds the storage-layout strategy plan 41 explicitly
declined, generalizing the exact mechanism plan 41 proved for
functions to type *declarations*.

Depends on: **plan 41** (interfaces and generics — the specialization
cache, mangling convention, and interface-bound-checking machinery this
plan reuses and generalizes, verified against real, current source
below, not against plan 41's own document text, which predates some
since-evolved details), **plan 08** (object-model — `ClassDef`, field
declarations, `emerald_alloc`-backed instance layout), and **plan 32**
(class-inheritance — this plan makes an explicit, disclosed decision
about generic classes and inheritance; see Decision log).

Concrete proof this plan targets — a user-defined `Stack[T]` with
`push`/`pop`/`peek`, instantiated as both `Stack[Int64]` and
`Stack[String]` in the same program, each independently correct:

```ruby
class Stack[T]
  items: Array[T]
  count: Int64

  def initialize -> Void
    @items = Array.new(8)
    @count = 0
  end

  def push(x: T) -> Void
    @items[@count] = x
    @count = @count + 1
  end

  def pop -> T
    @count = @count - 1
    @items[@count]
  end

  def peek -> T
    @items[@count - 1]
  end
end

s1: Stack[Int64] = Stack.new()
s1.push(10)
s1.push(20)
s1.push(30)
puts s1.pop
puts s1.peek

s2: Stack[String] = Stack.new()
s2.push("first")
s2.push("second")
puts s2.pop
puts s2.peek
```

Expected output: `30\n20\nsecond\nfirst\n`. `s1.pop` decrements
`count` from 3 to 2 and returns `items[2]` (`30`, the last value
pushed — real LIFO proof); `s1.peek` then reads `items[count - 1] =
items[1]` (`20`) without mutating `count` — proving `pop` and `peek`
are genuinely different operations, not aliases. `s2` repeats the same
sequence over `String` values (`"second"` then `"first"`) — real proof
that `Stack$$Int64` and `Stack$$String` (this plan's mangled
specialization symbols) are two independently laid-out, independently
compiled classes, not one generic body reached through a shared
dispatch trick, exactly mirroring the standard plan 41's own
`max$$Money`/`max$$Distance` worked example already set.

## Decision log

- **Plan 41's actual specialization mechanism, re-verified against
  current source rather than plan 41's own document (which predates
  some evolution): a flat cache keyed by `(generic name, concrete
  type)`, one LLVM function per entry, named via `mangled_generic_
  symbol(fn_name, concrete_class) = format!("{fn_name}$${concrete_
  class}")` (`crates/emerald-codegen/src/lib.rs:1157-1159`), populated
  by a whole-program AST walk, `collect_generic_specializations`
  (`lib.rs:1181-1226`), that finds every distinct `(generic_fn_name,
  concrete_class_name)` pair actually called anywhere in the program.
  `declare_user_functions` (`lib.rs:10283-10373`) skips registering any
  LLVM symbol at all for a generic function's own bare name
  (`Item::Function(f) if !f.type_params.is_empty() => {}`, line 10306)
  — only mangled specializations are ever callable symbols.** This
  plan generalizes the identical cache-key shape and mangling
  convention from `(function name, concrete type)` to `(class name,
  concrete type ARGS)` — a mangled class specialization symbol,
  `format!("{}$${}", class_name, type_args.join("$$"))` (`Stack$$
  Int64`, `Stack$$String`), doubles as both the class's synthesized
  `Type::Class(String)` name (sema — `Type::Class` is already just a
  name backed by a separate `ClassInfo` registry, `crates/emerald-
  sema/src/lib.rs:38`, so nothing structurally distinguishes a
  synthesized name from a source-declared one) and the prefix codegen's
  existing `{ClassName}_{method}` method-mangling convention already
  uses (`format!("{}_{}", c.name, mangled_operator_symbol(&m.name))`,
  `lib.rs:10319`, `build_method_call`'s own `format!("{recv_name}_
  {method}")`, `lib.rs:4718`). The payoff of this specific choice: every
  downstream mechanism that already keys off a class's plain name —
  field lookup (`ClassLayout.fields`), method dispatch
  (`{ClassName}_{method}`), `local_classes: HashMap<String, String>`
  variable-to-declared-class tracking — works completely unmodified
  against a monomorphized generic class, exactly as it already does
  against `Point`/`Money`/`Dog`. This plan adds a new name-*synthesis*
  step; it adds zero new dispatch paths.
- **Bounded generic classes reuse plan 41's real bound-checking
  machinery directly — not a parallel mechanism.** `class Box[T:
  Comparable]`'s conformance check is the exact same predicate plan 41
  already implements for a generic function's call-site check: `class_
  info.implements.as_deref() != Some(g.bound.as_str())`
  (`emerald-sema/src/lib.rs:1471`), reading `ClassInfo.implements`
  (`lib.rs:187`), itself populated from a class's `implements Comparable`
  declaration (`lib.rs:601`, `4734`) and validated at registration time
  by `check_interface_conformance` against the `interfaces: HashMap<
  String, InterfaceInfo>` registry (`lib.rs:219-224`, `4832`). This
  plan's instantiation step (leaf-sema-instantiation-depth-guard) runs
  the identical `implements == Some(bound)` check against each concrete
  type argument actually supplied, at the point a concrete
  instantiation (`Box[Money]`) is collected — the only change is *when*
  the check runs (instantiation time, not a function call site), never
  a second, differently-shaped bound predicate.
- **One AST-level widening is unavoidable, and it's a real, disclosed
  change to plan 41's already-shipped `TypeParam`/`Type::Generic`, not
  new plan-58-only types: `TypeParam.bound` widens from a mandatory
  `String` to `Option<String>`, and `Type::Generic`'s own bound field
  (`emerald-sema/src/lib.rs:64`, currently `Generic(String, String)`)
  widens to `Generic(String, Option<String>)`.** Verified this session:
  `crates/emerald-parser/src/ast.rs:513-517`'s `TypeParam { name:
  String, bound: String }` and `grammar.lalrpop`'s `TypeParam: TypeParam
  = { <name:Ident> ":" <bound:Ident> => TypeParam { name, bound } }`
  make a bound *syntactically* mandatory today — there is no bare-`T`
  alternative. Plan 41 never needed an unbounded type parameter (its
  own Decision log locks "no unbounded generics... `TypeParam`'s
  grammar... makes an unbounded parameter a parse error today rather
  than a silently-accepted-but-unchecked one"), but this plan's own
  primary example, `class Box[T]`/`class Stack[T]`, has no bound at all
  — the task's own headline case. Rather than inventing a second,
  class-only type-parameter shape, this plan widens the one `TypeParam`
  struct plan 41 already shipped (`bound: Option<String>`, a new
  bound-less grammar alternative, `<name:Ident> => TypeParam { name,
  bound: None }`) and requires every *function*-side reader of
  `type_param.bound` (`GenericFunctionSig.bound: String` stays
  unchanged — a top-level generic function's registration continues to
  reject a `None` bound with the existing "generic functions require a
  bound" diagnostic, now enforced by sema rather than structurally by
  the grammar) to keep behaving exactly as before. This is the same
  "grammar admits a superset, sema narrows with a real diagnostic"
  discipline this codebase already applies elsewhere (e.g. `resolve_
  chain`'s rejection of inheriting from a module) — not a shortcut, a
  disclosed, source-compatible widening of shared code, verified
  against every real current call site of `TypeParam.bound`/`Type::
  Generic` in `emerald-sema/src/lib.rs` (lines 64, 235, 1471, 1860-1928)
  before committing to it.
- **A generic class's field types and method signatures stay raw,
  unresolved strings in a template registry until a concrete
  instantiation is requested — mirroring `GenericFunctionSig.params_
  raw`/`return_type_raw` (`lib.rs:236-238`) exactly, for the identical
  reason: `resolve_type(name, classes)` (`lib.rs:254`) cannot resolve a
  bare `"T"`, and a generic class's own `ClassDef` is never run through
  `build_flattened_class_info` (`lib.rs:546`) at registration time.** A
  new `generic_classes: HashMap<String, &ClassDef>` registry, populated
  by the same registration pass that already builds `classes` (`check_
  program`, `lib.rs:4825`), skips `Item::Class(c)` whenever `!c.type_
  params.is_empty()` — the exact mirror of `function_signature`'s
  existing `if !f.type_params.is_empty() { ... }` short-circuit
  (`lib.rs:463`) that already keeps a generic function's bare name out
  of the ordinary `sigs` registry.
- **A generic class's own field types and method bodies are type-
  checked exactly ONCE, generically — not once per concrete
  instantiation.** This is the same choice plan 41 already made for a
  generic function's body ("checked exactly once, statically, before
  any concrete type is known — this is what makes it a real generic
  rather than duck-typed re-checking per call") and this plan makes it
  for the identical reason: re-type-checking `Stack[T]`'s `push`/`pop`/
  `peek` bodies separately for every concrete `T` actually used would
  be duck-typed re-checking wearing a generics costume, and would let a
  type error specific to one instantiation slip through if that
  instantiation happened not to be exercised by whatever the compiler
  chose to check. This plan reuses plan 41's own `GenericsCtx`/`Type::
  Generic`/`check_generic_function_body`-style machinery
  (`lib.rs:246-249`, `1860-1928`) directly, generalized from "one
  function's parameter list" to "a class's fields plus every method
  body": every field whose raw type string exactly equals a type
  parameter name resolves to `Type::Generic(name, bound)`; a method
  call on a `Type::Generic`-typed value (`t.compare_to(other)` inside a
  `Box[T: Comparable]` method) resolves against `interfaces[bound]`'s
  required signature the same way plan 41's `infer_expr_type` already
  does at line 1869; a method call on an *unbounded* `Type::Generic`
  (`bound: None`) is rejected outright — there is no interface to
  resolve the call against — with a real diagnostic ("cannot call a
  method on unbounded type parameter `T` — declare a bound, e.g.
  `Stack[T: Comparable]`, to call methods on values of type `T`"). This
  is exactly why `Stack[T]`'s own methods only ever *store* and
  *return* `T`-typed values, never call a method on one — a real,
  disclosed consequence of this design, not an accidental restriction
  of the worked example.
- **`Stack.new()` (a bare, `Ident`-headed constructor call with no
  explicit type-argument syntax at the call site — `grammar.lalrpop`'s
  `<recv:Ident> "." "new" "(" <args:Args> ")"` production, `recv` is
  always a plain `Ident`, never a compound `TypeName`) resolves its
  concrete instantiation from the SAME expected-type-providing
  positions plan 53 already carved out for `Ok`/`Err` construction — a
  real, disclosed narrowing, not a new ad hoc rule.** Plan 53's
  Decision log restricts `Ok(value)`/`Err(value)` construction to "the
  three expected-type-providing positions (`Let`/`Assign`/`Return`)"
  precisely because `infer_expr_type` carries no expected-type
  parameter anywhere in this compiler — `Expr::New("Stack", [])` has
  the identical problem: nothing about the bare name `"Stack"` alone
  identifies which concrete specialization is meant, since `Stack`
  itself is a template, not a type. `s1: Stack[Int64] = Stack.new()`
  supplies the concrete instantiation from the `Let`'s own declared
  type annotation; `Stack.new()` written anywhere else — a bare
  expression statement, a function-call argument, a field initializer
  with no adjacent annotation — has no expected type to draw the
  instantiation from and is rejected with a diagnostic naming the
  missing context, the identical shape of restriction `Ok`/`Err`
  already normalized for this codebase's callers.
- **Recursive/unbounded monomorphization is a real, concrete hazard,
  and it does not look like an infinite loop over infinitely long
  source text — it looks like ONE finite class declaration whose own
  field references a strictly larger instantiation of itself.** The
  danger case is not a user literally writing `Box[Box[Box[...]]]]`
  (impossible — source text is finite); it's:
  ```ruby
  class Box[T]
    value: T
    wrapped: Box[Box[T]]
  end
  ```
  Instantiating `Box[Int64]` requires resolving its `wrapped` field's
  type, `Box[Box[Int64]]` — a *different*, strictly larger
  instantiation than the one currently being resolved — which in turn
  requires resolving `Box[Box[Box[Int64]]]`, forever. A plain
  "already-visited" set (the mechanism `resolve_class_chain` already
  uses for inheritance-cycle detection, `emerald-codegen/src/
  lib.rs:328-352`, and its sema-side mirror) does **not** catch this:
  every step in the chain is a *genuinely new* `(class, type args)`
  key that was never seen before, so a same-key "already visited" check
  never fires — the pathological case is unbounded *growth*, not
  repetition. This plan's detection mechanism is therefore a **bounded
  instantiation-depth counter over an in-progress instantiation stack**
  (`in_progress: Vec<String>` of mangled names currently mid-
  resolution, threaded through `instantiate_generic_class`): pushed
  before resolving a specialization's fields/methods, popped after.
  Two rules, not one, because they answer two different questions:
  (1) if the mangled name being requested is *already* on the stack
  (an exact repeat — e.g. `Node[T]`'s own `next: Node[T]` field,
  requesting the identical `(class, args)` pair mid-resolution of
  itself), this is a completely ordinary, terminating self-reference —
  every field is a fixed 8-byte slot regardless of kind (`FieldInfo`,
  `emerald-codegen/src/lib.rs:226-229`; a self-referential field is
  just a `Ptr`-kind slot, exactly how an ordinary non-generic class's
  own self-reference, e.g. a linked-list `Node`, already compiles today
  with no special-casing) — so it short-circuits by returning `Type::
  Class(mangled)` directly against a *placeholder* `ClassInfo` entry
  inserted into `classes` before recursing into field resolution
  (insert-then-fill, not resolve-then-insert), never re-entering
  resolution; (2) if `in_progress.len()` exceeds a fixed bound (32 —
  small and deliberately conservative, since a real, well-formed
  generic type's instantiation graph in this project's own examples
  never approaches single digits of depth) *without* the repeat
  case ever firing — meaning every step really is a new, distinct,
  ever-larger key — instantiation is rejected with a diagnostic naming
  the class and the runaway chain ("generic type `Box` requires
  unbounded recursive instantiation (exceeded depth 32) while
  resolving `wrapped: Box[Box[T]]` — likely a self-referential generic
  field or method signature"), not a stack overflow and not a silent
  hang. The identical mechanism also catches a *cross-type* cycle
  (`class A[T] ... b: B[A[T]] ... end` / `class B[T] ... a: A[B[T]] ...
  end`) for the same reason: each step is still a new, growing key, so
  it still trips the depth bound, without needing a separate graph-
  cycle algorithm to recognize the two-class shape specifically.
  Codegen's own collection pass (leaf-codegen-monomorphized-classes)
  additionally carries the same bound defensively — codegen runs only
  on already-sema-checked input (the same contract every prior codegen
  leaf states, e.g. plan 41's leaf-codegen-monomorphization's own
  Dependencies note), so this is belt-and-suspenders, not the
  authoritative check.
- **Generic classes decline single inheritance in v1 — a generic class
  can be neither a superclass nor a subclass — a real, disclosed
  restriction, not an oversight.** Plan 32's inheritance machinery
  (`resolve_class_chain`, `build_class_layout`, `build_method_owners`,
  `emerald-codegen/src/lib.rs:328-412`) walks a `ClassDef.superclass`
  chain directly over `class_defs: HashMap<String, &ClassDef>` — every
  entry in that map is a real, source-declared `ClassDef` straight out
  of `program.items`. A monomorphized specialization (`Stack$$Int64`)
  has no such entry: it exists only as a *synthesized* `ClassDef` this
  plan's own codegen leaf builds on the fly (see below), never inserted
  into `program.items`. Making a specialization a legitimate
  inheritance participant — either `class Dog < Stack[Int64]` (a
  concrete instantiation as an ancestor) or a generic class itself
  declaring `class Box[T] < SomeClass` — would require synthesizing a
  real `ClassDef` for every specialization and *re-inserting* it back
  into whatever list codegen's inheritance walk consults, plus deciding
  what `SuperclassClause`'s grammar even means when the named
  superclass is itself a compound `TypeName` (a strictly larger,
  separately-motivated grammar change) rather than a bare `Ident`. None
  of this plan's own worked examples need it, and plan 32's Decision
  log already established the precedent of declining a materially
  bigger feature (real dynamic dispatch) when nothing in its own scope
  needed it. This plan does the same for generic-class inheritance:
  declined for v1, real future work, not silently assumed away.
- **Variance annotations (covariance/contravariance on type parameters
  — e.g. `class Box[out T]`/`class Sink[in T]`) are explicitly declined,
  real future work.** This project's existing type-compatibility checks
  are uniformly exact-match, never subtyping-aware (plan 32's Decision
  log makes the identical observation about override signatures: "every
  existing type-compatibility check in `emerald-sema` is exact-match,
  not subtyping-aware — matches that existing house style"); variance
  is a meaningfully different, larger feature (it changes what
  `is_assignable`, `lib.rs:442`, means for two *different* concrete
  instantiations of the same generic class, e.g. whether `Stack[Dog]`
  is assignable to a `Stack[Animal]`-typed variable) with no example in
  this plan's own scope that needs it.
- **This plan does not retroactively rewrite `Array[T]`/`Hash[K,V]`/
  `Pair[K,V]`/`Result[T,E]` as ordinary user-level generic classes,
  though it observes, as forward-looking commentary only, that this
  mechanism could in principle express all four.** Verified this
  session: `Type::Pair`'s own doc comment already states this precisely
  — "a hand-rolled, hard-coded compound type, the same way `Array[T]`/
  `Hash[K,V]` themselves already exist rather than a user-declarable
  generic class (plan 41's own contract covers generic *functions* and
  single-class `implements`, never generic *classes*)" (`emerald-sema/
  src/lib.rs:106-110`); `resolve_type`'s `Array[Elem]`/`Hash[K,V]`/
  `Pair[K,V]`/`Result[T,E]` branches (`lib.rs:304-351`) are each a
  hand-written, literal-reserved-keyword grammar/sema pair (plan 09's
  own Decision log, quoted by plan 41: "a real `Type` AST node... is
  deferred until a second compound-type shape... makes the string-
  parsing approach genuinely awkward"). Now that a second, general
  compound-type shape exists (this plan's own `<Ident> "[" ... "]"`
  `TypeName` alternative), rewriting those four as ordinary uses of
  this plan's own mechanism is a real, coherent piece of *future*
  cleanup — collapsing four hand-maintained hardcoded paths into one
  general one — but it touches four already-shipped, heavily-depended-
  on compiler paths (plans 09, 25, 42, 53) each with their own runtime
  representation quirks (`Hash[K,V]`'s `Int64`-key-only linear scan,
  `Result[T,E]`'s `?`-operator interaction), and doing it is separate,
  disclosed, out-of-scope work this plan does not attempt.
- **Nested generic-type arguments (`Box[Array[Int64]]`, or the
  self-referential `Box[Box[T]]` this plan's own hazard example needs)
  are syntactically real — this plan's new `TypeName` alternative
  accepts a full recursive `TypeName` per type argument, not the
  narrower bare-`Ident`-only convention `Array[Elem]`/`Hash[K,V]`/
  `Pair[K,V]`/`Result[T,E]` already use (`grammar.lalrpop:1182-1194`,
  each literally `<elem:Ident>`, never `<elem:TypeName>`).** This is a
  deliberate departure from the existing narrower convention, not an
  oversight: if this plan's own grammar restricted type arguments to a
  bare `Ident` the way the four hardcoded forms do, `Box[Box[Int64]]`
  would not parse at all, and the recursive-instantiation hazard this
  plan's own Decision log calls "not optional... a real hazard" would
  have no way to reach sema from real source text in the first place —
  a bare-`Ident` restriction here would silently hide the problem
  instead of solving it, not a neutral simplification.
- **Tooling integration, named only where genuinely affected.** Plan
  21's (`2026-09-08T220500Z-plan-21-lsp-symbols-and-navigation.md`)
  go-to-definition is a `\b(?:def|class|module)\s+NAME\b` regex search
  over top-level declarations — `class Stack[T]` still matches `class
  Stack` under that same pattern (the `[T]` clause trails the matched
  name), so this plan needs no LSP change at all, disclosed here rather
  than left to be independently rediscovered. Plan 24's
  (`2026-09-08T223500Z-plan-24-tree-sitter-grammar.md`) tree-sitter
  grammar already flagged its own terminal inventory as needing
  revisiting once sibling batches landed new syntax; this plan adds one
  more real, disclosed revisit: a `class_declaration` rule needs the
  same optional type-parameter-list production plan 24 will already
  need for `function_declaration` (plan 41's own `[T: Bound]` clause),
  not a separate mechanism. Plan 26's
  (`2026-09-08T225500Z-plan-26-parser-error-recovery.md`) `Item`-
  boundary `!`-recovery already resynchronizes past a malformed `class
  Box[` declaration the same way it resynchronizes past any other
  malformed top-level item — no change needed. Plan 35's
  (`2026-09-09T101000Z-plan-35-debug-info.md`) v1 scope is line-table
  debug info only, explicitly *not* per-type DIEs ("this plan proves 'a
  debugger can tell you which source line you're on,' not 'a debugger
  can inspect your variables'") — a monomorphized specialization's
  per-instruction `Spanned<T>` byte-offset spans are identical to the
  shared generic template's own spans regardless of which concrete
  type instantiated it, so this plan adds no new debug-info surface for
  plan 35 to handle. Plan 46's
  (`2026-09-09T112000Z-plan-46-package-manager-and-build.md`) `require`-
  based multi-file resolution already flattens a whole workspace into
  one `Program` before codegen runs (plan 23's design, which plan 46
  builds on unchanged) — this plan's whole-program collection passes
  (sema and codegen alike) need nothing beyond that already-flattened
  `Program`, so a generic class declared in one package and
  instantiated in a dependent package works the same way plan 41's own
  generic functions already do across a `require` boundary, with no
  new cross-package resolution logic. Plan 47's
  (`2026-09-09T113000Z-plan-47-repl-and-test-framework.md`) REPL is
  subprocess-per-line against the *whole accumulated session text*
  recompiled fresh each line (not an incremental single-line compile) —
  this is exactly what makes a generic class declared on one REPL line
  and instantiated on a later one work at all under this plan's
  whole-program monomorphization pass; a naive incremental-compile REPL
  design would have broken this, and plan 47's actual chosen design
  already avoids that trap for unrelated reasons.

## Leaf: leaf-ast-parser-generic-classes

### 1. Context
- Why: no AST shape exists for a class's own type-parameter list, and
  `TypeParam.bound`'s current mandatory `String` (`ast.rs:513-517`)
  cannot represent `class Box[T]`'s unbounded parameter — verified this
  session against both `ast.rs` and `grammar.lalrpop`'s `TypeParam`
  production, and against `TypeName`'s current compound-string
  alternatives (`grammar.lalrpop:1180-1207`), none of which admit a
  general `<Ident> "[" ... "]"` shape headed by a non-reserved name.
- Target state: `ClassDef` (`ast.rs:542-552`) gains `pub type_params:
  Vec<TypeParam>` (empty `Vec` for every class that doesn't declare
  one — additive, source-compatible, matching `Function.type_params`'s
  own precedent at `ast.rs:507`). `TypeParam` (`ast.rs:513-517`) widens
  `bound: String` to `bound: Option<String>`. `grammar.lalrpop`'s
  `ClassDef` production gains `<type_params:TypeParamClause?>`
  immediately after `<name:Ident>` (before `SuperclassClause?`),
  reusing `TypeParamClause`/`TypeParamList` (`grammar.lalrpop:1140-
  1155`, unchanged) verbatim; `TypeParam`'s own production gains a
  bound-less alternative, `<name:Ident> => TypeParam { name, bound:
  None }`, alongside its existing `<name:Ident> ":" <bound:Ident> =>
  TypeParam { name, bound: Some(bound) }`. `TypeName` gains one new
  alternative after its four existing hardcoded compound forms:
  `<name:Ident> "[" <args:GenericTypeArgs> "]" => format!("{name}[{}]",
  args.join(", "))`, with a new `GenericTypeArgs: Vec<String>`
  production accepting one-or-more comma-separated `TypeName`s
  (recursive, not bare-`Ident`-restricted — see Decision log).
- Dependencies: none (purely additive to `ast.rs`/`grammar.lalrpop`,
  plus the disclosed `TypeParam.bound` widening — every existing
  caller of `TypeParam { name, bound }`/`.bound` in `grammar.lalrpop`
  and `emerald-sema/src/lib.rs` updated to the `Some(...)`/`Option`-
  aware shape as part of this leaf, verified via `cargo build
  --workspace`, not assumed).

### 2. Acceptance Criteria
1. `class Box[T]` parses with `ClassDef.type_params == vec![TypeParam {
   name: "T".to_string(), bound: None }]`.
2. `class Box[T: Comparable]` parses with `bound: Some("Comparable"
   .to_string())`.
3. This plan's own worked `Stack[T]` example parses end to end,
   including `items: Array[T]` (`TypeName == "Array[T]"`, unchanged
   compound-string shape — `T` resolves the same way any other `Array`
   element name does at this grammar layer) and the `s1: Stack[Int64]`/
   `s2: Stack[String]` `Let` annotations (`TypeName == "Stack[Int64]"`/
   `"Stack[String]"`).
4. `Box[Box[Int64]]` parses as a single `TypeName` string,
   `"Box[Box[Int64]]"` — a parser-level AST-shape test proving type
   arguments recurse (see Decision log's nested-argument note).
5. `class NoBound[T](x: T)` (malformed — a paren-parameter-list
   immediately after a class name with no intervening field/method
   shape) is a real parse error, not a panic; a class with zero type
   parameters (`class Point ... end`, unchanged) still parses with
   `type_params: vec![]`.
6. No LALRPOP build-time conflicts introduced by the new `TypeName`
   alternative or by `TypeParamClause?` appearing in `ClassDef`
   (verified at build time, not assumed — same bar as every prior
   grammar-touching plan).
7. Regression: every prior plan's example (`hello.em`, `Point`,
   `classes.em`, `interfaces_generics.em`, the inheritance/field-access-
   sugar/actor examples) still parses identically; every existing
   `TypeParam`/`Type::Generic` construction site in `emerald-sema`
   still compiles against the widened `Option<String>` bound with
   unchanged accept/reject behavior for every plan-41 example (a
   top-level generic function with no bound is still rejected, now by
   sema instead of the grammar).

### 3. File & Module Structure
- **Modify:** `crates/emerald-parser/src/ast.rs`,
  `crates/emerald-parser/src/grammar.lalrpop`,
  `crates/emerald-parser/src/lib.rs` (tests)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Build (no LALR conflicts) | `cargo build -p emerald-parser` | clean | agent-claimed-locally |
| Test | `cargo test -p emerald-parser` | all pass, incl. the 5 new AST-shape cases above | agent-claimed-locally |
| Workspace (widened `TypeParam.bound` compiles everywhere) | `cargo build --workspace` | clean | agent-claimed-locally |

---

## Leaf: leaf-sema-generic-class-templates

### 1. Context
- Why: `emerald-sema` has no concept of a class carrying its own type
  parameters; `check_program`'s class-registration pass (`lib.rs:4825`
  onward) runs every `Item::Class` through `build_flattened_class_info`
  (`lib.rs:546`), which calls `resolve_type` on every field/method type
  string unconditionally — this would fail immediately on a bare `"T"`.
- Target state: a new `generic_classes: HashMap<String, &ClassDef>`
  registry (mirroring `generic_sigs`'s registration split, `lib.rs:
  463`/`4795`), populated instead of `classes` for any `Item::Class(c)`
  with `!c.type_params.is_empty()`. A new `check_generic_class_body`
  entry point (structurally parallel to plan 41's `check_generic_
  function_body`) type-checks the class's fields and every method body
  exactly once, generically: builds a `self`-fields environment mapping
  each field whose raw type string exactly equals a type-parameter name
  to `Type::Generic(name, bound)` (bound now `Option<String>` per
  leaf-ast-parser-generic-classes) and every other field via the
  ordinary `resolve_type`; threads the same `GenericsCtx`-shaped context
  plan 41 already built (`lib.rs:246-249`) through each method body's
  own `check_stmt`/`infer_expr_type` calls. A `Type::Generic`-typed
  receiver's method call resolves against `interfaces[bound]` exactly
  as plan 41's existing `infer_expr_type` arm already does
  (`lib.rs:1869-1894`) when `bound.is_some()`; when `bound.is_none()`,
  the call is rejected with a real diagnostic naming the missing bound
  (see Decision log).

### 2. Acceptance Criteria
1. This plan's `Stack[T]` template type-checks `Ok(())` exactly once
   (not once per instantiation) — `push`/`pop`/`peek`'s bodies all
   type-check against `Type::Generic("T", None)`, never against a
   concrete `Int64`/`String`.
2. A hypothetical `class Box[T: Comparable] ... def biggest(a: T, b: T)
   -> T; if a.compare_to(b) >= 0 then a else b end; end ... end`
   type-checks `Ok(())`, with `a.compare_to(b)` resolved via
   `interfaces["Comparable"]`'s required signature — real proof the
   bounded-generic-class body-check path is reachable and correct, not
   just the unbounded `Stack[T]` path.
3. A hypothetical `class Box[T] ... def show -> Void; puts @value.
   to_s; end ... end` (a method call on an *unbounded* `T`-typed value)
   is rejected with a diagnostic naming the missing bound — real proof
   the `bound: None` rejection path fires, not silently accepted.
4. `class Stack[T]`'s two instantiation call sites (`Stack[Int64]`,
   `Stack[String]`) are NOT independently re-type-checked against the
   template body — verified by asserting `check_generic_class_body`
   is invoked exactly once per generic class declaration regardless of
   how many concrete instantiations exist in the program.
5. Regression: every prior plan's sema test suite passes unmodified — a
   non-generic `Item::Class` never touches `generic_classes` or the new
   generic-body-check path (both empty/unreachable for `type_params:
   vec![]`).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 4 new cases above | agent-claimed-locally |

---

## Leaf: leaf-sema-instantiation-depth-guard

### 1. Context
- Why: nothing resolves a compound `"Stack[Int64]"`-shaped `TypeName`
  string into a real, concrete `ClassInfo` today — `resolve_type`
  (`lib.rs:254-361`) has branches for `Array[Elem]`/`Hash[K,V]`/
  `Pair[K,V]`/`Result[T,E]` only, each hardcoded, and falls through to
  `unknown type` for anything else; and nothing guards against the
  recursive-instantiation hazard the Decision log describes concretely
  above.
- Target state: a whole-program collection pass, `collect_generic_
  class_instantiations(program, generic_classes) -> Vec<(String,
  Vec<String>)>` (generic class name, concrete type-arg strings),
  walking every `TypeName`-producing position in source order — `Let`
  annotations, field/parameter/return-type strings — for a substring
  shaped `<base>[<args>]` where `<base>` is a known key of `generic_
  classes`, reusing the existing `starts_with`/`ends_with` + `split_
  top_level_commas` (`lib.rs:392-409`, already handles nested `[`/`(`
  depth correctly) parsing convention `resolve_type` already
  established for `Array`/`Hash`/`Pair`/`Result`, generalized from a
  fixed keyword prefix to any registered generic class name. For each
  distinct pair, `instantiate_generic_class(mangled_name, base_name,
  type_args, generic_classes, classes: &mut HashMap<String, ClassInfo>,
  in_progress: &mut Vec<String>) -> Result<(), Diagnostic>`:
  1. If `mangled_name` is already a key of `classes`, return `Ok(())`
     immediately (memoized — already instantiated, possibly by an
     earlier call site).
  2. If `mangled_name` is already on `in_progress`, insert a
     placeholder `ClassInfo` (empty `fields`/`methods`, real name) if
     not already present and return `Ok(())` — the same-key self-
     reference short-circuit (Decision log).
  3. If `in_progress.len() >= 32`, return `Err(Diagnostic)` naming
     `base_name` and the current `in_progress` chain — the depth-bound
     rejection (Decision log).
  4. Otherwise: check each `type_params[i].bound` against `type_
     args[i]` via the existing `implements == Some(bound)` predicate
     (`lib.rs:1471`'s exact check, relocated); push `mangled_name` onto
     `in_progress`; substitute every occurrence of each type-parameter
     name in the template's raw field/method type strings with the
     corresponding `type_args[i]` string (a whole-token replace, not a
     substring replace — `"T"` inside `"Total"` must never match);
     recursively resolve each substituted field/parameter/return type
     via `resolve_type`, which itself may recurse into `instantiate_
     generic_class` again for a nested generic reference (`Box[Box[
     Int64]]`'s inner `Box[Int64]`); pop `in_progress`; insert the
     completed `ClassInfo` under `mangled_name`.

### 2. Acceptance Criteria
1. This plan's own worked `Stack[Int64]`/`Stack[String]` example
   type-checks `Ok(())`, with both mangled `ClassInfo` entries present
   in the final `classes` registry under `"Stack$$Int64"`/`"Stack$$
   String"`.
2. `class Box[T] ... value: T; wrapped: Box[Box[T]]; end` (this plan's
   own hazard example), instantiated anywhere (`b: Box[Int64] = ...`),
   is rejected with a diagnostic naming `Box` and the depth bound —
   real, executed proof of the depth-guard, not just its presence in
   the Decision log.
3. The two-class cyclic case (`class A[T] ... b: B[A[T]] ... end` /
   `class B[T] ... a: A[B[T]] ... end`) is likewise rejected via the
   same depth bound when either is ever instantiated.
4. A legitimate same-key self-reference (`class Node[T] ... next:
   Node[T]; value: T; end`, instantiated as `Node[Int64]`) type-checks
   `Ok(())` — real proof the depth guard does not reject ordinary,
   terminating self-referential generic types (linked-list shaped),
   only genuinely unbounded ones.
5. `class Box[T: Comparable]` instantiated with a concrete type that
   does not implement `Comparable` (or declares no `implements` at
   all) is rejected with a diagnostic naming the missing requirement —
   the bound-check reuse from leaf-sema-generic-class-templates,
   exercised at instantiation time.
6. `Stack[NotAClass]` (an undefined concrete type argument) is rejected
   with a diagnostic, not a panic.
7. `Stack.new()` written with no adjacent expected-type-providing
   position (a bare expression statement) is rejected with a
   diagnostic naming the missing context (see Decision log).
8. Regression: every prior plan's sema test suite passes unmodified —
   a program with no generic-class instantiation never invokes this
   leaf's collection pass at all (empty `generic_classes` short-
   circuits immediately).

### 3. File & Module Structure
- **Modify:** `crates/emerald-sema/src/lib.rs`
- Dependencies: `leaf-sema-generic-class-templates` (the template
  registry and generic-body-check pass this leaf's instantiation step
  reads from and validates against).

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-sema` | all pass, incl. the 6 new diagnostic cases + 2 real-accept cases above | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

---

## Leaf: leaf-codegen-monomorphized-classes

### 1. Context
- Why: nothing compiles a generic class or a compound `"Stack[Int64]"`
  type reference to machine code — `declare_user_functions`
  (`lib.rs:10283-10373`) has no arm for a `type_params`-non-empty
  `Item::Class`, and `build_class_layout`/`resolve_class_chain`
  (`lib.rs:328-388`) operate on real, source-declared `ClassDef`s from
  `class_defs: HashMap<String, &ClassDef>` only.
- Target state: `declare_user_functions`'s `Item::Class(c)` arm gains a
  guard skipping any `c` with `!c.type_params.is_empty()` (mirroring
  the existing `Item::Function(f) if !f.type_params.is_empty() => {}`
  at line 10306). A new pass, `collect_generic_class_specializations
  (program, generic_classes: &HashMap<String, &ClassDef>) -> HashMap<
  String, HashSet<Vec<String>>>` (class name → every distinct concrete
  type-arg tuple actually instantiated anywhere in the whole program),
  implemented as the same style of AST walk as `collect_generic_
  specializations` (`lib.rs:1181-1226`) and this plan's own sema-side
  `collect_generic_class_instantiations` — codegen independently
  re-derives this from the raw AST rather than reusing sema's already-
  computed result, the same "no shared sema→codegen structure"
  architecture plan 32's Decision log already establishes as this
  backend's standing convention. For every distinct `(class_name,
  type_args)` pair: synthesize a fully concrete `ClassDef { name:
  mangled, superclass: None, implements: None, type_params: vec![],
  fields: <substituted>, methods: <substituted> }` by the identical
  whole-token substitution leaf-sema-instantiation-depth-guard already
  performs (re-derived independently here, per the same architecture
  note), then feed that synthesized `ClassDef` through the *existing*,
  unmodified `build_class_layout`, the *existing* `Item::Class` method-
  declaration arm (`lib.rs:10313-10323`), and the *existing* class-
  method-body codegen path in `compile_to_object_impl`
  (`lib.rs:11065-11080` region) — 100% reuse of ordinary, non-generic
  class codegen, exactly matching plan 41's own stated reuse philosophy
  for functions ("reuses that exact shape... runs the existing,
  unmodified single-function codegen path"). Each field/method symbol
  is registered under the mangled prefix (`Stack$$Int64_push`,
  `Stack$$Int64_initialize`, ...); a call site referencing `Stack[
  Int64]`'s local (`local_classes` tracking the mangled name, exactly
  as it already tracks any other declared class name) resolves to the
  matching mangled symbol and emits an ordinary direct LLVM `call` —
  identical call-emission code to every non-generic method call today.
  A defensive depth bound (32, matching leaf-sema-instantiation-depth-
  guard) guards this pass too, per the Decision log's belt-and-
  suspenders note.
- Dependencies: `leaf-sema-instantiation-depth-guard`,
  `leaf-sema-generic-class-templates` (codegen runs on already-checked
  input — every instantiation codegen attempts here has already been
  proven type-safe, bound-conformant, and depth-bounded by sema,
  matching every prior codegen leaf's stated contract, e.g. plan 41's
  leaf-codegen-monomorphization's own Dependencies note).

### 2. Acceptance Criteria
1. This plan's full worked `Stack[Int64]`/`Stack[String]` example,
   compiled, linked, and run, prints exactly `30\n20\nsecond\nfirst\n`
   — real executed proof both specializations are independently
   correct (see the worked example's own walkthrough above).
2. Exactly two specialized class layouts and their full method sets are
   emitted (`Stack$$Int64`/`Stack$$String`, each with `_initialize`/
   `_push`/`_pop`/`_peek`), and no bare `Stack`-prefixed symbol (e.g.
   `Stack_push`) is ever emitted — checkable directly from the exact
   output above, since `Stack$$Int64_push` and `Stack$$String_push`
   operate over genuinely different element representations (an
   `Int64` value vs. a `String` pointer packed into the same 8-byte
   `Array` element slot) and must each independently produce the
   correct result for that to be possible at all.
3. Every method call on a generic-class instance compiles to a direct
   LLVM `call` instruction against a statically-resolved, name-mangled
   symbol — no indirect call and no runtime type tag read anywhere in
   the emitted code for this feature, matching plan 41's Decision log's
   explicit vtable rejection, generalized to type declarations.
4. An unsupported/malformed shape reaching codegen directly (e.g. a
   concrete instantiation sema should already have rejected) defensively
   returns a descriptive `Err`, not a panic — same AC standard as every
   prior codegen plan (plan 08 AC4, plan 32 AC2, plan 41 AC4).

### 3. File & Module Structure
- **Modify:** `crates/emerald-codegen/src/lib.rs`
- **Create:** `examples/generic_classes.em` (this plan's worked `Stack[
  T]` example, matching the project's established `examples/*.em` +
  `emerald-cli/tests/examples.rs` compiled-and-run test convention)

### 4. Quality Gates
| Gate | Command | Pass | Witness level |
|------|---------|------|---------------|
| Test | `cargo test -p emerald-codegen` | all pass, incl. real linked-and-run `30\n20\nsecond\nfirst\n` | agent-claimed-locally |
| Workspace | `cargo test --workspace` | all pass | agent-claimed-locally |

## Total quality gate
```bash
cargo build --workspace
cargo test --workspace
```
