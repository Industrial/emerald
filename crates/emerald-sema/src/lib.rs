//! Name resolution + type checking over `emerald_parser::Program`
//! (plan-of-plans row 05, inception §17 steps 4–7 and §25.E).
//!
//! No source-span tracking yet (`crates/emerald-lexer`/`emerald-parser`
//! don't carry spans) — diagnostics are function/call-scoped text.
//! Line/column-precise diagnostics are `13 diagnostics`'s job.

use emerald_parser::{
  ClassDef, CompareOp, Expr, Function, Item, ModuleDef, Param, Program, RescueClause, Stmt,
  StringPart,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
  Int64,
  Float64,
  String,
  Boolean,
  Void,
  /// `:foo` (plan 44's Decision log) — a genuinely separate type from
  /// `String`, not an alias: every distinct spelling appearing anywhere
  /// in a compilation unit is enumerable at parse time, so equality is
  /// a cheap interned-integer compare in codegen (`ValKind::Symbol`),
  /// not a string compare.
  Symbol,
  /// Plan 25's Decision log: deliberately narrow — no `T?` nullable-type
  /// system, just a bare, standalone type a `nil` literal produces.
  Nil,
  /// An instance of a user-defined class, named by its declaration.
  Class(String),
  /// A packed, contiguous array of a single element type
  /// (`spec/TYPE_SYSTEM.md` §8) — named `Array[Elem]` at the source level.
  Array(Box<Type>),
  /// A key/value container, named `Hash[K, V]` at the source level
  /// (plan 25's Decision log: `Int64` keys only, a flat linear-scan
  /// representation in codegen — not a real hash table).
  Hash(Box<Type>, Box<Type>),
  /// A closure's parameter types and return type. Unlike `Type::Class`,
  /// which is just a name backed by a separate `ClassInfo` registry,
  /// there's no such registry for lambdas — the signature has to travel
  /// with the type value itself (plan 10's Decision log). The
  /// *source-level* annotation is always the bare keyword `Proc`; the
  /// real signature here always comes from the `Expr::Lambda` a `Proc`
  /// local was bound to (see `check_stmt`'s `Let` case).
  Proc(Vec<Type>, Box<Type>),
  /// A not-yet-concrete type-parameter reference (plan 41's Decision
  /// log) — legal only inside the body of the generic function that
  /// declares it. Carries both the type parameter's own name (`"T"`,
  /// used purely for diagnostics — e.g. naming which parameter
  /// disagreed at a call site) and its bound interface's name
  /// (`"Comparable"`), so a method call on a `Generic`-typed receiver
  /// (`a.compare_to(b)`) can resolve the interface's required method
  /// directly from the receiver's own inferred type, without a
  /// separately threaded "which interface bounds the parameter
  /// currently in scope" context value.
  Generic(String, String),
  /// `T?` (plan 43's Decision log) — the union of `T` and `Nil`,
  /// scoped to reference types only (`Class`/`String`/`Array`/`Hash` —
  /// every kind that already lowers to a pointer-backed `ValKind` in
  /// codegen, which has a spare `null` bit pattern to spend on nilness
  /// for free). `resolve_type` never constructs this over `Int64`/
  /// `Float64`/`Boolean`/`Proc`/`Nil`/`Generic` — boxing a value type
  /// just to steal a spare bit is the exact cost this project already
  /// declines to pay for arbitrary-precision `Integer` (`spec/
  /// TYPE_SYSTEM.md` §3).
  Nullable(Box<Type>),
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
  /// Plan 34's `&blk` marker, carried alongside the ordinary signature
  /// so a call site can tell whether it must attach a trailing block
  /// literal — mirrors `Function.block_param` (see its own doc comment
  /// for why this is a bare name, not a checkable `Type::Proc`).
  block_param: Option<String>,
  /// Plan 39's Decision log: `params[i]`'s declared name, parallel to
  /// `params`/`defaults` — lets a keyword-argument call site (`Expr::
  /// CallKw`) resolve each supplied name to its position, entirely at
  /// compile time.
  param_names: Vec<String>,
  /// Plan 39's Decision log: `params[i]`'s default value expression, if
  /// declared (`None` for every parameter without one — always `None`
  /// for every pre-plan-39 declaration). Filled into a call site's
  /// trailing omitted arguments, both positional and keyword.
  defaults: Vec<Option<Expr>>,
  /// Plan 39's Decision log: `Some(elem_ty)` when this function declares
  /// a trailing `*xs: Elem` splat parameter — every call-site argument
  /// beyond `params.len()` must have this type. `None` for every
  /// function that doesn't declare one.
  splat_elem: Option<Type>,
}

/// Shared by classes and modules (plan 12's Decision log — modules reuse
/// this registry, tagged `is_module: true`, rather than a separate
/// `modules` map threaded through every function that already carries
/// `classes`). A module's `fields` is always empty (`SEMANTICS.md`
/// §10.4 — no instance state to hold them).
#[derive(Debug, Clone)]
struct ClassInfo {
  /// Flattened — includes every ancestor's fields too (plan 32's
  /// Decision log), not just this class's own declaration.
  fields: HashMap<String, Type>,
  /// Flattened the same way — an inherited-but-not-overridden method
  /// resolves here to its defining ancestor's signature; an override
  /// replaces it.
  methods: HashMap<String, FunctionSig>,
  is_module: bool,
  /// `class Dog < Animal`'s `Animal` — `None` for a module (modules
  /// never inherit) or a class with no `<` clause. This is the class's
  /// OWN declared superclass, not a flattened chain.
  superclass: Option<String>,
  /// `implements Comparable` (plan 41's Decision log) — `None` for
  /// every class that doesn't declare one, always `None` for a module
  /// (the grammar's `ImplementsClause?` is only reachable from
  /// `ClassDef`).
  implements: Option<String>,
}

/// One `interface`'s single required method, kept as raw, unresolved
/// type-name strings (plan 41's Decision log) — `"Self"` isn't
/// resolvable via `resolve_type` until substituted with either a
/// concrete implementing class's name (conformance checking) or the
/// generic type parameter itself (generic-body checking), so resolving
/// eagerly at registration time would be premature.
#[derive(Debug, Clone)]
struct InterfaceInfo {
  method_name: String,
  params_raw: Vec<(String, String)>,
  return_type_raw: String,
}

/// A top-level generic function's registration (plan 41's Decision
/// log) — kept separate from `sigs`/`FunctionSig` (never both at once
/// for the same name: the ordinary `sigs` pass skips every function
/// whose `type_params` is non-empty) since its raw parameter/return
/// type strings need per-call-site substitution before they can be
/// resolved into real `Type`s at all.
#[derive(Debug, Clone)]
struct GenericFunctionSig {
  type_param: String,
  bound: String,
  params_raw: Vec<(String, String)>,
  return_type_raw: String,
}

