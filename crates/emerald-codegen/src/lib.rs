//! Emerald's codegen backend, built on LLVM via `inkwell`.
//!
//! Cranelift was v1's backend (`02 toolchain-prototype`); `16
//! codegen-backend-bakeoff` measured LLVM 5-18x faster on loop-heavy
//! code (`spec/COMPILER.md` has the full decision record and numbers).
//! This crate is the result of `consolidate-llvm-backend`: a faithful,
//! full-feature port of the old Cranelift backend onto LLVM, not a
//! redesign — every restriction the old backend had (lambdas only as a
//! top-level `Let`, method/index receivers must be a plain local
//! variable, one `rescue` clause, no inheritance) is preserved exactly.
//! Its own test suite below ports the old backend's tests verbatim
//! (same source strings, same expected outputs) as the proof.
//!
//! One notable, deliberate improvement over the old backend: class
//! instances / array bases / lambda environments / exception instances
//! are real LLVM `ptr` values here, not Cranelift's undifferentiated
//! `i64`. Byte layout is unchanged (every field/capture/element is
//! still 8 bytes, matching `runtime/emerald_runtime.c`'s `emerald_alloc`)
//! — this is a type-system correctness improvement with no behavior
//! change for well-typed Emerald programs.

use emerald_parser::{
  ClassDef, CompareOp, Expr, Function as AstFunction, Item, ModuleDef, Param, Program, Stmt,
};
use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
  CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, ValueKind};
use inkwell::{IntPredicate, OptimizationLevel};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Every runtime value this backend moves around is one of these
/// storage kinds — `Int64`/`Float64` scalars, `Ptr` (a class instance,
/// an `Array[T]` base address, or a `Proc`'s capture environment; all
/// just addresses at this level, exactly as they were Cranelift `i64`s
/// in the old backend), or `Str` (plan 19 — a pointer to a
/// null-terminated UTF-8 buffer, kept distinct from `Ptr` so `puts`/
/// `Add`/`Compare` can dispatch to the right runtime helper without a
/// side-table). `Void`/`Bool` are bookkeeping-only: `Void` never labels
/// an actual value, only a function's declared return kind; `Bool`
/// labels a `Expr::Compare`/`&&`/`||`/`!` result, or a real declared
/// `Boolean`-typed value (plan 18) — both share the same `i1` storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ValKind {
  Int64,
  Float64,
  Ptr,
  Str,
  Void,
  Bool,
  /// Plan 25 — deliberately narrow (no `T?` nullable-type system): a
  /// fixed `i64` sentinel (always `0`), real storage/param/return kind,
  /// never compared against anything but another `Nil`.
  Nil,
}

fn value_kind_for_type(ty: &str) -> ValKind {
  match ty {
    "Float64" => ValKind::Float64,
    "Int64" => ValKind::Int64,
    "Void" => ValKind::Void,
    // Plan 18: `Boolean` is a real, declarable type now (e.g. `def
    // noisy(n: Int64) -> Boolean`), not just an internal marker for a
    // `Compare`/`&&`/`||`/`!` result on its way straight into a
    // branch — both uses share the same `i1` storage kind.
    "Boolean" => ValKind::Bool,
    // Plan 19: `String` is a real, declarable type too — kept distinct
    // from the generic `Ptr` bucket (see `ValKind`'s doc comment).
    "String" => ValKind::Str,
    "Nil" => ValKind::Nil,
    // `Hash[K, V]` (plan 25) shares the generic `Ptr` bucket — unlike
    // `Array[Elem]`, indexing it needs a key type too, which the
    // side-table `local_classes` (repurposed to hold `"Hash[K, V]"`
    // strings alongside class names — see `build_index`) already
    // carries without needing a whole new parameter threaded through
    // every codegen function in this file.
    _ => ValKind::Ptr,
  }
}

/// The LLVM storage type for a `Let`/param/field/array-element/return
/// kind. `Void` never reaches here — an internal invariant (it only
/// ever labels a function's return kind, handled separately in
/// `make_fn_type`), not a user-input-dependent case.
fn local_llvm_type<'ctx>(context: &'ctx Context, kind: ValKind) -> BasicTypeEnum<'ctx> {
  match kind {
    ValKind::Int64 | ValKind::Nil => context.i64_type().into(),
    ValKind::Float64 => context.f64_type().into(),
    ValKind::Ptr | ValKind::Str => context.ptr_type(AddressSpace::default()).into(),
    ValKind::Bool => context.bool_type().into(),
    ValKind::Void => unreachable!("internal: Void never used as a storage type"),
  }
}

fn make_fn_type<'ctx>(
  context: &'ctx Context,
  param_kinds: &[ValKind],
  ret_kind: ValKind,
) -> FunctionType<'ctx> {
  let param_types: Vec<BasicMetadataTypeEnum> = param_kinds
    .iter()
    .map(|k| local_llvm_type(context, *k).into())
    .collect();
  match ret_kind {
    ValKind::Void => context.void_type().fn_type(&param_types, false),
    ValKind::Int64 | ValKind::Nil => context.i64_type().fn_type(&param_types, false),
    ValKind::Float64 => context.f64_type().fn_type(&param_types, false),
    ValKind::Ptr | ValKind::Str => context
      .ptr_type(AddressSpace::default())
      .fn_type(&param_types, false),
    ValKind::Bool => context.bool_type().fn_type(&param_types, false),
  }
}

/// A field's byte offset and storage kind within its class's instance
/// layout — every field is naively 8 bytes (matches the old Cranelift
/// backend's `FieldInfo`; see `spec/TYPE_SYSTEM.md` §8).
#[derive(Clone, Copy)]
struct FieldInfo {
  offset: u64,
  kind: ValKind,
}

struct ClassLayout {
  fields: HashMap<String, FieldInfo>,
  size: u64,
}

fn build_class_layout(c: &ClassDef) -> ClassLayout {
  let mut fields = HashMap::new();
  let mut offset = 0u64;
  for f in &c.fields {
    fields.insert(
      f.name.clone(),
      FieldInfo {
        offset,
        kind: value_kind_for_type(&f.ty),
      },
    );
    offset += 8;
  }
  ClassLayout {
    fields,
    size: offset,
  }
}

/// A lambda literal's captured-variable layout — the closure-conversion
/// counterpart to `ClassLayout` (env buffers are laid out exactly like
/// an instance's fields, one 8-byte slot per capture, in
/// first-occurrence order).
struct LambdaInfo {
  captures: Vec<String>,
  capture_offsets: HashMap<String, u64>,
  capture_kinds: HashMap<String, ValKind>,
}

// --- Free-variable analysis (language-only — no LLVM/Cranelift API) ---

fn collect_idents_in_expr(expr: &Expr, out: &mut Vec<String>) {
  match expr {
    Expr::Ident(name) => out.push(name.clone()),
    Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::InstanceVar(_)
    | Expr::Lambda { .. }
    | Expr::Bool(_)
    | Expr::Nil => {}
    Expr::ArrayNew(size) => collect_idents_in_expr(size, out),
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        collect_idents_in_expr(k, out);
        collect_idents_in_expr(v, out);
      }
    }
    Expr::Add(l, r)
    | Expr::Sub(l, r)
    | Expr::Mul(l, r)
    | Expr::Div(l, r)
    | Expr::Rem(l, r)
    | Expr::And(l, r)
    | Expr::Or(l, r)
    | Expr::BitAnd(l, r)
    | Expr::BitOr(l, r)
    | Expr::BitXor(l, r)
    | Expr::Shl(l, r)
    | Expr::Shr(l, r)
    | Expr::Index(l, r) => {
      collect_idents_in_expr(l, out);
      collect_idents_in_expr(r, out);
    }
    Expr::Neg(e) | Expr::Not(e) | Expr::BitNot(e) => collect_idents_in_expr(e, out),
    Expr::Compare(l, _, r) => {
      collect_idents_in_expr(l, out);
      collect_idents_in_expr(r, out);
    }
    Expr::Call(_, args) | Expr::New(_, args) => {
      for a in args {
        collect_idents_in_expr(a, out);
      }
    }
    Expr::MethodCall(recv, _, args) => {
      collect_idents_in_expr(recv, out);
      for a in args {
        collect_idents_in_expr(a, out);
      }
    }
    Expr::ArrayLit(elements) => {
      for e in elements {
        collect_idents_in_expr(e, out);
      }
    }
  }
}

fn collect_idents_in_stmt(stmt: &Stmt, referenced: &mut Vec<String>, bound: &mut HashSet<String>) {
  match stmt {
    Stmt::Let { name, value, .. } => {
      bound.insert(name.clone());
      collect_idents_in_expr(value, referenced);
    }
    Stmt::SetField { value, .. } => collect_idents_in_expr(value, referenced),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      collect_idents_in_expr(array, referenced);
      collect_idents_in_expr(index, referenced);
      collect_idents_in_expr(value, referenced);
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      collect_idents_in_expr(cond, referenced);
      for s in then_branch {
        collect_idents_in_stmt(s, referenced, bound);
      }
      if let Some(else_b) = else_branch {
        for s in else_b {
          collect_idents_in_stmt(s, referenced, bound);
        }
      }
    }
    Stmt::While { cond, body } => {
      collect_idents_in_expr(cond, referenced);
      for s in body {
        collect_idents_in_stmt(s, referenced, bound);
      }
    }
    Stmt::Return(Some(e)) => collect_idents_in_expr(e, referenced),
    Stmt::Return(None) | Stmt::Break | Stmt::Next => {}
    Stmt::Expr(e) => collect_idents_in_expr(e, referenced),
    Stmt::Raise(e) => collect_idents_in_expr(e, referenced),
    Stmt::Begin {
      body,
      rescue_var,
      rescue_body,
      ..
    } => {
      for s in body {
        collect_idents_in_stmt(s, referenced, bound);
      }
      bound.insert(rescue_var.clone());
      for s in rescue_body {
        collect_idents_in_stmt(s, referenced, bound);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      collect_idents_in_expr(scrutinee, referenced);
      for (values, body) in arms {
        for v in values {
          collect_idents_in_expr(v, referenced);
        }
        for s in body {
          collect_idents_in_stmt(s, referenced, bound);
        }
      }
      if let Some(else_b) = else_body {
        for s in else_b {
          collect_idents_in_stmt(s, referenced, bound);
        }
      }
    }
  }
}

fn free_vars_in_lambda(params: &[Param], body: &[Stmt]) -> Vec<String> {
  let mut referenced = Vec::new();
  let mut bound: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
  for s in body {
    collect_idents_in_stmt(s, &mut referenced, &mut bound);
  }
  let mut seen = HashSet::new();
  let mut captures = Vec::new();
  for name in referenced {
    if !bound.contains(&name) && seen.insert(name.clone()) {
      captures.push(name);
    }
  }
  captures
}

fn collect_lambda_infos(program: &Program) -> Result<HashMap<String, LambdaInfo>, String> {
  let mut top_level_types: HashMap<String, String> = HashMap::new();
  for item in &program.items {
    if let Item::Stmt(Stmt::Let { name, ty, .. }) = item {
      top_level_types.insert(name.clone(), ty.clone());
    }
  }

  let mut lambda_infos: HashMap<String, LambdaInfo> = HashMap::new();
  for item in &program.items {
    let Item::Stmt(Stmt::Let {
      name,
      ty,
      value: Expr::Lambda { params, body, .. },
    }) = item
    else {
      continue;
    };
    if ty != "Proc" {
      continue;
    }

    let captures = free_vars_in_lambda(params, body);
    let mut capture_offsets = HashMap::new();
    let mut capture_kinds = HashMap::new();
    for (i, cap_name) in captures.iter().enumerate() {
      capture_offsets.insert(cap_name.clone(), i as u64 * 8);
      let cap_ty_name = top_level_types.get(cap_name).ok_or_else(|| {
        format!("codegen: cannot determine the type of captured variable `{cap_name}`")
      })?;
      capture_kinds.insert(cap_name.clone(), value_kind_for_type(cap_ty_name));
    }

    lambda_infos.insert(
      name.clone(),
      LambdaInfo {
        captures,
        capture_offsets,
        capture_kinds,
      },
    );
  }
  Ok(lambda_infos)
}

/// Recursively collects every `Let`-bound name (and every `rescue`
/// clause's bound exception variable) reachable from `stmts`, at any
/// `While`/`If`/`Begin` nesting depth, so every one of them gets a
/// single `alloca` up front in the function's entry block — what makes
/// LLVM's `mem2reg` pass able to promote them straight to SSA
/// registers (an `alloca` created fresh inside a loop body, rather than
/// once at entry, defeats that). Mirrors the old Cranelift backend's
/// own flat, unscoped `vars` map (see `spec/SEMANTICS.md`'s flat-scope
/// note) — just hoisted up front instead of declared lazily, since
/// Cranelift's own `Variable` SSA construction doesn't need the entry-
/// block-dominance discipline LLVM's `alloca`+`mem2reg` does.
fn collect_lets(stmts: &[Stmt], out: &mut Vec<(String, ValKind)>) {
  for stmt in stmts {
    match stmt {
      Stmt::Let { name, ty, .. } => out.push((name.clone(), value_kind_for_type(ty))),
      Stmt::While { body, .. } => collect_lets(body, out),
      Stmt::If {
        then_branch,
        else_branch,
        ..
      } => {
        collect_lets(then_branch, out);
        if let Some(else_b) = else_branch {
          collect_lets(else_b, out);
        }
      }
      Stmt::Begin {
        body,
        rescue_var,
        rescue_body,
        ..
      } => {
        collect_lets(body, out);
        out.push((rescue_var.clone(), ValKind::Ptr));
        collect_lets(rescue_body, out);
      }
      Stmt::Case {
        arms, else_body, ..
      } => {
        for (_, body) in arms {
          collect_lets(body, out);
        }
        if let Some(else_b) = else_body {
          collect_lets(else_b, out);
        }
      }
      _ => {}
    }
  }
}

