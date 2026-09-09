/// Plan 22's Decision log: wraps every `Expr`/`Stmt` *position* with the
/// real byte-offset span LALRPOP's own `@L`/`@R` markers compute at
/// parse time — an outer wrapper around each node rather than a `span`
/// field bolted onto every enum variant, so `infer_expr_type`/
/// `check_stmt`/`build_expr`/`build_stmt` change their *signatures*
/// (taking `&Spanned<Expr>`/`&Spanned<Stmt>`) but not their match arms'
/// binding patterns. `Deref`/`DerefMut` to `T` keep `.node`-holding
/// field access ergonomic at the (many) call sites that only ever
/// wanted the node, never the span.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
  pub span: (usize, usize),
  pub node: T,
}

/// Deliberately `node`-only, not `#[derive(PartialEq)]` — a real gap
/// found implementing this plan, not the Decision log's original
/// assumption: comments are skipped as lexer whitespace (`match {}`'s
/// `r"#[^\n]*" => {}`), so `@L`/`@R` positions still shift by exactly a
/// leading comment's length even though the parsed `Expr`/`Stmt` shape
/// is identical either way. `trailing_and_leading_comments_dont_change_
/// the_ast` (this crate's own existing regression test, predating this
/// plan) asserts exactly that: two parses of comment-differing source
/// produce an equal AST — comparing spans too would make that
/// assertion fail on the equality it's specifically testing for.
/// Comparing `node` only keeps `assert_eq!` a real structural check
/// (this AST shape vs. that one) without conflating it with a
/// byte-offset check no test in this codebase actually wants from
/// `==` — a real span is still verified directly via `.span` wherever
/// a test's whole point *is* the position (see `leaf-ast-spans`' own
/// concrete-proof test).
impl<T: PartialEq> PartialEq for Spanned<T> {
  fn eq(&self, other: &Self) -> bool {
    self.node == other.node
  }
}

impl<T> std::ops::Deref for Spanned<T> {
  type Target = T;
  fn deref(&self) -> &T {
    &self.node
  }
}

impl<T> std::ops::DerefMut for Spanned<T> {
  fn deref_mut(&mut self) -> &mut T {
    &mut self.node
  }
}

impl<T> Spanned<T> {
  /// A synthetic node with no real source position — used only where
  /// codegen/rewrite passes construct a brand-new `Expr`/`Stmt` that
  /// was never actually written in source (e.g. the assert/assert_eq
  /// desugaring pass's synthesized `if`/`raise`).
  pub fn synthetic(node: T) -> Self {
    Spanned { span: (0, 0), node }
  }
}