/// Bundles the two registries a generic-aware type check needs beyond
/// `sigs`/`classes` — threaded as one additional parameter through the
/// whole `infer_expr_type`/`check_stmt` family (the same additive
/// pattern `self_fields: Option<&HashMap<...>>` already established),
/// always present (never `Option`) since it costs nothing to pass an
/// empty pair of maps through an ordinary, non-generic body.
struct GenericsCtx<'a> {
  interfaces: &'a HashMap<String, InterfaceInfo>,
  generic_sigs: &'a HashMap<String, GenericFunctionSig>,
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
    "Nil" => Ok(Type::Nil),
    "Symbol" => Ok(Type::Symbol),
    // Plan 43's Decision log: checked before every other compound-string
    // case below (`Array[Elem]?`/`Hash[K, V]?` recurse cleanly through
    // this) — scoped to reference types only (`Class`/`String`/`Array`/
    // `Hash`); `Int64?`/`Float64?`/`Boolean?`/`Proc?`/`Nil?`/a `T?`
    // referencing a generic type parameter are all rejected here, at the
    // type-annotation boundary, naming the exact reason.
    other if other.ends_with('?') => {
      let inner_name = &other[..other.len() - 1];
      let inner = resolve_type(inner_name, classes)?;
      match inner {
        Type::Class(_) | Type::String | Type::Array(_) | Type::Hash(_, _) => {
          Ok(Type::Nullable(Box::new(inner)))
        }
        other_inner => Err(Diagnostic::new(format!(
          "`{inner_name}?` is not supported — only reference types (a class, String, Array, or Hash) can be nullable, found {other_inner:?}"
        ))),
      }
    }
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
    // `"Hash[K, V]"` — same compound-string convention as `Array[Elem]`
    // above; the grammar's `HashPair` production always formats it with
    // exactly `", "` between `K` and `V` (`grammar.lalrpop`'s `TypeName`
    // rule), so a single `", "` split is unambiguous here.
    other if other.starts_with("Hash[") && other.ends_with(']') => {
      let inner = &other["Hash[".len()..other.len() - 1];
      let (k_name, v_name) = inner
        .split_once(", ")
        .ok_or_else(|| Diagnostic::new(format!("malformed Hash type annotation `{other}`")))?;
      let k_ty = resolve_type(k_name, classes)?;
      let v_ty = resolve_type(v_name, classes)?;
      Ok(Type::Hash(Box::new(k_ty), Box::new(v_ty)))
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

/// Plan 43's Decision log: replaces the raw `actual != declared`
/// equality check at every assignability check-site in this file —
/// exact-equality for every non-nullable `declared` (so every
/// non-`T?` program's accept/reject outcome is provably unchanged,
/// same predicate, same answer), additionally accepting `Type::Nil` or
/// the unwrapped inner type into a `Type::Nullable(inner)` `declared`,
/// per `spec/TYPE_SYSTEM.md` §10's assignability table (`nil → T?` ✓,
/// `T → T?` ✓ widens, `nil → T` ✗).
fn is_assignable(actual: &Type, declared: &Type) -> bool {
  if actual == declared {
    return true;
  }
  if let Type::Nullable(inner) = declared {
    return *actual == Type::Nil || actual == inner.as_ref();
  }
  false
}

fn function_signature(
  f: &Function,
  classes: &HashMap<String, ClassInfo>,
) -> Result<FunctionSig, Diagnostic> {
  // Plan 41's Decision log: the only callers that can ever reach this
  // with a `type_params`-non-empty `Function` are `build_flattened_
  // class_info` (a class method) and `module_info` (a module method) —
  // `check_program`'s own `sigs` registration pass skips a top-level
  // generic function entirely, routing it through `generic_sigs`
  // instead. Caught here, before `resolve_type` ever sees a bare `"T"`
  // and fails with a confusing "unknown type" diagnostic instead.
  if !f.type_params.is_empty() {
    return Err(Diagnostic::new(format!(
      "generic methods are not supported yet (`{}`)",
      f.name
    )));
  }
  let params = f
    .params
    .iter()
    .map(|p| resolve_type(&p.ty, classes))
    .collect::<Result<Vec<_>, _>>()?;
  let return_type = resolve_type(&f.return_type, classes)?;
  let param_names = f.params.iter().map(|p| p.name.clone()).collect();
  let defaults = f.params.iter().map(|p| p.default.clone()).collect();
  let splat_elem = f
    .splat_param
    .as_ref()
    .map(|p| resolve_type(&p.ty, classes))
    .transpose()?;
  Ok(FunctionSig {
    params,
    return_type,
    block_param: f.block_param.clone(),
    param_names,
    defaults,
    splat_elem,
  })
}

/// Walks `name`'s `superclass` chain via `classes`'s already-registered
/// `superclass` links (plan 32's Decision log) — this only needs pass
/// 1's stub registration (names + `superclass`, not yet flattened
/// fields/methods) since it just follows the chain of names. Returns
/// root-to-leaf order (the ultimate ancestor first, `name` itself
/// last). Errors by name on a cycle (a visited-set catches it the
/// moment any name reappears while walking up) or an undefined/module
/// ancestor name, rather than looping or panicking.
fn resolve_chain(
  name: &str,
  classes: &HashMap<String, ClassInfo>,
) -> Result<Vec<String>, Diagnostic> {
  let mut chain = Vec::new();
  let mut visited = HashSet::new();
  let mut current = name.to_string();
  loop {
    if !visited.insert(current.clone()) {
      return Err(Diagnostic::new(format!(
        "cyclic inheritance detected involving class `{current}`"
      )));
    }
    let info = classes
      .get(&current)
      .ok_or_else(|| Diagnostic::new(format!("undefined class `{current}`")))?;
    if info.is_module {
      return Err(Diagnostic::new(format!(
        "cannot inherit from module `{current}` — modules are namespaces, not instantiable"
      )));
    }
    chain.push(current.clone());
    match &info.superclass {
      Some(parent) => current = parent.clone(),
      None => break,
    }
  }
  chain.reverse();
  Ok(chain)
}

/// Builds one class's FLATTENED field/method tables (plan 32's
/// Decision log) — walks `name`'s chain root-to-leaf via `class_defs`
/// (the raw `ClassDef`s, keyed by name; independent of processing
/// order, since every lookup here goes straight to the source
/// declaration rather than a previously-computed `ClassInfo`), merging
/// each ancestor's own fields (erroring on a name already declared by
/// an earlier ancestor) and methods (an existing same-named entry is a
/// real override, checked for an exact signature match before being
/// replaced) in turn. `classes` (pass 1's stub registration) is only
/// used for name resolution (`resolve_type`/`function_signature`), the
/// same role it already played for a class with no superclass.
fn build_flattened_class_info(
  name: &str,
  class_defs: &HashMap<String, &ClassDef>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<ClassInfo, Diagnostic> {
  let chain = resolve_chain(name, classes)?;
  let mut fields: HashMap<String, Type> = HashMap::new();
  let mut field_owner: HashMap<String, String> = HashMap::new();
  let mut methods: HashMap<String, FunctionSig> = HashMap::new();
  for class_name in &chain {
    let c = class_defs
      .get(class_name.as_str())
      .expect("every name in a resolved chain came from a registered ClassDef");
    for f in &c.fields {
      if let Some(owner) = field_owner.get(&f.name) {
        return Err(Diagnostic::new(format!(
          "field `{}` already declared in superclass `{owner}`",
          f.name
        )));
      }
      fields.insert(f.name.clone(), resolve_type(&f.ty, classes)?);
      field_owner.insert(f.name.clone(), class_name.clone());
    }
    for m in &c.methods {
      let sig = function_signature(m, classes)?;
      // `initialize` is exempt from the invariant-signature override
      // check: every class's constructor is inherently class-specific
      // (this plan's own worked example has `Dog::initialize` take an
      // extra `breed_code` param `Animal::initialize` doesn't) — it's
      // not really "overriding" a shared method the way `describe` is,
      // it's each class's own independent construction signature.
      if m.name != "initialize" {
        if let Some(existing) = methods.get(&m.name) {
          if existing.params != sig.params || existing.return_type != sig.return_type {
            return Err(Diagnostic::new(format!(
              "method `{}` override in `{class_name}` has a different signature than the method it overrides: expected {:?} -> {:?}, found {:?} -> {:?}",
              m.name, existing.params, existing.return_type, sig.params, sig.return_type
            )));
          }
        }
      }
      methods.insert(m.name.clone(), sig);
    }
  }
  Ok(ClassInfo {
    fields,
    methods,
    is_module: false,
    superclass: class_defs[name].superclass.clone(),
    implements: class_defs[name].implements.clone(),
  })
}

/// A module's method table — built via the exact same `function_signature`
/// every free function's signature already goes through (plan 12's
/// Decision log: a module method type-checks like a free function,
/// because it is one, just namespaced). Always empty `fields`, never a
/// `superclass` (modules don't inherit).
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
    superclass: None,
    implements: None,
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
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  // Plan 40's Decision log: `Sub`/`Mul`/`Div`/`Rem` all share this
  // function, so this one branch covers `-`/`*`/`/` (three of this
  // plan's eight scoped tokens) plus `%` (outside the plan's literal
  // list, but the plan's own Target state describes routing through
  // this shared function as-is — a class simply won't have a `%`
  // method, giving the same "no operator method" diagnostic as any
  // other undeclared one).
  if let Type::Class(class_name) = &lt {
    return resolve_class_operator(op, class_name, rhs, env, sigs, classes, self_fields, gctx);
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
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

/// Plan 40's Decision log: routes an operator token (`"+"`, `"-"`,
/// `"*"`, `"/"`, `"=="`, `"[]"`, `"[]="`, ...) on a class-typed operand
/// to that class's own declared operator method — mirrors `Expr::
/// MethodCall`'s own existing arm exactly (a plain `HashMap` lookup by
/// method name, then the existing `check_args` for arity/type
/// checking against the method's own declared signature), not a new
/// dispatch mechanism.
#[allow(clippy::too_many_arguments)]
fn resolve_class_operator(
  op: &str,
  class_name: &str,
  rhs: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let info = classes
    .get(class_name)
    .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`")))?;
  let sig = info.methods.get(op).ok_or_else(|| {
    Diagnostic::new(format!(
      "class `{class_name}` has no operator method `{op}`"
    ))
  })?;
  check_args(
    op,
    std::slice::from_ref(rhs),
    &sig.params,
    env,
    sigs,
    classes,
    self_fields,
    gctx,
  )?;
  Ok(sig.return_type.clone())
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
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  if lt != Type::Boolean {
    return Err(Diagnostic::new(format!(
      "`{op}` requires a Boolean left operand, found {lt:?}"
    )));
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
  if rt != Type::Boolean {
    return Err(Diagnostic::new(format!(
      "`{op}` requires a Boolean right operand, found {rt:?}"
    )));
  }
  Ok(Type::Boolean)
}

/// `&`/`|`/`^`/`<<`/`>>` (plan 28's Decision log): `Int64`-only, unlike
/// `check_numeric_binop`'s `Int64`-or-`Float64` — bitwise operators have
/// no `Float64` semantics in this language.
#[allow(clippy::too_many_arguments)]
fn check_bitwise_binop(
  op: &str,
  lhs: &Expr,
  rhs: &Expr,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
  if lt != Type::Int64 {
    return Err(Diagnostic::new(format!(
      "`{op}` requires an Int64 left operand, found {lt:?}"
    )));
  }
  let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
  if rt != Type::Int64 {
    return Err(Diagnostic::new(format!(
      "`{op}` requires an Int64 right operand, found {rt:?}"
    )));
  }
  Ok(Type::Int64)
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
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  match expr {
    Expr::Ident(name) => env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`"))),
    Expr::Int(_) => Ok(Type::Int64),
    Expr::Float(_) => Ok(Type::Float64),
    // Plan 19: a real `Type::String` value at last (the annotation
    // already resolved; nothing could ever produce one before this).
    Expr::StringLit(_) => Ok(Type::String),
    // Plan 44: `Expr::Compare`'s existing generic `lt != rt` rule
    // already covers `Symbol == Symbol`/`!=` for free the moment this
    // produces a real `Type::Symbol` — no `Compare` arm change needed,
    // the identical "for free" shape plan 19 already established for
    // `String`.
    Expr::SymbolLit(_) => Ok(Type::Symbol),
    // Plan 36: a compiler-known stringification set only — `Int64`,
    // `Float64`, `String`, `Boolean` — not a generic, user-extensible
    // `to_s`/`Display` protocol (Decision log: no interface/protocol
    // mechanism exists yet to type-check "the receiver's declared type
    // has a method named `to_s`" against arbitrary future classes).
    // Always produces `Type::String` overall.
    Expr::Interpolate(parts) => {
      for part in parts {
        if let StringPart::Expr(e) = part {
          let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
          if !matches!(
            t,
            Type::Int64 | Type::Float64 | Type::String | Type::Boolean
          ) {
            return Err(Diagnostic::new(format!(
              "type `{t:?}` cannot be interpolated into a string — only Int64, Float64, String, and Boolean are supported"
            )));
          }
        }
      }
      Ok(Type::String)
    }
    // Plan 40's Decision log: checked *before* the existing `lt != rt`/
    // `lt ∉ {Int64, Float64, String}` rule below, which is otherwise
    // completely unchanged — `1 + 2`/`1.0 + 2.0`/`"a" + "b"` compile to
    // the identical instructions as before this plan.
    Expr::Add(lhs, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
      if let Type::Class(class_name) = &lt {
        return resolve_class_operator("+", class_name, rhs, env, sigs, classes, self_fields, gctx);
      }
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
      if lt != rt {
        return Err(Diagnostic::new(format!(
          "type mismatch: `+` requires both operands to have the same type, found {lt:?} and {rt:?}"
        )));
      }
      // Plan 19: `+` on two `String`s concatenates — this plan owns all
      // of `Add`'s `Type::String` case (the separate operators plan is
      // numeric/boolean-only and never touches `Add`/`String`).
      if lt != Type::Int64 && lt != Type::Float64 && lt != Type::String {
        return Err(Diagnostic::new(format!(
          "type `{lt:?}` does not support `+`"
        )));
      }
      Ok(lt)
    }
    Expr::Sub(lhs, rhs) => {
      check_numeric_binop("-", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Mul(lhs, rhs) => {
      check_numeric_binop("*", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Div(lhs, rhs) => {
      check_numeric_binop("/", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Rem(lhs, rhs) => {
      check_numeric_binop("%", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Neg(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Int64 && t != Type::Float64 {
        return Err(Diagnostic::new(format!(
          "type `{t:?}` does not support unary `-`"
        )));
      }
      Ok(t)
    }
    Expr::Not(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`!` requires a Boolean operand, found {t:?}"
        )));
      }
      Ok(Type::Boolean)
    }
    Expr::And(lhs, rhs) => {
      check_boolean_binop("&&", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Or(lhs, rhs) => {
      check_boolean_binop("||", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    // Plan 28: Int64-only bitwise operators.
    Expr::BitAnd(lhs, rhs) => {
      check_bitwise_binop("&", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitOr(lhs, rhs) => {
      check_bitwise_binop("|", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitXor(lhs, rhs) => {
      check_bitwise_binop("^", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Shl(lhs, rhs) => {
      check_bitwise_binop("<<", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::Shr(lhs, rhs) => {
      check_bitwise_binop(">>", lhs, rhs, env, sigs, classes, self_fields, gctx)
    }
    Expr::BitNot(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if t != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "`~` requires an Int64 operand, found {t:?}"
        )));
      }
      Ok(Type::Int64)
    }
    // Plan 40's Decision log: a real, verified gap this plan closes —
    // two same-class instances used to "type-check" here with no
    // operator involved at all (codegen then failed with a generic
    // error), since this arm previously had no restriction beyond
    // `lt == rt`. `==`/`!=` now require the class to declare `==`
    // (`!=` reuses the identical lookup and negates the result — Ruby's
    // own default); any other `CompareOp` on a class operand is an
    // explicit diagnostic rather than a confusing late codegen failure
    // (Ruby derives `<`/`>`/`<=`/`>=` from `<=>` via `Comparable`,
    // which this compiler has no mixin mechanism to reproduce — that
    // derivation is plan 41's job, once `<=>` is callable).
    Expr::Compare(lhs, op, rhs) => {
      let lt = infer_expr_type(lhs, env, sigs, classes, self_fields, gctx)?;
      if let Type::Class(class_name) = &lt {
        return match op {
          CompareOp::Eq | CompareOp::Ne => {
            let ret =
              resolve_class_operator("==", class_name, rhs, env, sigs, classes, self_fields, gctx)?;
            if ret != Type::Boolean {
              return Err(Diagnostic::new(format!(
                "class `{class_name}`'s `==` method must return Boolean, found {ret:?}"
              )));
            }
            Ok(Type::Boolean)
          }
          _ => Err(Diagnostic::new(format!(
            "ordering comparison `{op:?}` is not supported on class `{class_name}` — define `<=>`, not a direct `{op:?}` overload"
          ))),
        };
      }
      let rt = infer_expr_type(rhs, env, sigs, classes, self_fields, gctx)?;
      // Plan 43's Decision log: a `Nullable(_)` operand against `Nil` —
      // either order — always type-checks to `Boolean`, the explicit
      // nil-check alternative to `&.`. Scoped to exactly this shape
      // (never `Nullable == Nullable` between two different nilable
      // values), checked before the `lt != rt` strict-equality rule
      // below, which stays completely unchanged for every other pair.
      if matches!(
        (&lt, &rt),
        (Type::Nullable(_), Type::Nil) | (Type::Nil, Type::Nullable(_))
      ) {
        return Ok(Type::Boolean);
      }
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
      let arg_ty = infer_expr_type(&args[0], env, sigs, classes, self_fields, gctx)?;
      if arg_ty != Type::Int64 && arg_ty != Type::Float64 && arg_ty != Type::String {
        return Err(Diagnostic::new(format!(
          "`puts` does not support type {arg_ty:?}"
        )));
      }
      Ok(Type::Void)
    }
    // Plan 41's Decision log: call-site checking is a separate, later
    // pass from body-checking, and only it ever touches a real concrete
    // type — checked before the ordinary `sigs.get(name)` fallback below
    // since a generic function's own name is never present in `sigs` at
    // all (the registration pass skips it).
    Expr::Call(name, args) if gctx.generic_sigs.contains_key(name) => {
      let g = &gctx.generic_sigs[name];
      let arg_types = args
        .iter()
        .map(|a| infer_expr_type(a, env, sigs, classes, self_fields, gctx))
        .collect::<Result<Vec<_>, _>>()?;
      let mut concrete: Option<Type> = None;
      for (i, (_, raw)) in g.params_raw.iter().enumerate() {
        if raw != &g.type_param {
          continue;
        }
        let Some(actual) = arg_types.get(i) else {
          return Err(Diagnostic::new(format!(
            "`{name}` expects {} argument(s), found {}",
            g.params_raw.len(),
            args.len()
          )));
        };
        match &concrete {
          None => concrete = Some(actual.clone()),
          Some(c) if c != actual => {
            return Err(Diagnostic::new(format!(
              "type parameter `{}` resolved inconsistently in call to `{name}`: `{c:?}` at an earlier argument, `{actual:?}` at argument {}",
              g.type_param,
              i + 1
            )));
          }
          Some(_) => {}
        }
      }
      let concrete = concrete.ok_or_else(|| {
        Diagnostic::new(format!(
          "internal error: generic function `{name}` never uses its own type parameter `{}`",
          g.type_param
        ))
      })?;
      let Type::Class(concrete_class) = &concrete else {
        return Err(Diagnostic::new(format!(
          "type parameter `{}` in call to `{name}` resolved to non-class type {concrete:?} — only a class implementing `{}` is a legal generic argument",
          g.type_param, g.bound
        )));
      };
      let class_info = classes
        .get(concrete_class)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{concrete_class}`")))?;
      if class_info.implements.as_deref() != Some(g.bound.as_str()) {
        return Err(Diagnostic::new(format!(
          "`{concrete_class}` does not implement `{}`, required by generic function `{name}`'s type parameter `{}`",
          g.bound, g.type_param
        )));
      }
      let effective_params = g
        .params_raw
        .iter()
        .map(|(_, raw)| {
          if raw == &g.type_param {
            Ok(concrete.clone())
          } else {
            resolve_type(raw, classes)
          }
        })
        .collect::<Result<Vec<_>, _>>()?;
      check_args(
        name,
        args,
        &effective_params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if g.return_type_raw == g.type_param {
        Ok(concrete)
      } else {
        resolve_type(&g.return_type_raw, classes)
      }
    }
    Expr::Call(name, args) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`")))?;
      // Plan 34: a trailing block literal desugars into an extra,
      // implicit `Expr::Lambda` argument at parse time (`grammar.
      // lalrpop`'s Decision log) — it's not one of `sig.params`'
      // ordinary positional arguments, so it's excluded here before the
      // ordinary arity/type check. Its own legality (present when
      // required, well-typed, `yield`-arity-compatible) is
      // `check_block_call_sites`' separate job, not this one's.
      let positional =
        if sig.block_param.is_some() && matches!(args.last(), Some(Expr::Lambda { .. })) {
          &args[..args.len() - 1]
        } else {
          args.as_slice()
        };
      check_call_args(name, positional, sig, env, sigs, classes, self_fields, gctx)?;
      Ok(sig.return_type.clone())
    }
    // Plan 39's Decision log: resolved entirely at compile time by
    // name-to-position matching against `sig.param_names` — never a
    // runtime hash/dispatch. An unknown name, a duplicate name, or a
    // missing required (no-default) parameter is a real diagnostic
    // naming the offending keyword, not a panic.
    Expr::CallKw(name, kwargs) => {
      let sig = sigs
        .get(name)
        .ok_or_else(|| Diagnostic::new(format!("undefined function `{name}`")))?;
      let mut positional: Vec<Option<&Expr>> = vec![None; sig.param_names.len()];
      for (kw_name, kw_value) in kwargs {
        let Some(pos) = sig.param_names.iter().position(|p| p == kw_name) else {
          return Err(Diagnostic::new(format!(
            "unrecognized keyword `{kw_name}` for `{name}`"
          )));
        };
        if positional[pos].is_some() {
          return Err(Diagnostic::new(format!(
            "duplicate keyword `{kw_name}` in call to `{name}`"
          )));
        }
        positional[pos] = Some(kw_value);
      }
      for (i, slot) in positional.iter().enumerate() {
        if slot.is_none() && sig.defaults[i].is_none() {
          return Err(Diagnostic::new(format!(
            "`{name}` is missing required keyword `{}`",
            sig.param_names[i]
          )));
        }
      }
      for (i, slot) in positional.iter().enumerate() {
        let Some(value) = slot else { continue };
        let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
        if actual != sig.params[i] {
          return Err(Diagnostic::new(format!(
            "keyword `{}` to `{name}` has type {actual:?}, expected {:?}",
            sig.param_names[i], sig.params[i]
          )));
        }
      }
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
          gctx,
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
      check_args(
        method,
        args,
        &sig.params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(sig.return_type.clone())
    }
    // Plan 41's Decision log: a method call on a `Type::Generic`-typed
    // receiver — a call inside the body of the generic function that
    // declares it, before this specific call site's own concrete type
    // is known — resolves against the bound interface's required
    // signature, with `Self` substituted with `Type::Generic` itself
    // (not the `classes` registry, which a `Type::Generic` never
    // appears in). Guarded syntactically (`env.get(n)`, not a value
    // this arm has already computed) since match-arm guards can't run
    // a fallible `infer_expr_type` call.
    Expr::MethodCall(recv, method, args) if matches!(recv.as_ref(), Expr::Ident(n) if matches!(env.get(n), Some(Type::Generic(_, _)))) =>
    {
      let Expr::Ident(recv_name) = recv.as_ref() else {
        unreachable!()
      };
      let Some(Type::Generic(type_param, bound)) = env.get(recv_name) else {
        unreachable!()
      };
      let iface = gctx.interfaces.get(bound).ok_or_else(|| {
        Diagnostic::new(format!(
          "internal error: unknown interface `{bound}` bounding type parameter `{type_param}`"
        ))
      })?;
      if *method != iface.method_name {
        return Err(Diagnostic::new(format!(
          "type parameter `{type_param}` (bounded by `{bound}`) has no method `{method}` — only `{}` is available",
          iface.method_name
        )));
      }
      let self_ty = Type::Generic(type_param.clone(), bound.clone());
      let expected = iface
        .params_raw
        .iter()
        .map(|(_, raw)| {
          if raw == "Self" {
            Ok(self_ty.clone())
          } else {
            resolve_type(raw, classes)
          }
        })
        .collect::<Result<Vec<_>, _>>()?;
      check_args(
        method,
        args,
        &expected,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      if iface.return_type_raw == "Self" {
        Ok(self_ty)
      } else {
        resolve_type(&iface.return_type_raw, classes)
      }
    }
    // `.call` on a `Proc`-typed receiver dispatches against the
    // signature carried directly on `Type::Proc` (plan 10) — everything
    // else falls through to the existing `Type::Class` method lookup.
    Expr::MethodCall(recv, method, args) if method == "call" => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      let Type::Proc(param_types, return_type) = &recv_ty else {
        return Err(Diagnostic::new(format!(
          "method call `.call` on non-Proc type {recv_ty:?}"
        )));
      };
      check_args(
        "call",
        args,
        param_types,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok((**return_type).clone())
    }
    Expr::MethodCall(recv, method, args) => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      // Plan 43's Decision log: the plan's actual payoff — a direct
      // `.method` on a `T?` receiver is a compile-time diagnostic,
      // checked before the existing `Type::Class` match below (which
      // would otherwise reject it with the generic, less useful
      // "non-class type" message this arm already produces for other
      // mismatches).
      if let Type::Nullable(_) = &recv_ty {
        return Err(Diagnostic::new(format!(
          "method call `.{method}` on a nullable receiver (type {recv_ty:?}) — use safe navigation `&.` or an explicit `== nil` check"
        )));
      }
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
      check_args(
        method,
        args,
        &sig.params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(sig.return_type.clone())
    }
    // Plan 43's Decision log: scoped to a class-typed nullable receiver
    // whose dispatched method's return type is itself one of the four
    // pointer-representable kinds (`Class`/`String`/`Array`/`Hash`) —
    // `Int64`/`Float64`/`Boolean`/`Void`/`Nil` would need `&.`'s result
    // to be a boxed `Int64?`/etc., the exact cost this plan already
    // declines to pay. Reuses `check_args` for arity/type checking
    // against the method's own declared signature, same as the
    // ordinary `MethodCall` arm above.
    Expr::SafeCall(recv, method, args) => {
      let recv_ty = infer_expr_type(recv, env, sigs, classes, self_fields, gctx)?;
      let Type::Nullable(inner) = &recv_ty else {
        return Err(Diagnostic::new(format!(
          "`&.{method}` requires a nullable receiver, found {recv_ty:?} — use `.` instead"
        )));
      };
      let Type::Class(class_name) = inner.as_ref() else {
        return Err(Diagnostic::new(format!(
          "`&.{method}` is only supported on a nullable class-typed receiver, found {recv_ty:?}"
        )));
      };
      let info = classes.get(class_name).ok_or_else(|| {
        Diagnostic::new(format!("internal error: unregistered class `{class_name}`"))
      })?;
      let sig = info
        .methods
        .get(method)
        .ok_or_else(|| Diagnostic::new(format!("class `{class_name}` has no method `{method}`")))?;
      if !matches!(
        sig.return_type,
        Type::Class(_) | Type::String | Type::Array(_) | Type::Hash(_, _)
      ) {
        return Err(Diagnostic::new(format!(
          "`&.{method}` returns {:?}, which cannot be wrapped as a nullable result — only a class, String, Array, or Hash return type is supported",
          sig.return_type
        )));
      }
      check_args(
        method,
        args,
        &sig.params,
        env,
        sigs,
        classes,
        self_fields,
        gctx,
      )?;
      Ok(Type::Nullable(Box::new(sig.return_type.clone())))
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
    Expr::ArrayLit(elements) => {
      infer_array_lit_type(elements, env, sigs, classes, self_fields, gctx)
    }
    // Plan 25: `Hash`'s get/set reuse `Expr::Index`/`Stmt::SetIndex`
    // (see the Decision log) — a `Type::Hash(_, _)` arm sits alongside
    // the existing `Type::Array(_)` one rather than a parallel indexing
    // mechanism.
    Expr::Index(array, index) => {
      let array_ty = infer_expr_type(array, env, sigs, classes, self_fields, gctx)?;
      let index_ty = infer_expr_type(index, env, sigs, classes, self_fields, gctx)?;
      match array_ty {
        Type::Array(elem_ty) => {
          if index_ty != Type::Int64 {
            return Err(Diagnostic::new(format!(
              "array index must be Int64, found {index_ty:?}"
            )));
          }
          Ok(*elem_ty)
        }
        Type::Hash(key_ty, value_ty) => {
          if index_ty != *key_ty {
            return Err(Diagnostic::new(format!(
              "Hash key must be {key_ty:?}, found {index_ty:?}"
            )));
          }
          Ok(*value_ty)
        }
        // Plan 40's Decision log: the third arm of this now-three-way
        // match — a class declaring a one-parameter `"[]"` method.
        Type::Class(class_name) => resolve_class_operator(
          "[]",
          &class_name,
          index,
          env,
          sigs,
          classes,
          self_fields,
          gctx,
        ),
        other => Err(Diagnostic::new(format!(
          "`[...]` indexing requires an Array or a Hash, found {other:?}"
        ))),
      }
    }
    Expr::Lambda {
      params,
      return_type,
      body,
    } => infer_lambda_type(params, return_type, body, env, sigs, classes, gctx),
    // Plan 25: a real `Boolean` value, not just `Compare`'s byproduct.
    Expr::Bool(_) => Ok(Type::Boolean),
    // Plan 25: deliberately narrow — see `Type::Nil`'s doc comment.
    Expr::Nil => Ok(Type::Nil),
    // A `{}` empty literal has no key/value type to infer — same
    // reasoning `infer_array_lit_type` already applies to `[]` (plan
    // 09's Decision log), applied here for the second container kind.
    Expr::HashLit(pairs) => infer_hash_lit_type(pairs, env, sigs, classes, self_fields, gctx),
    Expr::ArrayNew(size) => {
      let size_ty = infer_expr_type(size, env, sigs, classes, self_fields, gctx)?;
      if size_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "`Array.new` size must be Int64, found {size_ty:?}"
        )));
      }
      // `Array.new(size)`'s element type comes from the enclosing
      // `Let`'s declared annotation (plan 25's Decision log — the same
      // source plan 09 already uses for an array literal's element
      // type) — `check_stmt`'s `Let` case special-cases this the same
      // way it already special-cases `Proc`, since `infer_expr_type`
      // alone has no declared-type context to draw on here.
      Err(Diagnostic::new(
        "`Array.new(...)` may only appear as a top-level `Let`'s value, where its element type is known from the declared annotation",
      ))
    }
  }
}

/// All key/value pairs of a hash literal must share one key type and
/// one value type (independently — a mixed-key-type *or*
/// mixed-value-type literal is rejected), and — since there's no
/// structured type annotation on the literal itself to fall back on —
/// the literal can't be empty (mirrors `infer_array_lit_type` exactly).
fn infer_hash_lit_type(
  pairs: &[(Expr, Expr)],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let Some(((first_k, first_v), rest)) = pairs.split_first() else {
    return Err(Diagnostic::new(
      "empty hash literals are not supported — the key/value types can't be inferred",
    ));
  };
  let key_ty = infer_expr_type(first_k, env, sigs, classes, self_fields, gctx)?;
  let value_ty = infer_expr_type(first_v, env, sigs, classes, self_fields, gctx)?;
  for (i, (k, v)) in rest.iter().enumerate() {
    let kt = infer_expr_type(k, env, sigs, classes, self_fields, gctx)?;
    if kt != key_ty {
      return Err(Diagnostic::new(format!(
        "hash literal pair {} has key type {kt:?}, expected {key_ty:?} (all keys must share one type)",
        i + 2
      )));
    }
    let vt = infer_expr_type(v, env, sigs, classes, self_fields, gctx)?;
    if vt != value_ty {
      return Err(Diagnostic::new(format!(
        "hash literal pair {} has value type {vt:?}, expected {value_ty:?} (all values must share one type)",
        i + 2
      )));
    }
  }
  Ok(Type::Hash(Box::new(key_ty), Box::new(value_ty)))
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
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let mut lambda_env = env.clone();
  let mut param_types = Vec::with_capacity(params.len());
  for p in params {
    let t = resolve_type(&p.ty, classes)?;
    param_types.push(t.clone());
    lambda_env.insert(p.name.clone(), t);
  }
  let declared_return = resolve_type(return_type, classes)?;
  // Plan 34: `yields_allowed = false` — a lambda/block literal's own
  // body is never itself a `yield`-legal context (only a function/
  // method that declares `block_param` is), whether this is plan 10's
  // original top-level `Proc` lambda or plan 34's block literal reusing
  // the same `Expr::Lambda` node.
  check_block(
    body,
    &mut lambda_env,
    sigs,
    classes,
    None,
    &declared_return,
    false,
    false,
    false,
    gctx,
  )?;
  check_implicit_return(
    body,
    &lambda_env,
    sigs,
    classes,
    None,
    &declared_return,
    "<lambda>",
    gctx,
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
  gctx: &GenericsCtx,
) -> Result<Type, Diagnostic> {
  let Some((first, rest)) = elements.split_first() else {
    return Err(Diagnostic::new(
      "empty array literals are not supported — the element type can't be inferred",
    ));
  };
  let elem_ty = infer_expr_type(first, env, sigs, classes, self_fields, gctx)?;
  for (i, e) in rest.iter().enumerate() {
    let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
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
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  if args.len() != expected.len() {
    return Err(Diagnostic::new(format!(
      "`{name}` expects {} argument(s), found {}",
      expected.len(),
      args.len()
    )));
  }
  for (i, (arg, expected_ty)) in args.iter().zip(expected).enumerate() {
    let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
    if !is_assignable(&actual, expected_ty) {
      return Err(Diagnostic::new(format!(
        "argument {} to `{name}` has type {actual:?}, expected {expected_ty:?}",
        i + 1
      )));
    }
  }
  Ok(())
}

/// Plan 39: `Expr::Call`'s own arity/type checking — unlike `check_args`
/// above (reused verbatim by `Expr::New`/module-method calls, which get
/// none of this plan's four features, per its Decision log's method/
/// non-function scoping), this accepts fewer than `sig.params.len()`
/// arguments as long as every missing trailing one has a declared
/// default, and — when `sig.splat_elem` is `Some` — accepts any number
/// of trailing arguments beyond `sig.params.len()`, type-checked
/// against the splat's declared element type.
#[allow(clippy::too_many_arguments)]
fn check_call_args(
  name: &str,
  args: &[Expr],
  sig: &FunctionSig,
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let required = sig.params.len();
  if sig.splat_elem.is_none() && args.len() > required {
    return Err(Diagnostic::new(format!(
      "`{name}` expects {required} argument(s), found {}",
      args.len()
    )));
  }
  if args.len() < required {
    for (i, default) in sig.defaults.iter().enumerate().skip(args.len()) {
      if default.is_none() {
        return Err(Diagnostic::new(format!(
          "`{name}` is missing required argument `{}`",
          sig.param_names[i]
        )));
      }
    }
  }
  let checked = args.len().min(required);
  for (i, arg) in args.iter().enumerate().take(checked) {
    let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
    if actual != sig.params[i] {
      return Err(Diagnostic::new(format!(
        "argument {} to `{name}` has type {actual:?}, expected {:?}",
        i + 1,
        sig.params[i]
      )));
    }
  }
  if let Some(splat_ty) = &sig.splat_elem {
    for (i, arg) in args.iter().enumerate().skip(required) {
      let actual = infer_expr_type(arg, env, sigs, classes, self_fields, gctx)?;
      if actual != *splat_ty {
        return Err(Diagnostic::new(format!(
          "trailing (splat) argument {} to `{name}` has type {actual:?}, expected {splat_ty:?}",
          i + 1
        )));
      }
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
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let array_ty = infer_expr_type(array, env, sigs, classes, self_fields, gctx)?;
  let index_ty = infer_expr_type(index, env, sigs, classes, self_fields, gctx)?;
  let (container, elem_ty, index_expected) = match array_ty {
    Type::Array(elem_ty) => ("array", *elem_ty, Type::Int64),
    Type::Hash(key_ty, value_ty) => ("Hash", *value_ty, *key_ty),
    // Plan 40's Decision log: sourced from a two-parameter `"[]="`
    // method's own declared signature (`(index, value) -> Void`) — the
    // rest of this function's index-type/value-type checking below is
    // reused completely unchanged for this third case.
    Type::Class(class_name) => {
      let info = classes
        .get(&class_name)
        .ok_or_else(|| Diagnostic::new(format!("undefined class `{class_name}`")))?;
      let sig = info.methods.get("[]=").ok_or_else(|| {
        Diagnostic::new(format!("class `{class_name}` has no operator method `[]=`"))
      })?;
      if sig.params.len() != 2 {
        return Err(Diagnostic::new(format!(
          "class `{class_name}`'s `[]=` method must take exactly 2 parameters (index, value), found {}",
          sig.params.len()
        )));
      }
      ("[]=", sig.params[1].clone(), sig.params[0].clone())
    }
    other => {
      return Err(Diagnostic::new(format!(
        "`[...] = ...` indexing requires an Array or a Hash, found {other:?}"
      )));
    }
  };
  if index_ty != index_expected {
    return Err(Diagnostic::new(format!(
      "{container} index must be {index_expected:?}, found {index_ty:?}"
    )));
  }
  let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
  if !is_assignable(&actual, &elem_ty) {
    return Err(Diagnostic::new(format!(
      "type mismatch in {container} assignment: element type is {elem_ty:?}, value has type {actual:?}"
    )));
  }
  Ok(())
}

/// `n1, n2, ... = v1, v2, ...` (plan 31's Decision log): fixed-arity
/// only — an arity mismatch is a real diagnostic, not a panic or silent
/// truncation/padding. Every `values` expression is type-checked before
/// any `names` binding is consulted for its target type, matching
/// codegen's own "evaluate all RHS before writing any target" ordering
/// (what makes `a, b = b, a` a real swap).
#[allow(clippy::too_many_arguments)]
fn check_multi_assign(
  names: &[String],
  values: &[Expr],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  if names.len() != values.len() {
    return Err(Diagnostic::new(format!(
      "multiple assignment arity mismatch: {} target(s), {} value(s)",
      names.len(),
      values.len()
    )));
  }
  let value_types = values
    .iter()
    .map(|v| infer_expr_type(v, env, sigs, classes, self_fields, gctx))
    .collect::<Result<Vec<_>, _>>()?;
  for (i, (name, actual)) in names.iter().zip(value_types).enumerate() {
    let declared = env
      .get(name)
      .cloned()
      .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`")))?;
    if !is_assignable(&actual, &declared) {
      return Err(Diagnostic::new(format!(
        "type mismatch in multiple assignment at position {}: `{name}` has type {declared:?}, value has type {actual:?}",
        i + 1
      )));
    }
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
  // Plan 38: mirrors `in_loop` exactly — threaded through every
  // `check_block`/`check_stmt` call unchanged, forced `true` only by
  // `check_begin` for each `RescueClause.body` (never for the `begin`'s
  // own try `body` or its `ensure` body). Gates `retry`'s legality the
  // same way `in_loop` gates `break`/`next`.
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  match stmt {
    // `Proc` is special-cased: the bare annotation carries no signature
    // (see `Type::Proc`'s doc comment), so instead of comparing against
    // `resolve_type("Proc", ...)`'s opaque placeholder, any actual
    // `Type::Proc(_, _)` is accepted and *that* — the real signature
    // inferred from the bound `Expr::Lambda` — is what's stored in `env`.
    Stmt::Let { name, ty, value } if ty == "Proc" => {
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !matches!(actual, Type::Proc(_, _)) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name}: Proc = ...`: expected a Proc (lambda literal), found {actual:?}"
        )));
      }
      env.insert(name.clone(), actual);
      Ok(())
    }
    // `Array.new(size)`'s element type comes from the enclosing `Let`'s
    // own declared annotation (plan 25's Decision log) — special-cased
    // the same way `Proc` is above, since `infer_expr_type` alone has
    // no declared-type context available to it.
    Stmt::Let {
      name,
      ty,
      value: Expr::ArrayNew(size),
    } => {
      let declared = resolve_type(ty, classes)?;
      if !matches!(declared, Type::Array(_)) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name}: {ty} = Array.new(...)`: `Array.new` produces an Array, not {declared:?}"
        )));
      }
      let size_ty = infer_expr_type(size, env, sigs, classes, self_fields, gctx)?;
      if size_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "`Array.new` size must be Int64, found {size_ty:?}"
        )));
      }
      env.insert(name.clone(), declared);
      Ok(())
    }
    Stmt::Let { name, ty, value } => {
      let declared = resolve_type(ty, classes)?;
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
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
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
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
    } => check_set_index(array, index, value, env, sigs, classes, self_fields, gctx),
    // Plan 31: `name` must already be bound — this is a reassignment,
    // never a fresh declaration (the Decision log's whole reason this
    // is a distinct `Stmt` from `Let`). Checked against the *existing*
    // binding's type, same "no implicit conversion" rule `Let` and
    // every other assignment shape here already enforces.
    Stmt::Assign { name, value } => {
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`")))?;
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name} = ...`: `{name}` has type {declared:?}, value has type {actual:?}"
        )));
      }
      Ok(())
    }
    // Plan 43's Decision log: genuinely conditional — assigns `default`
    // only when `name`'s current value is nil, then narrows `name`'s
    // tracked type from `Nullable(inner)` to `inner` directly (sound by
    // construction: either branch leaves `name` unconditionally
    // `inner`-typed). `default` itself must be the *unwrapped* `inner`
    // type, not `inner?` again — widening a still-nullable default
    // would make the narrowing unsound.
    Stmt::OrAssign { name, default } => {
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`")))?;
      let Type::Nullable(inner) = &declared else {
        return Err(Diagnostic::new(format!(
          "`{name} ||= ...` requires `{name}`'s declared type to be nullable, found {declared:?}"
        )));
      };
      let actual = infer_expr_type(default, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, inner) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name} ||= ...`: expected {inner:?}, found {actual:?}"
        )));
      }
      env.insert(name.clone(), (**inner).clone());
      Ok(())
    }
    // Plan 43's Decision log: the asymmetric twin of `OrAssign` above —
    // assigns `value` only when `name`'s current value is non-nil, and
    // deliberately does NOT narrow `name`'s tracked type (the
    // nil-and-skipped branch leaves it exactly as nilable as before).
    // `value` must be assignable to `name`'s *full* declared
    // `Nullable(inner)` type (so widening a plain `inner`-typed value
    // still works, via the same `is_assignable` helper).
    Stmt::AndAssign { name, value } => {
      let declared = env
        .get(name)
        .cloned()
        .ok_or_else(|| Diagnostic::new(format!("undefined variable `{name}`")))?;
      if !matches!(declared, Type::Nullable(_)) {
        return Err(Diagnostic::new(format!(
          "`{name} &&= ...` requires `{name}`'s declared type to be nullable, found {declared:?}"
        )));
      }
      let actual = infer_expr_type(value, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&actual, &declared) {
        return Err(Diagnostic::new(format!(
          "type mismatch in `{name} &&= ...`: expected {declared:?}, found {actual:?}"
        )));
      }
      Ok(())
    }
    Stmt::MultiAssign { names, values } => {
      check_multi_assign(names, values, env, sigs, classes, self_fields, gctx)
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields, gctx)?;
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
        in_rescue,
        yields_allowed,
        gctx,
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
          in_rescue,
          yields_allowed,
          gctx,
        )?;
      }
      Ok(())
    }
    Stmt::While { cond, body } => {
      let cond_ty = infer_expr_type(cond, env, sigs, classes, self_fields, gctx)?;
      if cond_ty != Type::Boolean {
        return Err(Diagnostic::new(format!(
          "`while` condition must be Boolean, found {cond_ty:?}"
        )));
      }
      check_block(
        body,
        env,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    Stmt::Return(Some(e)) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if !is_assignable(&t, return_type) {
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
    Stmt::Expr(e) => infer_expr_type(e, env, sigs, classes, self_fields, gctx).map(|_| ()),
    Stmt::Raise(e) => {
      let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
      if !matches!(t, Type::Class(_)) {
        return Err(Diagnostic::new(format!(
          "`raise` requires a class instance, found {t:?}"
        )));
      }
      Ok(())
    }
    // Plan 34: legal only inside a function/method declaring
    // `block_param` — `yields_allowed` carries that down from
    // `check_function_body`/`check_method_body`. This generic pass has
    // no specific attached block to check `args`' types against yet
    // (no single fixed block signature exists — see the Decision log),
    // so it only verifies `args` are well-formed expressions (catching
    // an undefined variable, etc.); the real per-attachment arity/type
    // check happens separately, once per call site, in
    // `check_block_call_sites`.
    Stmt::Yield(args) => {
      if !yields_allowed {
        return Err(Diagnostic::new(
          "`yield` used outside of a function or method that declares a block parameter (`&name`)",
        ));
      }
      for a in args {
        infer_expr_type(a, env, sigs, classes, self_fields, gctx)?;
      }
      Ok(())
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => check_begin(
      body,
      rescues,
      ensure,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    ),
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => check_case(
      scrutinee,
      arms,
      else_body,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    ),
    // Plan 30: `elements`'s element type is unified exactly as
    // `infer_array_lit_type` already does for a bare array literal
    // (reject empty, reject heterogeneous) — `for` just also binds
    // `var` at that type for `body`, and threads `in_loop = true` so
    // `break`/`next` are legal inside it, same as `while`.
    Stmt::For {
      var,
      elements,
      body,
    } => {
      // `infer_array_lit_type` returns the literal's own `Array(elem)`
      // type, not the element type `var` should be bound at — unwrap
      // one layer.
      let Type::Array(elem_ty) =
        infer_array_lit_type(elements, env, sigs, classes, self_fields, gctx)?
      else {
        unreachable!("infer_array_lit_type always returns Type::Array")
      };
      env.insert(var.clone(), *elem_ty);
      check_block(
        body,
        env,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    // Plan 37: no first-class `Range` value — `start`/`end` are each
    // independently checked as `Int64` (rejecting any other type by
    // name, not silently coercing), `var` is bound at `Int64`
    // unconditionally for `body`, and `in_loop = true` is threaded
    // exactly as `Stmt::For`'s own arm does. A reverse range
    // (`start > end`) is deliberately not rejected here — that's a
    // runtime shape (a well-typed, zero-iteration loop), not a type
    // error.
    Stmt::ForRange {
      var,
      start,
      end,
      exclusive: _,
      body,
    } => {
      let start_ty = infer_expr_type(start, env, sigs, classes, self_fields, gctx)?;
      if start_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "range start must be Int64, found {start_ty:?}"
        )));
      }
      let end_ty = infer_expr_type(end, env, sigs, classes, self_fields, gctx)?;
      if end_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "range end must be Int64, found {end_ty:?}"
        )));
      }
      env.insert(var.clone(), Type::Int64);
      check_block(
        body,
        env,
        sigs,
        classes,
        self_fields,
        return_type,
        true,
        in_rescue,
        yields_allowed,
        gctx,
      )
    }
    // Plan 38: legal only inside a `rescue` clause's own body — see
    // `check_begin`, which forces `in_rescue = true` only there, never
    // for the `begin`'s own try `body` or its `ensure` body.
    Stmt::Retry => {
      if !in_rescue {
        return Err(Diagnostic::new("`retry` outside of a rescue body"));
      }
      Ok(())
    }
  }
}

/// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan 20's
/// Decision log): the scrutinee and every `when` value must be
/// `Int64` — not `Float64`/`String`/`Class` — value-matched via the
/// same `CompareOp::Eq` `Expr::Compare` already performs, not
/// `spec/GRAMMAR.md`'s eventual method-dispatched `===`.
#[allow(clippy::too_many_arguments)]
fn check_case(
  scrutinee: &Expr,
  arms: &[(Vec<Expr>, Vec<Stmt>)],
  else_body: &Option<Vec<Stmt>>,
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let scrutinee_ty = infer_expr_type(scrutinee, env, sigs, classes, self_fields, gctx)?;
  if scrutinee_ty != Type::Int64 {
    return Err(Diagnostic::new(format!(
      "`case` scrutinee must be Int64, found {scrutinee_ty:?}"
    )));
  }
  for (values, body) in arms {
    for v in values {
      let value_ty = infer_expr_type(v, env, sigs, classes, self_fields, gctx)?;
      if value_ty != Type::Int64 {
        return Err(Diagnostic::new(format!(
          "`when` value must be Int64, found {value_ty:?}"
        )));
      }
    }
    check_block(
      body,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  if let Some(else_b) = else_body {
    check_block(
      else_b,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
}

/// `begin body rescue Type => e ... [rescue => e2 ...] [ensure ...] end`
/// (plan 38's Decision log). Flat scoping, same as everything else in
/// this compiler (plan 07's Decision log) — each typed clause's `var`
/// joins the same environment an `if`/`while` body's `Let`s already
/// flow into, not a fresh scope. A bare clause's `var` is never
/// inserted into `env` at all (no universal root class exists to type
/// it at — any reference inside that clause's body falls through to
/// the ordinary "undefined variable" diagnostic). `in_rescue` is
/// forced `true` only for each `RescueClause.body` — left unchanged
/// for the try `body` and the `ensure` body, so `retry` is illegal in
/// both, matching Ruby's own restriction.
#[allow(clippy::too_many_arguments)]
fn check_begin(
  body: &[Stmt],
  rescues: &[RescueClause],
  ensure: &Option<Vec<Stmt>>,
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  return_type: &Type,
  in_loop: bool,
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  check_block(
    body,
    env,
    sigs,
    classes,
    self_fields,
    return_type,
    in_loop,
    in_rescue,
    yields_allowed,
    gctx,
  )?;
  for rescue in rescues {
    if let Some(class_name) = &rescue.class_name {
      let rescue_ty = resolve_type(class_name, classes)?;
      if !matches!(rescue_ty, Type::Class(_)) {
        return Err(Diagnostic::new(format!(
          "`rescue {class_name}` must name a class, found {rescue_ty:?}"
        )));
      }
      env.insert(rescue.var.clone(), rescue_ty);
    }
    check_block(
      &rescue.body,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      true,
      yields_allowed,
      gctx,
    )?;
  }
  if let Some(ensure_body) = ensure {
    check_block(
      ensure_body,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
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
  in_rescue: bool,
  yields_allowed: bool,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    check_stmt(
      stmt,
      env,
      sigs,
      classes,
      self_fields,
      return_type,
      in_loop,
      in_rescue,
      yields_allowed,
      gctx,
    )?;
  }
  Ok(())
}

/// Checks the final-statement implicit-return rule shared by free
/// functions and methods (Ruby-style: a body whose last statement is a
/// bare expression returns that expression's value).
#[allow(clippy::too_many_arguments)]
fn check_implicit_return(
  body: &[Stmt],
  env: &HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  self_fields: Option<&HashMap<String, Type>>,
  declared_return: &Type,
  owner_name: &str,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  if let Some(Stmt::Expr(e)) = body.last() {
    let t = infer_expr_type(e, env, sigs, classes, self_fields, gctx)?;
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
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &f.params {
    env.insert(p.name.clone(), resolve_type(&p.ty, classes)?);
  }
  // Plan 39: a splat parameter is bound inside the body as a real
  // `Array[Elem]` — call sites pack it into one at each call site (the
  // same representation plan 09 already proved), so the body indexes
  // it exactly like any other array-typed local.
  if let Some(p) = &f.splat_param {
    let elem_ty = resolve_type(&p.ty, classes)?;
    env.insert(p.name.clone(), Type::Array(Box::new(elem_ty)));
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
    false,
    f.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &f.body,
    &env,
    sigs,
    classes,
    None,
    &declared_return,
    &f.name,
    gctx,
  )
}

fn check_method_body(
  class_name: &str,
  m: &Function,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  fields: &HashMap<String, Type>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  // Plan 39's Decision log: default parameter values and splat capture
  // are scoped to plain top-level `def` functions only — a real,
  // disclosed diagnostic here, not a silently-ignored parsed-but-dead
  // AST field.
  if let Some(p) = m.params.iter().find(|p| p.default.is_some()) {
    return Err(Diagnostic::new(format!(
      "default parameter values are not supported on methods yet (`{class_name}#{}`'s `{}`)",
      m.name, p.name
    )));
  }
  if m.splat_param.is_some() {
    return Err(Diagnostic::new(format!(
      "splat parameters are not supported on methods yet (`{class_name}#{}`)",
      m.name
    )));
  }
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
    false,
    m.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &m.body,
    &env,
    sigs,
    classes,
    Some(fields),
    &declared_return,
    &format!("{class_name}#{}", m.name),
    gctx,
  )
}

/// Plan 34: for every bare top-level statement call to a `block_param`-
/// declaring free function (`Stmt::Expr(Expr::Call(...))` — the only
/// shape a trailing block literal can attach to, since `BlockLiteral`
/// is grammar-restricted to `StmtPrimaryExpr`, never the non-Stmt-
/// initial `PrimaryExpr` — see `grammar.lalrpop`'s Decision log),
/// checks that a block is actually attached, type-checks that block's
/// own body, and then re-walks the callee's OWN body checking every
/// `Stmt::Yield` site's arguments against this specific attachment's
/// parameter types (Decision log: there is no single fixed block
/// signature to check against once — two call sites attaching
/// different blocks to the same function are each checked
/// independently). Scoped to `Expr::Call` only, not `MethodCall` — this
/// plan's own worked example and ACs only exercise a free function; a
/// `block_param`-declaring method is real, disclosed future work.
fn check_block_call_sites(
  program: &Program,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
) -> Vec<Diagnostic> {
  let mut diags = Vec::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        scan_block_call_sites(&f.body, sigs, classes, func_defs, gctx, &mut diags)
      }
      Item::Class(c) => {
        for m in &c.methods {
          scan_block_call_sites(&m.body, sigs, classes, func_defs, gctx, &mut diags);
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          scan_block_call_sites(&f.body, sigs, classes, func_defs, gctx, &mut diags);
        }
      }
      Item::Stmt(s) => scan_block_call_site(s, sigs, classes, func_defs, gctx, &mut diags),
      // Plan 41: an interface declares one required method signature,
      // never a body — nothing here can contain a `yield` site.
      Item::Interface(_) => {}
      // Plan 23: `emerald-driver`'s `resolve_program` (not yet
      // extracted in this codebase) is meant to strip every
      // `Item::Require` before `emerald-sema` ever sees a `Program` —
      // a no-op here, not an error, since a `require`-bearing `Program`
      // reaching this far is a real, disclosed gap this plan names
      // rather than papering over with a fabricated driver crate.
      Item::Require(_) => {}
      Item::Error => {}
    }
  }
  diags
}

fn scan_block_call_sites(
  stmts: &[Stmt],
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  for s in stmts {
    scan_block_call_site(s, sigs, classes, func_defs, gctx, diags);
  }
}

fn scan_block_call_site(
  stmt: &Stmt,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  match stmt {
    Stmt::Expr(Expr::Call(name, args)) => {
      let declares_block = sigs.get(name).is_some_and(|s| s.block_param.is_some());
      if declares_block {
        check_one_block_call_site(name, args, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      scan_block_call_sites(then_branch, sigs, classes, func_defs, gctx, diags);
      if let Some(else_b) = else_branch {
        scan_block_call_sites(else_b, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::While { body, .. } => scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags);
      for rescue in rescues {
        scan_block_call_sites(&rescue.body, sigs, classes, func_defs, gctx, diags);
      }
      if let Some(ensure_body) = ensure {
        scan_block_call_sites(ensure_body, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags);
      }
      if let Some(else_b) = else_body {
        scan_block_call_sites(else_b, sigs, classes, func_defs, gctx, diags);
      }
    }
    Stmt::For { body, .. } => scan_block_call_sites(body, sigs, classes, func_defs, gctx, diags),
    _ => {}
  }
}

fn check_one_block_call_site(
  name: &str,
  args: &[Expr],
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  func_defs: &HashMap<String, &Function>,
  gctx: &GenericsCtx,
  diags: &mut Vec<Diagnostic>,
) {
  let Some(Expr::Lambda {
    params: blk_params,
    body: blk_body,
    ..
  }) = args.last()
  else {
    diags.push(Diagnostic::new(format!(
      "`{name}` requires a trailing block (`{{ |params| ... }}`) — it declares a block parameter"
    )));
    return;
  };
  let mut blk_env: HashMap<String, Type> = HashMap::new();
  let mut blk_param_types = Vec::with_capacity(blk_params.len());
  for p in blk_params {
    let t = match resolve_type(&p.ty, classes) {
      Ok(t) => t,
      Err(d) => {
        diags.push(d);
        return;
      }
    };
    blk_param_types.push(t.clone());
    blk_env.insert(p.name.clone(), t);
  }
  if let Err(d) = check_block(
    blk_body,
    &mut blk_env,
    sigs,
    classes,
    None,
    &Type::Void,
    false,
    false,
    false,
    gctx,
  ) {
    diags.push(d);
    return;
  }
  let Some(callee) = func_defs.get(name) else {
    return;
  };
  let mut callee_env: HashMap<String, Type> = HashMap::new();
  for p in &callee.params {
    let t = match resolve_type(&p.ty, classes) {
      Ok(t) => t,
      Err(d) => {
        diags.push(d);
        return;
      }
    };
    callee_env.insert(p.name.clone(), t);
  }
  if let Err(d) = check_yields_against_block(
    &callee.body,
    &blk_param_types,
    &mut callee_env,
    sigs,
    classes,
    gctx,
  ) {
    diags.push(d);
  }
}

/// Re-walks a `block_param`-declaring function's body, tracking the
/// same flat local-variable environment `check_block` would (so a
/// `yield` site referencing an earlier `Let`-bound local still
/// type-checks), but checking only `Stmt::Yield` sites — every other
/// statement kind was already fully checked once by this function's own
/// routine `check_function_body` pass; re-verifying it here would be
/// redundant, not incorrect.
fn check_yields_against_block(
  stmts: &[Stmt],
  expected: &[Type],
  env: &mut HashMap<String, Type>,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  for stmt in stmts {
    match stmt {
      Stmt::Yield(args) => {
        if args.len() != expected.len() {
          return Err(Diagnostic::new(format!(
            "block arity mismatch: `yield` passes {} argument(s), attached block declares {} parameter(s)",
            args.len(),
            expected.len()
          )));
        }
        for (i, (a, want)) in args.iter().zip(expected).enumerate() {
          let actual = infer_expr_type(a, env, sigs, classes, None, gctx)?;
          if actual != *want {
            return Err(Diagnostic::new(format!(
              "type mismatch in `yield` argument {}: attached block's parameter has type {want:?}, value has type {actual:?}",
              i + 1
            )));
          }
        }
      }
      Stmt::Let { name, ty, .. } => {
        if let Ok(t) = resolve_type(ty, classes) {
          env.insert(name.clone(), t);
        }
      }
      Stmt::If {
        then_branch,
        else_branch,
        ..
      } => {
        check_yields_against_block(then_branch, expected, env, sigs, classes, gctx)?;
        if let Some(else_b) = else_branch {
          check_yields_against_block(else_b, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::While { body, .. } => {
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?
      }
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
        for rescue in rescues {
          if let Some(class_name) = &rescue.class_name {
            if let Ok(t) = resolve_type(class_name, classes) {
              env.insert(rescue.var.clone(), t);
            }
          }
          check_yields_against_block(&rescue.body, expected, env, sigs, classes, gctx)?;
        }
        if let Some(ensure_body) = ensure {
          check_yields_against_block(ensure_body, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::Case {
        arms, else_body, ..
      } => {
        for (_, body) in arms {
          check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
        }
        if let Some(else_b) = else_body {
          check_yields_against_block(else_b, expected, env, sigs, classes, gctx)?;
        }
      }
      Stmt::For {
        var,
        elements,
        body,
      } => {
        if let Ok(Type::Array(elem_ty)) =
          infer_array_lit_type(elements, env, sigs, classes, None, gctx)
        {
          env.insert(var.clone(), *elem_ty);
        }
        check_yields_against_block(body, expected, env, sigs, classes, gctx)?;
      }
      _ => {}
    }
  }
  Ok(())
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

  // Plan 41: interfaces register in their own pass, independent of
  // class/module registration order — an interface's required method
  // is kept as raw, unresolved type-name strings (`InterfaceInfo`'s
  // own doc comment), so nothing here needs `classes` populated yet.
  let mut interfaces: HashMap<String, InterfaceInfo> = HashMap::new();
  for item in &program.items {
    if let Item::Interface(idef) = item {
      interfaces.insert(
        idef.name.clone(),
        InterfaceInfo {
          method_name: idef.method_name.clone(),
          params_raw: idef
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.clone()))
            .collect(),
          return_type_raw: idef.return_type.clone(),
        },
      );
    }
  }

  // Modules register into the same two-pass table as classes (plan 12's
  // Decision log) — names first (so a class/module's own fields/methods
  // can reference any other class/module name regardless of declaration
  // order), then full field/method tables.
  let mut classes: HashMap<String, ClassInfo> = HashMap::new();
  // Plan 32: raw `ClassDef`s keyed by name, so `build_flattened_class_info`
  // can look up any ancestor's own field/method declarations directly —
  // independent of `program.items`' order, unlike a scheme that only
  // ever has each *previously computed* `ClassInfo` to work from.
  let mut class_defs: HashMap<String, &ClassDef> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      class_defs.insert(c.name.clone(), c);
      classes.insert(
        c.name.clone(),
        ClassInfo {
          fields: HashMap::new(),
          methods: HashMap::new(),
          is_module: false,
          superclass: c.superclass.clone(),
          implements: c.implements.clone(),
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
          superclass: None,
          implements: None,
        },
      );
    }
  }
  for item in &program.items {
    if let Item::Class(c) = item {
      match build_flattened_class_info(&c.name, &class_defs, &classes) {
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

  // Plan 41: checked immediately after a class with `implements.is_some()`
  // successfully registers its flattened `ClassInfo` (Decision log) — a
  // class whose OWN registration failed (`classes.get` came back `None`
  // above) never reaches this at all, matching the established "if
  // registration failed, skip downstream checks" pattern.
  for item in &program.items {
    if let Item::Class(c) = item {
      if let Some(iface_name) = &c.implements {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        if let Err(d) =
          check_interface_conformance(&c.name, iface_name, info, &interfaces, &classes)
        {
          diags.push(d);
        }
      }
    }
  }

  let mut sigs: HashMap<String, FunctionSig> = HashMap::new();
  // Plan 41: a top-level function whose `type_params` is non-empty is a
  // generic function — it never enters `sigs` at all (Decision log: the
  // ordinary `sigs` pass skips it entirely, so its name is never present
  // in both registries at once). Exactly one type parameter is required;
  // two or more is a real registration-time diagnostic, and the offending
  // function's name is tracked here so the later body-check loop skips
  // it too (already diagnosed once, not silently, not twice).
  let mut generic_sigs: HashMap<String, GenericFunctionSig> = HashMap::new();
  let mut bad_generic_fns: HashSet<String> = HashSet::new();
  // Plan 34: raw `Function`s keyed by name, alongside `sigs` — a
  // block-attaching call site needs the callee's actual body (to
  // re-walk its `Stmt::Yield` sites), not just its signature.
  let mut func_defs: HashMap<String, &Function> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      func_defs.insert(f.name.clone(), f);
      if f.type_params.is_empty() {
        match function_signature(f, &classes) {
          Ok(sig) => {
            sigs.insert(f.name.clone(), sig);
          }
          Err(d) => diags.push(d),
        }
      } else if f.type_params.len() != 1 {
        diags.push(Diagnostic::new(format!(
          "generic function `{}` declares {} type parameters — multiple type parameters are not supported",
          f.name,
          f.type_params.len()
        )));
        bad_generic_fns.insert(f.name.clone());
      } else {
        let tp = &f.type_params[0];
        generic_sigs.insert(
          f.name.clone(),
          GenericFunctionSig {
            type_param: tp.name.clone(),
            bound: tp.bound.clone(),
            params_raw: f
              .params
              .iter()
              .map(|p| (p.name.clone(), p.ty.clone()))
              .collect(),
            return_type_raw: f.return_type.clone(),
          },
        );
      }
    }
  }

  let gctx = GenericsCtx {
    interfaces: &interfaces,
    generic_sigs: &generic_sigs,
  };

  // Declared once, outside the loop: top-level statements share one
  // environment across the whole program in order (`x: Int64 = 10` then
  // `if x > 5 ...` needs `x` visible in a later Item::Stmt).
  let mut top_env: HashMap<String, Type> = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) if !f.type_params.is_empty() => {
        if bad_generic_fns.contains(&f.name) {
          continue;
        }
        let g = &generic_sigs[&f.name];
        if let Err(d) = check_generic_function_body(f, g, &sigs, &classes, &gctx) {
          diags.push(d);
        }
      }
      Item::Function(f) => {
        if let Err(d) = check_function_body(f, &sigs, &classes, &gctx) {
          diags.push(d);
        }
      }
      Item::Class(c) => {
        let Some(info) = classes.get(&c.name) else {
          continue;
        };
        for m in &c.methods {
          if let Err(d) = check_method_body(&c.name, m, &sigs, &classes, &info.fields, &gctx) {
            diags.push(d);
          }
        }
      }
      // A module method type-checks exactly like a free function (plan
      // 12's Decision log) — no `self`, no `@field` access.
      Item::Module(m) => {
        for f in &m.methods {
          if let Err(d) = check_function_body(f, &sigs, &classes, &gctx) {
            diags.push(d);
          }
        }
      }
      // No enclosing function return type (Void is a safe sentinel — a
      // bare `return` at top level is not exercised by this milestone),
      // and never inside a method (`self_fields: None`).
      Item::Stmt(s) => {
        if let Err(d) = check_stmt(
          s,
          &mut top_env,
          &sigs,
          &classes,
          None,
          &Type::Void,
          false,
          false,
          false,
          &gctx,
        ) {
          diags.push(d);
        }
      }
      // Plan 41: a general, user-declarable grammar production (Decision
      // log) — nothing left to do here beyond the two registration
      // passes above; an interface has no body of its own to check.
      Item::Interface(_) => {}
      // Plan 23: see `check_block_call_sites`'s own `Item::Require`
      // arm — a no-op here too, for the same reason.
      Item::Require(_) => {}
      // Plan 26's Decision log: `emerald_parser::parse`/`parse_named`
      // returns `Ok(program)` only when zero errors were recovered —
      // `program.items` then contains no `Item::Error` by construction,
      // so `check_program` never actually receives one.
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
    }
  }

  diags.extend(check_block_call_sites(
    program, &sigs, &classes, &func_defs, &gctx,
  ));

  if diags.is_empty() { Ok(()) } else { Err(diags) }
}

/// Plan 41's Decision log: substitutes every `"Self"` in the interface's
/// raw parameter/return-type strings with `class_name` itself, resolves
/// the substituted strings, and compares the result *exactly* (invariant,
/// not covariant — same discipline as plan 32's override check) against
/// `info.methods.get(&iface.method_name)` — the class's already-
/// **flattened** method table, so an interface requirement satisfied by
/// an *inherited* method is accepted for free.
fn check_interface_conformance(
  class_name: &str,
  iface_name: &str,
  info: &ClassInfo,
  interfaces: &HashMap<String, InterfaceInfo>,
  classes: &HashMap<String, ClassInfo>,
) -> Result<(), Diagnostic> {
  let iface = interfaces.get(iface_name).ok_or_else(|| {
    Diagnostic::new(format!(
      "class `{class_name}` declares `implements {iface_name}`, but no interface named `{iface_name}` is declared"
    ))
  })?;
  let expected_params = iface
    .params_raw
    .iter()
    .map(|(_, raw)| resolve_type(if raw == "Self" { class_name } else { raw }, classes))
    .collect::<Result<Vec<_>, _>>()?;
  let expected_return = resolve_type(
    if iface.return_type_raw == "Self" {
      class_name
    } else {
      &iface.return_type_raw
    },
    classes,
  )?;
  let Some(actual) = info.methods.get(&iface.method_name) else {
    return Err(Diagnostic::new(format!(
      "class `{class_name}` declares `implements {iface_name}` but does not define required method `{}`",
      iface.method_name
    )));
  };
  if actual.params != expected_params || actual.return_type != expected_return {
    return Err(Diagnostic::new(format!(
      "class `{class_name}`'s `{}` does not match interface `{iface_name}`'s required signature: expected {expected_params:?} -> {expected_return:?}, found {:?} -> {:?}",
      iface.method_name, actual.params, actual.return_type
    )));
  }
  Ok(())
}

/// Plan 41's Decision log: a generic function's body is type-checked
/// exactly **once**, statically, before any concrete type is known — a
/// parameter (or return type) whose declared type name equals the
/// function's own type parameter is bound to `Type::Generic(name, bound)`
/// directly (bypassing `resolve_type`, which cannot resolve a bare
/// `"T"`), so a method call on it resolves against the bound interface's
/// required signature (`infer_expr_type`'s `Expr::MethodCall` arm), not
/// against the `classes` registry.
fn check_generic_function_body(
  f: &Function,
  g: &GenericFunctionSig,
  sigs: &HashMap<String, FunctionSig>,
  classes: &HashMap<String, ClassInfo>,
  gctx: &GenericsCtx,
) -> Result<(), Diagnostic> {
  let mut env = HashMap::new();
  for p in &f.params {
    let t = if p.ty == g.type_param {
      Type::Generic(g.type_param.clone(), g.bound.clone())
    } else {
      resolve_type(&p.ty, classes)?
    };
    env.insert(p.name.clone(), t);
  }
  let declared_return = if f.return_type == g.type_param {
    Type::Generic(g.type_param.clone(), g.bound.clone())
  } else {
    resolve_type(&f.return_type, classes)?
  };
  check_block(
    &f.body,
    &mut env,
    sigs,
    classes,
    None,
    &declared_return,
    false,
    false,
    f.block_param.is_some(),
    gctx,
  )?;
  check_implicit_return(
    &f.body,
    &env,
    sigs,
    classes,
    None,
    &declared_return,
    &f.name,
    gctx,
  )
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
    // Plan 39: a call omitting a required (no-default) trailing
    // parameter now names it directly, rather than a blanket "expects N
    // argument(s)" — a real, plan-required diagnostic improvement (AC4:
    // "a compile-time arity diagnostic naming the missing required
    // parameter").
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject 1-arg call to 2-arg add");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("missing required argument `b`"))
    );
  }

  #[test]
  fn rejects_too_many_arguments_to_a_non_splat_function() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(1, 2, 3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a 3-arg call to a 2-arg add");
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

  // Plan 19 (string literals).

  #[test]
  fn accepts_string_let_and_puts() {
    let src = "s: String = \"hello\"\nputs s\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_string_concat() {
    let src = "a: String = \"foo\" + \"bar\"\nputs a\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_string_plus_int() {
    let src = "puts \"foo\" + 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject String + Int64");
    assert!(errs[0].message.contains("String") && errs[0].message.contains("Int64"));
  }

  #[test]
  fn accepts_string_equality_compare() {
    let src = "if \"abc\" == \"abc\"\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 20 (comments and case/when).

  const CASE_EXAMPLE: &str = "n: Int64 = 2\nlabel: Int64 = 0\ncase n\nwhen 1\n  label: Int64 = 10\nwhen 2, 3\n  label: Int64 = 20\nelse\n  label: Int64 = 99\nend\nputs label\n";

  #[test]
  fn accepts_case_when_example() {
    let program = emerald_parser::parse(CASE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_float64_case_scrutinee() {
    let src = "n: Float64 = 1.0\ncase n\nwhen 1\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a Float64 scrutinee");
    assert!(errs[0].message.contains("Int64"));
  }

  // Plan 25 (stdlib expansion).

  #[test]
  fn accepts_bool_literal_example() {
    let src = "def check(flag: Boolean) -> Int64\n  if flag\n    return 1\n  end\n  return 0\nend\n\nputs check(true)\nputs check(false)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_nil_literal_example() {
    let src = "def check_nil(x: Nil) -> Int64\n  if x == nil\n    return 1\n  end\n  return 0\nend\n\nputs check_nil(nil)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_nil_assigned_to_non_nil_type() {
    let src = "x: Int64 = nil\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `x: Int64 = nil`");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("Nil"));
  }

  #[test]
  fn accepts_hash_literal_get_and_set() {
    let src =
      "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\nputs h[2]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_empty_hash_literal() {
    let src = "h: Hash[Int64, Int64] = {}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty hash literal");
    assert!(errs[0].message.contains("empty hash literals"));
  }

  #[test]
  fn rejects_mixed_value_type_hash_literal() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 2.0}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a mixed value-type hash literal");
    assert!(errs[0].message.contains("value type"));
  }

  #[test]
  fn accepts_array_new_with_literal_and_runtime_size() {
    let src = "arr: Array[Int64] = Array.new(5)\nn: Int64 = 5\narr2: Array[Int64] = Array.new(n)\nputs arr[0]\nputs arr2[0]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_array_new_size_mismatch_with_declared_type() {
    let src = "arr: Int64 = Array.new(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject Array.new assigned to a non-Array type");
    assert!(errs[0].message.contains("Array.new"));
  }

  // Plan 28 (bitwise operators).

  const BITWISE_EXAMPLE: &str = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\ndef has_flag(flags: Int64, flag: Int64) -> Boolean\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ)\n  puts 1\nend\nif has_flag(perms, EXEC)\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";

  #[test]
  fn accepts_bitwise_example() {
    let program = emerald_parser::parse(BITWISE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_float64_bit_and_operand() {
    let src = "puts 2.0 & 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Float64 & Int64");
    assert!(errs[0].message.contains("&"));
    assert!(errs[0].message.contains("Int64") || errs[0].message.contains("Float64"));
  }

  #[test]
  fn rejects_float64_bit_not_operand() {
    let src = "puts ~2.5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `~2.5`");
    assert!(errs[0].message.contains("~"));
    assert!(errs[0].message.contains("Float64"));
  }

  #[test]
  fn bitwise_operators_type_check_as_int64() {
    let src = "a: Int64 = 1 << 2\nb: Int64 = 1 & 2\nc: Int64 = ~1\nputs a\nputs b\nputs c\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 30 (for-in iteration).

  #[test]
  fn accepts_for_in_with_var_usable_at_element_type() {
    let src = "for x in [1, 2, 3]\n  y: Int64 = x + 1\n  puts y\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_for_in_over_empty_array_literal() {
    // Grammar requires `[Args]`, and `Args` itself is grammar-legal
    // empty (`[]` parses fine) — this is a sema-level rejection, same
    // diagnostic as a bare `Expr::ArrayLit`.
    let src = "for x in []\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an empty for-in literal");
    assert!(errs[0].message.contains("empty array literals"));
  }

  #[test]
  fn rejects_for_in_over_heterogeneous_array_literal() {
    let src = "for x in [1, \"two\"]\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a heterogeneous for-in literal");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_break_and_next_inside_for_in() {
    let src = "for x in [1, 2, 3]\n  if x == 2\n    next\n  end\n  if x == 3\n    break\n  end\n  puts x\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 31 (compound and multiple assignment).

  #[test]
  fn accepts_bare_reassignment_of_an_existing_local() {
    let src = "x: Int64 = 1\nx = 2\nputs x\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_reassignment_of_an_undeclared_local() {
    let src = "y = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject reassigning an undeclared local");
    assert!(errs[0].message.contains("undefined variable"));
  }

  #[test]
  fn rejects_reassignment_type_mismatch() {
    let src = "x: Int64 = 1\nx = \"mismatched\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject Int64 reassigned to a String");
    assert!(errs[0].message.contains("Int64") && errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_compound_plus_assign_accumulator() {
    let src =
      "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5\n  total += i\n  i += 1\nend\nputs total\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_multiple_assignment_swap() {
    let src = "a: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_multiple_assignment_positional_type_mismatch() {
    let src = "a: Int64 = 1\nb: Int64 = 2\na, b = \"s\", 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a positional type mismatch in `a, b = ...`");
    assert!(errs[0].message.contains("position 1"));
  }

  #[test]
  fn rejects_multiple_assignment_arity_mismatch() {
    let src = "a: Int64 = 1\nb: Int64 = 2\na, b = 1, 2, 3\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an arity mismatch");
    assert!(errs[0].message.contains("arity mismatch"));
  }

  #[test]
  fn rejects_multiple_assignment_undefined_target() {
    let src = "a: Int64 = 1\na, z = 1, 2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undefined multi-assign target");
    assert!(errs[0].message.contains("undefined variable"));
  }

  // Plan 32 (class inheritance).

  const INHERITANCE_EXAMPLE: &str = "class Animal\n  age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def age -> Int64\n    @age\n  end\n\n  def describe -> Int64\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  def initialize(age: Int64, breed_code: Int64) -> Void\n    @age = age\n    @breed_code = breed_code\n  end\n\n  def describe -> Int64\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";

  #[test]
  fn accepts_inheritance_example() {
    let program = emerald_parser::parse(INHERITANCE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_cyclic_inheritance() {
    let src = "class A < B\n  x: Int64\nend\n\nclass B < A\n  y: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject cyclic inheritance");
    assert!(errs.iter().any(|e| e.message.contains("cyclic")));
  }

  #[test]
  fn rejects_field_redeclared_from_ancestor() {
    let src = "class Animal\n  age: Int64\nend\n\nclass Dog < Animal\n  age: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a subclass redeclaring an ancestor's field");
    assert!(errs[0].message.contains("age") && errs[0].message.contains("Animal"));
  }

  #[test]
  fn rejects_override_with_mismatched_signature() {
    let src = "class Animal\n  def speak(volume: Int64) -> Int64\n    volume\n  end\nend\n\nclass Dog < Animal\n  def speak(volume: Float64) -> Int64\n    1\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject an override with a mismatched signature");
    assert!(errs[0].message.contains("speak"));
  }

  #[test]
  fn rejects_undeclared_superclass() {
    let src = "class Dog < NotAClass\n  x: Int64\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject an undeclared superclass");
    assert!(errs[0].message.contains("NotAClass"));
  }

  // Plan 33 (field-access sugar).

  const READ_FIELD_EXAMPLE: &str = "class Point\n  read x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";

  #[test]
  fn accepts_read_field_accessed_from_outside() {
    let program = emerald_parser::parse(READ_FIELD_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_read_field_accessed_from_outside() {
    // Opt-in per field, not blanket exposure: `y` has no `read` marker,
    // so `p.y` must be rejected with the same "no such method"
    // diagnostic plan 08 already produces for any undeclared method.
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.y\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject `p.y` — `y` has no `read` marker");
    assert!(errs[0].message.contains('y'));
  }

  // Plan 34 (blocks and yield).

  const BLOCKS_EXAMPLE: &str = "def repeat(n: Int64, &blk) -> Void\n  i: Int64 = 0\n  while i < n\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) { |i: Int64| puts i }\n";

  #[test]
  fn accepts_blocks_and_yield_example() {
    let program = emerald_parser::parse(BLOCKS_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_to_block_param_function_with_no_trailing_block() {
    let src = "def repeat(n: Int64, &blk) -> Void\n  yield n\nend\n\nrepeat(3)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject calling a &blk function with no block");
    assert!(errs[0].message.contains("repeat") && errs[0].message.contains("block"));
  }

  #[test]
  fn rejects_block_arity_mismatch_against_yield() {
    let src = "def repeat(n: Int64, &blk) -> Void\n  i: Int64 = 0\n  while i < n\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) { |i: Int64, extra: Int64| puts i }\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject a block whose arity doesn't match yield's call sites");
    assert!(errs[0].message.contains("arity"));
  }

  #[test]
  fn rejects_yield_outside_a_block_param_function() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  yield 5\n  a + b\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject `yield` in a function with no block parameter");
    assert!(errs[0].message.contains("yield"));
  }

  // Plan 23 (multi-file compilation).

  #[test]
  fn item_require_is_a_no_op_in_sema() {
    // `emerald-driver`'s resolution step (plan 17) isn't extracted in
    // this codebase yet, so a `Program` still containing `Item::
    // Require` is real, disclosed, in-scope input here — sema treats
    // it as a no-op rather than panicking or rejecting the whole
    // program.
    let src = "require helpers\nputs 1\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 36 (string interpolation and heredocs).

  #[test]
  fn accepts_interpolation_of_all_compiler_known_types() {
    let src = "s: String = \"a\"\nn: Int64 = 1\nf: Float64 = 2.5\nb: Boolean = true\nputs \"#{s} #{n} #{f} #{b}\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_interpolation_of_an_unsupported_type() {
    let src = "arr: Array[Int64] = [1]\nputs \"#{arr}\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject interpolating an Array — not a compiler-known stringifiable type");
    assert!(errs[0].message.contains("cannot be interpolated"));
  }

  // Plan 37 (ranges and range-based iteration).

  #[test]
  fn accepts_range_for_in_with_literal_endpoints() {
    let src = "for i in 1..5\n  x: Int64 = i + 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_range_for_in_with_non_literal_endpoint_expression() {
    // No parenthesized-expression grouping exists anywhere in this
    // grammar (verified: no `"(" Expr ")"` production, only call-arg
    // and param-list parens) — `n - 1` parses fine unparenthesized as
    // the range's end operand, the real proof endpoints are arbitrary
    // expressions, not just literals.
    let src = "n: Int64 = 5\nfor i in 0..n - 1\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_non_int64_range_endpoint() {
    let src = "for i in \"a\"..\"z\"\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("must reject a String range endpoint, not just Int64");
    assert!(errs[0].message.contains("String"));
  }

  #[test]
  fn accepts_break_and_next_inside_a_range_for_in() {
    let src =
      "for i in 1..5\n  if i == 3\n    break\n  end\n  if i == 2\n    next\n  end\n  puts i\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 38 (full exception model).

  const FULL_EXCEPTION_EXAMPLE: &str = "class NotFoundError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\nclass TimeoutError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise TimeoutError.new(7)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";

  #[test]
  fn accepts_the_full_worked_example_two_typed_rescues_and_ensure() {
    let program = emerald_parser::parse(FULL_EXCEPTION_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_referencing_a_bare_rescue_bound_name_inside_its_own_body() {
    let src = "begin\n  puts 1\nrescue => e\n  puts e\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("a bare rescue's bound name must never enter env — no universal root class");
    assert!(errs[0].message.contains("undefined variable"));
  }

  #[test]
  fn rejects_retry_at_top_level() {
    let src = "retry\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject top-level retry");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn rejects_retry_inside_a_begin_try_body() {
    // A bare `rescue => e` avoids needing any class declared — this
    // test is purely about where `retry` is (il)legal, unrelated to a
    // typed clause's own class-resolution.
    let src = "begin\n  retry\nrescue => e\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("must reject retry inside the begin's own try body, not just top level");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn rejects_retry_inside_an_ensure_body() {
    let src = "begin\n  puts 1\nrescue => e\n  puts 2\nensure\n  retry\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject retry inside an ensure body");
    assert!(errs[0].message.contains("retry"));
  }

  #[test]
  fn accepts_retry_inside_a_rescue_clause_body() {
    let src = "begin\n  puts 1\nrescue => e\n  retry\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  // Plan 39 (function signature completeness).

  #[test]
  fn accepts_call_omitting_a_defaulted_trailing_argument() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n\nputs inc(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_call_overriding_a_default_with_an_explicit_argument() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n\nputs inc(5, 10)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_call_omitting_a_required_no_default_argument() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n\nputs inc()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`n` has no default — must be required");
    assert!(errs[0].message.contains("missing required argument `n`"));
  }

  #[test]
  fn rejects_default_parameter_on_a_method() {
    let src = "class Foo\n  def m(x: Int64 = 0) -> Int64\n    x\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("default parameter values are not supported on methods yet");
    assert!(errs[0].message.contains("not supported on methods"));
  }

  const GREET_EXAMPLE: &str = "def greet(name: String, times: Int64 = 1) -> Void\n  i: Int64 = 0\n  while i < times\n    puts name\n    i += 1\n  end\nend\n\ngreet(name: \"yo\")\ngreet(name: \"hi\", times: 2)\n";

  #[test]
  fn accepts_the_greet_worked_example_keyword_calls_and_defaults() {
    let program = emerald_parser::parse(GREET_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_misspelled_keyword_argument() {
    let src = "def greet(name: String) -> Void\n  puts name\nend\n\ngreet(nam: \"hi\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`nam` is not a declared parameter of greet");
    assert!(errs[0].message.contains("unrecognized keyword `nam`"));
  }

  #[test]
  fn rejects_call_missing_a_required_keyword() {
    // A bare `greet()` (no `name:` at all) would parse as an ordinary,
    // zero-arg `Expr::Call`, not `Expr::CallKw` — this test instead
    // supplies the defaulted keyword while omitting the required one,
    // to genuinely exercise `Expr::CallKw`'s own missing-keyword path.
    let src =
      "def greet(name: String, times: Int64 = 1) -> Void\n  puts name\nend\n\ngreet(times: 5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`name` has no default — must be required");
    assert!(errs[0].message.contains("missing required keyword `name`"));
  }

  #[test]
  fn rejects_duplicate_keyword_argument() {
    let src =
      "def greet(name: String) -> Void\n  puts name\nend\n\ngreet(name: \"hi\", name: \"yo\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("must reject a duplicate keyword");
    assert!(errs[0].message.contains("duplicate keyword `name`"));
  }

  const SUM_ALL_EXAMPLE: &str = "def sum_all(*xs: Int64) -> Int64\n  total: Int64 = 0\n  i: Int64 = 0\n  while i < 4\n    total += xs[i]\n    i += 1\n  end\n  total\nend\n\nputs sum_all(1, 2, 3, 4)\n";

  #[test]
  fn accepts_splat_call_type_checking_trailing_arguments() {
    let program = emerald_parser::parse(SUM_ALL_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_splat_call_with_zero_trailing_arguments() {
    let src = "def sum_all(*xs: Int64) -> Int64\n  0\nend\n\nputs sum_all()\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_splat_call_with_a_mismatched_trailing_argument_type() {
    let src = "def sum_all(*xs: Int64) -> Int64\n  0\nend\n\nputs sum_all(1, \"x\")\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("a String trailing argument must not match Int64");
    assert!(errs[0].message.contains("trailing (splat) argument 2"));
  }

  #[test]
  fn rejects_splat_parameter_on_a_method() {
    let src = "class Foo\n  def m(*xs: Int64) -> Int64\n    0\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs =
      check_program(&program).expect_err("splat parameters are not supported on methods yet");
    assert!(errs[0].message.contains("not supported on methods"));
  }

  // Plan 40 (operator overloading).

  const VECTOR2_EXAMPLE: &str = "class Vector2\n  read x: Float64\n  read y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def +(other: Vector2) -> Vector2\n    Vector2.new(@x + other.x, @y + other.y)\n  end\n\n  def ==(other: Vector2) -> Boolean\n    @x == other.x && @y == other.y\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0, 2.0)\nv2: Vector2 = Vector2.new(3.0, 4.0)\nv3: Vector2 = v1 + v2\nputs v3.x\nputs v3.y\nif v1 == v2\n  puts 1\nelse\n  puts 0\nend\nif v1 == v1\n  puts 1\nelse\n  puts 0\nend\n";

  #[test]
  fn accepts_the_vector2_worked_example() {
    let program = emerald_parser::parse(VECTOR2_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_plus_on_a_class_with_no_plus_method() {
    let src = "class Vector2\n  read x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nv3: Vector2 = v1 + v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Vector2 declares no `+` method");
    assert!(
      errs[0]
        .message
        .contains("class `Vector2` has no operator method `+`")
    );
  }

  #[test]
  fn rejects_operator_call_with_a_mismatched_argument_type() {
    let src = "class Vector2\n  read x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = x\n  end\n\n  def +(other: Vector2) -> Vector2\n    Vector2.new(@x + other.x)\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = v1 + 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`+` expects a Vector2, not an Int64");
    assert!(errs[0].message.contains("argument 1 to `+`"));
  }

  #[test]
  fn rejects_eq_on_a_class_with_no_eq_method() {
    let src = "class Vector2\n  read x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nb: Boolean = v1 == v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Vector2 declares no `==` method");
    assert!(
      errs[0]
        .message
        .contains("class `Vector2` has no operator method `==`")
    );
  }

  #[test]
  fn rejects_ordering_comparison_on_a_class() {
    let src = "class Vector2\n  read x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = x\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nb: Boolean = v1 < v2\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program)
      .expect_err("ordering comparisons on a class are not supported without `<=>`");
    assert!(errs[0].message.contains("ordering comparison"));
  }

  const BAG_EXAMPLE: &str = "class Bag\n  data: Array[Int64]\n\n  def initialize(a: Int64, b: Int64, c: Int64) -> Void\n    @data = [a, b, c]\n  end\n\n  def [](i: Int64) -> Int64\n    @data[i]\n  end\n\n  def []=(i: Int64, v: Int64) -> Void\n    @data[i] = v\n  end\nend\n\nb: Bag = Bag.new(10, 20, 30)\nputs b[0] + b[1] + b[2]\nb[1] = 99\nputs b[1]\n";

  #[test]
  fn accepts_the_bag_index_operator_example_read_and_write() {
    let program = emerald_parser::parse(BAG_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_index_operator_with_a_mismatched_index_type() {
    let src = "class Bag\n  data: Array[Int64]\n\n  def initialize(a: Int64) -> Void\n    @data = [a]\n  end\n\n  def [](i: Int64) -> Int64\n    @data[i]\n  end\nend\n\nb: Bag = Bag.new(1)\nx: Int64 = b[\"nope\"]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`[]` expects an Int64 index, not a String");
    assert!(errs[0].message.contains("argument 1 to `[]`"));
  }

  #[test]
  fn rejects_index_write_on_a_class_with_no_index_set_method() {
    let src = "class Bag\n  data: Array[Int64]\n\n  def initialize(a: Int64) -> Void\n    @data = [a]\n  end\n\n  def [](i: Int64) -> Int64\n    @data[i]\n  end\nend\n\nb: Bag = Bag.new(1)\nb[0] = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Bag declares no `[]=` method");
    assert!(
      errs[0]
        .message
        .contains("class `Bag` has no operator method `[]=`")
    );
  }

  // Plan 41 (interfaces and generics).

  const COMPARABLE_MAX_EXAMPLE: &str = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Int64\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  def initialize(meters: Int64) -> Void\n    @meters = meters\n  end\n\n  def compare_to(other: Distance) -> Int64\n    @meters - other.meters\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

  #[test]
  fn accepts_the_comparable_max_worked_example() {
    let program = emerald_parser::parse(COMPARABLE_MAX_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_implements_with_no_matching_method_defined() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Money never defines compare_to");
    assert!(errs[0].message.contains("Money"));
    assert!(errs[0].message.contains("Comparable"));
    assert!(errs[0].message.contains("compare_to"));
  }

  #[test]
  fn rejects_implements_with_a_mismatched_method_signature() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Boolean\n    true\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("compare_to returns the wrong type");
    assert!(errs[0].message.contains("does not match interface"));
  }

  #[test]
  fn rejects_implements_naming_an_undefined_interface() {
    let src = "class Money implements NotAnInterface\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("NotAnInterface is never declared");
    assert!(errs[0].message.contains("NotAnInterface"));
  }

  #[test]
  fn accepts_conformance_satisfied_by_an_inherited_method() {
    // Real proof conformance is checked against the flattened method
    // table, not just the class's own declared methods (plan 32's
    // contribution) — `Dog` declares no `describe` of its own at all.
    // Uses a `Self`-free interface method deliberately: `Self`
    // substitutes to the *leaf* class declaring `implements` (invariant,
    // not covariant — see the Decision log), so an ancestor's own
    // `Self`-typed method (typed at the ancestor's own name) would
    // conflict with that leaf substitution on a completely separate
    // axis this AC isn't testing.
    let src = "interface Describable\n  def describe(label: String) -> Int64\nend\n\nclass Animal\n  read age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def describe(label: String) -> Int64\n    @age\n  end\nend\n\nclass Dog < Animal implements Describable\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn generic_function_body_type_checks_once_against_the_bound_interface() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\ndef describe[T: Comparable](a: T, b: T) -> Int64\n  a.compare_to(b)\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_a_generic_call_with_inconsistent_type_parameter_arguments() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Int64\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  def initialize(meters: Int64) -> Void\n    @meters = meters\n  end\n\n  def compare_to(other: Distance) -> Int64\n    @meters - other.meters\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nd1: Distance = Distance.new(100)\nboom: Money = max(m1, d1)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("`T` resolves to both Money and Distance");
    assert!(errs[0].message.contains("inconsistently"));
  }

  #[test]
  fn rejects_a_generic_call_whose_concrete_type_does_not_implement_the_bound() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Widget\n  read id: Int64\n\n  def initialize(id: Int64) -> Void\n    @id = id\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\nw1: Widget = Widget.new(1)\nw2: Widget = Widget.new(2)\nboom: Widget = max(w1, w2)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Widget does not implement Comparable");
    assert!(errs[0].message.contains("does not implement"));
  }

  #[test]
  fn rejects_multiple_type_parameters_at_registration_time() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\ndef bad[T: Comparable, U: Comparable](a: T, b: U) -> T\n  a\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("two type parameters are not supported");
    assert!(
      errs[0]
        .message
        .contains("multiple type parameters are not supported")
    );
  }

  #[test]
  fn rejects_generic_methods_on_a_class() {
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Box\n  def pick[T: Comparable](a: T, b: T) -> T\n    a\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("generic methods are not supported");
    assert!(
      errs
        .iter()
        .any(|d| d.message.contains("generic methods are not supported"))
    );
  }

  // Plan 43 (nullable types and safe navigation).

  const GREETER_PREFIX: &str = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\n";

  const NULLABLE_WORKED_EXAMPLE: &str = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\ndef greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";

  #[test]
  fn accepts_the_nullable_worked_example() {
    let program = emerald_parser::parse(NULLABLE_WORKED_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_a_real_class_value_and_nil_both_widening_into_a_nullable_let() {
    let src = format!("{GREETER_PREFIX}g1: Greeter? = Greeter.new(\"ada\")\ng2: Greeter? = nil\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_nil_into_a_non_nullable_int64_let() {
    let src = "x: Int64 = nil\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn rejects_a_nullable_value_type_annotation() {
    let src = "y: Int64? = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Int64? is not supported");
    assert!(errs[0].message.contains("nullable"));
  }

  #[test]
  fn rejects_a_direct_method_call_on_a_nullable_receiver() {
    let src = format!(
      "{GREETER_PREFIX}def greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  return g.shout\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err("g is nullable, .shout is unguarded");
    assert!(errs[0].message.contains("&.") || errs[0].message.contains("nil"));
  }

  #[test]
  fn nullable_vs_nil_comparison_type_checks_to_boolean() {
    let src = format!(
      "{GREETER_PREFIX}def is_missing(id: Int64) -> Boolean\n  g: Greeter? = find_greeter(id)\n  return g == nil\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn safe_call_on_a_nullable_class_receiver_type_checks_to_the_wrapped_return_type() {
    let src = format!(
      "{GREETER_PREFIX}def greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_safe_call_on_a_non_nullable_receiver() {
    let src = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef greet -> String?\n  g: Greeter = Greeter.new(\"ada\")\n  return g&.shout\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("g is never nil, & . is illegal");
    assert!(errs[0].message.contains("nullable"));
  }

  #[test]
  fn rejects_safe_call_on_a_method_returning_a_value_type() {
    let src = "class Greeter\n  age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def years -> Int64\n    @age\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  return nil\nend\n\ndef ages(id: Int64) -> Int64\n  g: Greeter? = find_greeter(id)\n  x: Int64? = g&.years\n  return 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Int64 is not pointer-representable");
    assert!(errs.iter().any(|d| d.message.contains("Int64")));
  }

  #[test]
  fn or_assign_narrows_the_tracked_type_so_a_later_return_type_checks() {
    let src = format!(
      "{GREETER_PREFIX}def greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n"
    );
    let program = emerald_parser::parse(&src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  const UPGRADE_EXAMPLE: &str = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\ndef upgrade(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  g &&= Greeter.new(\"upgraded\")\n  message: String? = g&.shout\n  message ||= \"still nobody\"\n  return message\nend\n\nputs upgrade(1)\nputs upgrade(2)\n";

  #[test]
  fn accepts_the_and_assign_upgrade_worked_example() {
    let program = emerald_parser::parse(UPGRADE_EXAMPLE).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_or_assign_on_a_non_nullable_target() {
    let src = "s: String = \"x\"\ns ||= \"y\"\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("s is not nullable");
    assert!(errs[0].message.contains("nullable"));
  }

  #[test]
  fn rejects_and_assign_on_a_non_nullable_target() {
    let src =
      format!("{GREETER_PREFIX}s: Greeter = Greeter.new(\"ada\")\ns &&= Greeter.new(\"b\")\n");
    let program = emerald_parser::parse(&src).expect("should parse");
    let errs = check_program(&program).expect_err("s is not nullable");
    assert!(errs[0].message.contains("nullable"));
  }

  #[test]
  fn rejects_or_assign_default_that_is_itself_nullable() {
    let src = "message: String? = nil\nother: String? = nil\nmessage ||= other\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  // Plan 44 (symbols).

  #[test]
  fn accepts_a_symbol_typed_let() {
    let src = "x: Symbol = :foo\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn symbol_equality_and_inequality_both_type_check_to_boolean() {
    let src = "if :foo == :foo\n  puts 1\nelse\n  puts 0\nend\nif :foo == :bar\n  puts 1\nelse\n  puts 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn rejects_symbol_compared_against_string() {
    let src = "if :foo == \"foo\"\n  puts 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let errs = check_program(&program).expect_err("Symbol is not String");
    assert!(errs[0].message.contains("Symbol"));
    assert!(errs[0].message.contains("String"));
  }

  #[test]
  fn rejects_int64_literal_assigned_to_a_symbol_typed_let() {
    let src = "x: Symbol = 5\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert!(check_program(&program).is_err());
  }

  #[test]
  fn accepts_a_symbol_keyed_hash_literal_matching_its_declared_annotation() {
    let src = "h: Hash[Symbol, Int64] = {:a => 1, :b => 2}\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }

  #[test]
  fn accepts_the_symbols_worked_example() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar\n  puts 1\nelse\n  puts 0\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    assert_eq!(check_program(&program), Ok(()));
  }
}
