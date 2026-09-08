//! Name resolution + type checking over `emerald_parser::Program`
//! (plan-of-plans row 05, inception §17 steps 4–7 and §25.E).
//!
//! No source-span tracking yet (`crates/emerald-lexer`/`emerald-parser`
//! don't carry spans) — diagnostics are function/call-scoped text.
//! Line/column-precise diagnostics are `13 diagnostics`'s job.

use emerald_parser::{Expr, Function, Item, Program, Stmt};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
  Int64,
  String,
  Boolean,
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
    "Boolean" => Ok(Type::Boolean),
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
    Expr::Compare(lhs, op, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs)?;
      let rt = infer_expr_type(rhs, env, sigs)?;
      if lt != rt {
        return Err(Diagnostic::new(format!(
          "type mismatch: `{op:?}` requires both operands to have the same type, found {lt:?} and {rt:?}"
        )));
      }
      Ok(Type::Boolean)
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

/// Type-checks one statement, threading a mutable local-variable
/// environment and the enclosing function's declared return type (used to
/// check every `return <expr>`, not just a trailing one). `in_loop` gates
/// `break`/`next` legality.
fn check_stmt(
  stmt: &Stmt,
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  return_type: Type,
  in_loop: bool,
) -> Result<(), Diagnostic> {
  match stmt {
    Stmt::Let { name, ty, value } => {
      let declared = resolve_type_name(ty)?;
      let actual = infer_expr_type(value, env, sigs)?;
      if actual != declared {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name}: {ty} = ...`: declared type {declared:?}, value has type {actual:?}"
        )));
      }
      env.insert(name.clone(), declared);
      Ok(())
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_ty = infer_expr_type(cond, env, sigs)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`if` condition must be Boolean, found {cond_ty:?} (no truthy/falsy coercion — spec/GRAMMAR.md §5)"
        )));
      }
      check_block(then_branch, env, sigs, return_type, in_loop)?;
      if let Some(else_b) = else_branch {
        check_block(else_b, env, sigs, return_type, in_loop)?;
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      let cond_ty = infer_expr_type(cond, env, sigs)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`while` condition must be Boolean, found {cond_ty:?}"
        )));
      }
      check_block(body, env, sigs, return_type, true)
    }
    Stmt::Return(Some(e)) => {
      let t = infer_expr_type(e, env, sigs)?;
      if t != return_type {
        return Err(Diagnostic::new(format!(
          "type mismatch: `return` value has type {t:?} but the enclosing function declares {return_type:?}"
        )));
      }
      Ok(())
    }
    Stmt::Return(None) => Ok(()),
    Stmt::Break => {
      if !in_loop {
        return Err(Diagnostic::new("`break` outside of a loop"));
      }
      Ok(())
    }
    Stmt::Next => {
      if !in_loop {
        return Err(Diagnostic::new("`next` outside of a loop"));
      }
      Ok(())
    }
    Stmt::Expr(e) => infer_expr_type(e, env, sigs).map(|_| ()),
  }
}

fn check_block(
  stmts: &[Stmt],
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  return_type: Type,
  in_loop: bool,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    check_stmt(stmt, env, sigs, return_type, in_loop)?;
  }
  Ok(())
}

fn check_function_body(
  f: &Function,
  sigs: &HashMap<String, FunctionSig>,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &f.params {
    env.insert(p.name.clone(), resolve_type_name(&p.ty)?);
  }
  let declared_return = resolve_type_name(&f.return_type)?;
  check_block(&f.body, &mut env, sigs, declared_return, false)?;

  // Implicit-return check: a function whose body's last statement is a
  // bare expression returns that expression's value (Ruby-style implicit
  // return), matching inception §17's `add` example. Every explicit
  // `return` was already checked against `declared_return` in check_stmt
  // above, regardless of position.
  if let Some(Stmt::Expr(e)) = f.body.last() {
    let t = infer_expr_type(e, &env, sigs)?;
    if t != declared_return {
      return Err(Diagnostic::new(format!(
        "type mismatch in function `{}`: body has type {t:?} but declared return type is {declared_return:?}",
        f.name
      )));
    }
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

  let mut top_env: HashMap<String, Type> = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        if let Err(d) = check_function_body(f, &sigs) {
          diags.push(d);
        }
      }
      // Top-level statements share one environment across the whole
      // program in order (`x: Int64 = 10` then `if x > 5 ...` needs `x`
      // visible) and have no enclosing function return type — Void is a
      // safe sentinel since a bare `return` at top level is not exercised
      // by this milestone.
      Item::Stmt(s) => {
        if let Err(d) = check_stmt(s, &mut top_env, &sigs, Type::Void, false) {
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

  const MILESTONE2: &str = "x: Int64 = 10\n\nif x > 5\n  puts x\nend\n";

  #[test]
  fn accepts_inception_milestone2_example() {
    let program = emerald_parser::parse(MILESTONE2).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_boolean_if_condition() {
    let src = "x: Int64 = 10\n\nif x\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject non-Boolean if condition");
    assert!(errs[0].message.contains("must be Boolean"));
  }

  #[test]
  fn rejects_break_outside_loop() {
    let src = "break\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject break outside a loop");
    assert!(errs[0].message.contains("outside of a loop"));
  }

  #[test]
  fn accepts_while_loop_with_break() {
    let src = "x: Int64 = 0\n\nwhile x < 3\n  x: Int64 = x + 1\n  break\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }
}
