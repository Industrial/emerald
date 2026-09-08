#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
  pub name: String,
  pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
  Ident(String),
  Add(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: String,
  pub body: Expr,
}
