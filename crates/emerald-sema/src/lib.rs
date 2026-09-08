//! Name resolution + type checking over `emerald_parser::Program`
//! (plan-of-plans row 05, inception §17 steps 4–7 and §25.E).
//!
//! No source-span tracking yet (`crates/emerald-lexer`/`emerald-parser`
//! don't carry spans) — diagnostics are function/call-scoped text.
//! Line/column-precise diagnostics are `13 diagnostics`'s job.

use emerald_parser::{ClassDef, Expr, Function, Item, ModuleDef, Param, Program, Stmt};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
  Int64,
  Float64,
  String,
  Boolean,
  Void,
  /// An instance of a user-defined class, named by its declaration.
  Class(String),
  /// A packed, contiguous array of a single element type
  /// (`spec/TYPE_SYSTEM.md` §8) — named `Array[Elem]` at the source level.
  Array(Box<Type>),
  /// A closure's parameter types and return type. Unlike `Type::Class`,
  /// which is just a name backed by a separate `ClassInfo` registry,
  /// there's no such registry for lambdas — the signature has to travel
  /// with the type value itself (plan 10's Decision log). The
  /// *source-level* annotation is always the bare keyword `Proc`; the
  /// real signature here always comes from the `Expr::Lambda` a `Proc`
  /// local was bound to (see `check_stmt`'s `Let` case).
  Proc(Vec<Type>, Box<Type>),
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

#[derive(Debug, Clone)]
struct FunctionSig {
  params: Vec<Type>,
  return_type: Type,
}

/// Shared by classes and modules (plan 12's Decision log — modules reuse
/// this registry, tagged `is_module: true`, rather than a separate
/// `modules` map threaded through every function that already carries
/// `classes`). A module's `fields` is always empty (`SEMANTICS.md`
/// §10.4 — no instance state to hold them).
#[derive(Debug, Clone)]
struct ClassInfo {
  fields: HashMap<String, Type>,
  methods: HashMap<String, FunctionSig>,
  is_module: bool,
}

/// Resolves a type name against `spec/TYPE_SYSTEM.md`'s primitives, then
/// against declared classes. Errors on any unrecognized name rather than
/// silently accepting it.
fn resolve_type(name: &str, classes: &HashMap<String, ClassInfo>) -> Result<Type, Diagnostic> {
  match name {
    "Int64" => Ok(Type::Int64),
    "Float64" => Ok(Type::Float64),
    "String" => Ok(Type::String),
    "Void" => Ok(Type::Void),
    "Boolean" => Ok(Type::Boolean),
    // Modules are namespaces, not types (plan 12's Decision log) — a
    // module name is excluded here so `x: MathUtils = ...` correctly
    // falls through to the `unknown type` error below, not `Type::Class`.
    other if classes.get(other).is_some_and(|c| !c.is_module) => Ok(Type::Class(other.to_string())),
    // The grammar hands compound array annotations over as a plain
    // `"Array[Elem]"` string (plan 09's Decision log — no structured
    // type-annotation AST node yet), so this is where it turns into
    // `Type::Array`. Recurses on `Elem` so `Array[Array[Int64]]` works
    // for free, even though nothing exercises it yet.
    other if other.starts_with("Array[") && other.ends_with(']') => {
      let elem_name = &other["Array[".len()..other.len() - 1];
      let elem_ty = resolve_type(elem_name, classes)?;
      Ok(Type::Array(Box::new(elem_ty)))
    }
    // A bare `Proc` annotation carries no signature (see `Type::Proc`'s
    // doc comment) — this opaque placeholder is only ever reached outside
    // `check_stmt`'s `Let` special case (which instead stores the real
    // signature straight from the bound `Expr::Lambda`), e.g. if `Proc`
    // were used as a function parameter/return type, which this plan
    // doesn't exercise.
    "Proc" => Ok(Type::Proc(Vec::new(), Box::new(Type::Void))),
    other => Err(Diagnostic::new(format!("unknown type `{other}`"))),
  }
}

fn function_signature(
  f: &Function,
  classes: &HashMap<String, ClassInfo>,
) -> Result<FunctionSig, Diagnostic> {
  let params = f
    .params
    .iter()
    .map(|p| resolve_type(&p.ty, classes))
    .collect::<Result<Vec<_>, _>>()?;
  let return_type = resolve_type(&f.return_type, classes)?;
  Ok(FunctionSig {
    params,
    return_type,
  })
}

