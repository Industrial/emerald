#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
  pub name: String,
  pub ty: String,
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
  /// `ClassName.new(args)`.
  New(String, Vec<Expr>),
  /// `receiver.method(args)` — `args` is empty for a bare `receiver.method`
  /// call (plan `08`'s Point example, `sum`/`initialize` never take
  /// arguments); plan `10` is the first to actually parse a non-empty
  /// argument list here.
  MethodCall(Box<Expr>, String, Vec<Expr>),
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
  /// `begin body rescue Type => e rescue_body end` — a single typed
  /// handler (plan `11`'s Decision log: no `ensure`, no multiple
  /// `rescue` clauses, no bare catch-all yet).
  Begin {
    body: Vec<Stmt>,
    rescue_type: String,
    rescue_var: String,
    rescue_body: Vec<Stmt>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Vec<Stmt>,
}

/// A class declaration: fields (reusing `Param`'s `{name, ty}` shape —
/// structurally identical to a parameter declaration) and methods
/// (ordinary `Function`s, including `initialize`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
  pub name: String,
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
  Stmt(Stmt),
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
