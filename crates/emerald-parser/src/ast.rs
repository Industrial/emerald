#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
  pub name: String,
  pub ty: String,
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
  Add(Box<Expr>, Box<Expr>),
  Compare(Box<Expr>, CompareOp, Box<Expr>),
  Call(String, Vec<Expr>),
  /// `ClassName.new(args)`.
  New(String, Vec<Expr>),
  /// `receiver.method` — bare, zero-argument method calls only (see plan
  /// `08`'s Implementation Notes).
  MethodCall(Box<Expr>, String, Vec<Expr>),
  /// `@name` — instance-variable read, valid only inside a method body.
  InstanceVar(String),
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

/// A single top-level construct: a function definition, a class
/// definition, or a top-level statement (e.g. `x: Int64 = 10`,
/// `if x > 5 ... end`, `puts x`).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  Function(Function),
  Class(ClassDef),
  Stmt(Stmt),
}

/// A full Emerald source file — inception §17's milestone unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
  pub items: Vec<Item>,
}