/// Builds one class's field/method tables. `classes` must already contain
/// an entry for every class name this class's fields/methods reference
/// (including, trivially, itself — see `check_program`'s two-pass
/// registration).
fn class_info(c: &ClassDef, classes: &HashMap<String, ClassInfo>) -> Result<ClassInfo, Diagnostic> {
  let mut fields = HashMap::new();
  for f in &c.fields {
    fields.insert(f.name.clone(), resolve_type(&f.ty, classes)?);
  }
  let mut methods = HashMap::new();
  for m in &c.methods {
    methods.insert(m.name.clone(), function_signature(m, classes)?);
  }
  Ok(ClassInfo {
    fields,
    methods,
    is_module: false,
  })
}

/// A module's method table — built via the exact same `function_signature`
/// every free function's signature already goes through (plan 12's
/// Decision log: a module method type-checks like a free function,
/// because it is one, just namespaced). Always empty `fields`.
fn module_info(
  m: &ModuleDef,
  classes: &HashMap<String, ClassInfo>,
) -> Result<ClassInfo, Diagnostic> {
  let mut methods = HashMap::new();
  for f in &m.methods {
    methods.insert(f.name.clone(), function_signature(f, classes)?);
  }
  Ok(ClassInfo {
    fields: HashMap::new(),
    methods,
    is_module: true,
  })
}

/// `Sub`/`Mul`/`Div`/`Rem` each apply the exact rule `Add` already
/// enforces (plan 18's Decision log): both operands must resolve to the
/// same numeric type, no implicit conversion.
#[allow(clippy::too_many_arguments)]
fn check_numeric_binop(
  op: &str,
  lhs: &Expr,
  rhs: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields)?;
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields)?;
  if lt != rt {
    return Err(Diagnostic::new(format!(
      "type mismatch: `{op}` requires both operands to have the same type, found {lt:?} and {rt:?}"
    )));
  }
  if lt != Type::Int64 && lt != Type::Float64 {
    return Err(Diagnostic::new(format!(
      "type `{lt:?}` does not support `{op}`"
    )));
  }
  Ok(lt)
}

/// `&&`/`||` require both operands `Boolean`, return `Boolean`.
#[allow(clippy::too_many_arguments)]
fn check_boolean_binop(
  op: &str,
  lhs: &Expr,
  rhs: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields)?;
  if lt != Type::Boolean {
    return Err(Diagnostic::new(format!(
      "`{op}` requires a Boolean left operand, found {lt:?}"
    )));
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields)?;
  if rt != Type::Boolean {
    return Err(Diagnostic::new(format!(
      "`{op}` requires a Boolean right operand, found {rt:?}"
    )));
  }
  Ok(Type::Boolean)
}

