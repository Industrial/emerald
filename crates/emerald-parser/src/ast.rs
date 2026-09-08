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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
  Ident(String),
  Int(i64),
  Add(Box<Expr>, Box<Expr>),
  Compare(Box<Expr>, CompareOp, Box<Expr>),
  Call(String, Vec<Expr>),
}

/// One statement in a block (a function body or the program's top level).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
  Let {
    name: String,
    ty: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Vec<Stmt>,
}

/// A single top-level construct: a function definition, or a top-level
/// statement (e.g. `x: Int64 = 10`, `if x > 5 ... end`, `puts x`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
  Function(Function),
  Stmt(Stmt),
}

/// A full Emerald source file — inception §17's milestone unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
  pub items: Vec<Item>,
}