/// Pre-allocates one `alloca` per `(name, kind)` pair in the function's
/// current (entry) block, skipping any name already present in `vars`
/// (a param/capture that happens to share a name with a `Let` inside
/// the body — the existing slot wins, matching the old backend's own
/// "redefine, don't redeclare" behavior for a re-`Let` of an existing
/// name).
fn prealloc_lets<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  decls: &[(String, ValKind)],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
) -> Result<(), String> {
  for (name, kind) in decls {
    if vars.contains_key(name) {
      continue;
    }
    let alloca = builder
      .build_alloca(local_llvm_type(context, *kind), name)
      .map_err(|e| e.to_string())?;
    vars.insert(name.clone(), (alloca, *kind));
  }
  Ok(())
}

/// The `setjmp`/longjmp-based exception runtime's imported functions
/// (NOT true native unwinding — see `runtime/emerald_runtime.c`'s own
/// comment for the full rationale). `setjmp` is called *directly* by
/// generated code (`build_begin`), never wrapped in a runtime helper —
/// `longjmp` must target a `setjmp` call site in a still-live stack
/// frame, which a wrapper function would violate.
#[derive(Clone, Copy)]
struct ExceptionRuntimeFuncs<'ctx> {
  setjmp: FunctionValue<'ctx>,
  push_handler: FunctionValue<'ctx>,
  handler_jmpbuf: FunctionValue<'ctx>,
  pop_handler: FunctionValue<'ctx>,
  free_handler: FunctionValue<'ctx>,
  handler_tag: FunctionValue<'ctx>,
  handler_exception_ptr: FunctionValue<'ctx>,
  raise: FunctionValue<'ctx>,
}