/// `self_fields` is `Some(&class.fields)` while checking a method body,
/// `None` everywhere else — gates `@field` legality (plan 08 AC4).
#[allow(clippy::too_many_arguments)]
fn infer_expr_type(
  expr: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<Type, Diagnostic> {
  match expr {
    Expr::Ident(name) => env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"))),
    Expr::Int(_) => Ok(Type::Int64),
    Expr::Float(_) => Ok(Type::Float64),
    Expr::Add(lhs, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields)?;
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields)?;
      if lt != rt {
        return Err(Diagnostic::new(format!(
          "type mismatch: `+` requires both operands to have the same type, found {lt:?} and {rt:?}"
        )));
      }
      if lt != Type::Int64 && lt != Type::Float64 {
        return Err(Diagnostic::new(format!(
          "type `{lt:?}` does not support `+`"
        )));
      }
      Ok(lt)
    }
    Expr::Sub(lhs, rhs) => check_numeric_binop("-", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Mul(lhs, rhs) => check_numeric_binop("*", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Div(lhs, rhs) => check_numeric_binop("/", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Rem(lhs, rhs) => check_numeric_binop("%", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Neg(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
      if t != Type::Int64 && t != Type::Float64 {
        return Err(Diagnostic::new(format!(
          "type `{t:?}` does not support unary `-`"
        )));
      }
      Ok(t)
    }
    Expr::Not(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
      if t != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`!` requires a Boolean operand, found {t:?}"
        )));
      }
      Ok(Type::Boolean)
    }
    Expr::And(lhs, rhs) => check_boolean_binop("&&", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Or(lhs, rhs) => check_boolean_binop("||", lhs, rhs, env, sigs, classes, self_fields),
    Expr::Compare(lhs, op, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields)?;
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields)?;
      if lt != rt {
        return Err(Diagnostic::new(format!(
          "type mismatch: `{op:?}` requires both operands to have the same type, found {lt:?} and {rt:?}"
        )));
      }
      Ok(Type::Boolean)
    }
    // `puts` is a compiler intrinsic, not an overloaded function (locks
    // spec/SEMANTICS.md §3's "no overloading in v1") — it accepts exactly
    // one Int64 or Float64 argument, checked here directly rather than via
    // a `FunctionSig` in `sigs` (see plan 08's Decision log).
    Expr::Call(name, args) if name == "puts" => {
      if args.len() != 1 {
        return Err(Diagnostic::new(format!(
          "`puts` expects 1 argument, found {}",
          args.len()
        )));
      }
      let arg_ty = infer_expr_type(&args[0], env, sigs, classes, self_fields)?;
      if arg_ty != Type::Int64 && arg_ty != Type::Float64 {
        return Err(Diagnostic::new(format!(
          "`puts` does not support type {arg_ty:?}"
        )));
      }
      Ok(Type::Void)
    }
    Expr::Call(name, args) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`")))?;
      check_args(name, args, &sig.params, env, sigs, classes, self_fields)?;
      Ok(sig.return_type.clone())
    }
    Expr::New(class_name, args) => {
      let info = classes
        .get(class_name)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`")))?;
      if info.is_module {
        return Err(Diagnostic::new(format!(
          "cannot `.new` module `{class_name}` — modules are namespaces, not instantiable"
        )));
      }
      match info.methods.get("initialize") {
        Some(sig) => check_args(
          "initialize",
          args,
          &sig.params,
          env,
          sigs,
          classes,
          self_fields,
        )?,
        None if args.is_empty() => {}
        None => {
          return Err(Diagnostic::new(format!(
            "`{class_name}.new` called with {} argument(s), but `{class_name}` declares no `initialize`",
            args.len()
          )));
        }
      }
      Ok(Type::Class(class_name.clone()))
    }
    // `Name.method(args)` on a module (plan 12) dispatches straight to
    // its method table — checked *before* the `.call`/`Type::Proc` arm
    // below (a module could in principle declare a method named `call`)
    // and before `infer_expr_type(recv)` runs at all, since a bare
    // module reference isn't a value — evaluating it as one would fail
    // with "undefined variable" (a module name is never in `env`).
    Expr::MethodCall(recv, method, args) if matches!(recv.as_ref(), Expr::Ident(n) if classes.get(n).is_some_and(|c| c.is_module)) =>
    {
      let Expr::Ident(module_name) = recv.as_ref() else {
        unreachable!()
      };
      let info = &classes[module_name];
      let sig = info.methods.get(method).ok_or_else(|| {
        Diagnostic::new(format!("module `{module_name}` has no method `{method}`"))
      })?;
      check_args(method, args, &sig.params, env, sigs, classes, self_fields)?;
      Ok(sig.return_type.clone())
    }
    // `.call` on a `Proc`-typed receiver dispatches against the
    // signature carried directly on `Type::Proc` (plan 10) — everything
    // else falls through to the existing `Type::Class` method lookup.
    Expr::MethodCall(recv, method, args) if method == "call" => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields)?;
      let Type::Proc(param_types, return_type) = &recv_ty else {
        return Err(Diagnostic::new(format!(
          "method call `.call` on non-Proc type {recv_ty:?}"
        )));
      };
      check_args("call", args, param_types, env, sigs, classes, self_fields)?;
      Ok((**return_type).clone())
    }
    Expr::MethodCall(recv, method, args) => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields)?;
      let Type::Class(class_name) = &recv_ty else {
        return Err(Diagnostic::new(format!(
          "method call `.{method}` on non-class type {recv_ty:?}"
        )));
      };
      let info = classes.get(class_name).ok_or_else(|| {
        Diagnostic::new(format!("internal error: unregistered class `{class_name}`"))
      })?;
      let sig = info
        .methods
        .get(method)
        .ok_or_else(|| Diagnostic::new(format!("class `{class_name}` has no method `{method}`")))?;
      check_args(method, args, &sig.params, env, sigs, classes, self_fields)?;
      Ok(sig.return_type.clone())
    }
    Expr::InstanceVar(name) => {
      let fields = self_fields
        .ok_or_else(|| Diagnostic::new(format!("`@{name}` used outside of a method body")))?;
      fields
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined field `@{name}`")))
    }
    // Empty arrays are rejected (plan 09's Decision log): with no
    // structured type annotation on the literal itself, an empty
    // `[]` has no element type to infer — `Array[T]`'s declared `T`
    // on the enclosing `Let` isn't visible from here.
    Expr::ArrayLit(elements) => infer_array_lit_type(elements, env, sigs, classes, self_fields),
    Expr::Index(array, index) => {
      let array_ty = infer_expr_type(array, env, sigs, classes, self_fields)?;
      let Type::Array(elem_ty) = array_ty else {
        return Err(Diagnostic::new(format!(
          "`[...]` indexing requires an Array, found {array_ty:?}"
        )));
      };
      let index_ty = infer_expr_type(index, env, sigs, classes, self_fields)?;
      if index_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "array index must be Int64, found {index_ty:?}"
        )));
      }
      Ok(*elem_ty)
    }
    Expr::Lambda {
      params,
      return_type,
      body,
    } => infer_lambda_type(params, return_type, body, env, sigs, classes),
  }
}

