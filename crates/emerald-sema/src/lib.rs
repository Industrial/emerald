//! Name resolution + type checking over `emerald_parser::Program`
//! (plan-of-plans row 05, inception §17 steps 4–7 and §25.E).
//!
//! No source-span tracking yet (`crates/emerald-lexer`/`emerald-parser`
//! don't carry spans) — diagnostics are function/call-scoped text.
//! Line/column-precise diagnostics are `13 diagnostics`'s job.

use emerald_parser::{Expr, Function, Item, Program};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
  Int64,
  String,
  Void,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
  pub message: String,
}

impl Diagnostic {
  fn new(message: impl Into<String>) -> Self {
    Self {
      message: message.into(),
    }
  }
}

/// Resolves a type name against `spec/TYPE_SYSTEM.md`'s primitives. Errors
/// on any unrecognized name rather than silently accepting it.
pub fn resolve_type_name(name: &str) -> Result<Type, Diagnostic> {
  match name {
    "Int64" => Ok(Type::Int64),
    "String" => Ok(Type::String),
    "Void" => Ok(Type::Void),
    other => Err(Diagnostic::new(format!("unknown type `{other}`"))),
  }
}

#[derive(Debug, Clone)]
struct FunctionSig {
  params: Vec<Type>,
  return_type: Type,
}

fn function_signature(f: &Function) -> Result<FunctionSig, Diagnostic> {
  let params = f
    .params
    .iter()
    .map(|p| resolve_type_name(&p.ty))
    .collect::<Result<Vec<_>, _>>()?;
  let return_type = resolve_type_name(&f.return_type)?;
  Ok(FunctionSig {
    params,
    return_type,
  })
}

fn infer_expr_type(
  expr: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
) -> Result<Type, Diagnostic> {
  match expr {
    Expr::Ident(name) => env
      .get(name)
      .copied()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"))),
    Expr::Int(_) => Ok(Type::Int64),
    Expr::Add(lhs, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs)?;
      let rt = infer_expr_type(rhs, env, sigs)?;
      if lt != rt {
        return Err(Diagnostic::new(format!(
          "type mismatch: `+` requires both operands to have the same type, found {lt:?} and {rt:?}"
        )));
      }
      if lt != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "type `{lt:?}` does not support `+`"
        )));
      }
      Ok(lt)
    }
    Expr::Call(name, args) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`")))?;
      if args.len() != sig.params.len() {
        return Err(Diagnostic::new(format!(
          "function `{name}` expects {} argument(s), found {}",
          sig.params.len(),
          args.len()
        )));
      }
      for (i, (arg, expected)) in args.iter().zip(&sig.params).enumerate() {
        let actual = infer_expr_type(arg, env, sigs)?;
        if actual != *expected {
          return Err(Diagnostic::new(format!(
            "argument {} to `{name}` has type {actual:?}, expected {expected:?}",
            i + 1
          )));
        }
      }
      Ok(sig.return_type)
    }
  }
}

fn check_function_body(
  f: &Function,
  sigs: &HashMap<String, FunctionSig>,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &f.params {
    env.insert(p.name.clone(), resolve_type_name(&p.ty)?);
  }
  let body_type = infer_expr_type(&f.body, &env, sigs)?;
  let declared_return = resolve_type_name(&f.return_type)?;
  if body_type != declared_return {
    return Err(Diagnostic::new(format!(
      "type mismatch in function `{}`: body has type {body_type:?} but declared return type is {declared_return:?}",
      f.name
    )));
  }
  Ok(())
}

fn builtin_signatures() -> HashMap<String, FunctionSig> {
  let mut sigs = HashMap::new();
  sigs.insert(
    "puts".to_string(),
    FunctionSig {
      params: vec![Type::Int64],
      return_type: Type::Void,
    },
  );
  sigs
}

/// Type-checks a full program: builds a function-signature table (name
/// resolution) then checks every function body and top-level call
/// expression. Two-pass so a top-level call to a function defined later
/// in the same `Item*` list still resolves.
pub fn check_program(program: &Program) -> Result<(), Vec<Diagnostic>> {
  let mut sigs = builtin_signatures();
  let mut diags = Vec::new();

  for item in &program.items {
    if let Item::Function(f) = item {
      match function_signature(f) {
        Ok(sig) => {
          sigs.insert(f.name.clone(), sig);
        }
        Err(d) => diags.push(d),
      }
    }
  }

  for item in &program.items {
    match item {
      Item::Function(f) => {
        if let Err(d) = check_function_body(f, &sigs) {
          diags.push(d);
        }
      }
      Item::Expr(e) => {
        if let Err(d) = infer_expr_type(e, &HashMap::new(), &sigs) {
          diags.push(d);
        }
      }
    }
  }

  if diags.is_empty() { Ok(()) } else { Err(diags) }
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_EM: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn accepts_hello_em() {
    let program = emerald_parser::parse(HELLO_EM).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_inception_25e_mismatch_with_useful_diagnostic() {
    let src = "def add(a: Int64, b: String) -> Int64\n  a + b\nend";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 + String");
    assert_eq!(errs.len(), 1);
    let msg = &errs[0].message;
    assert!(msg.contains("Int64"), "diagnostic should name Int64: {msg}");
    assert!(
      msg.contains("String"),
      "diagnostic should name String: {msg}"
    );
  }

  #[test]
  fn rejects_undefined_variable_without_panicking() {
    let src = "def add(a: Int64) -> Int64\n  a + b\nend";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject undefined `b`");
    assert!(errs[0].message.contains("undefined variable `b`"));
  }

  #[test]
  fn rejects_arity_mismatch_at_call_site() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 1-arg call to 2-arg add");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("expects 2 argument"))
    );
  }

  #[test]
  fn rejects_undefined_function_without_panicking() {
    let src = "puts undefined_fn(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject call to undefined function");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("undefined function `undefined_fn`"))
    );
  }
}
