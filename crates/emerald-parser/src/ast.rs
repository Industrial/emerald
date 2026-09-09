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
  pub default: Option<Expr>,
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
  Expr(Box<Expr>),
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
  Add(Box<Expr>, Box<Expr>),
  Sub(Box<Expr>, Box<Expr>),
  Mul(Box<Expr>, Box<Expr>),
  Div(Box<Expr>, Box<Expr>),
  Rem(Box<Expr>, Box<Expr>),
  Neg(Box<Expr>),
  Not(Box<Expr>),
  And(Box<Expr>, Box<Expr>),
  Or(Box<Expr>, Box<Expr>),
  Compare(Box<Expr>, CompareOp, Box<Expr>),
  Call(String, Vec<Expr>),
  /// `f(name: value, ...)` (plan 39's Decision log) — a plain function
  /// call whose arguments are resolved entirely at compile time by
  /// name-to-position matching against the callee's declared parameter
  /// names, never a runtime hash/dispatch. Scoped to bare function
  /// calls only (never `.method(...)`/`.new(...)`) and to all-keyword
  /// call sites (mixing positional and keyword arguments in one call is
  /// out of scope) — a call with any `name:` argument parses as this
  /// node instead of `Expr::Call`.
  CallKw(String, Vec<(String, Expr)>),
  /// `ClassName.new(args)`.
  New(String, Vec<Expr>),
  /// `receiver.method(args)` — `args` is empty for a bare `receiver.method`
  /// call (plan `08`'s Point example, `sum`/`initialize` never take
  /// arguments); plan `10` is the first to actually parse a non-empty
  /// argument list here.
  MethodCall(Box<Expr>, String, Vec<Expr>),
  /// `receiver&.method(args)` (plan 43's Decision log) — kept distinct
  /// from `MethodCall`, not a reuse: sema's dispatch (nullable receiver
  /// only, pointer-representable return type only) and codegen (a real
  /// is-nil-guarded branch + PHI, producing a `U?` result) are both
  /// genuinely different, not just an evaluation-order variant of an
  /// ordinary call.
  SafeCall(Box<Expr>, String, Vec<Expr>),
  /// `@name` — instance-variable read, valid only inside a method body.
  InstanceVar(String),
  /// `[e1, e2, ...]` — an array literal.
  ArrayLit(Vec<Expr>),
  /// `array[index]` — an indexed read.
  Index(Box<Expr>, Box<Expr>),
  /// `->(params) -> ReturnType { body }` — a lambda literal (plan `10`'s
  /// Decision log: by-value capture, top-level-`Let`-only, statically
  /// dispatched `.call`).
  Lambda {
    params: Vec<Param>,
    return_type: String,
    body: Vec<Stmt>,
  },
  /// `true`/`false` (plan 25's Decision log) — a real `Boolean` value,
  /// not just `Compare`'s byproduct.
  Bool(bool),
  /// `nil` (plan 25's Decision log — deliberately narrow: no `T?`
  /// nullable-type system, just a bare `Nil`-typed value).
  Nil,
  /// `{k1 => v1, k2 => v2, ...}` (plan 25's Decision log) — `Int64`
  /// keys only, no `Symbol`-keyed `{a: 1}` shorthand.
  HashLit(Vec<(Expr, Expr)>),
  /// `Array.new(size)` (plan 25's Decision log) — a dedicated node, not
  /// a reuse of `Expr::New`, since `Array` is a reserved keyword, not a
  /// class name in the class registry.
  ArrayNew(Box<Expr>),
  /// `a & b` (plan 28's Decision log) — `Int64`-only bitwise AND.
  BitAnd(Box<Expr>, Box<Expr>),
  /// `a | b` — `Int64`-only bitwise OR.
  BitOr(Box<Expr>, Box<Expr>),
  /// `a ^ b` — `Int64`-only bitwise XOR.
  BitXor(Box<Expr>, Box<Expr>),
  /// `~a` — `Int64`-only bitwise NOT.
  BitNot(Box<Expr>),
  /// `a << b` — `Int64`-only left shift.
  Shl(Box<Expr>, Box<Expr>),
  /// `a >> b` — `Int64`-only arithmetic (signed) right shift.
  Shr(Box<Expr>, Box<Expr>),
}