/// A lambda body is checked exactly like a function body — the outer
/// scope's locals plus the lambda's own params, no `@field` access (`None`
/// self_fields; plan 10's Decision log restricts lambdas to top-level
/// `Let`s, where there's no enclosing method anyway).
fn infer_lambda_type(
  params: &[Param],
  return_type: &str,
  body: &[Stmt],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Type, Diagnostic> {
  let mut lambda_env = env.clone();
  let mut param_types = Vec::with_capacity(params.len());
  for p in params {
    let t = resolve_type(&p.ty, classes)?;
    param_types.push(t.clone());
    lambda_env.insert(p.name.clone(), t);
  }
  let declared_return = resolve_type(return_type, classes)?;
  check_block(
    body,
    &mut lambda_env,
    sigs,
    classes,
    None,
    &declared_return,
    false,
  )?;
  check_implicit_return(
    body,
    &lambda_env,
    sigs,
    classes,
    None,
    &declared_return,
    "<lambda>",
  )?;
  Ok(Type::Proc(param_types, Box::new(declared_return)))
}

/// All elements of an array literal must share one type, and — since
/// there's no structured type annotation on the literal itself to fall
/// back on — the literal can't be empty (plan 09's Decision log).
fn infer_array_lit_type(
  elements: &[Expr],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<Type, Diagnostic> {
  let Some((first, rest)) = elements.split_first() else {
    return Err(Diagnostic::new(
      "empty array literals are not supported — the element type can't be inferred",
    ));
  };
  let elem_ty = infer_expr_type(first, env, sigs, classes, self_fields)?;
  for (i, e) in rest.iter().enumerate() {
    let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
    if t != elem_ty {
      return Err(Diagnostic::new(format!(
        "array literal element {} has type {t:?}, expected {elem_ty:?} (all elements must share one type)",
        i + 2
      )));
    }
  }
  Ok(Type::Array(Box::new(elem_ty)))
}

#[allow(clippy::too_many_arguments)]
fn check_args(
  name: &str,
  args: &[Expr],
  expected: &[Type],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<(), Diagnostic> {
  if args.len() != expected.len() {
    return Err(Diagnostic::new(format!(
      "`{name}` expects {} argument(s), found {}",
      expected.len(),
      args.len()
    )));
  }
  for (i, (arg, expected_ty)) in args.iter().zip(expected).enumerate() {
    let actual = infer_expr_type(arg, env, sigs, classes, self_fields)?;
    if actual != *expected_ty {
      return Err(Diagnostic::new(format!(
        "argument {} to `{name}` has type {actual:?}, expected {expected_ty:?}",
        i + 1
      )));
    }
  }
  Ok(())
}

/// `arr[i] = value` — array element must be Int64-indexed and the RHS
/// must match the array's element type.
#[allow(clippy::too_many_arguments)]
fn check_set_index(
  array: &Expr,
  index: &Expr,
  value: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
) -> Result<(), Diagnostic> {
  let array_ty = infer_expr_type(array, env, sigs, classes, self_fields)?;
  let Type::Array(elem_ty) = array_ty else {
    return Err(Diagnostic::new(format!(
      "`[...] = ...` indexing requires an Array, found {array_ty:?}"
    )));
  };
  let index_ty = infer_expr_type(index, env, sigs, classes, self_fields)?;
  if index_ty != Type::Int64 {
    return Err(Diagnostic::new(format!(
      "array index must be Int64, found {index_ty:?}"
    )));
  }
  let actual = infer_expr_type(value, env, sigs, classes, self_fields)?;
  if actual != *elem_ty {
    return Err(Diagnostic::new(format!(
      "type mismatch in array assignment: element type is {elem_ty:?}, value has type {actual:?}"
    )));
  }
  Ok(())
}

/// Type-checks one statement, threading a mutable local-variable
/// environment and the enclosing function's declared return type (used to
/// check every `return <expr>`, not just a trailing one). `in_loop` gates
/// `break`/`next` legality. `self_fields` is `Some` only inside a method
/// body (see `infer_expr_type`).
#[allow(clippy::too_many_arguments)]
fn check_stmt(
  stmt: &Stmt,
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
) -> Result<(), Diagnostic> {
  match stmt {
    // `Proc` is special-cased: the bare annotation carries no signature
    // (see `Type::Proc`'s doc comment), so instead of comparing against
    // `resolve_type("Proc", ...)`'s opaque placeholder, any actual
    // `Type::Proc(_, _)` is accepted and *that* — the real signature
    // inferred from the bound `Expr::Lambda` — is what's stored in `env`.
    Stmt::Let { name, ty, value } if ty == "Proc" => {
      let actual = infer_expr_type(value, env, sigs, classes, self_fields)?;
      if !matches!(actual, Type::Proc(_, _)) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name}: Proc = ...`: expected a Proc (lambda literal), found {actual:?}"
        )));
      }
      env.insert(name.clone(), actual);
      Ok(())
    }
    Stmt::Let { name, ty, value } => {
      let declared = resolve_type(ty, classes)?;
      let actual = infer_expr_type(value, env, sigs, classes, self_fields)?;
      if actual != declared {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name}: {ty} = ...`: declared type {declared:?}, value has type {actual:?}"
        )));
      }
      env.insert(name.clone(), declared);
      Ok(())
    }
    Stmt::SetField { name, value } => {
      let fields = self_fields
        .ok_or_else(|| Diagnostic::new(format!("`@{name} = ...` used outside of a method body")))?;
      let declared = fields
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined field `@{name}`")))?;
      let actual = infer_expr_type(value, env, sigs, classes, self_fields)?;
      if actual != declared {
        return Err(Diagnostic::new(format!(
          "type mismatch in `@{name} = ...`: field declared {declared:?}, value has type {actual:?}"
        )));
      }
      Ok(())
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => check_set_index(array, index, value, env, sigs, classes, self_fields),
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`if` condition must be Boolean, found {cond_ty:?} (no truthy/falsy coercion — spec/GRAMMAR.md §5)"
        )));
      }
      check_block(
        then_branch,
        env,
        sigs,
        classes,
        self_fields,
        return_type,
        in_loop,
      )?;
      if let Some(else_b) = else_branch {
        check_block(
          else_b,
          env,
          sigs,
          classes,
          self_fields,
          return_type,
          in_loop,
        )?;
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`while` condition must be Boolean, found {cond_ty:?}"
        )));
      }
      check_block(body, env, sigs, classes, self_fields, return_type, true)
    }
    Stmt::Return(Some(e)) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
      if t != *return_type {
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
    Stmt::Expr(e) => infer_expr_type(e, env, sigs, classes, self_fields).map(|_| ()),
    Stmt::Raise(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
      if !matches!(t, Type::Class(_)) {
        return Err(Diagnostic::new(format!(
          "`raise` requires a class instance, found {t:?}"
        )));
      }
      Ok(())
    }
    Stmt::Begin {
      body,
      rescue_type,
      rescue_var,
      rescue_body,
    } => check_begin(
      body,
      rescue_type,
      rescue_var,
      rescue_body,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
    ),
  }
}