/// Lets a test compare a real, parsed `Spanned<T>` directly against a
/// bare `T` literal (`assert_eq!(*value, Expr::Int(10))`) without a
/// `.node` accessor at every flat (non-nested) comparison site — there
/// is no span on the bare-literal side to compare, so this compares
/// `node` only. `std`'s own blanket impls (`&A: PartialEq<&B>`,
/// `Vec<A>: PartialEq<Vec<B>>`, `Option<A>: PartialEq<Option<B>>`) then
/// make `&Spanned<T>`/`Vec<Spanned<T>>`/`Option<Spanned<T>>` comparable
/// to `&T`/`Vec<T>`/`Option<T>` the same way, for free.
impl<T: PartialEq> PartialEq<T> for Spanned<T> {
  fn eq(&self, other: &T) -> bool {
    self.node == *other
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
  pub name: String,
  pub ty: String,
  /// Plan 39's Decision log: `Some(_)` only for a `def` function
  /// parameter declared with a trailing `= <literal>` — always `None`
  /// for a class field, a lambda parameter, or a block parameter (the
  /// shared `Param`/`Params` grammar productions those reuse can never
  /// produce `Some`; only the function-only `FuncParam` production
  /// can). Restricted to compile-time-evaluable literal tokens
  /// (`Int`/`Float`/`StringLit`/`Bool`/`Nil`) — never an arbitrary
  /// expression, and never a reference to another parameter.
  pub default: Option<Spanned<Expr>>,
}

/// Decodes a raw `"..."` token's `\"`/`\n` escapes into the literal's
/// real `String` value (plan 19's Decision log — only these two
/// escapes are recognized; `grammar.lalrpop`'s `StringLitTok` regex
/// admits no other backslash sequence, so an unrecognized escape fails
/// to lex as a string token at all — a real parse error, not a silent
/// pass-through). Strips the surrounding quotes. Lives here rather than
/// inline in `grammar.lalrpop` — LALRPOP's grammar-file format doesn't
/// support a bare top-level `fn` definition the way a plain Rust module
/// does.
pub fn decode_string_lit(raw: &str) -> String {
  let inner = &raw[1..raw.len() - 1];
  let mut out = String::with_capacity(inner.len());
  let mut chars = inner.chars();
  while let Some(c) = chars.next() {
    if c == '\\' {
      match chars.next() {
        Some('"') => out.push('"'),
        Some('n') => out.push('\n'),
        _ => unreachable!("the lexer's regex only admits \\\" and \\n escapes"),
      }
    } else {
      out.push(c);
    }
  }
  out
}

/// One piece of an interpolated string (plan 36's Decision log) — a
/// run of literal text, or a `#{...}` span's already-parsed `Expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
  Literal(String),
  Expr(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
  Lt,
  Gt,
  Le,
  Ge,
  Eq,
  Ne,
}

// NOTE: `Eq` (not just `PartialEq`) is intentionally not derived below —
// `Expr::Float(f64)` can't implement `Eq` (NaN != NaN violates Eq's
// reflexivity requirement), and that's transitive through every type that
// contains an `Expr`. Nothing in this workspace needs `Expr`/`Stmt`/etc. as
// a `HashMap`/`HashSet` key; `assert_eq!` in tests only needs `PartialEq`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
  Ident(String),
  Int(i64),
  Float(f64),
  /// `"..."` — plan 19's Decision log: a real, decoded `String` value
  /// (only `\"`/`\n` escapes are ever recognized by the lexer that
  /// produces this — see `grammar.lalrpop`'s `StringLitTok`).
  StringLit(String),
  /// `"...#{expr}..."` (plan 36's Decision log) — a *compile-time*
  /// parse, never a runtime `eval`: each `#{...}` span is re-parsed by
  /// the same static expression grammar that parses the rest of the
  /// program. Only ever constructed when the literal actually contains
  /// at least one `#{...}` span — a plain string with none still
  /// produces `Expr::StringLit`, unchanged, for zero codegen
  /// regression on the overwhelmingly common case.
  Interpolate(Vec<StringPart>),
  /// `:foo` (plan 44's Decision log) — carries the spelling with the
  /// leading `:` already stripped, mirroring `decode_string_lit`'s own
  /// "AST already holds the real value" convention. A genuinely
  /// separate type/runtime representation from `String`, not an alias —
  /// see `emerald-sema`'s `Type::Symbol` and `emerald-codegen`'s
  /// `ValKind::Symbol`.
  SymbolLit(String),
  Add(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Sub(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Mul(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Div(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Rem(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Neg(Box<Spanned<Expr>>),
  Not(Box<Spanned<Expr>>),
  And(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Or(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Compare(Box<Spanned<Expr>>, CompareOp, Box<Spanned<Expr>>),
  Call(String, Vec<Spanned<Expr>>),
  /// `f(name: value, ...)` (plan 39's Decision log) — a plain function
  /// call whose arguments are resolved entirely at compile time by
  /// name-to-position matching against the callee's declared parameter
  /// names, never a runtime hash/dispatch. Scoped to bare function
  /// calls only (never `.method(...)`/`.new(...)`) and to all-keyword
  /// call sites (mixing positional and keyword arguments in one call is
  /// out of scope) — a call with any `name:` argument parses as this
  /// node instead of `Expr::Call`.
  CallKw(String, Vec<(String, Spanned<Expr>)>),
  /// `ClassName.new(args)`.
  New(String, Vec<Spanned<Expr>>),
  /// `receiver.method(args)` — `args` is empty for a bare `receiver.method`
  /// call (plan `08`'s Point example, `sum`/`initialize` never take
  /// arguments); plan `10` is the first to actually parse a non-empty
  /// argument list here.
  MethodCall(Box<Spanned<Expr>>, String, Vec<Spanned<Expr>>),
  /// `receiver&.method(args)` (plan 43's Decision log) — kept distinct
  /// from `MethodCall`, not a reuse: sema's dispatch (nullable receiver
  /// only, pointer-representable return type only) and codegen (a real
  /// is-nil-guarded branch + PHI, producing a `U?` result) are both
  /// genuinely different, not just an evaluation-order variant of an
  /// ordinary call.
  SafeCall(Box<Spanned<Expr>>, String, Vec<Spanned<Expr>>),
  /// `@name` — instance-variable read, valid only inside a method body.
  InstanceVar(String),
  /// `[e1, e2, ...]` — an array literal.
  ArrayLit(Vec<Spanned<Expr>>),
  /// `array[index]` — an indexed read.
  Index(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `->(params) -> ReturnType { body }` — a lambda literal (plan `10`'s
  /// Decision log: by-value capture, top-level-`Let`-only, statically
  /// dispatched `.call`).
  Lambda {
    params: Vec<Param>,
    return_type: String,
    body: Vec<Spanned<Stmt>>,
  },
  /// `true`/`false` (plan 25's Decision log) — a real `Boolean` value,
  /// not just `Compare`'s byproduct.
  Bool(bool),
  /// `nil` (plan 25's Decision log — deliberately narrow: no `T?`
  /// nullable-type system, just a bare `Nil`-typed value).
  Nil,
  /// `{k1 => v1, k2 => v2, ...}` (plan 25's Decision log) — `Int64`
  /// keys only, no `Symbol`-keyed `{a: 1}` shorthand.
  HashLit(Vec<(Spanned<Expr>, Spanned<Expr>)>),
  /// `Array.new(size)` (plan 25's Decision log) — a dedicated node, not
  /// a reuse of `Expr::New`, since `Array` is a reserved keyword, not a
  /// class name in the class registry.
  ArrayNew(Box<Spanned<Expr>>),
  /// `a & b` (plan 28's Decision log) — `Int64`-only bitwise AND.
  BitAnd(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a | b` — `Int64`-only bitwise OR.
  BitOr(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a ^ b` — `Int64`-only bitwise XOR.
  BitXor(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `~a` — `Int64`-only bitwise NOT.
  BitNot(Box<Spanned<Expr>>),
  /// `a << b` — `Int64`-only left shift.
  Shl(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a >> b` — `Int64`-only arithmetic (signed) right shift.
  Shr(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `return a, b` (plan 39's Decision log) — legal ONLY as `Stmt::
  /// Return`'s direct argument, never a first-class value that can be
  /// stored in a variable, passed as an argument, or nested inside
  /// another tuple. Sema accepts this only when the enclosing
  /// function's declared return type is `Type::Tuple` of matching
  /// arity and per-position types.
  TupleLit(Vec<Spanned<Expr>>),
  /// `Ok(value)` (plan 53's Decision log) — a `Result[T, E]` constructor,
  /// checked only in the three expected-type-providing positions
  /// (`Let`'s declared type, `Assign`'s recorded type, `Return`'s
  /// threaded return type) since `infer_expr_type` carries no
  /// expected-type parameter anywhere in this compiler.
  Ok(Box<Spanned<Expr>>),
  /// `Err(value)` — `Result[T, E]`'s other constructor, same
  /// expected-type-position restriction as `Ok` above.
  Err(Box<Spanned<Expr>>),
  /// `expr?` (plan 53's Decision log) — legal ONLY as a `Stmt::Let`'s or
  /// `Stmt::Assign`'s direct value, inside a function whose own declared
  /// return type is `Result[_, E]` for an exactly-matching `E`. Unwraps
  /// `Ok`'s payload in place; on `Err`, immediately returns the same
  /// `Result` value from the enclosing function, unchanged.
  Try(Box<Spanned<Expr>>),
  /// `ActorName.spawn(args)` (plan 54's Decision log) — deliberately
  /// distinct from `Expr::New`, not a reuse disambiguated later in sema:
  /// keeps `Counter.new(0)`/`AnyClass.spawn(0)` both real, symmetric,
  /// grammar-level-textually-distinct call forms. Allocates the
  /// instance from its own arena (plan 51) instead of the shared heap;
  /// every method call on the result still compiles as an ordinary
  /// synchronous call in this plan — only isolation is proven here.
  Spawn(String, Vec<Spanned<Expr>>),
  /// `supervise do <name> = <Class>.spawn(<args>) ... end` (plan 57's
  /// Decision log) — a block, reusing plan 34's own established
  /// mechanism, not a declarative list (`.spawn`'s own argument lists
  /// are ordinary Emerald expressions, and a block body is the only
  /// existing shape able to hold a statement sequence with ordinary
  /// expression arguments). Deliberately NOT a new restricted AST
  /// shape either — an ordinary `Vec<Spanned<Stmt>>`, the exact same
  /// list an `if`/`while`/method body already carries; sema (not the
  /// grammar) restricts it to a flat list of `<name> = <Class>.spawn
  /// (<args>)` (or a bare, unnamed `<Class>.spawn(<args>)`) statements,
  /// the same "grammar stays general, sema narrows" discipline plan
  /// 31 already established for its own restricted-shape leaves.
  Supervise(Vec<Spanned<Stmt>>),
  /// `ActorName.remote(addr, name)` (plan 60's Decision log) —
  /// `.spawn`'s distributed counterpart: connects to `addr` (a
  /// `"host:port"` `String`) and resolves `name`, raising a real,
  /// catchable `RemoteActorError` on any failure rather than returning
  /// a null the caller could dereference. The returned value is usable
  /// at every call site `.spawn`'s own already is (Design decision 1 —
  /// both are the identical tagged `EmeraldActorRef*` shape).
  Remote {
    class: String,
    addr: Box<Spanned<Expr>>,
    name: Box<Spanned<Expr>>,
  },
}

/// One statement in a block (a function body or the program's top level).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
  Let {
    name: String,
    ty: String,
    value: Spanned<Expr>,
  },
  /// `@name = value` — instance-variable write, valid only inside a
  /// method body. No type annotation (the field's type is already
  /// declared on the class), unlike `Let`.
  SetField {
    name: String,
    value: Spanned<Expr>,
  },
  /// `array[index] = value` — an indexed write.
  SetIndex {
    array: Spanned<Expr>,
    index: Spanned<Expr>,
    value: Spanned<Expr>,
  },
  /// `name = value` — reassignment of an already-declared plain local,
  /// no type annotation restated (plan 31's Decision log: the
  /// prerequisite shape neither compound nor multiple assignment can
  /// exist without). `+=`/`-=`/`*=`/`/=`/`%=` desugar into this at
  /// parse time (`value` becomes the corresponding `Expr::Add`/`Sub`/
  /// `Mul`/`Div`/`Rem` over the current value). Sema requires `name`
  /// already present in scope — this is never a fresh declaration.
  Assign {
    name: String,
    value: Spanned<Expr>,
  },
  /// `n1, n2, ... = v1, v2, ...` — fixed-arity multiple assignment over
  /// already-declared plain locals (plan 31's Decision log: no fresh
  /// multi-declaration, no mixed instance-var/index targets, no
  /// splat). `values` are evaluated in full before any `names` target
  /// is written — codegen's proof this actually matters is a real
  /// swap (`a, b = b, a`).
  MultiAssign {
    names: Vec<String>,
    values: Vec<Spanned<Expr>>,
  },
  If {
    cond: Spanned<Expr>,
    then_branch: Vec<Spanned<Stmt>>,
    else_branch: Option<Vec<Spanned<Stmt>>>,
  },
  While {
    cond: Spanned<Expr>,
    body: Vec<Spanned<Stmt>>,
  },
  Return(Option<Spanned<Expr>>),
  Break,
  Next,
  Expr(Spanned<Expr>),
  /// `raise <expr>` — `expr` must evaluate to a class instance (plan
  /// `11`'s Decision log: in practice always a direct `ClassName.new(args)`
  /// call, the only shape codegen supports).
  Raise(Spanned<Expr>),
  /// `begin body rescue Type => e ... [rescue => e2 ...] [ensure ...]
  /// end` (plan 38's Decision log) — one or more `rescue` clauses tried
  /// in source order (subtype-aware: a clause naming a superclass
  /// matches any raised subclass), an optional trailing `ensure` that
  /// always runs, on every exit path (normal fallthrough, a matched
  /// `rescue` clause's own fallthrough, and the mismatch-exhausted
  /// re-raise path alike). Generalizes plan 11's original single-typed-
  /// clause, no-`ensure` shape (`rescues.len() == 1`, `ensure: None`).
  Begin {
    body: Vec<Spanned<Stmt>>,
    rescues: Vec<RescueClause>,
    ensure: Option<Vec<Spanned<Stmt>>>,
  },
  /// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan
  /// 20's Decision log): value-match via the same `CompareOp::Eq`
  /// `Expr::Compare` already performs, over an `Int64`-only scrutinee —
  /// not `spec/GRAMMAR.md`'s eventual method-dispatched `===` (no
  /// operator-overload dispatch mechanism exists in this compiler).
  /// Each arm's `Vec<Expr>` holds one or more `when` values; the arm
  /// matches if the scrutinee equals *any* of them. First matching arm
  /// wins, source order, matching Ruby's own semantics.
  /// Plan 52's Decision log: `arms` now carries a `CasePattern` per arm
  /// instead of a bare `Vec<Spanned<Expr>>` — `Values` is this
  /// statement's original Int64-value-match shape, unchanged behavior,
  /// just wrapped; `Variant` is this plan's addition (a closed-enum
  /// tag match with per-arm-scoped bindings).
  Case {
    scrutinee: Spanned<Expr>,
    arms: Vec<CaseArm>,
    else_body: Option<Vec<Spanned<Stmt>>>,
  },
  /// `for var in [e1, e2, ...] body end` (plan 30's Decision log) —
  /// `elements` is restricted to a literal array at the grammar level
  /// (an arbitrary `Array[T]`-typed scrutinee is a parse error, not a
  /// sema error), since `Array[T]`'s own representation carries no
  /// runtime length to iterate against. Codegen desugars this straight
  /// into the same index-based `while` shape plan 09's array traversal
  /// already proved.
  For {
    var: String,
    elements: Vec<Spanned<Expr>>,
    body: Vec<Spanned<Stmt>>,
  },
  /// `yield <args>` (plan 34's Decision log) — legal only inside a
  /// function that declares `Function.block_param`. Codegen lowers this
  /// via call-site specialization: a `yield`-using function is compiled
  /// fresh per call site that attaches a literal block, and each
  /// `Stmt::Yield` becomes a direct call to that block's synthesized
  /// function — never an indirect/first-class call.
  Yield(Vec<Spanned<Expr>>),
  /// `for var in start..end body end` (`exclusive: false`) or
  /// `start...end` (`exclusive: true`) — plan 37's Decision log: no
  /// first-class `Range` value exists anywhere (this shape only ever
  /// appears directly as a `for...in` scrutinee, grammar-restricted the
  /// same way `Stmt::For`'s own literal-array scrutinee is); `start`/
  /// `end` are arbitrary `Int64`-typed expressions, evaluated once each
  /// before the loop begins, not just literals.
  ForRange {
    var: String,
    start: Spanned<Expr>,
    end: Spanned<Expr>,
    exclusive: bool,
    body: Vec<Spanned<Stmt>>,
  },
  /// `retry` (plan 38's Decision log) — legal only inside a `rescue`
  /// clause's own body (not the `begin`'s try body, not `ensure`),
  /// re-attempts the enclosing `begin` construct from scratch. Does
  /// not run the enclosing `ensure` on its way back — it doesn't exit
  /// the `begin` construct at all, unlike `Return`/`Break`/`Next`/
  /// `Raise`.
  Retry,
  /// `name ||= default` (plan 43's Decision log) — assign `default` only
  /// if `name`'s current value is nil, restricted to a plain already-
  /// declared local (never `@field`/an index target), the same
  /// restriction plan 31's `+=`-family compound assignment already
  /// makes. Genuinely conditional, not desugared into `Stmt::Assign`
  /// the way `+=`/etc. are — sema additionally narrows `name`'s tracked
  /// type from `Nullable(inner)` to `inner` immediately after this
  /// statement (sound by construction: either branch leaves `name`
  /// unconditionally `inner`-typed).
  OrAssign {
    name: String,
    default: Spanned<Expr>,
  },
  /// `name &&= value` (plan 43's Decision log) — assign `value` only if
  /// `name`'s current value is non-nil; the asymmetric twin of
  /// `OrAssign` that does NOT narrow `name`'s tracked type (the
  /// nil-and-skipped branch leaves it exactly as nilable as before).
  AndAssign {
    name: String,
    value: Spanned<Expr>,
  },
  /// `case scrutinee when Ok(ok_var) ok_body when Err(err_var) err_body
  /// end` (plan 53's Decision log) — a small, dedicated destructuring
  /// form for `Result[T, E]`, deliberately independent of plan 52's
  /// `Stmt::Case`/`CasePattern` mechanism (see that plan's own Decision
  /// log for why). Both arms are mandatory, fixed order (`Ok` then
  /// `Err`), no `else`. `ok_var`/`err_var` are bound only inside their
  /// own arm's body, at the scrutinee's real, statically-known `T`/`E`.
  MatchResult {
    scrutinee: Spanned<Expr>,
    ok_var: String,
    ok_body: Vec<Spanned<Stmt>>,
    err_var: String,
    err_body: Vec<Spanned<Stmt>>,
  },
}

/// One `when ... body` arm of a `Stmt::Case` (plan 22's Decision log —
/// named to keep `arms`' field type out of clippy's `type_complexity`
/// lint, not a new AST concept).
pub type CaseArm = (CasePattern, Vec<Spanned<Stmt>>);

/// Plan 52's Decision log: `Values` is plan 20's original `when v1, v2,
/// ...` shape (Int64-only value-match), wrapped unchanged; `Variant` is
/// this plan's `when VariantName(b1, b2, ...)` closed-enum tag match —
/// `bindings` are plain identifiers only (no nested pattern), scoped to
/// this arm's own body alone, never the surrounding flat `env`.
#[derive(Debug, Clone, PartialEq)]
pub enum CasePattern {
  Values(Vec<Spanned<Expr>>),
  Variant { name: String, bindings: Vec<String> },
}

/// One `rescue` clause of a `Stmt::Begin` (plan 38's Decision log).
/// `class_name: None` is a bare `rescue => e` catch-all — matches
/// unconditionally, and `var` is never bound in `env` (no universal
/// root class exists to type it at).
#[derive(Debug, Clone, PartialEq)]
pub struct RescueClause {
  pub class_name: Option<String>,
  pub var: String,
  pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Vec<Spanned<Stmt>>,
  /// `&blk` in the parameter list (plan 34's Decision log) — a bare
  /// name, not a `Param`: unlike an ordinary parameter, its type can't
  /// be pinned at the function's own declaration site (no first-class
  /// `Proc` value exists to check a caller's argument against; a
  /// `yield`-using function is instead call-site-specialized fresh
  /// against whichever block literal a given call attaches). `None` for
  /// every function that doesn't declare one — unchanged from before
  /// this plan.
  pub block_param: Option<String>,
  /// `*xs: Int64` (plan 39's Decision log) — the trailing splat
  /// parameter's *element* type only (e.g. `Int64`, not `Array[Int64]`).
  /// Ordered after ordinary/defaulted parameters and before an optional
  /// `&blk`; packed into a real `Array[Elem]` at each call site (plan
  /// 09's actual representation), never a variadic LLVM signature — the
  /// compiled function itself stays fixed-arity. `None` for every
  /// function that doesn't declare one.
  pub splat_param: Option<Param>,
  /// `[T: Bound]` (plan 41's Decision log) — legal only on a top-level
  /// function; sema rejects a non-empty `type_params` found on a class
  /// method or module function with a real diagnostic, and rejects more
  /// than one entry with a real diagnostic (the grammar's `TypeParamList`
  /// is a general comma list — the single-type-parameter, bound-only
  /// restriction is entirely sema's job, not the grammar's). Empty
  /// `Vec` for every function that doesn't declare one — additive,
  /// source-compatible with every prior plan.
  pub type_params: Vec<TypeParam>,
}

/// `T: Comparable` inside a generic function's or generic class's `[...]`
/// clause. Plan 58's Decision log: `bound` widens from a mandatory
/// `String` (plan 41's original, function-only design — no `def
/// identity[T](x: T)` unbounded form) to `Option<String>`, since a
/// generic *class*'s own headline case (`class Box[T] ... end`,
/// `class Stack[T] ... end`) has no bound at all. A real, disclosed,
/// source-compatible widening of the one `TypeParam` shape plan 41
/// already shipped, not a second, class-only type-parameter struct —
/// every FUNCTION-side reader keeps requiring a bound (now enforced by
/// sema's own diagnostic instead of structurally by the grammar).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
  pub name: String,
  pub bound: Option<String>,
}

/// `interface Comparable def compare_to(other: Self) -> Int64 end` (plan
/// 41's Decision log) — structurally exactly one required method: the
/// grammar has no `FuncDef*`-style repetition inside `interface ...
/// end`, so a two-method interface is a parse error, not a silently
/// accepted-but-unchecked shape. Reuses `Param`'s `{name, ty}` shape for
/// the method's parameters, the same reuse plan 08 established for class
/// fields.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDef {
  pub name: String,
  pub method_name: String,
  pub params: Vec<Param>,
  pub return_type: String,
}

/// A class declaration: fields (reusing `Param`'s `{name, ty}` shape —
/// structurally identical to a parameter declaration) and methods
/// (ordinary `Function`s, including `initialize`). `superclass` is
/// `class Dog < Animal`'s `Animal` (plan 32's Decision log) — single
/// inheritance only, no `include`/`extend` mixin composition, no real
/// dynamic dispatch (every call still resolves by the receiver's
/// *declared* class, same as before this plan).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
  pub name: String,
  pub superclass: Option<String>,
  /// `implements Comparable` (plan 41's Decision log) — a single
  /// interface name, checked structurally (conformance, not
  /// assignability) at class-declaration time. `None` for every class
  /// that doesn't declare one.
  pub implements: Option<String>,
  pub fields: Vec<Param>,
  pub methods: Vec<Function>,
  /// `class Stack[T]`/`class Box[T: Comparable]` (plan 58's Decision
  /// log) — empty `Vec` for every class that doesn't declare one,
  /// additive and source-compatible, mirroring `Function.type_params`'s
  /// own precedent above. A non-empty `type_params` marks this `ClassDef`
  /// as a raw, unresolved TEMPLATE — `emerald-sema` registers it into a
  /// separate `generic_classes` registry instead of the ordinary
  /// `classes` table, and it is never monomorphized without a real
  /// concrete instantiation actually written somewhere in the program.
  pub type_params: Vec<TypeParam>,
}

/// One `fn <name>(<params>): <ReturnType>` declaration inside an
/// `unsafe extern "C" { ... }` block (plan 59's Decision log). Reuses
/// `Param`'s `{name, ty}` shape verbatim for params, the same reuse
/// `ClassDef`/`InterfaceDef` already make — but deliberately `:` rather
/// than `def`'s `->` (grammar-level, not just the mandatory `unsafe`
/// prefix) so an extern declaration is visually distinct from an
/// ordinary function signature at a glance.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternFn {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
}

/// `unsafe extern "C" { fn ... ... }` (plan 59's Decision log) — `abi`
/// is the decoded string literal (`"C"`), checked against exactly that
/// value by `emerald-sema` (the explicit C++ decline), not restricted
/// at the grammar level (a bare string literal, not a reserved keyword
/// set, so a bad ABI is a real sema diagnostic naming it, not a parse
/// error).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
  pub abi: String,
  pub fns: Vec<ExternFn>,
}

/// A namespace-only module (plan `12`'s Decision log — `SEMANTICS.md`
/// §10 explicitly removes `include`/`extend` mixin composition): a flat
/// collection of methods, called as `ModuleName.method(args)`. No
/// fields — there's no instance for them to live on.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
  pub name: String,
  pub methods: Vec<Function>,
}

/// `Circle(Float64)` inside `enum Shape = Circle(Float64) | ...` (plan
/// 52's Decision log) — `fields` are raw `TypeName` strings, the same
/// string-based type-annotation convention `Param.ty` already uses,
/// deferring resolution to sema like every other type annotation in
/// this AST. At least one field is required — no bare, nullary tag
/// variants in this plan.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
  pub name: String,
  pub fields: Vec<String>,
}

/// `enum Shape = Circle(Float64) | Square(Float64) | ...` (plan 52's
/// Decision log) — a CLOSED set of variants, fixed at declaration: no
/// grammar path reopens an already-declared enum from another file,
/// the deliberate opposite of `ClassDef`'s open, extensible hierarchy.
/// Pure data — no method bodies, no `implements`, no superclass.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
  pub name: String,
  pub variants: Vec<EnumVariant>,
}

/// `actor Counter ... end` (plan 54's Decision log) — a flat,
/// non-inheriting top-level declaration reusing `ClassDef`'s own field/
/// method syntax verbatim, its own struct rather than `ClassDef` plus a
/// marker flag: unlike `ClassDef`, `superclass` doesn't exist as a
/// field at all — `actor Foo < Bar` is unrepresentable in this AST, a
/// real grammar-level decline, not merely a semantic rejection of an
/// otherwise-parseable shape. Instances live in their own arena (plan
/// 51's mechanism) instead of the shared never-freed heap; this plan
/// proves isolation only — every method call still compiles as an
/// ordinary, synchronous call (no scheduler, no mailbox exist yet).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorDef {
  pub name: String,
  pub fields: Vec<Param>,
  pub methods: Vec<Function>,
}

/// A single top-level construct: a function definition, a class
/// definition, a module definition, or a top-level statement (e.g.
/// `x: Int64 = 10`, `if x > 5 ... end`, `puts x`).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  Function(Function),
  Class(ClassDef),
  Module(ModuleDef),
  /// `actor Counter ... end` (plan 54's Decision log).
  Actor(ActorDef),
  /// `enum Shape = Circle(Float64) | ...` (plan 52's Decision log).
  Enum(EnumDef),
  /// `interface Comparable ... end` (plan 41's Decision log) — a
  /// general, user-declarable grammar production, not a fourth
  /// hardcoded builtin the way `Array`/`Hash`/`Proc` are.
  Interface(InterfaceDef),
  /// `unsafe extern "C" { ... }` (plan 59's Decision log) — the one
  /// deliberate, explicitly-fenced escape from this compiler's
  /// otherwise-total static safety.
  Extern(ExternBlock),
  Stmt(Spanned<Stmt>),
  /// `require <path>` (plan 23's Decision log) — a bare, unquoted,
  /// `/`-separated path (no string-literal syntax dependency), always
  /// relative to the *containing* file, `.em` implied. Reachable only
  /// from `Program`'s top-level `Item*` rule, never from `Stmt*` — a
  /// `require` inside a function body is a real parse error, not
  /// silently accepted. Plan 17's `emerald-driver` doesn't splice
  /// multi-file `require`s itself (that's `emerald-cli`'s own
  /// `require.rs`, plan 46) — one reaching `emerald-sema`/
  /// `emerald-codegen` directly is a real, disclosed gap: sema treats
  /// it as a no-op, codegen returns a descriptive `Err` rather than
  /// silently ignoring it.
  Require(String),
  /// `test "description" do ... end` (plan 47's Decision log) — a
  /// narrow, self-contained grammar addition (its own `"do" Stmt*
  /// "end"` production, not plan 34's general `{ |params| body }`
  /// block syntax, which explicitly declined a `do...end` form).
  /// `body` is compiled as its own synthesized zero-parameter,
  /// `Void`-returning function, wrapped in `begin ... rescue
  /// AssertionError => e ... end` by `emerald_codegen::
  /// compile_test_harness` (`leaf-test-runner`) — never reachable via
  /// the ordinary `emerald <file>` compile path, which rejects a
  /// `Program` containing one.
  Test {
    description: String,
    body: Vec<Spanned<Stmt>>,
  },
  /// A top-level construct LALRPOP's `!` error-recovery mechanism
  /// resynchronized past (plan 26's Decision log) — a real parse error
  /// was recorded for it. Never appears in a `Program` `parse`/
  /// `parse_named` actually returns `Ok(_)` for: the moment any
  /// `Item::Error` (or any other recovered error) exists, the whole
  /// call returns `Err(Vec<ParseError>)` instead. `emerald-sema`/
  /// `emerald-codegen` never see this variant — their match arms for
  /// it are `unreachable!()`, not a defensive `Err`.
  Error,
}

/// A full Emerald source file — inception §17's milestone unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
  pub items: Vec<Item>,
}
