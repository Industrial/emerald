#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
  pub name: String,
  pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
  Ident(String),
  Int(i64),
  Add(Box<Expr>, Box<Expr>),
  Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Expr,
}

/// A single top-level construct: a function definition, or a top-level
/// call statement (e.g. `puts add(20, 22)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
  Function(Function),
  Expr(Expr),
}

/// A full Emerald source file — inception §17's milestone-1 unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
  pub items: Vec<Item>,
}