/// `begin body rescue Type => e rescue_body end`. Flat scoping, same as
/// everything else in this compiler (plan 07's Decision log) —
/// `rescue_var` joins the same environment an `if`/`while` body's `Let`s
/// already flow into, not a fresh scope.
#[allow(clippy::too_many_arguments)]
fn check_begin(
  body: &[Stmt],
  rescue_type: &str,
  rescue_var: &str,
  rescue_body: &[Stmt],
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
) -> Result<(), Diagnostic> {
  check_block(body, env, sigs, classes, self_fields, return_type, in_loop)?;
  let rescue_ty = resolve_type(rescue_type, classes)?;
  if !matches!(rescue_ty, Type::Class(_)) {
    return Err(Diagnostic::new(format!(
      "`rescue {rescue_type}` must name a class, found {rescue_ty:?}"
    )));
  }
  env.insert(rescue_var.to_string(), rescue_ty);
  check_block(
    rescue_body,
    env,
    sigs,
    classes,
    self_fields,
    return_type,
    in_loop,
  )
}

#[allow(clippy::too_many_arguments)]
fn check_block(
  stmts: &[Stmt],
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    check_stmt(stmt, env, sigs, classes, self_fields, return_type, in_loop)?;
  }
  Ok(())
}

/// Checks the final-statement implicit-return rule shared by free
/// functions and methods (Ruby-style: a body whose last statement is a
/// bare expression returns that expression's value).
fn check_implicit_return(
  body: &[Stmt],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  declared_return: &Type,
  owner_name: &str,
) -> Result<(), Diagnostic> {
  if let Some(Stmt::Expr(e)) = body.last() {
    let t = infer_expr_type(e, env, sigs, classes, self_fields)?;
    if t != *declared_return {
      return Err(Diagnostic::new(format!(
        "type mismatch in `{owner_name}`: body has type {t:?} but declared return type is {declared_return:?}"
      )));
    }
  }
  Ok(())
}