fn declare_exception_runtime_funcs<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
) -> ExceptionRuntimeFuncs<'ctx> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let i32_ty = context.i32_type();
  let void_ty = context.void_type();

  let setjmp = module.add_function(
    "setjmp",
    i32_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let push_handler = module.add_function(
    "emerald_push_handler",
    ptr_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let handler_jmpbuf = module.add_function(
    "emerald_handler_jmpbuf",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let pop_handler = module.add_function(
    "emerald_pop_handler",
    void_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let free_handler = module.add_function(
    "emerald_free_handler",
    void_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let handler_tag = module.add_function(
    "emerald_handler_tag",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let handler_exception_ptr = module.add_function(
    "emerald_handler_exception_ptr",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let raise = module.add_function(
    "emerald_raise",
    void_ty.fn_type(&[i64_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );

  ExceptionRuntimeFuncs {
    setjmp,
    push_handler,
    handler_jmpbuf,
    pop_handler,
    free_handler,
    handler_tag,
    handler_exception_ptr,
    raise,
  }
}

/// The header/exit blocks of the innermost enclosing loop, for `break`
/// (jump to `exit`) / `next` (jump back to `header`) to target.
struct LoopTargets<'ctx> {
  header: BasicBlock<'ctx>,
  exit: BasicBlock<'ctx>,
}

/// Context that's fixed for the duration of compiling one function/
/// method/lambda body. `self_ctx` is `Some((self_ptr, &class.fields))`
/// only while compiling a method body.
#[derive(Clone, Copy)]
struct Ctx<'a, 'ctx> {
  user_func_ids: &'a HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  classes: &'a HashMap<String, ClassLayout>,
  print_i64: FunctionValue<'ctx>,
  print_f64: FunctionValue<'ctx>,
  alloc: FunctionValue<'ctx>,
  /// Plan 19's `String` runtime helpers.
  print_str: FunctionValue<'ctx>,
  string_concat: FunctionValue<'ctx>,
  string_eq: FunctionValue<'ctx>,
  self_ctx: Option<(PointerValue<'ctx>, &'a HashMap<String, FieldInfo>)>,
  /// `{lambda's Let name} -> (its synthesized `__lambda_{name}` function,
  /// its declared return kind)`, for statically dispatching `.call`.
  lambda_func_ids: &'a HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  lambda_infos: &'a HashMap<String, LambdaInfo>,
  /// `{class name} -> a stable integer tag (declaration order)` —
  /// `rescue`'s matching mechanism, standing in for RTTI (no
  /// inheritance exists in this compiler, so exact-tag equality is
  /// fully correct, not a shortcut).
  class_tags: &'a HashMap<String, i64>,
  exc_funcs: ExceptionRuntimeFuncs<'ctx>,
  /// Names of every top-level `module` — `build_method_call` checks
  /// this before anything else to route `Name.method(args)` to the
  /// module's `{Name}_{method}` function directly, with no receiver
  /// value at all (modules have no fields/self, unlike classes/Procs).
  module_names: &'a HashSet<String>,
  /// Plan 25's `Array.new(size)` (`calloc`-backed, unlike `alloc`'s
  /// bare `malloc`) and `Hash[K, V]` key-not-found abort helper.
  alloc_zeroed: FunctionValue<'ctx>,
  hash_key_not_found: FunctionValue<'ctx>,
}

/// `local_classes` maps a local variable name to its declared class
/// name (from `Stmt::Let`'s explicit type annotation) — this backend
/// has no typed IR to consult, so a `.method` call on a local resolves
/// its receiver's class this way rather than from the (type-erased)
/// runtime pointer value. `local_array_elem_types` is the same idea for
/// `Array[Elem]` locals, mapping to `Elem`'s storage kind.
type IntBinOp<'ctx> = fn(
  &Builder<'ctx>,
  IntValue<'ctx>,
  IntValue<'ctx>,
  &str,
) -> Result<IntValue<'ctx>, inkwell::builder::BuilderError>;
type FloatBinOp<'ctx> =
  fn(
    &Builder<'ctx>,
    inkwell::values::FloatValue<'ctx>,
    inkwell::values::FloatValue<'ctx>,
    &str,
  ) -> Result<inkwell::values::FloatValue<'ctx>, inkwell::builder::BuilderError>;

/// `Sub`/`Mul`/`Div`/`Rem` (plan 18) each apply the exact rule `Add`
/// already enforces: both operands must be Int64 or both Float64, no
/// implicit conversion. `float_op` is `None` for `Rem` — LLVM has no
/// native floating-point-remainder instruction and this backend has no
/// general libm-call plumbing yet (plan 18's Decision log).
#[allow(clippy::too_many_arguments)]
fn build_numeric_binop<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  op_symbol: &str,
  lhs: &Expr,
  rhs: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
  int_op: IntBinOp<'ctx>,
  float_op: Option<FloatBinOp<'ctx>>,
  name: &str,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (l, lk) = build_expr(
    context,
    builder,
    lhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (r, rk) = build_expr(
    context,
    builder,
    rhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  match (lk, rk) {
    (ValKind::Int64, ValKind::Int64) => {
      let v =
        int_op(builder, l.into_int_value(), r.into_int_value(), name).map_err(|e| e.to_string())?;
      Ok((v.into(), ValKind::Int64))
    }
    (ValKind::Float64, ValKind::Float64) => {
      let Some(float_op) = float_op else {
        return Err(format!("codegen: `{op_symbol}` does not support Float64"));
      };
      let v = float_op(builder, l.into_float_value(), r.into_float_value(), name)
        .map_err(|e| e.to_string())?;
      Ok((v.into(), ValKind::Float64))
    }
    _ => Err(format!(
      "codegen: `{op_symbol}` operands must both be Int64 or both Float64"
    )),
  }
}

/// `&`/`|`/`^`/`<<`/`>>` (plan 28): Int64-only — sema has already
/// rejected any non-Int64 operand by the time codegen sees these nodes,
/// so unlike `build_numeric_binop` there's no Float64 branch to dispatch
/// on; the operand-kind check here is still real (not a panic) for any
/// caller that reaches codegen without going through sema first (the
/// same defensive standard every prior codegen plan holds to).
#[allow(clippy::too_many_arguments)]
fn build_bitwise_binop<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  op_symbol: &str,
  lhs: &Expr,
  rhs: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
  int_op: IntBinOp<'ctx>,
  name: &str,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (l, lk) = build_expr(
    context,
    builder,
    lhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (r, rk) = build_expr(
    context,
    builder,
    rhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if lk != ValKind::Int64 || rk != ValKind::Int64 {
    return Err(format!(
      "codegen: `{op_symbol}` operands must both be Int64"
    ));
  }
  let v =
    int_op(builder, l.into_int_value(), r.into_int_value(), name).map_err(|e| e.to_string())?;
  Ok((v.into(), ValKind::Int64))
}

/// `>>` on a signed `Int64` always wants arithmetic (sign-preserving)
/// shift — `sshr`, not `ushr` (plan 28's Decision log). `Builder::
/// build_right_shift` takes a `sign_extend` flag `IntBinOp`'s fn-pointer
/// shape has no room for, so this bakes `true` in and matches
/// `IntBinOp`'s shape directly, the same trick `build_numeric_binop`'s
/// callers already use for `Builder::build_int_sub`/etc.
fn build_sshr<'ctx>(
  builder: &Builder<'ctx>,
  lhs: IntValue<'ctx>,
  rhs: IntValue<'ctx>,
  name: &str,
) -> Result<IntValue<'ctx>, inkwell::builder::BuilderError> {
  builder.build_right_shift(lhs, rhs, true, name)
}

/// `&&`/`||` (plan 18): real short-circuit branching, the same
/// `append_basic_block`/`build_conditional_branch`/merge shape
/// `Stmt::If` already uses — not an eager, unconditionally-evaluated
/// bitwise AND/OR. For `&&`: a false left operand branches straight to
/// `merge` carrying `false`, never evaluating (and never emitting code
/// for) the right operand; a true left operand falls into a block that
/// evaluates the right operand and branches to the same `merge`
/// carrying that result. Mirrored, inverted, for `||`. The two
/// incoming values are reconciled with a `phi` node at `merge`.
#[allow(clippy::too_many_arguments)]
fn build_short_circuit<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  is_and: bool,
  lhs: &Expr,
  rhs: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let op_symbol = if is_and { "&&" } else { "||" };
  let (lv, lk) = build_expr(
    context,
    builder,
    lhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if lk != ValKind::Bool {
    return Err(format!(
      "codegen: `{op_symbol}` requires a Boolean left operand"
    ));
  }
  let lhs_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block")?;
  let func = lhs_block
    .get_parent()
    .ok_or("codegen: internal error — block has no parent function")?;
  let rhs_block = context.append_basic_block(func, if is_and { "and.rhs" } else { "or.rhs" });
  let merge_block = context.append_basic_block(func, if is_and { "and.merge" } else { "or.merge" });

  let lv_int = lv.into_int_value();
  if is_and {
    builder
      .build_conditional_branch(lv_int, rhs_block, merge_block)
      .map_err(|e| e.to_string())?;
  } else {
    builder
      .build_conditional_branch(lv_int, merge_block, rhs_block)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(rhs_block);
  let (rv, rk) = build_expr(
    context,
    builder,
    rhs,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if rk != ValKind::Bool {
    return Err(format!(
      "codegen: `{op_symbol}` requires a Boolean right operand"
    ));
  }
  // The right operand may itself have branched internally (nested
  // `&&`/`||`), so the predecessor to record for the `phi` is wherever
  // the builder actually ended up, not `rhs_block` itself.
  let rhs_end_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block after right operand")?;
  builder
    .build_unconditional_branch(merge_block)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(merge_block);
  let phi = builder
    .build_phi(
      context.bool_type(),
      if is_and { "andresult" } else { "orresult" },
    )
    .map_err(|e| e.to_string())?;
  let short_circuit_val = context.bool_type().const_int(u64::from(!is_and), false);
  phi.add_incoming(&[
    (&short_circuit_val, lhs_block),
    (&rv.into_int_value(), rhs_end_block),
  ]);
  Ok((phi.as_basic_value(), ValKind::Bool))
}

fn build_expr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  expr: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  match expr {
    Expr::Ident(name) => {
      let (ptr, kind) = *vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let loaded = builder
        .build_load(local_llvm_type(context, kind), ptr, name)
        .map_err(|e| e.to_string())?;
      Ok((loaded, kind))
    }
    Expr::Int(n) => Ok((
      context.i64_type().const_int(*n as u64, true).into(),
      ValKind::Int64,
    )),
    Expr::Float(f) => Ok((context.f64_type().const_float(*f).into(), ValKind::Float64)),
    // Plan 19: `"..."` — a compile-time-constant byte sequence
    // (plus a trailing NUL, matching `String`'s null-terminated
    // representation) materialized as a private global constant, with
    // a pointer to it returned as this expression's value.
    // `build_global_string_ptr` deliberately does *not* deduplicate
    // identical literals — a real, disclosed, low-risk optimization
    // left for later (plan 19's Decision log AC2).
    Expr::StringLit(s) => {
      let global = builder
        .build_global_string_ptr(s, "strlit")
        .map_err(|e| e.to_string())?;
      Ok((global.as_pointer_value().into(), ValKind::Str))
    }
    Expr::Add(lhs, rhs) => {
      let (l, lk) = build_expr(
        context,
        builder,
        lhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let (r, rk) = build_expr(
        context,
        builder,
        rhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      match (lk, rk) {
        (ValKind::Int64, ValKind::Int64) => {
          let sum = builder
            .build_int_add(l.into_int_value(), r.into_int_value(), "addtmp")
            .map_err(|e| e.to_string())?;
          Ok((sum.into(), ValKind::Int64))
        }
        (ValKind::Float64, ValKind::Float64) => {
          let sum = builder
            .build_float_add(l.into_float_value(), r.into_float_value(), "faddtmp")
            .map_err(|e| e.to_string())?;
          Ok((sum.into(), ValKind::Float64))
        }
        // Plan 19: `+` on two `String`s concatenates.
        (ValKind::Str, ValKind::Str) => {
          let call = builder
            .build_call(ctx.string_concat, &[l.into(), r.into()], "concattmp")
            .map_err(|e| e.to_string())?;
          Ok((call_result(call)?, ValKind::Str))
        }
        _ => {
          Err("codegen: `+` operands must both be Int64, both Float64, or both String".to_string())
        }
      }
    }
    Expr::Sub(lhs, rhs) => build_numeric_binop(
      context,
      builder,
      "-",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_int_sub,
      Some(Builder::build_float_sub),
      "subtmp",
    ),
    Expr::Mul(lhs, rhs) => build_numeric_binop(
      context,
      builder,
      "*",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_int_mul,
      Some(Builder::build_float_mul),
      "multmp",
    ),
    // Integer division/remainder by a runtime-zero divisor traps (LLVM's
    // `sdiv`/`srem` lower straight to hardware `idiv`, which raises
    // `SIGFPE` on zero — the same disclosed behavior the old Cranelift
    // backend's `sdiv`/`srem` had; not a new safety regression).
    Expr::Div(lhs, rhs) => build_numeric_binop(
      context,
      builder,
      "/",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_int_signed_div,
      Some(Builder::build_float_div),
      "divtmp",
    ),
    // `Float64 %` is out of scope (plan 18's Decision log — no libm-call
    // plumbing exists yet for `fmod`); `Int64 %` lowers directly to
    // `srem`.
    Expr::Rem(lhs, rhs) => build_numeric_binop(
      context,
      builder,
      "%",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_int_signed_rem,
      None,
      "remtmp",
    ),
    Expr::Neg(e) => {
      let (v, k) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      match k {
        ValKind::Int64 => {
          let n = builder
            .build_int_neg(v.into_int_value(), "negtmp")
            .map_err(|e| e.to_string())?;
          Ok((n.into(), ValKind::Int64))
        }
        ValKind::Float64 => {
          let n = builder
            .build_float_neg(v.into_float_value(), "fnegtmp")
            .map_err(|e| e.to_string())?;
          Ok((n.into(), ValKind::Float64))
        }
        _ => Err("codegen: unary `-` requires an Int64 or Float64 operand".to_string()),
      }
    }
    Expr::Not(e) => {
      let (v, k) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if k != ValKind::Bool {
        return Err("codegen: `!` requires a Boolean (comparison) operand".to_string());
      }
      let n = builder
        .build_not(v.into_int_value(), "nottmp")
        .map_err(|e| e.to_string())?;
      Ok((n.into(), ValKind::Bool))
    }
    // Real short-circuit branching (not an eager bitwise AND/OR over two
    // unconditionally-evaluated operands) — see `build_short_circuit`.
    Expr::And(lhs, rhs) => build_short_circuit(
      context,
      builder,
      true,
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Expr::Or(lhs, rhs) => build_short_circuit(
      context,
      builder,
      false,
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Expr::BitAnd(lhs, rhs) => build_bitwise_binop(
      context,
      builder,
      "&",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_and,
      "andtmp",
    ),
    Expr::BitOr(lhs, rhs) => build_bitwise_binop(
      context,
      builder,
      "|",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_or,
      "ortmp",
    ),
    Expr::BitXor(lhs, rhs) => build_bitwise_binop(
      context,
      builder,
      "^",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_xor,
      "xortmp",
    ),
    Expr::Shl(lhs, rhs) => build_bitwise_binop(
      context,
      builder,
      "<<",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      Builder::build_left_shift,
      "shltmp",
    ),
    Expr::Shr(lhs, rhs) => build_bitwise_binop(
      context,
      builder,
      ">>",
      lhs,
      rhs,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
      build_sshr,
      "shrtmp",
    ),
    Expr::BitNot(e) => {
      let (v, k) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if k != ValKind::Int64 {
        return Err("codegen: `~` requires an Int64 operand".to_string());
      }
      let n = builder
        .build_not(v.into_int_value(), "bitnottmp")
        .map_err(|e| e.to_string())?;
      Ok((n.into(), ValKind::Int64))
    }
    Expr::Compare(lhs, op, rhs) => {
      let (l, lk) = build_expr(
        context,
        builder,
        lhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let (r, rk) = build_expr(
        context,
        builder,
        rhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let cmp = match (lk, rk) {
        (ValKind::Int64, ValKind::Int64) => {
          let pred = match op {
            CompareOp::Lt => IntPredicate::SLT,
            CompareOp::Gt => IntPredicate::SGT,
            CompareOp::Le => IntPredicate::SLE,
            CompareOp::Ge => IntPredicate::SGE,
            CompareOp::Eq => IntPredicate::EQ,
            CompareOp::Ne => IntPredicate::NE,
          };
          builder
            .build_int_compare(pred, l.into_int_value(), r.into_int_value(), "cmptmp")
            .map_err(|e| e.to_string())?
        }
        (ValKind::Float64, ValKind::Float64) => {
          use inkwell::FloatPredicate;
          let pred = match op {
            CompareOp::Lt => FloatPredicate::OLT,
            CompareOp::Gt => FloatPredicate::OGT,
            CompareOp::Le => FloatPredicate::OLE,
            CompareOp::Ge => FloatPredicate::OGE,
            CompareOp::Eq => FloatPredicate::OEQ,
            CompareOp::Ne => FloatPredicate::ONE,
          };
          builder
            .build_float_compare(pred, l.into_float_value(), r.into_float_value(), "fcmptmp")
            .map_err(|e| e.to_string())?
        }
        // Plan 19: `==`/`!=` on two `String`s work "for free" under
        // this generic `lt == rt` dispatch — `<`/`>`/`<=`/`>=` on
        // strings are a real, disclosed, pre-existing gap (no
        // lexicographic ordering is defined anywhere) this plan
        // exposes but doesn't fix; they fail loudly here rather than
        // silently emitting wrong code.
        (ValKind::Str, ValKind::Str) if matches!(op, CompareOp::Eq | CompareOp::Ne) => {
          let call = builder
            .build_call(ctx.string_eq, &[l.into(), r.into()], "streqtmp")
            .map_err(|e| e.to_string())?;
          let eq_i64 = call_result(call)?.into_int_value();
          let one = context.i64_type().const_int(1, false);
          let pred = if matches!(op, CompareOp::Eq) {
            IntPredicate::EQ
          } else {
            IntPredicate::NE
          };
          builder
            .build_int_compare(pred, eq_i64, one, "strcmptmp")
            .map_err(|e| e.to_string())?
        }
        (ValKind::Str, ValKind::Str) => {
          return Err(format!(
            "codegen: `{op:?}` is not supported on String — only `==`/`!=` are (no lexicographic ordering is defined)"
          ));
        }
        // Plan 25: `Nil == Nil`/`Nil != Nil` — trivial (both operands
        // are always the same fixed `i64` sentinel) but real: a genuine
        // `icmp`, not hand-folded to a constant, so it flows through
        // the same generic machinery every other comparison does.
        (ValKind::Nil, ValKind::Nil) if matches!(op, CompareOp::Eq | CompareOp::Ne) => {
          let pred = if matches!(op, CompareOp::Eq) {
            IntPredicate::EQ
          } else {
            IntPredicate::NE
          };
          builder
            .build_int_compare(pred, l.into_int_value(), r.into_int_value(), "nilcmptmp")
            .map_err(|e| e.to_string())?
        }
        (ValKind::Nil, ValKind::Nil) => {
          return Err(format!(
            "codegen: `{op:?}` is not supported on Nil — only `==`/`!=` are"
          ));
        }
        _ => return Err("codegen: comparison operands must both be Int64 or both Float64".into()),
      };
      Ok((cmp.into(), ValKind::Bool))
    }
    Expr::Call(name, args) => {
      let (fv, ret_kind) = *ctx.user_func_ids.get(name).ok_or_else(|| {
        format!("codegen: unsupported call to `{name}` (not a compiled user function)")
      })?;
      if ret_kind == ValKind::Void {
        return Err(format!(
          "codegen: `{name}` returns Void and can't be used as a value"
        ));
      }
      let mut arg_vals = Vec::with_capacity(args.len());
      for a in args {
        let (v, _) = build_expr(
          context,
          builder,
          a,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        arg_vals.push(v.into());
      }
      let call = builder
        .build_call(fv, &arg_vals, "calltmp")
        .map_err(|e| e.to_string())?;
      let result = call_result(call)?;
      Ok((result, ret_kind))
    }
    Expr::New(class_name, args) => {
      let layout = ctx
        .classes
        .get(class_name)
        .ok_or_else(|| format!("codegen: unknown class `{class_name}`"))?;
      let size_val = context.i64_type().const_int(layout.size, false);
      let alloc_call = builder
        .build_call(ctx.alloc, &[size_val.into()], "newtmp")
        .map_err(|e| e.to_string())?;
      let ptr = call_result(alloc_call)?.into_pointer_value();

      let init_key = format!("{class_name}_initialize");
      if let Some(&(init_fv, _)) = ctx.user_func_ids.get(&init_key) {
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![ptr.into()];
        for a in args {
          let (v, _) = build_expr(
            context,
            builder,
            a,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?;
          call_args.push(v.into());
        }
        builder
          .build_call(init_fv, &call_args, "inittmp")
          .map_err(|e| e.to_string())?;
      }
      Ok((ptr.into(), ValKind::Ptr))
    }
    Expr::MethodCall(recv, method, args) => build_method_call(
      context,
      builder,
      recv,
      method,
      args,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Expr::InstanceVar(name) => {
      let (self_ptr, fields) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name}` used outside of a method body"))?;
      let field = *fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?;
      let loaded = load_field(context, builder, self_ptr, field)?;
      Ok((loaded, field.kind))
    }
    Expr::ArrayLit(elements) => {
      let ptr = build_array_lit(
        context,
        builder,
        elements,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok((ptr.into(), ValKind::Ptr))
    }
    Expr::Index(array, index) => {
      let (v, kind) = build_index(
        context,
        builder,
        array,
        index,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok((v, kind))
    }
    // A lambda literal only has codegen meaning at a top-level `Let`'s
    // value (`build_lambda_let`, invoked from `build_stmt`). Reached
    // from anywhere else, it's an unsupported shape, not a panic.
    Expr::Lambda { .. } => {
      Err("codegen: lambda literals are only supported as a top-level `Let`'s value".to_string())
    }
    // Plan 25 (stdlib expansion).
    Expr::Bool(b) => Ok((
      context.bool_type().const_int(u64::from(*b), false).into(),
      ValKind::Bool,
    )),
    Expr::Nil => Ok((context.i64_type().const_int(0, false).into(), ValKind::Nil)),
    Expr::HashLit(pairs) => {
      let ptr = build_hash_lit(
        context,
        builder,
        pairs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok((ptr.into(), ValKind::Ptr))
    }
    // `Array.new(size)`'s element type is only known from the enclosing
    // `Let`'s declared annotation — `build_stmt`'s dedicated `Let`
    // guard arm handles the one position this is actually reachable
    // from; reached from anywhere else, it's an unsupported shape.
    Expr::ArrayNew(_) => {
      Err("codegen: `Array.new(...)` may only appear as a top-level `Let`'s value".to_string())
    }
  }
}

/// `{k1 => v1, k2 => v2, ...}` (plan 25's Decision log): allocates
/// `8 + n*16` bytes via `emerald_alloc` — an `i64` pair-count header
/// (so `build_hash_lookup`'s linear scan knows when to stop) followed
/// by `n` flat `(key, value)` pairs, 8 bytes each.
#[allow(clippy::too_many_arguments)]
fn build_hash_lit<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  pairs: &[(Expr, Expr)],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  let byte_size = context
    .i64_type()
    .const_int(8 + pairs.len() as u64 * 16, false);
  let call = builder
    .build_call(ctx.alloc, &[byte_size.into()], "hashlit")
    .map_err(|e| e.to_string())?;
  let ptr = call_result(call)?.into_pointer_value();

  let count = context.i64_type().const_int(pairs.len() as u64, false);
  builder.build_store(ptr, count).map_err(|e| e.to_string())?;

  for (i, (k, v)) in pairs.iter().enumerate() {
    let (kv, _) = build_expr(
      context,
      builder,
      k,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let key_ptr = field_ptr(context, builder, ptr, 8 + i as u64 * 16)?;
    builder
      .build_store(key_ptr, kv)
      .map_err(|e| e.to_string())?;

    let (vv, _) = build_expr(
      context,
      builder,
      v,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let value_ptr = field_ptr(context, builder, ptr, 8 + i as u64 * 16 + 8)?;
    builder
      .build_store(value_ptr, vv)
      .map_err(|e| e.to_string())?;
  }
  Ok(ptr)
}

/// Extracts a call instruction's return value, erroring (not panicking)
/// if it was actually void — an internal-consistency check, since every
/// call site here already checked its callee's declared return kind
/// before deciding to use the result as a value.
fn call_result(call: inkwell::values::CallSiteValue<'_>) -> Result<BasicValueEnum<'_>, String> {
  match call.try_as_basic_value() {
    ValueKind::Basic(v) => Ok(v),
    ValueKind::Instruction(_) => {
      Err("codegen: internal error — call used as a value returned void".to_string())
    }
  }
}

fn load_field<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  base_ptr: PointerValue<'ctx>,
  field: FieldInfo,
) -> Result<BasicValueEnum<'ctx>, String> {
  let field_ptr = field_ptr(context, builder, base_ptr, field.offset)?;
  builder
    .build_load(local_llvm_type(context, field.kind), field_ptr, "fieldval")
    .map_err(|e| e.to_string())
}

fn field_ptr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  base_ptr: PointerValue<'ctx>,
  offset: u64,
) -> Result<PointerValue<'ctx>, String> {
  let idx = context.i64_type().const_int(offset, false);
  unsafe {
    builder
      .build_in_bounds_gep(context.i8_type(), base_ptr, &[idx], "fieldptr")
      .map_err(|e| e.to_string())
  }
}

/// `receiver.method(args)`. `.call` on a receiver known to
/// `ctx.lambda_func_ids` dispatches statically to that lambda's
/// synthesized function; `Name.method(args)` on a known module name
/// dispatches to `{Name}_{method}` with no receiver value at all;
/// everything else is the `{Class}_{method}` dispatch established for
/// ordinary class methods.
#[allow(clippy::too_many_arguments)]
fn build_method_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Expr,
  method: &str,
  args: &[Expr],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let Expr::Ident(recv_name) = recv else {
    return Err(
      "codegen: method calls are only supported on a plain local-variable receiver".to_string(),
    );
  };

  if ctx.module_names.contains(recv_name) {
    let key = format!("{recv_name}_{method}");
    let (fv, ret_kind) = *ctx
      .user_func_ids
      .get(&key)
      .ok_or_else(|| format!("codegen: unsupported module method call `{recv_name}.{method}`"))?;
    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> =
      Vec::with_capacity(args.len());
    for a in args {
      let (v, _) = build_expr(
        context,
        builder,
        a,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      call_args.push(v.into());
    }
    let call = builder
      .build_call(fv, &call_args, "modcalltmp")
      .map_err(|e| e.to_string())?;
    return Ok((call_result(call)?, ret_kind));
  }

  let (fv, ret_kind) = if method == "call" {
    *ctx.lambda_func_ids.get(recv_name).ok_or_else(|| {
      format!("codegen: `.call` on `{recv_name}` — not a lambda literal bound to a top-level `Let`")
    })?
  } else {
    let class_name = local_classes.get(recv_name).ok_or_else(|| {
      format!("codegen: cannot determine the class of `{recv_name}` for `.{method}`")
    })?;
    let key = format!("{class_name}_{method}");
    *ctx
      .user_func_ids
      .get(&key)
      .ok_or_else(|| format!("codegen: unsupported method call `{class_name}.{method}`"))?
  };

  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![recv_val.into()];
  for a in args {
    let (v, _) = build_expr(
      context,
      builder,
      a,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    call_args.push(v.into());
  }
  let call = builder
    .build_call(fv, &call_args, "methcalltmp")
    .map_err(|e| e.to_string())?;
  Ok((call_result(call)?, ret_kind))
}

/// `emerald_alloc`s a flat `elements.len() * 8`-byte buffer, then
/// stores each element at its `i * 8` offset — no length prefix, no
/// bounds checking. The returned pointer carries no element-type
/// information; nothing about a bare `Expr::ArrayLit` value does, which
/// is why indexing (`build_index`) only works through a named local via
/// `local_array_elem_types`.
#[allow(clippy::too_many_arguments)]
fn build_array_lit<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  elements: &[Expr],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  let size_val = context
    .i64_type()
    .const_int(elements.len() as u64 * 8, false);
  let call = builder
    .build_call(ctx.alloc, &[size_val.into()], "arralloc")
    .map_err(|e| e.to_string())?;
  let ptr = call_result(call)?.into_pointer_value();
  for (i, e) in elements.iter().enumerate() {
    let (v, _) = build_expr(
      context,
      builder,
      e,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let elem_ptr = field_ptr(context, builder, ptr, i as u64 * 8)?;
    builder
      .build_store(elem_ptr, v)
      .map_err(|e| e.to_string())?;
  }
  Ok(ptr)
}

/// `"Hash[K, V]"` -> `(K's kind, V's kind)` — the same compound-string
/// convention `emerald-sema`'s `resolve_type` uses, parsed here too
/// since codegen keeps its own independent side-table (plan 25's
/// Decision log on `local_classes` doubling for this).
fn parse_hash_type(s: &str) -> Option<(ValKind, ValKind)> {
  let inner = s.strip_prefix("Hash[")?.strip_suffix(']')?;
  let (k, v) = inner.split_once(", ")?;
  Some((value_kind_for_type(k), value_kind_for_type(v)))
}

/// Linear-scans a `Hash[K, V]`'s `[count:i64][(key,value) pairs]`
/// buffer (plan 25's Decision log — a flat, `O(n)` representation, not
/// a real hash table) for a matching key, returning a pointer to that
/// pair's value slot. No match calls `emerald_hash_key_not_found`
/// (never returns) and marks the fallthrough unreachable — a real,
/// disclosed runtime abort, not silent undefined behavior.
fn build_hash_lookup<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  base_ptr: PointerValue<'ctx>,
  key_val: IntValue<'ctx>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  let func = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block")?
    .get_parent()
    .ok_or("codegen: internal error — block has no parent function")?;

  let i64_ty = context.i64_type();
  let count = builder
    .build_load(i64_ty, base_ptr, "hashcount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  let idx_ptr = builder
    .build_alloca(i64_ty, "hashidx")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_ptr, i64_ty.const_int(0, false))
    .map_err(|e| e.to_string())?;

  let header_blk = context.append_basic_block(func, "hash.header");
  let body_blk = context.append_basic_block(func, "hash.body");
  let next_blk = context.append_basic_block(func, "hash.next");
  let found_blk = context.append_basic_block(func, "hash.found");
  let notfound_blk = context.append_basic_block(func, "hash.notfound");

  builder
    .build_unconditional_branch(header_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(header_blk);
  let i = builder
    .build_load(i64_ty, idx_ptr, "i")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let cond = builder
    .build_int_compare(IntPredicate::SLT, i, count, "hashcond")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(cond, body_blk, notfound_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(body_blk);
  let sixteen = i64_ty.const_int(16, false);
  let eight = i64_ty.const_int(8, false);
  let pair_off = builder
    .build_int_mul(i, sixteen, "pairoff")
    .map_err(|e| e.to_string())?;
  let key_off = builder
    .build_int_add(pair_off, eight, "keyoff")
    .map_err(|e| e.to_string())?;
  let key_ptr = unsafe {
    builder
      .build_in_bounds_gep(context.i8_type(), base_ptr, &[key_off], "keyptr")
      .map_err(|e| e.to_string())?
  };
  let pair_key = builder
    .build_load(i64_ty, key_ptr, "pairkey")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let matches = builder
    .build_int_compare(IntPredicate::EQ, pair_key, key_val, "keymatch")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(matches, found_blk, next_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(next_blk);
  let one = i64_ty.const_int(1, false);
  let i_next = builder
    .build_int_add(i, one, "inext")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_ptr, i_next)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(header_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(notfound_blk);
  builder
    .build_call(ctx.hash_key_not_found, &[], "keynotfound")
    .map_err(|e| e.to_string())?;
  builder.build_unreachable().map_err(|e| e.to_string())?;

  builder.position_at_end(found_blk);
  let value_off = builder
    .build_int_add(key_off, eight, "valoff")
    .map_err(|e| e.to_string())?;
  let value_ptr = unsafe {
    builder
      .build_in_bounds_gep(context.i8_type(), base_ptr, &[value_off], "valptr")
      .map_err(|e| e.to_string())?
  };
  Ok(value_ptr)
}

/// `arr[i]` / `h[k]`. Same "plain local-variable" restriction as
/// `MethodCall`'s receiver — dispatches on which side-table knows
/// `arr_name`: `local_array_elem_types` (an `Array`) or `local_classes`
/// holding a `"Hash[K, V]"` string (plan 25's Decision log).
#[allow(clippy::too_many_arguments)]
fn build_index<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  array: &Expr,
  index: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let Expr::Ident(arr_name) = array else {
    return Err(
      "codegen: indexing is only supported on a plain local-variable receiver".to_string(),
    );
  };
  let (base, _) = build_expr(
    context,
    builder,
    array,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (idx, idx_kind) = build_expr(
    context,
    builder,
    index,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if let Some(&elem_kind) = local_array_elem_types.get(arr_name) {
    if idx_kind != ValKind::Int64 {
      return Err("codegen: array index must be Int64".to_string());
    }
    let elem_llvm_ty = local_llvm_type(context, elem_kind);
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(
          elem_llvm_ty,
          base.into_pointer_value(),
          &[idx.into_int_value()],
          "elemptr",
        )
        .map_err(|e| e.to_string())?
    };
    let loaded = builder
      .build_load(elem_llvm_ty, elem_ptr, "elemval")
      .map_err(|e| e.to_string())?;
    return Ok((loaded, elem_kind));
  }
  if let Some((key_kind, value_kind)) = local_classes.get(arr_name).and_then(|s| parse_hash_type(s))
  {
    if idx_kind != key_kind {
      return Err(format!("codegen: Hash key must be {key_kind:?}"));
    }
    let value_ptr = build_hash_lookup(
      context,
      builder,
      base.into_pointer_value(),
      idx.into_int_value(),
      ctx,
    )?;
    let value_llvm_ty = local_llvm_type(context, value_kind);
    let loaded = builder
      .build_load(value_llvm_ty, value_ptr, "hashval")
      .map_err(|e| e.to_string())?;
    return Ok((loaded, value_kind));
  }
  Err(format!(
    "codegen: cannot determine the element type of `{arr_name}` for indexing"
  ))
}

/// `arr[i] = value` / `h[k] = value`. Same dispatch as `build_index`'s
/// read side.
#[allow(clippy::too_many_arguments)]
fn build_set_index<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  array: &Expr,
  index: &Expr,
  value: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let Expr::Ident(arr_name) = array else {
    return Err(
      "codegen: indexed assignment is only supported on a plain local-variable receiver"
        .to_string(),
    );
  };
  let (base, _) = build_expr(
    context,
    builder,
    array,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (idx, idx_kind) = build_expr(
    context,
    builder,
    index,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if let Some(&elem_kind) = local_array_elem_types.get(arr_name) {
    if idx_kind != ValKind::Int64 {
      return Err("codegen: array index must be Int64".to_string());
    }
    let (v, _) = build_expr(
      context,
      builder,
      value,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let elem_llvm_ty = local_llvm_type(context, elem_kind);
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(
          elem_llvm_ty,
          base.into_pointer_value(),
          &[idx.into_int_value()],
          "elemptr",
        )
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(elem_ptr, v)
      .map_err(|e| e.to_string())?;
    return Ok(false);
  }
  if let Some((key_kind, _value_kind)) =
    local_classes.get(arr_name).and_then(|s| parse_hash_type(s))
  {
    if idx_kind != key_kind {
      return Err(format!("codegen: Hash key must be {key_kind:?}"));
    }
    let (v, _) = build_expr(
      context,
      builder,
      value,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let value_ptr = build_hash_lookup(
      context,
      builder,
      base.into_pointer_value(),
      idx.into_int_value(),
      ctx,
    )?;
    builder
      .build_store(value_ptr, v)
      .map_err(|e| e.to_string())?;
    return Ok(false);
  }
  Err(format!(
    "codegen: cannot determine the element type of `{arr_name}` for indexing"
  ))
}

/// `puts <inner>` — a call-site-polymorphic intrinsic (not an
/// overloaded user function) that routes to `emerald_print_i64` or
/// `emerald_print_f64` based on the built argument's actual kind.
#[allow(clippy::too_many_arguments)]
fn build_puts<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  arg: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let (v, kind) = build_expr(
    context,
    builder,
    arg,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let target = match kind {
    ValKind::Int64 => ctx.print_i64,
    ValKind::Float64 => ctx.print_f64,
    ValKind::Str => ctx.print_str,
    _ => return Err("codegen: `puts` only supports Int64/Float64/String values".to_string()),
  };
  builder
    .build_call(target, &[v.into()], "puts")
    .map_err(|e| e.to_string())?;
  Ok(())
}

/// `name: Proc = ->(...) -> T { ... }`. Allocates the env buffer
/// (sized by capture count), snapshots each captured local's *current*
/// value into its slot (by value, not by reference), and stores the
/// resulting pointer into `name`'s pre-allocated slot — exactly the
/// value `.call` later passes as the synthesized lambda function's
/// leading `env` argument.
fn build_lambda_let<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  name: &str,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let info = ctx.lambda_infos.get(name).ok_or_else(|| {
    format!("codegen: lambda literal bound to `{name}` is not supported outside a top-level `Let`")
  })?;
  let size_val = context
    .i64_type()
    .const_int(info.captures.len() as u64 * 8, false);
  let call = builder
    .build_call(ctx.alloc, &[size_val.into()], "envalloc")
    .map_err(|e| e.to_string())?;
  let env_ptr = call_result(call)?.into_pointer_value();
  for cap_name in &info.captures {
    let (cap_ptr, cap_kind) = *vars.get(cap_name).ok_or_else(|| {
      format!("codegen: captured variable `{cap_name}` is not in scope at `{name}`'s creation site")
    })?;
    let val = builder
      .build_load(local_llvm_type(context, cap_kind), cap_ptr, cap_name)
      .map_err(|e| e.to_string())?;
    let offset = info.capture_offsets[cap_name];
    let slot_ptr = field_ptr(context, builder, env_ptr, offset)?;
    builder
      .build_store(slot_ptr, val)
      .map_err(|e| e.to_string())?;
  }
  let (name_ptr, _) = *vars
    .get(name)
    .expect("pre-allocated by prealloc_lets for every top-level Let");
  builder
    .build_store(name_ptr, env_ptr)
    .map_err(|e| e.to_string())?;
  Ok(())
}

/// Emits one statement. Returns `true` if the statement emitted a
/// block terminator (`return`/the loop-jump for `break`/`next`/
/// `raise`'s unreachable) — callers must not emit further instructions
/// into the current block afterward.
#[allow(clippy::too_many_arguments)]
fn build_stmt<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  stmt: &Stmt,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  match stmt {
    Stmt::Let {
      name,
      ty,
      value: Expr::Lambda { .. },
    } if ty == "Proc" => {
      build_lambda_let(context, builder, name, vars, ctx)?;
      Ok(false)
    }
    // `Array.new(size)`'s element type comes from this `Let`'s own
    // declared annotation (plan 25's Decision log) — special-cased the
    // same way `Proc` is above, since `build_expr` alone has no
    // declared-type context to draw on.
    Stmt::Let {
      name,
      ty,
      value: Expr::ArrayNew(size),
    } => {
      let elem_name = ty
        .strip_prefix("Array[")
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| {
          format!("codegen: `{name}: {ty} = Array.new(...)` — declared type is not an Array")
        })?;
      let elem_kind = value_kind_for_type(elem_name);
      let (size_val, size_kind) = build_expr(
        context,
        builder,
        size,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if size_kind != ValKind::Int64 {
        return Err("codegen: `Array.new` size must be Int64".to_string());
      }
      let elem_size = context.i64_type().const_int(8, false);
      let byte_size = builder
        .build_int_mul(size_val.into_int_value(), elem_size, "arraynewbytes")
        .map_err(|e| e.to_string())?;
      let call = builder
        .build_call(ctx.alloc_zeroed, &[byte_size.into()], "arraynew")
        .map_err(|e| e.to_string())?;
      let ptr = call_result(call)?;
      local_array_elem_types.insert(name.clone(), elem_kind);
      let (dst, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder.build_store(dst, ptr).map_err(|e| e.to_string())?;
      Ok(false)
    }
    Stmt::Let { name, ty, value } => {
      let (v, _) = build_expr(
        context,
        builder,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if ctx.classes.contains_key(ty.as_str()) {
        local_classes.insert(name.clone(), ty.clone());
      }
      if let Some(elem_name) = ty.strip_prefix("Array[").and_then(|s| s.strip_suffix(']')) {
        local_array_elem_types.insert(name.clone(), value_kind_for_type(elem_name));
      }
      // Plan 25: `local_classes` doubles as the side-table for
      // `"Hash[K, V]"` locals too (see `value_kind_for_type`'s doc
      // comment) — `build_index`/`build_set_index` check for this
      // prefix to route to hash-lookup codegen instead of array
      // indexing.
      if ty.starts_with("Hash[") {
        local_classes.insert(name.clone(), ty.clone());
      }
      let (ptr, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      Ok(false)
    }
    Stmt::SetField { name, value } => {
      let (self_ptr, fields) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name} = ...` used outside of a method body"))?;
      let field = *fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?;
      let (v, _) = build_expr(
        context,
        builder,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let fp = field_ptr(context, builder, self_ptr, field.offset)?;
      builder.build_store(fp, v).map_err(|e| e.to_string())?;
      Ok(false)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => build_set_index(
      context,
      builder,
      array,
      index,
      value,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Stmt::Expr(Expr::Call(name, args)) if name == "puts" && args.len() == 1 => {
      build_puts(
        context,
        builder,
        &args[0],
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok(false)
    }
    Stmt::Expr(e) => {
      build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok(false)
    }
    Stmt::Return(Some(e)) => {
      let (v, _) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder.build_return(Some(&v)).map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Return(None) => {
      builder.build_return(None).map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Break => {
      let target = loop_stack
        .last()
        .ok_or("codegen: `break` outside of a loop")?;
      builder
        .build_unconditional_branch(target.exit)
        .map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Next => {
      let target = loop_stack
        .last()
        .ok_or("codegen: `next` outside of a loop")?;
      builder
        .build_unconditional_branch(target.header)
        .map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_val = build_bool(
        context,
        builder,
        cond,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let then_blk = context.append_basic_block(func, "if.then");
      let merge_blk = context.append_basic_block(func, "if.merge");
      let else_target = if else_branch.is_some() {
        context.append_basic_block(func, "if.else")
      } else {
        merge_blk
      };
      builder
        .build_conditional_branch(cond_val, then_blk, else_target)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(then_blk);
      let then_terminated = build_block(
        context,
        builder,
        func,
        then_branch,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ret_kind,
        ctx,
      )?;
      if !then_terminated {
        builder
          .build_unconditional_branch(merge_blk)
          .map_err(|e| e.to_string())?;
      }

      // Plan 29 (surfaced by `elsif`'s fully-covering nested-If chains,
      // but a pre-existing latent bug in `if`/`else` on its own): when
      // there's an `else` and BOTH branches unconditionally terminate
      // (e.g. every arm of an `elsif` chain `return`s), `merge_blk`
      // ends up with zero predecessors — neither branch ever
      // unconditionally-branches into it, and (unlike the no-`else`
      // case below) the top conditional branch's false edge goes to a
      // real `if.else` block instead. LLVM's verifier rejects a basic
      // block with no terminator at all, so an unreachable-but-still-
      // appended `merge_blk` needs an explicit `unreachable` terminator
      // rather than being left empty.
      let has_else = else_branch.is_some();
      let mut else_terminated = false;
      if let Some(else_branch) = else_branch {
        builder.position_at_end(else_target);
        else_terminated = build_block(
          context,
          builder,
          func,
          else_branch,
          vars,
          local_classes,
          local_array_elem_types,
          loop_stack,
          ret_kind,
          ctx,
        )?;
        if !else_terminated {
          builder
            .build_unconditional_branch(merge_blk)
            .map_err(|e| e.to_string())?;
        }
      }

      let if_terminated = has_else && then_terminated && else_terminated;
      builder.position_at_end(merge_blk);
      if if_terminated {
        builder.build_unreachable().map_err(|e| e.to_string())?;
      }
      Ok(if_terminated)
    }
    Stmt::While { cond, body } => {
      let header_blk = context.append_basic_block(func, "while.cond");
      let body_blk = context.append_basic_block(func, "while.body");
      let exit_blk = context.append_basic_block(func, "while.after");

      builder
        .build_unconditional_branch(header_blk)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(header_blk);
      let cond_val = build_bool(
        context,
        builder,
        cond,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder
        .build_conditional_branch(cond_val, body_blk, exit_blk)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(body_blk);
      loop_stack.push(LoopTargets {
        header: header_blk,
        exit: exit_blk,
      });
      let body_terminated = build_block(
        context,
        builder,
        func,
        body,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ret_kind,
        ctx,
      )?;
      loop_stack.pop();
      if !body_terminated {
        builder
          .build_unconditional_branch(header_blk)
          .map_err(|e| e.to_string())?;
      }

      builder.position_at_end(exit_blk);
      Ok(false)
    }
    Stmt::Raise(e) => build_raise(
      context,
      builder,
      e,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Stmt::Begin {
      body,
      rescue_type,
      rescue_var,
      rescue_body,
    } => build_begin(
      context,
      builder,
      func,
      body,
      rescue_type,
      rescue_var,
      rescue_body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ret_kind,
      ctx,
    ),
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => build_case(
      context,
      builder,
      func,
      scrutinee,
      arms,
      else_body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ret_kind,
      ctx,
    ),
  }
}

/// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan 20):
/// lowers to the same `icmp(Equal)` `Expr::Compare`'s `Eq` arm already
/// emits, chained across arms in source order (first matching arm
/// wins) — a multi-value arm's values are OR-ed together (bitwise
/// `or` over `i1`s, equivalent to logical or), falling through to
/// `else_body` (or nothing) if no arm matches.
#[allow(clippy::too_many_arguments)]
fn build_case<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  scrutinee: &Expr,
  arms: &[(Vec<Expr>, Vec<Stmt>)],
  else_body: &Option<Vec<Stmt>>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let (scrut_val, scrut_kind) = build_expr(
    context,
    builder,
    scrutinee,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if scrut_kind != ValKind::Int64 {
    return Err("codegen: `case` scrutinee must be Int64".to_string());
  }
  let scrut_int = scrut_val.into_int_value();

  let merge_blk = context.append_basic_block(func, "case.merge");

  for (values, body) in arms {
    let arm_blk = context.append_basic_block(func, "case.arm");
    let next_check_blk = context.append_basic_block(func, "case.next");

    let mut cond: Option<IntValue> = None;
    for v in values {
      let (v_val, v_kind) = build_expr(
        context,
        builder,
        v,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if v_kind != ValKind::Int64 {
        return Err("codegen: `when` value must be Int64".to_string());
      }
      let eq = builder
        .build_int_compare(
          IntPredicate::EQ,
          scrut_int,
          v_val.into_int_value(),
          "wheneq",
        )
        .map_err(|e| e.to_string())?;
      cond = Some(match cond {
        None => eq,
        Some(prev) => builder
          .build_or(prev, eq, "whenor")
          .map_err(|e| e.to_string())?,
      });
    }
    let cond = cond.expect("the grammar guarantees at least one when-value per arm");
    builder
      .build_conditional_branch(cond, arm_blk, next_check_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(arm_blk);
    let terminated = build_block(
      context,
      builder,
      func,
      body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ret_kind,
      ctx,
    )?;
    if !terminated {
      builder
        .build_unconditional_branch(merge_blk)
        .map_err(|e| e.to_string())?;
    }

    builder.position_at_end(next_check_blk);
  }

  // Reached only when no arm matched.
  if let Some(else_b) = else_body {
    let terminated = build_block(
      context,
      builder,
      func,
      else_b,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ret_kind,
      ctx,
    )?;
    if !terminated {
      builder
        .build_unconditional_branch(merge_blk)
        .map_err(|e| e.to_string())?;
    }
  } else {
    builder
      .build_unconditional_branch(merge_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(merge_blk);
  Ok(false)
}

/// Builds a `While`/`If` condition — always an `Expr::Compare` in every
/// program this compiler accepts (the only boolean-producing expression
/// in the AST) — straight to an `i1`.
#[allow(clippy::too_many_arguments)]
fn build_bool<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  cond: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<IntValue<'ctx>, String> {
  let (v, kind) = build_expr(
    context,
    builder,
    cond,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if kind != ValKind::Bool {
    return Err("codegen: a condition must be a comparison expression".to_string());
  }
  Ok(v.into_int_value())
}

/// `raise <expr>` — `expr` must be a direct `ClassName.new(args)` call,
/// built exactly like an ordinary `Expr::New` (reusing `build_expr`,
/// not reimplemented here), then handed to `emerald_raise` with the
/// class's compile-time-known tag.
#[allow(clippy::too_many_arguments)]
fn build_raise<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  e: &Expr,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let Expr::New(class_name, _) = e else {
    return Err("codegen: `raise` only supports a direct `ClassName.new(args)` expression".into());
  };
  let tag = *ctx
    .class_tags
    .get(class_name)
    .ok_or_else(|| format!("codegen: unknown class `{class_name}` in `raise`"))?;
  let (exc_ptr, _) = build_expr(
    context,
    builder,
    e,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let tag_val = context.i64_type().const_int(tag as u64, true);
  builder
    .build_call(
      ctx.exc_funcs.raise,
      &[tag_val.into(), exc_ptr.into()],
      "raise",
    )
    .map_err(|e| e.to_string())?;
  // `emerald_raise` never returns to this call site in practice (it
  // longjmps to a handler, or exits the process on an uncaught
  // exception) — LLVM still requires the block to end in an explicit
  // terminator.
  builder.build_unreachable().map_err(|e| e.to_string())?;
  Ok(true)
}

/// `begin body rescue Type => e rescue_body end`. Pushes a handler,
/// calls `setjmp` *directly*, and branches on the result: zero means
/// this is the normal first pass through (run `body`), nonzero means a
/// `longjmp` landed here — the landing pad then compares the caught
/// class tag against `rescue_type`'s, binding `rescue_var` and running
/// `rescue_body` on a match, or re-raising to the next-outer handler
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn build_begin<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  body: &[Stmt],
  rescue_type: &str,
  rescue_var: &str,
  rescue_body: &[Stmt],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let rescue_tag = *ctx
    .class_tags
    .get(rescue_type)
    .ok_or_else(|| format!("codegen: unknown class `{rescue_type}` in `rescue`"))?;

  let push_call = builder
    .build_call(ctx.exc_funcs.push_handler, &[], "pushhandler")
    .map_err(|e| e.to_string())?;
  let handler_ptr = call_result(push_call)?.into_pointer_value();

  let jmpbuf_call = builder
    .build_call(
      ctx.exc_funcs.handler_jmpbuf,
      &[handler_ptr.into()],
      "jmpbuf",
    )
    .map_err(|e| e.to_string())?;
  let jmpbuf_ptr = call_result(jmpbuf_call)?.into_pointer_value();

  let setjmp_call = builder
    .build_call(ctx.exc_funcs.setjmp, &[jmpbuf_ptr.into()], "setjmpres")
    .map_err(|e| e.to_string())?;
  let setjmp_result = call_result(setjmp_call)?.into_int_value();

  let try_blk = context.append_basic_block(func, "begin.try");
  let rescue_blk = context.append_basic_block(func, "begin.rescue");
  let merge_blk = context.append_basic_block(func, "begin.merge");
  let zero = context.i32_type().const_int(0, false);
  let is_first_pass = builder
    .build_int_compare(IntPredicate::EQ, setjmp_result, zero, "isfirstpass")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(is_first_pass, try_blk, rescue_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(try_blk);
  let try_terminated = build_block(
    context,
    builder,
    func,
    body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ret_kind,
    ctx,
  )?;
  if !try_terminated {
    builder
      .build_call(ctx.exc_funcs.pop_handler, &[], "pophandler")
      .map_err(|e| e.to_string())?;
    builder
      .build_unconditional_branch(merge_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(rescue_blk);
  let tag_call = builder
    .build_call(
      ctx.exc_funcs.handler_tag,
      &[handler_ptr.into()],
      "caughttag",
    )
    .map_err(|e| e.to_string())?;
  let caught_tag = call_result(tag_call)?.into_int_value();
  let expected_tag = context.i64_type().const_int(rescue_tag as u64, true);
  let tag_matches = builder
    .build_int_compare(IntPredicate::EQ, caught_tag, expected_tag, "tagmatches")
    .map_err(|e| e.to_string())?;

  let match_blk = context.append_basic_block(func, "begin.match");
  let mismatch_blk = context.append_basic_block(func, "begin.mismatch");
  builder
    .build_conditional_branch(tag_matches, match_blk, mismatch_blk)
    .map_err(|e| e.to_string())?;

  // Caught, but it isn't this `rescue`'s type — free this handler and
  // propagate to the next-outer one.
  builder.position_at_end(mismatch_blk);
  let exc_ptr_call = builder
    .build_call(
      ctx.exc_funcs.handler_exception_ptr,
      &[handler_ptr.into()],
      "mismatchexc",
    )
    .map_err(|e| e.to_string())?;
  let mismatch_exc_ptr = call_result(exc_ptr_call)?;
  builder
    .build_call(
      ctx.exc_funcs.free_handler,
      &[handler_ptr.into()],
      "freehandler",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_call(
      ctx.exc_funcs.raise,
      &[caught_tag.into(), mismatch_exc_ptr.into()],
      "reraise",
    )
    .map_err(|e| e.to_string())?;
  builder.build_unreachable().map_err(|e| e.to_string())?;

  builder.position_at_end(match_blk);
  let exc_ptr_call2 = builder
    .build_call(
      ctx.exc_funcs.handler_exception_ptr,
      &[handler_ptr.into()],
      "matchexc",
    )
    .map_err(|e| e.to_string())?;
  let match_exc_ptr = call_result(exc_ptr_call2)?;
  builder
    .build_call(
      ctx.exc_funcs.free_handler,
      &[handler_ptr.into()],
      "freehandler2",
    )
    .map_err(|e| e.to_string())?;

  let (rescue_var_ptr, _) = *vars
    .get(rescue_var)
    .expect("pre-allocated by prealloc_lets for every Begin's rescue_var");
  builder
    .build_store(rescue_var_ptr, match_exc_ptr)
    .map_err(|e| e.to_string())?;
  local_classes.insert(rescue_var.to_string(), rescue_type.to_string());

  let rescue_terminated = build_block(
    context,
    builder,
    func,
    rescue_body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ret_kind,
    ctx,
  )?;
  if !rescue_terminated {
    builder
      .build_unconditional_branch(merge_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(merge_blk);
  Ok(false)
}

/// Emits a straight-line sequence of statements. Stops early (without
/// erroring) after any statement that terminates the current block —
/// anything syntactically after `return`/`break`/`next`/`raise` in the
/// same list is unreachable and must not be emitted into an
/// already-terminated LLVM block.
#[allow(clippy::too_many_arguments)]
fn build_block<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  stmts: &[Stmt],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  for stmt in stmts {
    let terminated = build_stmt(
      context,
      builder,
      func,
      stmt,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ret_kind,
      ctx,
    )?;
    if terminated {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Builds a function/method/lambda's `Vec<Stmt>` body, honoring
/// Ruby-style implicit return: if the body doesn't already end in an
/// explicit terminator, the last statement — if a bare `Stmt::Expr` —
/// has its value returned, matching `emerald-sema`'s implicit-return
/// check.
#[allow(clippy::too_many_arguments)]
fn build_function_body<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  body: &[Stmt],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  ret_kind: ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let mut loop_stack = Vec::new();
  let Some((last, init)) = body.split_last() else {
    builder.build_return(None).map_err(|e| e.to_string())?;
    return Ok(());
  };

  let terminated = build_block(
    context,
    builder,
    func,
    init,
    vars,
    local_classes,
    local_array_elem_types,
    &mut loop_stack,
    ret_kind,
    ctx,
  )?;
  if terminated {
    return Ok(());
  }

  match last {
    Stmt::Expr(e) => {
      let (v, _) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder.build_return(Some(&v)).map_err(|e| e.to_string())?;
    }
    other => {
      let terminated = build_stmt(
        context,
        builder,
        func,
        other,
        vars,
        local_classes,
        local_array_elem_types,
        &mut loop_stack,
        ret_kind,
        ctx,
      )?;
      // A Void-returning body whose last statement isn't a
      // value-producing `Stmt::Expr` (e.g. `initialize`'s trailing
      // `@y = y`) needs an explicit empty `return` — nothing else
      // would ever terminate the block.
      if !terminated && ret_kind == ValKind::Void {
        builder.build_return(None).map_err(|e| e.to_string())?;
      }
    }
  }
  Ok(())
}

fn param_kinds(params: &[Param]) -> Vec<ValKind> {
  params.iter().map(|p| value_kind_for_type(&p.ty)).collect()
}

/// Binds `f`'s declared params to fresh entry-block `alloca`s (storing
/// each incoming SSA parameter value into its slot), populating
/// `local_classes`/`local_array_elem_types` bookkeeping for any
/// class-/array-typed parameter along the way.
#[allow(clippy::too_many_arguments)]
fn bind_params<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  params: &[Param],
  param_offset: u32,
  classes: &HashMap<String, ClassLayout>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
) -> Result<(), String> {
  for (i, p) in params.iter().enumerate() {
    let kind = value_kind_for_type(&p.ty);
    let param_val = func
      .get_nth_param(param_offset + i as u32)
      .expect("declared signature has this many params");
    let alloca = builder
      .build_alloca(local_llvm_type(context, kind), &p.name)
      .map_err(|e| e.to_string())?;
    builder
      .build_store(alloca, param_val)
      .map_err(|e| e.to_string())?;
    vars.insert(p.name.clone(), (alloca, kind));
    if classes.contains_key(p.ty.as_str()) {
      local_classes.insert(p.name.clone(), p.ty.clone());
    }
    if let Some(elem_name) = p
      .ty
      .strip_prefix("Array[")
      .and_then(|s| s.strip_suffix(']'))
    {
      local_array_elem_types.insert(p.name.clone(), value_kind_for_type(elem_name));
    }
  }
  Ok(())
}

fn define_user_function<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  f: &AstFunction,
  fv: FunctionValue<'ctx>,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(fv, "entry");
  builder.position_at_end(entry);

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();
  bind_params(
    context,
    builder,
    fv,
    &f.params,
    0,
    gen_ctx.classes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
  )?;

  let mut decls = Vec::new();
  collect_lets(&f.body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let ret_kind = value_kind_for_type(&f.return_type);
  build_function_body(
    context,
    builder,
    fv,
    &f.body,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    ret_kind,
    gen_ctx,
  )
}

/// Compiles one class method as `{ClassName}_{methodName}`, taking an
/// implicit leading `self: ptr` parameter ahead of the method's own
/// declared parameters. `self_fields` (the class's field layout) is
/// threaded through `gen_ctx.self_ctx` for the method body's `@field`
/// reads/writes.
fn define_method<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  m: &AstFunction,
  fv: FunctionValue<'ctx>,
  self_fields: &HashMap<String, FieldInfo>,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(fv, "entry");
  builder.position_at_end(entry);

  let self_ptr = fv
    .get_nth_param(0)
    .expect("methods always declare a leading self param")
    .into_pointer_value();

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();
  bind_params(
    context,
    builder,
    fv,
    &m.params,
    1,
    gen_ctx.classes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
  )?;

  let mut decls = Vec::new();
  collect_lets(&m.body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let method_ctx = Ctx {
    self_ctx: Some((self_ptr, self_fields)),
    ..*gen_ctx
  };
  let ret_kind = value_kind_for_type(&m.return_type);
  build_function_body(
    context,
    builder,
    fv,
    &m.body,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    ret_kind,
    &method_ctx,
  )
}

/// Compiles one lambda's synthesized `__lambda_{name}` function — an
/// implicit leading `env: ptr` parameter ahead of the lambda's own
/// declared params, with each captured name pre-loaded from `env` into
/// an ordinary local before the body runs, so `build_expr`'s
/// `Expr::Ident` path doesn't need any special case for a captured vs.
/// a locally-declared name.
#[allow(clippy::too_many_arguments)]
fn define_lambda<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  params: &[Param],
  return_type: &str,
  body: &[Stmt],
  info: &LambdaInfo,
  fv: FunctionValue<'ctx>,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(fv, "entry");
  builder.position_at_end(entry);

  let env_ptr = fv
    .get_nth_param(0)
    .expect("lambdas always declare a leading env param")
    .into_pointer_value();

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();

  for cap_name in &info.captures {
    let kind = info.capture_kinds[cap_name];
    let offset = info.capture_offsets[cap_name];
    let slot_ptr = field_ptr(context, builder, env_ptr, offset)?;
    let val = builder
      .build_load(local_llvm_type(context, kind), slot_ptr, cap_name)
      .map_err(|e| e.to_string())?;
    let alloca = builder
      .build_alloca(local_llvm_type(context, kind), cap_name)
      .map_err(|e| e.to_string())?;
    builder
      .build_store(alloca, val)
      .map_err(|e| e.to_string())?;
    vars.insert(cap_name.clone(), (alloca, kind));
  }

  bind_params(
    context,
    builder,
    fv,
    params,
    1,
    gen_ctx.classes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
  )?;

  let mut decls = Vec::new();
  collect_lets(body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let ret_kind = value_kind_for_type(return_type);
  build_function_body(
    context,
    builder,
    fv,
    body,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    ret_kind,
    gen_ctx,
  )
}

/// Builds `main` (`extern "C" fn() -> i32`): evaluates the top-level
/// statements (including `puts` calls and lambda-creating `Let`s) via
/// the already-declared runtime/user functions, and returns 0.
fn define_main<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  main_fn: FunctionValue<'ctx>,
  program: &Program,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(main_fn, "entry");
  builder.position_at_end(entry);

  let top_stmts: Vec<Stmt> = program
    .items
    .iter()
    .filter_map(|it| match it {
      Item::Stmt(s) => Some(s.clone()),
      _ => None,
    })
    .collect();

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();
  let mut decls = Vec::new();
  collect_lets(&top_stmts, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let mut loop_stack = Vec::new();
  let terminated = build_block(
    context,
    builder,
    main_fn,
    &top_stmts,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    &mut loop_stack,
    ValKind::Int64, // main's own AST-level "return kind" is never consulted -- top-level has no `return`
    gen_ctx,
  )?;
  if !terminated {
    let zero = context.i32_type().const_int(0, false);
    builder
      .build_return(Some(&zero))
      .map_err(|e| e.to_string())?;
  }
  Ok(())
}

/// Declares every user function/method/module-method/lambda's LLVM
/// signature up front (a two-pass declare-then-define structure, same
/// shape as the old Cranelift backend's `declare_function`-before-
/// `define_function` discipline) — so a call to a function declared
/// later in source order, or a mutually-referencing pair, both resolve.
fn declare_user_functions<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  classes: &HashMap<String, ClassLayout>,
) -> HashMap<String, (FunctionValue<'ctx>, ValKind)> {
  let mut user_func_ids = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        let ret_kind = value_kind_for_type(&f.return_type);
        let fn_ty = make_fn_type(context, &param_kinds(&f.params), ret_kind);
        let fv = module.add_function(&f.name, fn_ty, Some(Linkage::External));
        user_func_ids.insert(f.name.clone(), (fv, ret_kind));
      }
      Item::Class(c) => {
        for m in &c.methods {
          let ret_kind = value_kind_for_type(&m.return_type);
          let mut kinds = vec![ValKind::Ptr]; // self
          kinds.extend(param_kinds(&m.params));
          let fn_ty = make_fn_type(context, &kinds, ret_kind);
          let mangled = format!("{}_{}", c.name, m.name);
          let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
          user_func_ids.insert(mangled, (fv, ret_kind));
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          let ret_kind = value_kind_for_type(&f.return_type);
          let fn_ty = make_fn_type(context, &param_kinds(&f.params), ret_kind);
          let mangled = format!("{}_{}", m.name, f.name);
          let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
          user_func_ids.insert(mangled, (fv, ret_kind));
        }
      }
      Item::Stmt(_) => {}
      // Plan 26: `emerald_parser::parse`/`parse_named` only ever
      // returns `Ok(program)` with zero recovered errors, meaning no
      // `Item::Error` in `program.items` — codegen never receives one.
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
    }
  }
  let _ = classes; // kept in the signature for symmetry with the define pass
  user_func_ids
}

fn declare_lambda_functions<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  lambda_infos: &HashMap<String, LambdaInfo>,
) -> HashMap<String, (FunctionValue<'ctx>, ValKind)> {
  let mut lambda_func_ids = HashMap::new();
  for item in &program.items {
    let Item::Stmt(Stmt::Let {
      name,
      ty,
      value: Expr::Lambda {
        params,
        return_type,
        ..
      },
    }) = item
    else {
      continue;
    };
    if ty != "Proc" || !lambda_infos.contains_key(name) {
      continue;
    }
    let ret_kind = value_kind_for_type(return_type);
    let mut kinds = vec![ValKind::Ptr]; // env
    kinds.extend(param_kinds(params));
    let fn_ty = make_fn_type(context, &kinds, ret_kind);
    let fv = module.add_function(&format!("__lambda_{name}"), fn_ty, Some(Linkage::External));
    lambda_func_ids.insert(name.clone(), (fv, ret_kind));
  }
  lambda_func_ids
}

/// Compiles a type-checked `Program` to a native object file at
/// `out_path` via LLVM, at `OptimizationLevel::Aggressive`. One
/// exported function per `Item::Function`, `{Class}_{method}` per
/// class method, `{Module}_{method}` per module method,
/// `__lambda_{name}` per top-level `Proc` `Let`, plus a `main`
/// (`extern "C" fn() -> i32`) that evaluates the top-level statements.
pub fn compile_to_object(program: &Program, out_path: &Path) -> Result<(), String> {
  Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;
  let triple = TargetMachine::get_default_triple();
  let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
  let target_machine = target
    .create_target_machine(
      &triple,
      &TargetMachine::get_host_cpu_name().to_string(),
      &TargetMachine::get_host_cpu_features().to_string(),
      OptimizationLevel::Aggressive,
      RelocMode::Default,
      CodeModel::Default,
    )
    .ok_or_else(|| "codegen: failed to create a target machine".to_string())?;

  let context = Context::create();
  let module = context.create_module("emerald_module");
  module.set_triple(&triple);
  module.set_data_layout(&target_machine.get_target_data().get_data_layout());
  let builder = context.create_builder();

  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let f64_ty = context.f64_type();
  let void_ty = context.void_type();

  let print_i64 = module.add_function(
    "emerald_print_i64",
    void_ty.fn_type(&[i64_ty.into()], false),
    Some(Linkage::External),
  );
  let print_f64 = module.add_function(
    "emerald_print_f64",
    void_ty.fn_type(&[f64_ty.into()], false),
    Some(Linkage::External),
  );
  let alloc = module.add_function(
    "emerald_alloc",
    ptr_ty.fn_type(&[i64_ty.into()], false),
    Some(Linkage::External),
  );
  // Plan 19 (string literals).
  let print_str = module.add_function(
    "emerald_print_str",
    void_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_concat = module.add_function(
    "emerald_string_concat",
    ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_eq = module.add_function(
    "emerald_string_eq",
    i64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  // Plan 25 (stdlib expansion).
  let alloc_zeroed = module.add_function(
    "emerald_alloc_zeroed",
    ptr_ty.fn_type(&[i64_ty.into()], false),
    Some(Linkage::External),
  );
  let hash_key_not_found = module.add_function(
    "emerald_hash_key_not_found",
    void_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let exc_funcs = declare_exception_runtime_funcs(&context, &module);

  // Class layouts (field offsets/kinds) and a stable per-class integer
  // tag (declaration order) for `rescue` matching — computed once,
  // independent of declaration order between classes (no class
  // references another's fields yet, so no ordering dependency here).
  let mut classes: HashMap<String, ClassLayout> = HashMap::new();
  let mut class_tags: HashMap<String, i64> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      classes.insert(c.name.clone(), build_class_layout(c));
      class_tags.insert(c.name.clone(), class_tags.len() as i64);
    }
  }

  let module_names: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Module(ModuleDef { name, .. }) => Some(name.clone()),
      _ => None,
    })
    .collect();

  let lambda_infos = collect_lambda_infos(program)?;
  let user_func_ids = declare_user_functions(&context, &module, program, &classes);
  let lambda_func_ids = declare_lambda_functions(&context, &module, program, &lambda_infos);

  let gen_ctx = Ctx {
    user_func_ids: &user_func_ids,
    classes: &classes,
    print_i64,
    print_f64,
    alloc,
    print_str,
    string_concat,
    string_eq,
    self_ctx: None,
    lambda_func_ids: &lambda_func_ids,
    lambda_infos: &lambda_infos,
    class_tags: &class_tags,
    exc_funcs,
    module_names: &module_names,
    alloc_zeroed,
    hash_key_not_found,
  };

  for item in &program.items {
    match item {
      Item::Function(f) => {
        let (fv, _) = user_func_ids[&f.name];
        define_user_function(&context, &builder, f, fv, &gen_ctx)?;
      }
      Item::Class(c) => {
        let layout = &classes[&c.name];
        for m in &c.methods {
          let mangled = format!("{}_{}", c.name, m.name);
          let (fv, _) = user_func_ids[&mangled];
          define_method(&context, &builder, m, fv, &layout.fields, &gen_ctx)?;
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          let mangled = format!("{}_{}", m.name, f.name);
          let (fv, _) = user_func_ids[&mangled];
          // A module method has no `self`/`@field` access — compiles
          // exactly like a free function, just under a mangled name
          // and a signature with no leading self param.
          define_user_function(&context, &builder, f, fv, &gen_ctx)?;
        }
      }
      Item::Stmt(Stmt::Let {
        name,
        ty,
        value: Expr::Lambda {
          params,
          return_type,
          body,
        },
      }) if ty == "Proc" => {
        if let (Some(info), Some(&(fv, _))) = (lambda_infos.get(name), lambda_func_ids.get(name)) {
          define_lambda(
            &context,
            &builder,
            params,
            return_type,
            body,
            info,
            fv,
            &gen_ctx,
          )?;
        }
      }
      Item::Stmt(_) => {}
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
    }
  }

  let main_ty = context.i32_type().fn_type(&[], false);
  let main_fn = module.add_function("main", main_ty, Some(Linkage::External));
  define_main(&context, &builder, main_fn, program, &gen_ctx)?;

  module.verify().map_err(|e| e.to_string())?;

  let pass_options = inkwell::passes::PassBuilderOptions::create();
  module
    .run_passes("default<O3>", &target_machine, pass_options)
    .map_err(|e| e.to_string())?;

  target_machine
    .write_to_file(&module, FileType::Object, out_path)
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::process::Command;

  fn runtime_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/emerald_runtime.c")
  }

  /// Compiles `src` to an object file, links it with the runtime shim
  /// via `cc`, runs the resulting binary, and returns its captured
  /// stdout. Real, executed proof — not a simulation.
  fn compile_link_run(src: &str) -> String {
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = dir.join(format!("emerald_codegen_aot_{unique}.o"));
    let bin_path = dir.join(format!("emerald_codegen_aot_bin_{unique}"));

    compile_to_object(&program, &obj_path).expect("should compile to object file");

    let status = Command::new("cc")
      .arg("-no-pie")
      .arg(&obj_path)
      .arg(runtime_path())
      .arg("-o")
      .arg(&bin_path)
      .status()
      .expect("failed to invoke cc");
    assert!(status.success(), "linking should succeed");

    let output = Command::new(&bin_path)
      .output()
      .expect("failed to run compiled binary");
    assert!(output.status.success(), "compiled binary should exit 0");

    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();

    String::from_utf8_lossy(&output.stdout).into_owned()
  }

  #[test]
  fn compiles_hello_em_to_an_object_file() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
    let program = emerald_parser::parse(src).expect("should parse");

    let dir = std::env::temp_dir();
    let out_path = dir.join(format!("emerald_codegen_test_{}.o", std::process::id()));
    compile_to_object(&program, &out_path).expect("should compile to object file");

    let bytes = std::fs::read(&out_path).expect("object file should exist");
    assert!(!bytes.is_empty(), "object file should not be empty");
    assert_eq!(
      &bytes[0..4],
      b"\x7fELF",
      "should be a valid ELF object file"
    );

    std::fs::remove_file(&out_path).ok();
  }

  #[test]
  fn hello_em_linked_and_run_prints_42() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn inception_milestone2_linked_and_run_prints_10() {
    let src = "x: Int64 = 10\n\nif x > 5\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "10\n");
  }

  #[test]
  fn while_loop_sums_1_to_3_and_prints_6() {
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "6\n");
  }

  #[test]
  fn break_exits_loop_early() {
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4\n  if i == 2\n    break\n  end\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def sum -> Float64\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn inception_point_example_linked_and_run_prints_5() {
    assert_eq!(compile_link_run(POINT_EXAMPLE), "5\n");
  }

  const ARRAY_EXAMPLE: &str = "arr: Array[Int64] = [10, 20, 30]\nsum: Int64 = 0\ni: Int64 = 0\nwhile i < 3\n  sum: Int64 = sum + arr[i]\n  i: Int64 = i + 1\nend\narr[1] = 99\nputs sum\nputs arr[1]\n";

  #[test]
  fn plan_09_collections_example_linked_and_run() {
    assert_eq!(compile_link_run(ARRAY_EXAMPLE), "60\n99\n");
  }

  #[test]
  fn array_literal_index_read_and_write_linked_and_run() {
    let src =
      "arr: Array[Int64] = [10, 20, 30]\nputs arr[0] + arr[1] + arr[2]\narr[1] = 99\nputs arr[1]\n";
    assert_eq!(compile_link_run(src), "60\n99\n");
  }

  #[test]
  fn array_of_float64_linked_and_run() {
    let src = "arr: Array[Float64] = [1.5, 2.5]\nputs arr[0] + arr[1]\n";
    assert_eq!(compile_link_run(src), "4\n");
  }

  #[test]
  fn lambda_capture_and_call_linked_and_run() {
    let src = "x: Int64 = 10\nadd_x: Proc = ->(y: Int64) -> Int64 { y + x }\nputs add_x.call(5)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  #[test]
  fn lambda_with_no_captures_linked_and_run() {
    let src = "add_one: Proc = ->(y: Int64) -> Int64 { y + 1 }\nputs add_one.call(41)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn plan_11_exceptions_example_linked_and_run() {
    assert_eq!(compile_link_run(EXCEPTION_EXAMPLE), "99\n");
  }

  #[test]
  fn no_exception_raised_skips_rescue_entirely() {
    let src = "def risky(x: Int64) -> Int64\n  if x > 100\n    raise MyError.new(99)\n  end\n  return x\nend\n\nclass MyError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nbegin\n  puts risky(5)\nrescue MyError => e\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "5\n");
  }

  #[test]
  fn plan_12_module_example_linked_and_run() {
    let src = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn unsupported_top_level_shape_errors_not_panics() {
    let program = Program {
      items: vec![Item::Stmt(Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Int(1), Expr::Int(2)],
      )))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_should_not_exist.o");
    // Grammar can't actually produce 2-arg `puts` (its production takes
    // exactly one Expr), but codegen must not panic on this AST shape —
    // it falls through to the generic `Stmt::Expr` arm, calling
    // `build_expr` on `Expr::Call("puts", [..])`, which errors because
    // `"puts"` isn't a compiled user function.
    assert!(compile_to_object(&program, &out).is_err());
  }

  #[test]
  fn sum_benchmark_program_matches_expected_output() {
    let src = std::fs::read_to_string(
      std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/sum/sum.em"),
    )
    .expect("benchmarks/sum/sum.em should exist");
    assert_eq!(compile_link_run(&src), "49999995000000\n");
  }

  #[test]
  fn array_traversal_benchmark_program_matches_expected_output() {
    let src = std::fs::read_to_string(
      std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/array_traversal/array_traversal.em"),
    )
    .expect("benchmarks/array_traversal/array_traversal.em should exist");
    assert_eq!(compile_link_run(&src), "210000000\n");
  }

  // Plan 18 (arithmetic & logical operators).

  const ARITHMETIC_EXAMPLE: &str = "def factorial(n: Int64) -> Int64\n  if n <= 1\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";

  #[test]
  fn arithmetic_example_linked_and_run() {
    assert_eq!(compile_link_run(ARITHMETIC_EXAMPLE), "120\n3\n2\n7\n");
  }

  const SHORT_CIRCUIT_EXAMPLE: &str = "def noisy(n: Int64) -> Boolean\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1)\n  puts 100\nend\nif x > -10 && noisy(3)\n  puts 300\nend\nif x < 0 || noisy(2)\n  puts 200\nend\nif x > 0 || noisy(4)\n  puts 400\nend\n";

  #[test]
  fn short_circuit_example_linked_and_run() {
    // Real executed proof that `noisy(1)`/`noisy(2)` are genuinely
    // never invoked (their `puts` never fires) — `1` and `2` never
    // appear in the output, not just that the final booleans are right.
    assert_eq!(
      compile_link_run(SHORT_CIRCUIT_EXAMPLE),
      "3\n300\n200\n4\n400\n"
    );
  }

  #[test]
  fn integer_division_by_zero_traps() {
    // A single-element `[0]` array load is *not* good enough here: the
    // store-then-immediate-load of a literal `0` is fully visible to
    // LLVM within one function, so O3's local memory optimizations
    // forward-substitute it back to a compile-time constant, and
    // constant-time `sdiv x, 0` is undefined behavior LLVM is free to
    // optimize away entirely (observed: the binary exited 0 instead of
    // trapping). This instead mirrors `array_traversal_benchmark_
    // program_matches_expected_output`'s own proven-not-constant-folded
    // shape (that test's real, non-instant measured run time is direct
    // evidence O3 doesn't fully evaluate it at compile time): sum a
    // 20-element array over 1,000,000 iterations to a real runtime
    // total of exactly 210000000, then subtract that same literal back
    // out — genuinely 0 only once the loop has actually run, not
    // something the optimizer can fold away.
    let src = "arr: Array[Int64] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]\ntotal: Int64 = 0\nrep: Int64 = 0\nwhile rep < 1000000\n  i: Int64 = 0\n  while i < 20\n    total: Int64 = total + arr[i]\n    i: Int64 = i + 1\n  end\n  rep: Int64 = rep + 1\nend\nzero: Int64 = total - 210000000\nputs 10 / zero\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!(
      "{}_{:?}_divzero",
      std::process::id(),
      std::thread::current().id()
    );
    let obj_path = dir.join(format!("emerald_codegen_aot_{unique}.o"));
    let bin_path = dir.join(format!("emerald_codegen_aot_bin_{unique}"));
    compile_to_object(&program, &obj_path).expect("should compile to object file");
    let status = Command::new("cc")
      .arg("-no-pie")
      .arg(&obj_path)
      .arg(runtime_path())
      .arg("-o")
      .arg(&bin_path)
      .status()
      .expect("failed to invoke cc");
    assert!(status.success(), "linking should succeed");
    let output = Command::new(&bin_path)
      .output()
      .expect("failed to run compiled binary");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();
    assert!(
      !output.status.success(),
      "division by a runtime-zero divisor should not exit 0, got {:?}",
      output.status
    );
    use std::os::unix::process::ExitStatusExt;
    assert!(
      output.status.signal().is_some(),
      "division by a runtime-zero divisor should terminate via a signal (trap), got {:?}",
      output.status
    );
  }

  #[test]
  fn unsupported_operator_shapes_error_not_panic() {
    // Float64 `%` is explicitly out of scope (plan 18's Decision log) —
    // codegen must reject it with an `Err`, not panic.
    let program = Program {
      items: vec![Item::Stmt(Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Rem(
          Box::new(Expr::Float(1.5)),
          Box::new(Expr::Float(2.0)),
        )],
      )))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_rem_float_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 19 (string literals).

  const STRING_EXAMPLE: &str = "s: String = \"hello\"\nputs s\na: String = \"foo\" + \"bar\"\nputs a\nif \"abc\" == \"abc\"\n  puts \"equal\"\nend\nputs \"line1\\nline2\"\nputs \"a\\\"b\"\n";

  #[test]
  fn string_example_linked_and_run() {
    assert_eq!(
      compile_link_run(STRING_EXAMPLE),
      "hello\nfoobar\nequal\nline1\nline2\na\"b\n"
    );
  }

  #[test]
  fn two_identical_string_literals_each_produce_a_valid_independent_pointer() {
    // AC2: no requirement that they alias the same data object.
    let src = "puts \"same\"\nputs \"same\"\n";
    assert_eq!(compile_link_run(src), "same\nsame\n");
  }

  #[test]
  fn string_ordering_comparison_errors_not_panics() {
    // AC3: `<`/`>`/`<=`/`>=` on two Strings — no lexicographic ordering
    // is defined anywhere — is a descriptive `Err`, not a panic and not
    // silently-wrong generated code.
    let program = Program {
      items: vec![Item::Stmt(Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Compare(
          Box::new(Expr::StringLit("a".into())),
          CompareOp::Lt,
          Box::new(Expr::StringLit("b".into())),
        )],
      )))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_string_lt_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 20 (comments and case/when).

  fn case_example(n: i64) -> String {
    format!(
      "# classify an integer by a fixed set of buckets\nn: Int64 = {n}\nlabel: Int64 = 0\ncase n\nwhen 1\n  label: Int64 = 10\nwhen 2, 3\n  label: Int64 = 20\nelse\n  label: Int64 = 99\nend\nputs label\n"
    )
  }

  #[test]
  fn case_single_value_arm_matches() {
    assert_eq!(compile_link_run(&case_example(1)), "10\n");
  }

  #[test]
  fn case_multi_value_arm_matches() {
    // AC1/AC2: proves the multi-value `when 2, 3` arm — both distinct
    // values, both real, executed code paths.
    assert_eq!(compile_link_run(&case_example(2)), "20\n");
    assert_eq!(compile_link_run(&case_example(3)), "20\n");
  }

  #[test]
  fn case_no_match_falls_to_else() {
    assert_eq!(compile_link_run(&case_example(7)), "99\n");
  }

  #[test]
  fn case_with_no_else_and_no_match_falls_through_harmlessly() {
    // AC3: no diagnostic, no crash — matches `if` with no `else`.
    let src = "n: Int64 = 7\ncase n\nwhen 1\n  puts 1\nend\nputs 42\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  // Plan 25 (stdlib expansion).

  const BOOL_EXAMPLE: &str = "def check(flag: Boolean) -> Int64\n  if flag\n    return 1\n  end\n  return 0\nend\n\nputs check(true)\nputs check(false)\n";

  #[test]
  fn bool_literal_example_linked_and_run() {
    assert_eq!(compile_link_run(BOOL_EXAMPLE), "1\n0\n");
  }

  const NIL_EXAMPLE: &str = "def check_nil(x: Nil) -> Int64\n  if x == nil\n    return 1\n  end\n  return 0\nend\n\nputs check_nil(nil)\n";

  #[test]
  fn nil_literal_example_linked_and_run() {
    assert_eq!(compile_link_run(NIL_EXAMPLE), "1\n");
  }

  const HASH_EXAMPLE: &str =
    "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\nputs h[2]\n";

  #[test]
  fn hash_literal_get_and_set_linked_and_run() {
    assert_eq!(compile_link_run(HASH_EXAMPLE), "20\n99\n");
  }

  #[test]
  fn hash_missing_key_aborts_at_runtime() {
    let src = "h: Hash[Int64, Int64] = {1 => 10}\nputs h[99]\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!(
      "{}_{:?}_hashmiss",
      std::process::id(),
      std::thread::current().id()
    );
    let obj_path = dir.join(format!("emerald_codegen_aot_{unique}.o"));
    let bin_path = dir.join(format!("emerald_codegen_aot_bin_{unique}"));
    compile_to_object(&program, &obj_path).expect("should compile to object file");
    let status = Command::new("cc")
      .arg("-no-pie")
      .arg(&obj_path)
      .arg(runtime_path())
      .arg("-o")
      .arg(&bin_path)
      .status()
      .expect("failed to invoke cc");
    assert!(status.success(), "linking should succeed");
    let output = Command::new(&bin_path)
      .output()
      .expect("failed to run compiled binary");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();
    assert!(
      !output.status.success(),
      "a missing Hash key should abort, not exit 0"
    );
    assert!(
      String::from_utf8_lossy(&output.stderr).contains("Hash key not found"),
      "stderr should report the abort reason, got {:?}",
      output.stderr
    );
  }

  const ARRAY_NEW_EXAMPLE: &str = "arr: Array[Int64] = Array.new(5)\nputs arr[0]\ni: Int64 = 0\nwhile i < 5\n  arr[i] = i\n  i: Int64 = i + 1\nend\nputs arr[3]\n";

  #[test]
  fn array_new_zero_filled_and_writable_linked_and_run() {
    assert_eq!(compile_link_run(ARRAY_NEW_EXAMPLE), "0\n3\n");
  }

  #[test]
  fn array_new_accepts_runtime_computed_size() {
    let src =
      "n: Int64 = 3\narr: Array[Int64] = Array.new(n)\narr[0] = 7\nputs arr[0]\nputs arr[1]\n";
    assert_eq!(compile_link_run(src), "7\n0\n");
  }

  // Plan 28 (bitwise operators).

  const BITWISE_EXAMPLE: &str = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\ndef has_flag(flags: Int64, flag: Int64) -> Boolean\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ)\n  puts 1\nend\nif has_flag(perms, EXEC)\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";

  #[test]
  fn bitwise_example_linked_and_run() {
    assert_eq!(compile_link_run(BITWISE_EXAMPLE), "3\n1\n1\n-1\n16\n16\n");
  }

  #[test]
  fn shift_setting_the_sign_bit_prints_correct_two_s_complement_value() {
    // AC2: `1 << 63` sets Int64's sign bit — a real proof `ishl`'s bit
    // pattern and the runtime's signed-print path agree, not garbage.
    let src = "puts 1 << 63\n";
    assert_eq!(compile_link_run(src), "-9223372036854775808\n");
  }

  #[test]
  fn bitwise_operator_on_non_int64_operand_errors_not_panics() {
    // Same defensive-`Err`-not-panic standard as every prior codegen
    // plan (see `unsupported_operator_shapes_error_not_panic` above) —
    // codegen itself rejects a Float64 operand even though sema (leaf
    // 2) already would have caught it first in the normal pipeline.
    let program = Program {
      items: vec![Item::Stmt(Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::BitAnd(
          Box::new(Expr::Float(1.5)),
          Box::new(Expr::Int(1)),
        )],
      )))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_bitand_float_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 29 (control-flow completeness): elsif/unless/until are pure
  // parse-time desugarings into existing Stmt::If/Stmt::While shapes —
  // no codegen source changes, real compiled-and-run proof only.

  const ELSIF_EXAMPLE: &str = "def grade(score: Int64) -> Int64\n  if score >= 90\n    return 4\n  elsif score >= 80\n    return 3\n  elsif score >= 70\n    return 2\n  else\n    return 1\n  end\nend\n\nputs grade(95)\nputs grade(85)\nputs grade(72)\nputs grade(50)\n";

  #[test]
  fn elsif_example_linked_and_run() {
    assert_eq!(compile_link_run(ELSIF_EXAMPLE), "4\n3\n2\n1\n");
  }

  const UNLESS_UNTIL_EXAMPLE: &str = "def describe(x: Int64) -> Int64\n  unless x > 0\n    return 0\n  end\n  return 1\nend\n\nputs describe(-5)\nputs describe(5)\n\ni: Int64 = 0\nuntil i >= 3\n  puts i\n  i: Int64 = i + 1\nend\n";

  #[test]
  fn unless_until_example_linked_and_run() {
    assert_eq!(compile_link_run(UNLESS_UNTIL_EXAMPLE), "0\n1\n0\n1\n2\n");
  }
}