/// One statement in a block (a function body or the program's top level).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
  Let {
    name: String,
    ty: String,
    value: Expr,
  },
  /// `@name = value` — instance-variable write, valid only inside a
  /// method body. No type annotation (the field's type is already
  /// declared on the class), unlike `Let`.
  SetField {
    name: String,
    value: Expr,
  },
  /// `array[index] = value` — an indexed write.
  SetIndex {
    array: Expr,
    index: Expr,
    value: Expr,
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
    value: Expr,
  },
  /// `n1, n2, ... = v1, v2, ...` — fixed-arity multiple assignment over
  /// already-declared plain locals (plan 31's Decision log: no fresh
  /// multi-declaration, no mixed instance-var/index targets, no
  /// splat). `values` are evaluated in full before any `names` target
  /// is written — codegen's proof this actually matters is a real
  /// swap (`a, b = b, a`).
  MultiAssign {
    names: Vec<String>,
    values: Vec<Expr>,
  },
  If {
    cond: Expr,
    then_branch: Vec<Stmt>,
    else_branch: Option<Vec<Stmt>>,
  },
  While {
    cond: Expr,
    body: Vec<Stmt>,
  },
  Return(Option<Expr>),
  Break,
  Next,
  Expr(Expr),
  /// `raise <expr>` — `expr` must evaluate to a class instance (plan
  /// `11`'s Decision log: in practice always a direct `ClassName.new(args)`
  /// call, the only shape codegen supports).
  Raise(Expr),
  /// `begin body rescue Type => e ... [rescue => e2 ...] [ensure ...]
  /// end` (plan 38's Decision log) — one or more `rescue` clauses tried
  /// in source order (subtype-aware: a clause naming a superclass
  /// matches any raised subclass), an optional trailing `ensure` that
  /// always runs, on every exit path (normal fallthrough, a matched
  /// `rescue` clause's own fallthrough, and the mismatch-exhausted
  /// re-raise path alike). Generalizes plan 11's original single-typed-
  /// clause, no-`ensure` shape (`rescues.len() == 1`, `ensure: None`).
  Begin {
    body: Vec<Stmt>,
    rescues: Vec<RescueClause>,
    ensure: Option<Vec<Stmt>>,
  },
  /// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan
  /// 20's Decision log): value-match via the same `CompareOp::Eq`
  /// `Expr::Compare` already performs, over an `Int64`-only scrutinee —
  /// not `spec/GRAMMAR.md`'s eventual method-dispatched `===` (no
  /// operator-overload dispatch mechanism exists in this compiler).
  /// Each arm's `Vec<Expr>` holds one or more `when` values; the arm
  /// matches if the scrutinee equals *any* of them. First matching arm
  /// wins, source order, matching Ruby's own semantics.
  Case {
    scrutinee: Expr,
    arms: Vec<(Vec<Expr>, Vec<Stmt>)>,
    else_body: Option<Vec<Stmt>>,
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
    elements: Vec<Expr>,
    body: Vec<Stmt>,
  },
  /// `yield <args>` (plan 34's Decision log) — legal only inside a
  /// function that declares `Function.block_param`. Codegen lowers this
  /// via call-site specialization: a `yield`-using function is compiled
  /// fresh per call site that attaches a literal block, and each
  /// `Stmt::Yield` becomes a direct call to that block's synthesized
  /// function — never an indirect/first-class call.
  Yield(Vec<Expr>),
  /// `for var in start..end body end` (`exclusive: false`) or
  /// `start...end` (`exclusive: true`) — plan 37's Decision log: no
  /// first-class `Range` value exists anywhere (this shape only ever
  /// appears directly as a `for...in` scrutinee, grammar-restricted the
  /// same way `Stmt::For`'s own literal-array scrutinee is); `start`/
  /// `end` are arbitrary `Int64`-typed expressions, evaluated once each
  /// before the loop begins, not just literals.
  ForRange {
    var: String,
    start: Expr,
    end: Expr,
    exclusive: bool,
    body: Vec<Stmt>,
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
    default: Expr,
  },
  /// `name &&= value` (plan 43's Decision log) — assign `value` only if
  /// `name`'s current value is non-nil; the asymmetric twin of
  /// `OrAssign` that does NOT narrow `name`'s tracked type (the
  /// nil-and-skipped branch leaves it exactly as nilable as before).
  AndAssign {
    name: String,
    value: Expr,
  },
}

/// One `rescue` clause of a `Stmt::Begin` (plan 38's Decision log).
/// `class_name: None` is a bare `rescue => e` catch-all — matches
/// unconditionally, and `var` is never bound in `env` (no universal
/// root class exists to type it at).
#[derive(Debug, Clone, PartialEq)]
pub struct RescueClause {
  pub class_name: Option<String>,
  pub var: String,
  pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Vec<Stmt>,
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

/// `T: Comparable` inside a generic function's `[...]` clause — the bound
/// is mandatory (plan 41's Decision log: no `def identity[T](x: T)`
/// unbounded form).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
  pub name: String,
  pub bound: String,
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

/// A single top-level construct: a function definition, a class
/// definition, a module definition, or a top-level statement (e.g.
/// `x: Int64 = 10`, `if x > 5 ... end`, `puts x`).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  Function(Function),
  Class(ClassDef),
  Module(ModuleDef),
  /// `interface Comparable ... end` (plan 41's Decision log) — a
  /// general, user-declarable grammar production, not a fourth
  /// hardcoded builtin the way `Array`/`Hash`/`Proc` are.
  Interface(InterfaceDef),
  Stmt(Stmt),
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
    body: Vec<Stmt>,
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