fn check_function_body(
  f: &Function,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &f.params {
    env.insert(p.name.clone(), resolve_type(&p.ty, classes)?);
  }
  let declared_return = resolve_type(&f.return_type, classes)?;
  check_block(
    &f.body,
    &mut env,
    sigs,
    classes,
    None,
    &declared_return,
    false,
  )?;
  check_implicit_return(
    &f.body,
    &env,
    sigs,
    classes,
    None,
    &declared_return,
    &f.name,
  )
}

fn check_method_body(
  class_name: &str,
  m: &Function,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  fields: &HashMap<String, Type>,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &m.params {
    env.insert(p.name.clone(), resolve_type(&p.ty, classes)?);
  }
  let declared_return = resolve_type(&m.return_type, classes)?;
  check_block(
    &m.body,
    &mut env,
    sigs,
    classes,
    Some(fields),
    &declared_return,
    false,
  )?;
  check_implicit_return(
    &m.body,
    &env,
    sigs,
    classes,
    Some(fields),
    &declared_return,
    &format!("{class_name}#{}", m.name),
  )
}

/// Type-checks a full program: registers classes (two sub-passes — names
/// first so field/method types can reference class names, then full
/// field/method tables) and free-function signatures, then checks every
/// function body, every class's method bodies, and every top-level
/// statement. `puts` is handled directly in `infer_expr_type`, not
/// registered here (see plan 08's Decision log) — free functions are
/// enough two-pass ordering for a top-level call to a function defined
/// later in the same `Item*` list to still resolve.
pub fn check_program(program: &Program) -> Result<(), Vec<Diagnostic>> {
  let mut diags = Vec::new();

  // Modules register into the same two-pass table as classes (plan 12's
  // Decision log) — names first (so a class/module's own fields/methods
  // can reference any other class/module name regardless of declaration
  // order), then full field/method tables.
  let mut classes: HashMap<String, ClassInfo> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      classes.insert(
        c.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
        },
      );
    }
    if let Item::Module(m) = item {
      classes.insert(
        m.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: true,
        },
      );
    }
  }
  for item in &program.items {
    if let Item::Class(c) = item {
      match class_info(c, &classes) {
        Ok(info) => {
          classes.insert(c.name.clone(), info);
        }
        Err(d) => diags.push(d),
      }
    }
    if let Item::Module(m) = item {
      match module_info(m, &classes) {
        Ok(info) => {
          classes.insert(m.name.clone(), info);
        }
        Err(d) => diags.push(d),
      }
    }
  }

  let mut sigs: HashMap<String, FunctionSig> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      match function_signature(f, &classes) {
        Ok(sig) => {
          sigs.insert(f.name.clone(), sig);
        }
        Err(d) => diags.push(d),
      }
    }
  }

  // Declared once, outside the loop: top-level statements share one
  // environment across the whole program in order (`x: Int64 = 10` then
  // `if x > 5 ...` needs `x` visible in a later Item::Stmt).
  let mut top_env: HashMap<String, Type> = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        if let Err(d) = check_function_body(f, &sigs, &classes) {
          diags.push(d);
        }
      }
      Item::Class(c) => {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        for m in &c.methods {
          if let Err(d) = check_method_body(&c.name, m, &sigs, &classes, &info.fields) {
            diags.push(d);
          }
        }
      }
      // A module method type-checks exactly like a free function (plan
      // 12's Decision log) — no `self`, no `@field` access.
      Item::Module(m) => {
        for f in &m.methods {
          if let Err(d) = check_function_body(f, &sigs, &classes) {
            diags.push(d);
          }
        }
      }
      // No enclosing function return type (Void is a safe sentinel — a
      // bare `return` at top level is not exercised by this milestone),
      // and never inside a method (`self_fields: None`).
      Item::Stmt(s) => {
        if let Err(d) = check_stmt(s, &mut top_env, &sigs, &classes, None, &Type::Void, false) {
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

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def sum -> Float64\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn accepts_inception_point_example() {
    let program = emerald_parser::parse(POINT_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_field_assignment_type_mismatch() {
    let src =
      "class Point\n  x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = 1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 assigned to a Float64 field");
    let msg = &errs[0].message;
    assert!(
      msg.contains("Float64") && msg.contains("Int64"),
      "diagnostic should name both types: {msg}"
    );
  }

  #[test]
  fn rejects_instance_var_outside_method_body() {
    let src = "puts @x\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject @x outside a method body");
    assert!(errs[0].message.contains("outside of a method body"));
  }

  #[test]
  fn rejects_undeclared_method_call() {
    let src = "class Point\n  x: Float64\nend\n\np: Point = Point.new()\nputs p.missing\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a call to an undeclared method");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("has no method `missing`"))
    );
  }

  #[test]
  fn rejects_method_call_on_non_class_receiver() {
    let src = "x: Int64 = 5\nputs x.sum\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject .sum on an Int64 receiver");
    assert!(errs.iter().any(|d| d.message.contains("non-class type")));
  }

  const ARRAY_EXAMPLE: &str =
    "arr: Array[Int64] = [10, 20, 30]\narr[1] = 99\nputs arr[1]\nputs arr[0] + arr[2]\n";

  #[test]
  fn accepts_array_literal_index_read_and_write() {
    let program = emerald_parser::parse(ARRAY_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_empty_array_literal() {
    let src = "arr: Array[Int64] = []\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty array literal");
    assert!(errs[0].message.contains("empty array literals"));
  }

  #[test]
  fn rejects_mixed_type_array_literal() {
    let src = "arr: Array[Int64] = [1, 2.0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a mixed-type array literal");
    assert!(errs[0].message.contains("all elements must share one type"));
  }

  #[test]
  fn rejects_non_int64_array_index() {
    let src = "arr: Array[Int64] = [1, 2]\nputs arr[2.0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a non-Int64 array index");
    assert!(errs[0].message.contains("array index must be Int64"));
  }

  #[test]
  fn rejects_array_element_type_mismatch_on_write() {
    let src = "arr: Array[Int64] = [1, 2]\narr[0] = 2.0\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject writing a Float64 into an Array[Int64]");
    assert!(
      errs[0]
        .message
        .contains("type mismatch in array assignment")
    );
  }

  #[test]
  fn rejects_indexing_a_non_array() {
    let src = "x: Int64 = 5\nputs x[0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject indexing a non-Array");
    assert!(errs[0].message.contains("requires an Array"));
  }

  const LAMBDA_EXAMPLE: &str =
    "x: Int64 = 10\nadd_x: Proc = ->(y: Int64) -> Int64 { y + x }\nputs add_x.call(5)\n";

  #[test]
  fn accepts_lambda_capture_and_call() {
    let program = emerald_parser::parse(LAMBDA_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_arity_mismatch() {
    let src = "add_x: Proc = ->(y: Int64) -> Int64 { y }\nputs add_x.call(5, 6)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 2-arg call to a 1-param Proc");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("expects 1 argument"))
    );
  }

  #[test]
  fn rejects_call_argument_type_mismatch() {
    let src = "add_x: Proc = ->(y: Int64) -> Int64 { y }\nputs add_x.call(1.5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a Float64 argument to an Int64 param");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("Int64") && d.message.contains("Float64"))
    );
  }

  #[test]
  fn rejects_call_on_non_proc_receiver() {
    let src = "x: Int64 = 5\nputs x.call(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject .call on a non-Proc receiver");
    assert!(errs[0].message.contains("non-Proc type"));
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn accepts_raise_and_rescue() {
    let program = emerald_parser::parse(EXCEPTION_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_raise_of_non_class_value() {
    let src = "raise 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject raising a non-class value");
    assert!(
      errs[0]
        .message
        .contains("`raise` requires a class instance")
    );
  }

  #[test]
  fn rejects_rescue_naming_a_non_class_type() {
    let src = "begin\n  puts 1\nrescue Int64 => e\n  puts e\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `rescue Int64`");
    assert!(errs[0].message.contains("must name a class"));
  }

  const MODULE_EXAMPLE: &str = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";

  #[test]
  fn accepts_module_namespaced_call() {
    let program = emerald_parser::parse(MODULE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_instantiating_a_module() {
    let src = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.new()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `.new` on a module");
    assert!(errs[0].message.contains("cannot `.new` module"));
  }

  #[test]
  fn rejects_undeclared_module_method() {
    let src = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.missing(1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undeclared module method");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("has no method `missing`"))
    );
  }

  #[test]
  fn rejects_module_call_arity_mismatch() {
    let src = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.double(1, 2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a 2-arg call to a 1-param module method");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("expects 1 argument"))
    );
  }

  #[test]
  fn rejects_module_used_as_a_type_annotation() {
    let src = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nx: MathUtils = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a module used as a type annotation");
    assert!(errs[0].message.contains("unknown type"));
  }

  // Plan 18 (arithmetic & logical operators).

  const ARITHMETIC_EXAMPLE: &str = "def factorial(n: Int64) -> Int64\n  if n <= 1\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";
  const SHORT_CIRCUIT_EXAMPLE: &str = "def noisy(n: Int64) -> Boolean\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1)\n  puts 100\nend\nif x > -10 && noisy(3)\n  puts 300\nend\nif x < 0 || noisy(2)\n  puts 200\nend\nif x > 0 || noisy(4)\n  puts 400\nend\n";

  #[test]
  fn accepts_arithmetic_example() {
    let program = emerald_parser::parse(ARITHMETIC_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_short_circuit_example() {
    let program = emerald_parser::parse(SHORT_CIRCUIT_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_mismatched_numeric_sub_types() {
    let src = "puts 5 - 2.0\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 - Float64");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("Float64"));
  }

  #[test]
  fn rejects_not_on_non_boolean_operand() {
    let src = "puts !5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `!5`");
    assert!(errs[0].message.contains("Boolean"));
  }

  #[test]
  fn rejects_and_with_non_boolean_left_operand() {
    // No parenthesized-grouping production exists in this grammar (a
    // real, disclosed gap outside plan 18's scope) — `5 && 3 > 1`
    // still exercises the intended shape since `&&` binds looser than
    // comparison, so the right operand is `3 > 1` either way.
    let src = "if 5 && 3 > 1\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `5 && ...`");
    assert!(errs[0].message.contains("Boolean"));
  }

  #[test]
  fn sub_and_neg_agree_on_int64() {
    let src = "x: Int64 = 0 - 5\ny: Int64 = -5\nputs x\nputs y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn sub_and_neg_agree_on_float64() {
    let src = "x: Float64 = 0.0 - 5.0\ny: Float64 = -5.0\nputs x\nputs y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }
}
