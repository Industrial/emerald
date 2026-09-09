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
  CaseArm, ClassDef, CompareOp, Expr, Function as AstFunction, Item, ModuleDef, Param, Program,
  RescueClause, Spanned, Stmt, StringPart,
};
use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
  CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{
  BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue, ValueKind,
};
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
  /// Plan 44 — `i64`-backed, exactly like `Int64`/`Nil` above: a
  /// compile-time-assigned dense integer ID (`Ctx::symbol_table`), not
  /// a pointer — symbol equality is a plain `icmp` on this kind, never
  /// a runtime string comparison.
  Symbol,
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
    "Symbol" => ValKind::Symbol,
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
    ValKind::Int64 | ValKind::Nil | ValKind::Symbol => context.i64_type().into(),
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
    ValKind::Int64 | ValKind::Nil | ValKind::Symbol => {
      context.i64_type().fn_type(&param_types, false)
    }
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

/// Codegen independently re-derives the class hierarchy from the raw
/// `Program`/`ClassDef` list (plan 32's Decision log — this backend has
/// no typed IR / no shared sema→codegen data structure anywhere, a
/// standing architectural fact since plan 06). Walks `name`'s
/// `superclass` chain via `class_defs`, root-to-leaf. Mirrors
/// `emerald-sema`'s own `resolve_chain` (same cycle-detection
/// discipline), but returns a plain `Err(String)` — sema already
/// rejects a cyclic/undefined chain before codegen runs in the normal
/// pipeline; this defends a direct, sema-bypassing codegen call the
/// same way every other function here does.
fn resolve_class_chain(
  name: &str,
  class_defs: &HashMap<String, &ClassDef>,
) -> Result<Vec<String>, String> {
  let mut chain = Vec::new();
  let mut visited = HashSet::new();
  let mut current = name.to_string();
  loop {
    if !visited.insert(current.clone()) {
      return Err(format!(
        "codegen: cyclic inheritance detected involving class `{current}`"
      ));
    }
    let c = class_defs
      .get(current.as_str())
      .ok_or_else(|| format!("codegen: undefined class `{current}`"))?;
    chain.push(current.clone());
    match &c.superclass {
      Some(parent) => current = parent.clone(),
      None => break,
    }
  }
  chain.reverse();
  Ok(chain)
}

/// Ancestor fields first, in ancestor-to-descendant chain order, then
/// `name`'s own fields appended (plan 32's Decision log) — the
/// load-bearing invariant that makes an inherited method's compiled
/// field offsets (e.g. `Animal_initialize` writing `@age` at offset 0)
/// still correct when invoked on a `Dog` instance, since `Dog`'s
/// layout is required to agree with `Animal`'s for every field
/// `Animal` itself declares.
fn build_class_layout(
  name: &str,
  class_defs: &HashMap<String, &ClassDef>,
) -> Result<ClassLayout, String> {
  let chain = resolve_class_chain(name, class_defs)?;
  let mut fields = HashMap::new();
  let mut offset = 0u64;
  for class_name in &chain {
    let c = class_defs[class_name.as_str()];
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
  }
  Ok(ClassLayout {
    fields,
    size: offset,
  })
}

/// `{class name} -> {method name} -> defining class name}` for every
/// declared class (plan 32's Decision log) — root-to-leaf overlay, same
/// walk `build_class_layout` does for fields: a method not overridden
/// by `name` itself resolves to whichever ancestor actually declares
/// it; an override (a same-named method `name` also declares) replaces
/// it, since the chain walk reaches `name` last.
fn build_method_owners(
  class_defs: &HashMap<String, &ClassDef>,
) -> Result<HashMap<String, HashMap<String, String>>, String> {
  let mut result = HashMap::new();
  for name in class_defs.keys() {
    let chain = resolve_class_chain(name, class_defs)?;
    let mut owners: HashMap<String, String> = HashMap::new();
    for class_name in &chain {
      let c = class_defs[class_name.as_str()];
      for m in &c.methods {
        owners.insert(m.name.clone(), class_name.clone());
      }
    }
    result.insert(name.clone(), owners);
  }
  Ok(result)
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

fn collect_idents_in_expr(expr: &Spanned<Expr>, out: &mut Vec<String>) {
  match &expr.node {
    Expr::Ident(name) => out.push(name.clone()),
    Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
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
    // Plan 39: only each keyword's own *value* expression is a
    // free-variable site — the keyword names themselves are just
    // parameter-name labels, not identifier references.
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        collect_idents_in_expr(v, out);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
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
    // Plan 36: an interpolation's literal spans have no idents; each
    // `#{...}` span's expression is a normal free-variable site (e.g. a
    // lambda body interpolating a captured outer local must still
    // capture it).
    Expr::Interpolate(parts) => {
      for part in parts {
        if let StringPart::Expr(e) = part {
          collect_idents_in_expr(e, out);
        }
      }
    }
  }
}

fn collect_idents_in_stmt(
  stmt: &Spanned<Stmt>,
  referenced: &mut Vec<String>,
  bound: &mut HashSet<String>,
) {
  match &stmt.node {
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
    // Plan 31: `name`/`names` are reassignments of an already-bound
    // outer name, never a fresh declaration — unlike `Stmt::Let` above,
    // this pushes to `referenced`, not `bound` (a lambda body
    // reassigning a captured outer variable still needs that name
    // captured, not treated as if declared here).
    Stmt::Assign { name, value } => {
      referenced.push(name.clone());
      collect_idents_in_expr(value, referenced);
    }
    // Plan 43's Decision log: mirrors `Stmt::Assign` immediately above
    // exactly — always a reassignment of an already-bound outer name,
    // never a fresh declaration.
    Stmt::OrAssign { name, default } => {
      referenced.push(name.clone());
      collect_idents_in_expr(default, referenced);
    }
    Stmt::AndAssign { name, value } => {
      referenced.push(name.clone());
      collect_idents_in_expr(value, referenced);
    }
    Stmt::MultiAssign { names, values } => {
      referenced.extend(names.iter().cloned());
      for v in values {
        collect_idents_in_expr(v, referenced);
      }
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
      rescues,
      ensure,
    } => {
      for s in body {
        collect_idents_in_stmt(s, referenced, bound);
      }
      for rescue in rescues {
        bound.insert(rescue.var.clone());
        for s in &rescue.body {
          collect_idents_in_stmt(s, referenced, bound);
        }
      }
      if let Some(ensure_body) = ensure {
        for s in ensure_body {
          collect_idents_in_stmt(s, referenced, bound);
        }
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
    // Plan 30: `var` is bound (like a `Let`'s `name`), `elements` are
    // referenced (they're evaluated in the enclosing scope, before the
    // loop var exists), `body` recurses normally.
    Stmt::For {
      var,
      elements,
      body,
    } => {
      bound.insert(var.clone());
      for e in elements {
        collect_idents_in_expr(e, referenced);
      }
      for s in body {
        collect_idents_in_stmt(s, referenced, bound);
      }
    }
    // Plan 34: `yield`'s args are ordinary references, evaluated in the
    // enclosing (callee's) scope — nothing about `yield` binds a new
    // name.
    Stmt::Yield(args) => {
      for a in args {
        collect_idents_in_expr(a, referenced);
      }
    }
    // Plan 37: mirrors `Stmt::For`'s own arm immediately above — `var`
    // is bound, `start`/`end` are referenced (evaluated in the
    // enclosing scope, before the loop var exists), `body` recurses
    // normally.
    Stmt::ForRange {
      var,
      start,
      end,
      body,
      ..
    } => {
      bound.insert(var.clone());
      collect_idents_in_expr(start, referenced);
      collect_idents_in_expr(end, referenced);
      for s in body {
        collect_idents_in_stmt(s, referenced, bound);
      }
    }
    // Plan 38: `retry` binds/references nothing — it's a bare jump.
    Stmt::Retry => {}
  }
}

/// Plan 44's Decision log: a second use of this file's existing
/// exhaustive `Expr`/`Stmt` walker shape (`collect_idents_in_expr`/
/// `_stmt` immediately above), substituting "record every distinct
/// `Expr::SymbolLit` spelling, first occurrence wins" for "record every
/// `Expr::Ident` reference." Assigns the next unused dense ID
/// (`table.len() as i64`) to each newly-seen spelling — two `:foo`
/// occurrences anywhere in the program produce the exact same ID.
fn collect_symbols_in_expr(expr: &Spanned<Expr>, table: &mut HashMap<String, i64>) {
  match &expr.node {
    Expr::SymbolLit(name) => {
      if !table.contains_key(name) {
        let id = table.len() as i64;
        table.insert(name.clone(), id);
      }
    }
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::InstanceVar(_)
    | Expr::Lambda { .. }
    | Expr::Bool(_)
    | Expr::Nil => {}
    Expr::ArrayNew(size) => collect_symbols_in_expr(size, table),
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        collect_symbols_in_expr(k, table);
        collect_symbols_in_expr(v, table);
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
      collect_symbols_in_expr(l, table);
      collect_symbols_in_expr(r, table);
    }
    Expr::Neg(e) | Expr::Not(e) | Expr::BitNot(e) => collect_symbols_in_expr(e, table),
    Expr::Compare(l, _, r) => {
      collect_symbols_in_expr(l, table);
      collect_symbols_in_expr(r, table);
    }
    Expr::Call(_, args) | Expr::New(_, args) => {
      for a in args {
        collect_symbols_in_expr(a, table);
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        collect_symbols_in_expr(v, table);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      collect_symbols_in_expr(recv, table);
      for a in args {
        collect_symbols_in_expr(a, table);
      }
    }
    Expr::ArrayLit(elements) => {
      for e in elements {
        collect_symbols_in_expr(e, table);
      }
    }
    Expr::Interpolate(parts) => {
      for part in parts {
        if let StringPart::Expr(e) = part {
          collect_symbols_in_expr(e, table);
        }
      }
    }
  }
}

fn collect_symbols_in_stmt(stmt: &Spanned<Stmt>, table: &mut HashMap<String, i64>) {
  match &stmt.node {
    Stmt::Let { value, .. } => collect_symbols_in_expr(value, table),
    Stmt::SetField { value, .. } => collect_symbols_in_expr(value, table),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      collect_symbols_in_expr(array, table);
      collect_symbols_in_expr(index, table);
      collect_symbols_in_expr(value, table);
    }
    Stmt::Assign { value, .. } => collect_symbols_in_expr(value, table),
    Stmt::OrAssign { default, .. } => collect_symbols_in_expr(default, table),
    Stmt::AndAssign { value, .. } => collect_symbols_in_expr(value, table),
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        collect_symbols_in_expr(v, table);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      collect_symbols_in_expr(cond, table);
      for s in then_branch {
        collect_symbols_in_stmt(s, table);
      }
      if let Some(else_b) = else_branch {
        for s in else_b {
          collect_symbols_in_stmt(s, table);
        }
      }
    }
    Stmt::While { cond, body } => {
      collect_symbols_in_expr(cond, table);
      for s in body {
        collect_symbols_in_stmt(s, table);
      }
    }
    Stmt::Return(Some(e)) => collect_symbols_in_expr(e, table),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Expr(e) => collect_symbols_in_expr(e, table),
    Stmt::Raise(e) => collect_symbols_in_expr(e, table),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_symbols_in_stmt(s, table);
      }
      for rescue in rescues {
        for s in &rescue.body {
          collect_symbols_in_stmt(s, table);
        }
      }
      if let Some(ensure_body) = ensure {
        for s in ensure_body {
          collect_symbols_in_stmt(s, table);
        }
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      collect_symbols_in_expr(scrutinee, table);
      for (values, body) in arms {
        for v in values {
          collect_symbols_in_expr(v, table);
        }
        for s in body {
          collect_symbols_in_stmt(s, table);
        }
      }
      if let Some(else_b) = else_body {
        for s in else_b {
          collect_symbols_in_stmt(s, table);
        }
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        collect_symbols_in_expr(e, table);
      }
      for s in body {
        collect_symbols_in_stmt(s, table);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      collect_symbols_in_expr(start, table);
      collect_symbols_in_expr(end, table);
      for s in body {
        collect_symbols_in_stmt(s, table);
      }
    }
    Stmt::Yield(args) => {
      for a in args {
        collect_symbols_in_expr(a, table);
      }
    }
  }
}

/// Plan 44's Decision log: the whole-program driver — walks every
/// top-level `Item::Function` body, every `ClassDef` method body, every
/// `ModuleDef` method body, and every top-level `Item::Stmt`, in
/// `program.items` order, assigning IDs via `collect_symbols_in_stmt`.
/// The collector walks every declared function/method regardless of
/// call reachability (no reachability analysis exists in this compiler
/// to make that distinction safely) — dead code still gets a real ID.
fn collect_program_symbols(program: &Program) -> HashMap<String, i64> {
  let mut table = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        for s in &f.body {
          collect_symbols_in_stmt(s, &mut table);
        }
      }
      Item::Class(c) => {
        for m in &c.methods {
          for s in &m.body {
            collect_symbols_in_stmt(s, &mut table);
          }
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          for s in &f.body {
            collect_symbols_in_stmt(s, &mut table);
          }
        }
      }
      Item::Stmt(s) => collect_symbols_in_stmt(s, &mut table),
      // Plan 47: never actually reached — `compile_to_object`'s own
      // prologue desugars every `Item::Test` into an `Item::Function`
      // before this runs — handled anyway, the same way a function
      // body is, for robustness against any future caller that skips
      // that prologue.
      Item::Test { body, .. } => {
        for s in body {
          collect_symbols_in_stmt(s, &mut table);
        }
      }
      Item::Interface(_) | Item::Require(_) | Item::Error => {}
    }
  }
  table
}

/// Plan 41's Decision log: resolves a generic call site's own concrete
/// argument type — a plain local via the walk's own accumulated
/// `local_classes` (the same side-table `bind_params`/`Stmt::Let`
/// codegen already builds incrementally), or a direct `ClassName.new(
/// ...)` literal. Anything else (a nested call's return value, a
/// method-call result, ...) is out of scope for this plan's own worked
/// example — sema has already accepted the whole program, so a shape
/// this can't resolve is a real, disclosed codegen gap (AC4), not a
/// miscompile.
fn resolve_arg_concrete_class<'a>(
  arg: &'a Spanned<Expr>,
  local_classes: &'a HashMap<String, String>,
) -> Option<&'a str> {
  match &arg.node {
    Expr::Ident(name) => local_classes.get(name).map(String::as_str),
    Expr::New(class_name, _) => Some(class_name.as_str()),
    _ => None,
  }
}

/// Shared by `collect_generic_specializations`'s program-wide collection
/// pass and each individual call site's own codegen (`build_call_expr`),
/// so the two can never disagree about which mangled symbol a given
/// call resolves to.
fn mangled_generic_call_symbol(
  name: &str,
  g: &AstFunction,
  args: &[Spanned<Expr>],
  local_classes: &HashMap<String, String>,
) -> Result<String, String> {
  let type_param = g
    .type_params
    .first()
    .ok_or_else(|| format!("codegen: internal error — `{name}` has no type parameter"))?;
  let mut concrete: Option<&str> = None;
  for (i, p) in g.params.iter().enumerate() {
    if p.ty == type_param.name {
      if let Some(c) = args
        .get(i)
        .and_then(|a| resolve_arg_concrete_class(a, local_classes))
      {
        concrete = Some(c);
      }
    }
  }
  let concrete = concrete.ok_or_else(|| {
    format!(
      "codegen: could not resolve generic function `{name}`'s type parameter `{}` to a concrete class at this call site",
      type_param.name
    )
  })?;
  Ok(mangled_generic_symbol(name, concrete))
}

fn mangled_generic_symbol(fn_name: &str, concrete_class: &str) -> String {
  format!("{fn_name}$${concrete_class}")
}

/// The initial `local_classes` a function/method body's collection walk
/// starts from — every class-typed parameter, mirroring `bind_params`'s
/// own `classes.contains_key(p.ty.as_str())` bookkeeping exactly, minus
/// the actual LLVM binding (this is a pure-AST pass, no `Builder`).
fn param_local_classes(
  params: &[Param],
  classes: &HashMap<String, ClassLayout>,
) -> HashMap<String, String> {
  params
    .iter()
    .filter(|p| classes.contains_key(p.ty.as_str()))
    .map(|p| (p.name.clone(), p.ty.clone()))
    .collect()
}

/// Plan 41's Decision log: a program-wide AST walk (the same style as
/// `collect_idents_in_expr`/`collect_idents_in_stmt`) collecting every
/// distinct `(generic function name, concrete class name)` pair actually
/// called anywhere in the whole program — one compiled LLVM function is
/// emitted per entry here, and no others.
fn collect_generic_specializations(
  program: &Program,
  generic_fns: &HashMap<String, &AstFunction>,
  classes: &HashMap<String, ClassLayout>,
) -> HashMap<String, HashSet<String>> {
  let mut out: HashMap<String, HashSet<String>> = HashMap::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        let mut local_classes = param_local_classes(&f.params, classes);
        for s in &f.body {
          collect_specializations_in_stmt(s, generic_fns, &mut local_classes, classes, &mut out);
        }
      }
      Item::Class(c) => {
        for m in &c.methods {
          let mut local_classes = param_local_classes(&m.params, classes);
          for s in &m.body {
            collect_specializations_in_stmt(s, generic_fns, &mut local_classes, classes, &mut out);
          }
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          let mut local_classes = param_local_classes(&f.params, classes);
          for s in &f.body {
            collect_specializations_in_stmt(s, generic_fns, &mut local_classes, classes, &mut out);
          }
        }
      }
      _ => {}
    }
  }
  // Top-level statements share one flat environment across the whole
  // program in source order (mirrors `emerald-sema`'s own `top_env`
  // threading in `check_program`) — one shared, accumulating
  // `local_classes` across the whole `Item::Stmt` sequence, not a fresh
  // one per statement.
  let mut top_local_classes: HashMap<String, String> = HashMap::new();
  for item in &program.items {
    if let Item::Stmt(s) = item {
      collect_specializations_in_stmt(s, generic_fns, &mut top_local_classes, classes, &mut out);
    }
  }
  out
}

fn collect_specializations_in_expr(
  expr: &Spanned<Expr>,
  generic_fns: &HashMap<String, &AstFunction>,
  local_classes: &HashMap<String, String>,
  out: &mut HashMap<String, HashSet<String>>,
) {
  if let Expr::Call(name, args) = &expr.node {
    if let Some(g) = generic_fns.get(name) {
      if let Some(type_param) = g.type_params.first() {
        for (i, p) in g.params.iter().enumerate() {
          if p.ty == type_param.name {
            if let Some(concrete) = args
              .get(i)
              .and_then(|a| resolve_arg_concrete_class(a, local_classes))
            {
              out
                .entry(name.clone())
                .or_default()
                .insert(concrete.to_string());
            }
          }
        }
      }
    }
  }
  match &expr.node {
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::InstanceVar(_)
    | Expr::Lambda { .. }
    | Expr::Bool(_)
    | Expr::Nil => {}
    Expr::ArrayNew(size) => collect_specializations_in_expr(size, generic_fns, local_classes, out),
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        collect_specializations_in_expr(k, generic_fns, local_classes, out);
        collect_specializations_in_expr(v, generic_fns, local_classes, out);
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
      collect_specializations_in_expr(l, generic_fns, local_classes, out);
      collect_specializations_in_expr(r, generic_fns, local_classes, out);
    }
    Expr::Neg(e) | Expr::Not(e) | Expr::BitNot(e) => {
      collect_specializations_in_expr(e, generic_fns, local_classes, out)
    }
    Expr::Compare(l, _, r) => {
      collect_specializations_in_expr(l, generic_fns, local_classes, out);
      collect_specializations_in_expr(r, generic_fns, local_classes, out);
    }
    Expr::Call(_, args) | Expr::New(_, args) => {
      for a in args {
        collect_specializations_in_expr(a, generic_fns, local_classes, out);
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        collect_specializations_in_expr(v, generic_fns, local_classes, out);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      collect_specializations_in_expr(recv, generic_fns, local_classes, out);
      for a in args {
        collect_specializations_in_expr(a, generic_fns, local_classes, out);
      }
    }
    Expr::ArrayLit(elements) => {
      for e in elements {
        collect_specializations_in_expr(e, generic_fns, local_classes, out);
      }
    }
    Expr::Interpolate(parts) => {
      for part in parts {
        if let StringPart::Expr(e) = part {
          collect_specializations_in_expr(e, generic_fns, local_classes, out);
        }
      }
    }
  }
}

fn collect_specializations_in_stmt(
  stmt: &Spanned<Stmt>,
  generic_fns: &HashMap<String, &AstFunction>,
  local_classes: &mut HashMap<String, String>,
  classes: &HashMap<String, ClassLayout>,
  out: &mut HashMap<String, HashSet<String>>,
) {
  match &stmt.node {
    Stmt::Let { name, ty, value } => {
      collect_specializations_in_expr(value, generic_fns, local_classes, out);
      if classes.contains_key(ty.as_str()) {
        local_classes.insert(name.clone(), ty.clone());
      }
    }
    Stmt::SetField { value, .. } => {
      collect_specializations_in_expr(value, generic_fns, local_classes, out)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      collect_specializations_in_expr(array, generic_fns, local_classes, out);
      collect_specializations_in_expr(index, generic_fns, local_classes, out);
      collect_specializations_in_expr(value, generic_fns, local_classes, out);
    }
    Stmt::Assign { value, .. } => {
      collect_specializations_in_expr(value, generic_fns, local_classes, out)
    }
    Stmt::OrAssign { default, .. } => {
      collect_specializations_in_expr(default, generic_fns, local_classes, out)
    }
    Stmt::AndAssign { value, .. } => {
      collect_specializations_in_expr(value, generic_fns, local_classes, out)
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        collect_specializations_in_expr(v, generic_fns, local_classes, out);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      collect_specializations_in_expr(cond, generic_fns, local_classes, out);
      for s in then_branch {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
      if let Some(else_b) = else_branch {
        for s in else_b {
          collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
        }
      }
    }
    Stmt::While { cond, body } => {
      collect_specializations_in_expr(cond, generic_fns, local_classes, out);
      for s in body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
    }
    Stmt::Return(Some(e)) => collect_specializations_in_expr(e, generic_fns, local_classes, out),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Expr(e) => collect_specializations_in_expr(e, generic_fns, local_classes, out),
    Stmt::Raise(e) => collect_specializations_in_expr(e, generic_fns, local_classes, out),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
      for rescue in rescues {
        for s in &rescue.body {
          collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
        }
      }
      if let Some(ensure_body) = ensure {
        for s in ensure_body {
          collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
        }
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      collect_specializations_in_expr(scrutinee, generic_fns, local_classes, out);
      for (values, body) in arms {
        for v in values {
          collect_specializations_in_expr(v, generic_fns, local_classes, out);
        }
        for s in body {
          collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
        }
      }
      if let Some(else_b) = else_body {
        for s in else_b {
          collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
        }
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        collect_specializations_in_expr(e, generic_fns, local_classes, out);
      }
      for s in body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      collect_specializations_in_expr(start, generic_fns, local_classes, out);
      collect_specializations_in_expr(end, generic_fns, local_classes, out);
      for s in body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
    }
    Stmt::Yield(args) => {
      for a in args {
        collect_specializations_in_expr(a, generic_fns, local_classes, out);
      }
    }
  }
}

/// Plan 41's Decision log: textually substitutes `concrete_class` for
/// the function's own type parameter in its `params`/`return_type`
/// strings, producing a fully concrete `Function` AST that the
/// *existing*, unmodified single-function codegen path (`define_user_
/// function`) can compile exactly as if it had been written by hand for
/// this one concrete type — 100% reuse of the non-generic machinery.
fn substitute_generic_function(
  f: &AstFunction,
  type_param: &str,
  concrete_class: &str,
) -> AstFunction {
  let substitute = |ty: &str| -> String {
    if ty == type_param {
      concrete_class.to_string()
    } else {
      ty.to_string()
    }
  };
  AstFunction {
    name: f.name.clone(),
    params: f
      .params
      .iter()
      .map(|p| Param {
        name: p.name.clone(),
        ty: substitute(&p.ty),
        default: p.default.clone(),
      })
      .collect(),
    return_type: substitute(&f.return_type),
    body: f.body.clone(),
    block_param: f.block_param.clone(),
    splat_param: f.splat_param.clone(),
    type_params: Vec::new(),
  }
}

fn free_vars_in_lambda(params: &[Param], body: &[Spanned<Stmt>]) -> Vec<String> {
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
    if let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, .. },
      ..
    }) = item
    {
      top_level_types.insert(name.clone(), ty.clone());
    }
  }

  let mut lambda_infos: HashMap<String, LambdaInfo> = HashMap::new();
  for item in &program.items {
    let Item::Stmt(Spanned {
      node:
        Stmt::Let {
          name,
          ty,
          value:
            Spanned {
              node: Expr::Lambda { params, body, .. },
              ..
            },
        },
      ..
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
fn collect_lets(stmts: &[Spanned<Stmt>], out: &mut Vec<(String, ValKind)>) {
  for stmt in stmts {
    match &stmt.node {
      Stmt::Let { name, ty, .. } => out.push((name.clone(), value_kind_for_type(ty))),
      Stmt::While { body, .. } => collect_lets(body, out),
      // Plan 30: `var` itself is deliberately NOT hoisted here — unlike
      // a `Let`'s declared `ty` string, there's no syntactic type
      // annotation to derive its `ValKind` from ahead of time (sema
      // unifies it from `elements`' inferred types, which needs
      // context this purely-syntactic pre-pass doesn't have). Its
      // alloca is built inline at the `Stmt::For` codegen site instead,
      // once the first element's real `ValKind` is known. Any ordinary
      // `Let`s inside `body` still need hoisting, same as every other
      // loop/branch body here.
      Stmt::For { body, .. } => collect_lets(body, out),
      // Plan 37: unlike `Stmt::For`'s `var` immediately above, `var`'s
      // `ValKind` *is* knowable statically here — a Range's endpoints
      // are Int64-only by construction (see the plan's Decision log),
      // with zero dependence on either operand's shape — so it's
      // hoisted up front exactly like an ordinary `Let`, a real
      // simplification `Stmt::For`'s own case structurally can't take.
      Stmt::ForRange { var, body, .. } => {
        out.push((var.clone(), ValKind::Int64));
        collect_lets(body, out);
      }
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
      // Plan 38: only a TYPED clause's `var` gets a slot — a bare
      // clause's `var` is never stored into anything (per the sema
      // leaf, it's never bound in `env` either; see `build_begin`'s
      // own multi-rescue chaining). `ensure`'s own `Let`s need hoisting
      // too, same as `body`/each rescue's own `body`.
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        collect_lets(body, out);
        for rescue in rescues {
          if rescue.class_name.is_some() {
            out.push((rescue.var.clone(), ValKind::Ptr));
          }
          collect_lets(&rescue.body, out);
        }
        if let Some(ensure_body) = ensure {
          collect_lets(ensure_body, out);
        }
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

/// Plan 38: does `stmt` (at any nesting depth) contain a `Stmt::Retry`?
/// Used to decide whether `compile_to_object` must skip optimization
/// for correctness (see its own call site's Decision-log comment).
fn stmt_contains_retry(stmt: &Spanned<Stmt>) -> bool {
  match &stmt.node {
    Stmt::Retry => true,
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      then_branch.iter().any(stmt_contains_retry)
        || else_branch
          .as_ref()
          .is_some_and(|b| b.iter().any(stmt_contains_retry))
    }
    Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
      body.iter().any(stmt_contains_retry)
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      body.iter().any(stmt_contains_retry)
        || rescues
          .iter()
          .any(|r| r.body.iter().any(stmt_contains_retry))
        || ensure
          .as_ref()
          .is_some_and(|b| b.iter().any(stmt_contains_retry))
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      arms
        .iter()
        .any(|(_, body)| body.iter().any(stmt_contains_retry))
        || else_body
          .as_ref()
          .is_some_and(|b| b.iter().any(stmt_contains_retry))
    }
    _ => false,
  }
}

/// Plan 38: does `program` use `retry` anywhere — in a free function, a
/// method, or a top-level statement?
fn program_uses_retry(program: &Program) -> bool {
  fn body_uses_retry(body: &[Spanned<Stmt>]) -> bool {
    body.iter().any(stmt_contains_retry)
  }
  program.items.iter().any(|item| match item {
    Item::Function(f) => body_uses_retry(&f.body),
    Item::Class(c) => c.methods.iter().any(|m| body_uses_retry(&m.body)),
    Item::Module(m) => m.methods.iter().any(|m| body_uses_retry(&m.body)),
    Item::Stmt(s) => stmt_contains_retry(s),
    _ => false,
  })
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
  // Plan 38: `setjmp` can return TWICE (once directly, once via a
  // later `longjmp`) — a fact real C compilers know only because
  // `<setjmp.h>` marks it specially; since this backend declares
  // `setjmp` as an ordinary external function at the LLVM IR level
  // (bypassing Clang's C frontend entirely), that attribute is never
  // attached automatically. Without it, LLVM's optimizer is free to
  // assume the call returns once — sound for plan 11's original
  // straight-line (no loop back to before the call) usage, but a real
  // miscompile once plan 38's `retry` introduces a genuine control-flow
  // loop back to `begin.retry`'s own `setjmp` call (verified this
  // session: omitting this attribute produces a binary that hangs).
  let returns_twice_id = Attribute::get_named_enum_kind_id("returns_twice");
  let returns_twice_attr = context.create_enum_attribute(returns_twice_id, 0);
  setjmp.add_attribute(AttributeLoc::Function, returns_twice_attr);
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
/// `Copy` (plan 38): `Stmt::Break`/`Stmt::Next` need to read the target
/// out of `loop_stack` before separately re-borrowing it mutably to
/// duplicate-emit `ensure_stack`.
#[derive(Clone, Copy)]
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
  /// Plan 36's compiler-known interpolation stringifiers.
  int64_to_string: FunctionValue<'ctx>,
  float64_to_string: FunctionValue<'ctx>,
  bool_to_string: FunctionValue<'ctx>,
  self_ctx: Option<(PointerValue<'ctx>, &'a HashMap<String, FieldInfo>)>,
  /// `{lambda's Let name} -> (its synthesized `__lambda_{name}` function,
  /// its declared return kind)`, for statically dispatching `.call`.
  lambda_func_ids: &'a HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  lambda_infos: &'a HashMap<String, LambdaInfo>,
  /// `{class name} -> a stable integer tag (declaration order)` —
  /// `rescue`'s matching mechanism, standing in for RTTI.
  class_tags: &'a HashMap<String, i64>,
  /// `{class name T} -> tags of every class that either *is* T or
  /// descends from it` (plan 38's Decision log) — what makes `rescue`
  /// subtype-aware: a clause naming a superclass matches any raised
  /// subclass too, not just an exact-tag match. Precomputed once, for
  /// the whole compiled program, from the same chain representation
  /// plan 32's own `resolve_class_chain` already builds.
  rescue_tag_sets: &'a HashMap<String, Vec<i64>>,
  /// `{class name} -> {method name} -> defining class name}` (plan 32's
  /// Decision log) — resolves which ancestor's compiled `{Class}_
  /// {method}` symbol a call actually targets, since only the class
  /// that *declares* a method gets an LLVM function generated for it
  /// (`d.age` on a `Dog` that never declares `age` must call
  /// `Animal_age`, since `Dog_age` was never compiled).
  method_owners: &'a HashMap<String, HashMap<String, String>>,
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
  /// Plan 34: `{name} -> its raw AST}` for every `block_param`-
  /// declaring free function — never compiled as an ordinary LLVM
  /// function (see `declare_user_functions`'s matching arm), only ever
  /// inline-expanded per call site by `build_inline_block_call`, which
  /// needs the callee's real `params`/`body` to do that.
  block_funcs: &'a HashMap<String, &'a AstFunction>,
  /// Plan 39's Decision log: `{name} -> its raw AST}` for EVERY
  /// top-level free function (unlike `block_funcs` above, not just
  /// `block_param`-declaring ones) — `Expr::Call`/`Expr::CallKw`'s own
  /// codegen needs the callee's real `Function.params[i].default`/
  /// `splat_param` to fill in omitted trailing arguments and pack a
  /// splat's variadic tail, neither of which `user_func_ids` (compiled
  /// LLVM handles only, no parameter-level detail) can answer.
  func_defs: &'a HashMap<String, &'a AstFunction>,
  /// `Some((block's params, block's body))` while compiling a
  /// `block_param`-declaring function's body inline at one specific
  /// call site that attached a literal block — `Stmt::Yield`'s codegen
  /// reads this to know what to substitute. `None` everywhere else
  /// (ordinary function/method/lambda bodies never reach a `Stmt::
  /// Yield` — sema already guarantees that).
  yield_target: Option<(&'a [Param], &'a [Spanned<Stmt>])>,
  /// `{symbol spelling} -> a dense compile-time integer ID` (plan 44's
  /// Decision log) — every distinct `:foo` spelling anywhere in the
  /// whole program, assigned in first-occurrence order, mirroring
  /// `class_tags`' own shape (a single flat table built once before any
  /// function body compiles). No runtime interning table exists at all
  /// — the symbol set is closed and fully enumerable at parse time.
  symbol_table: &'a HashMap<String, i64>,
  /// Plan 45's curated `String` intrinsic surface — declared once in
  /// `compile_to_object`, alongside `print_str`/`string_concat`/
  /// `string_eq`, dispatched from `build_method_call`/`build_index`
  /// once the receiver's `ValKind` is `Str`.
  string_length: FunctionValue<'ctx>,
  string_upcase: FunctionValue<'ctx>,
  string_downcase: FunctionValue<'ctx>,
  string_strip: FunctionValue<'ctx>,
  string_to_i: FunctionValue<'ctx>,
  string_to_f: FunctionValue<'ctx>,
  string_char_at: FunctionValue<'ctx>,
  string_slice: FunctionValue<'ctx>,
  string_split_count: FunctionValue<'ctx>,
  string_split: FunctionValue<'ctx>,
  /// Plan 45's Decision log: `File` reuses plan 12's `Name.method(args)`
  /// dispatch shape but is a separate, hard-coded arm in `build_method_
  /// call` — `File` is never a `ModuleDef`, so it never populates
  /// `user_func_ids` the way a real module's methods do.
  file_read: FunctionValue<'ctx>,
  file_write: FunctionValue<'ctx>,
  /// `gets()` — dispatched directly in `build_expr`'s `Expr::Call`
  /// handling, the same way `puts` is, since it must be usable as an
  /// expression.
  gets: FunctionValue<'ctx>,
  /// Populates `ARGV`/`ARGC` at the top of generated `main` (`leaf-
  /// argv-and-gets`'s own dedicated construction site — never called
  /// anywhere else).
  build_argv: FunctionValue<'ctx>,
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
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
  int_op: IntBinOp<'ctx>,
  float_op: Option<FloatBinOp<'ctx>>,
  name: &str,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  // Plan 40's Decision log: checked before either operand is built —
  // when the LHS is a class-typed plain local, delegate to the exact
  // same static `build_method_call` an ordinary `a.foo()` already
  // uses, with `op_symbol` as the method name and `[rhs]` as the
  // argument list. Covers `-`/`*`/`/` (three of this plan's eight
  // scoped tokens) plus `%` (outside the plan's literal list, but
  // routed identically — see `check_numeric_binop`'s own sema-side
  // comment).
  if let Expr::Ident(name) = &lhs.node {
    if local_classes.contains_key(name) {
      return build_method_call(
        context,
        builder,
        lhs,
        op_symbol,
        std::slice::from_ref(rhs),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      );
    }
  }
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
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
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
  lhs: &Spanned<Expr>,
  rhs: &Spanned<Expr>,
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

/// Plan 36: string interpolation folds every part into one `char*` via
/// repeated `ctx.string_concat` calls — a `Literal` part reuses
/// `StringLit`'s own `build_global_string_ptr`; an `Expr` part is built
/// normally then dispatched by its returned `ValKind` onto the matching
/// compiler-known stringifier (sema already rejected any other kind,
/// so anything else reaching here is an internal-error `Err`, not a
/// user-facing one). `parts` is never empty — `interpolate.rs` only
/// ever constructs `Expr::Interpolate` after pushing at least one
/// `StringPart::Expr`.
fn build_interpolate<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  parts: &[StringPart],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let mut acc: Option<PointerValue> = None;
  for part in parts {
    let part_ptr = match part {
      StringPart::Literal(s) => {
        let global = builder
          .build_global_string_ptr(s, "strlit")
          .map_err(|e| e.to_string())?;
        global.as_pointer_value()
      }
      StringPart::Expr(e) => {
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
          ValKind::Str => v.into_pointer_value(),
          ValKind::Int64 => {
            let call = builder
              .build_call(ctx.int64_to_string, &[v.into()], "i64tostr")
              .map_err(|e| e.to_string())?;
            call_result(call)?.into_pointer_value()
          }
          ValKind::Float64 => {
            let call = builder
              .build_call(ctx.float64_to_string, &[v.into()], "f64tostr")
              .map_err(|e| e.to_string())?;
            call_result(call)?.into_pointer_value()
          }
          ValKind::Bool => {
            let extended = builder
              .build_int_z_extend(v.into_int_value(), context.i64_type(), "boolext")
              .map_err(|e| e.to_string())?;
            let call = builder
              .build_call(ctx.bool_to_string, &[extended.into()], "booltostr")
              .map_err(|e| e.to_string())?;
            call_result(call)?.into_pointer_value()
          }
          other => {
            return Err(format!(
              "codegen: internal error — sema should have rejected interpolating a `{other:?}` value"
            ));
          }
        }
      }
    };
    acc = Some(match acc {
      None => part_ptr,
      Some(prev) => {
        let call = builder
          .build_call(
            ctx.string_concat,
            &[prev.into(), part_ptr.into()],
            "interpconcat",
          )
          .map_err(|e| e.to_string())?;
        call_result(call)?.into_pointer_value()
      }
    });
  }
  let result = acc.ok_or("codegen: internal error — empty string interpolation")?;
  Ok((result.into(), ValKind::Str))
}

fn build_expr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  expr: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  match &expr.node {
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
    // Plan 44's Decision log: looked up in the whole-program compile-
    // time symbol table (`ctx.symbol_table`, built once by `collect_
    // symbols_in_expr`/`_stmt` before any function body compiles) and
    // lowered to a plain `i64` constant — the same one-instruction
    // shape as `Expr::Int`, never a runtime interning lookup. An
    // internal-error `Err` (never a panic) if somehow missing, which
    // the pre-pass's completeness should make unreachable.
    Expr::SymbolLit(name) => {
      let id = *ctx
        .symbol_table
        .get(name)
        .ok_or_else(|| format!("codegen: internal error — symbol `:{name}` has no assigned ID"))?;
      Ok((
        context.i64_type().const_int(id as u64, false).into(),
        ValKind::Symbol,
      ))
    }
    // Plan 36: string interpolation — see `build_interpolate` below.
    Expr::Interpolate(parts) => build_interpolate(
      context,
      builder,
      parts,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    // Plan 40's Decision log: checked before either operand is built —
    // see `build_numeric_binop`'s identical branch for `-`/`*`/`/`.
    Expr::Add(lhs, rhs) if matches!(&lhs.node, Expr::Ident(name) if local_classes.contains_key(name)) => {
      build_method_call(
        context,
        builder,
        lhs,
        "+",
        std::slice::from_ref(rhs.as_ref()),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
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
    // Plan 40's Decision log: `==`/`!=` on a class-typed LHS delegate
    // to the class's own `==` method (a bare local receiver only, per
    // `build_method_call`'s own existing restriction) — `!=` calls the
    // identical `==` method and inverts the returned `i1` (Ruby's own
    // default: no separate `!=` overload token). Sema already rejects
    // any other `CompareOp` on a class operand, so reaching this arm
    // with one is an internal-error `Err`, not a panic.
    Expr::Compare(lhs, op, rhs) if matches!(&lhs.node, Expr::Ident(name) if local_classes.contains_key(name)) =>
    {
      if !matches!(op, CompareOp::Eq | CompareOp::Ne) {
        return Err(format!(
          "codegen: internal error — ordering comparison `{op:?}` on a class operand should have been rejected by sema"
        ));
      }
      let (result, kind) = build_method_call(
        context,
        builder,
        lhs,
        "==",
        std::slice::from_ref(rhs.as_ref()),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if kind != ValKind::Bool {
        return Err(
          "codegen: internal error — a class's `==` method should return Boolean (sema should have rejected this)"
            .to_string(),
        );
      }
      if matches!(op, CompareOp::Ne) {
        let inverted = builder
          .build_not(result.into_int_value(), "netmp")
          .map_err(|e| e.to_string())?;
        Ok((inverted.into(), ValKind::Bool))
      } else {
        Ok((result, ValKind::Bool))
      }
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
        // Plan 44's Decision log: `Symbol == Symbol`/`!=` — a plain
        // `icmp` on two `i64`s (compile-time-assigned dense IDs), the
        // same one-instruction shape as the `Nil` arm immediately
        // above; never leaves the current basic block, unlike `String`
        // equality's real `emerald_string_eq` call. Ordering
        // comparisons are declined — a symbol's integer ID is assigned
        // by arbitrary first-occurrence source order, not by spelling,
        // so exposing `<`/`>` would silently expose a compiler
        // implementation detail as a meaningful ordering.
        (ValKind::Symbol, ValKind::Symbol) if matches!(op, CompareOp::Eq | CompareOp::Ne) => {
          let pred = if matches!(op, CompareOp::Eq) {
            IntPredicate::EQ
          } else {
            IntPredicate::NE
          };
          builder
            .build_int_compare(pred, l.into_int_value(), r.into_int_value(), "symcmptmp")
            .map_err(|e| e.to_string())?
        }
        (ValKind::Symbol, ValKind::Symbol) => {
          return Err(format!(
            "codegen: `{op:?}` is not supported on Symbol — only `==`/`!=` are"
          ));
        }
        // Plan 43's Decision log: a `Nullable(_)` operand (`ValKind::
        // Ptr`/`Str`, both real pointers) against a literal `nil`
        // (`ValKind::Nil`, `Expr::Nil`'s fixed `i64` `0` sentinel —
        // plan 25's design, NOT a pointer) — either order — lowers as a
        // genuine null-pointer test on the pointer-backed side
        // (`build_is_null`), not a general pointer-equality comparison.
        // Distinct from the `(Nil, Nil)` case above (a bare `Nil`-typed
        // variable's own comparison, e.g. `x: Nil` — unrelated to `T?`).
        (ValKind::Ptr | ValKind::Str, ValKind::Nil)
        | (ValKind::Nil, ValKind::Ptr | ValKind::Str)
          if matches!(op, CompareOp::Eq | CompareOp::Ne) =>
        {
          let ptr_val = if lk == ValKind::Nil {
            r.into_pointer_value()
          } else {
            l.into_pointer_value()
          };
          let is_null = builder
            .build_is_null(ptr_val, "isniltest")
            .map_err(|e| e.to_string())?;
          if matches!(op, CompareOp::Ne) {
            builder
              .build_not(is_null, "nilnetmp")
              .map_err(|e| e.to_string())?
          } else {
            is_null
          }
        }
        _ => return Err("codegen: comparison operands must both be Int64 or both Float64".into()),
      };
      Ok((cmp.into(), ValKind::Bool))
    }
    // Plan 45's Decision log: unlike `puts` (a dedicated `Stmt`-level
    // keyword), `gets` must be usable as an expression — dispatched
    // here directly, the same hard-coded-name shape `File`/String
    // intrinsics use, checked before the generic `Expr::Call` arm below
    // (`gets` is never in `ctx.user_func_ids`, so it could never reach
    // that arm's lookup successfully anyway).
    Expr::Call(name, args) if name == "gets" => {
      if !args.is_empty() {
        return Err(format!(
          "codegen: `gets` expects 0 arguments, found {}",
          args.len()
        ));
      }
      let call = builder
        .build_call(ctx.gets, &[], "getstmp")
        .map_err(|e| e.to_string())?;
      Ok((call_result(call)?, ValKind::Str))
    }
    Expr::Call(name, args) => {
      let (result, ret_kind) = build_call_expr(
        context,
        builder,
        name,
        args,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if ret_kind == ValKind::Void {
        return Err(format!(
          "codegen: `{name}` returns Void and can't be used as a value"
        ));
      }
      Ok((result, ret_kind))
    }
    // Plan 39's Decision log: resolved entirely at compile time — every
    // supplied `name: value` is matched against the callee's declared
    // parameter names (sema already guarantees no unknown/duplicate/
    // missing-required name reaches codegen), and any parameter left
    // unsupplied is filled from its declared default.
    Expr::CallKw(name, kwargs) => {
      let (result, ret_kind) = build_call_kw_expr(
        context,
        builder,
        name,
        kwargs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if ret_kind == ValKind::Void {
        return Err(format!(
          "codegen: `{name}` returns Void and can't be used as a value"
        ));
      }
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

      // Plan 32: `initialize` resolves through `method_owners` too — a
      // subclass that doesn't declare its own `initialize` inherits the
      // nearest ancestor's, same as any other method.
      let init_key = ctx
        .method_owners
        .get(class_name.as_str())
        .and_then(|owners| owners.get("initialize"))
        .map(|owner| format!("{owner}_initialize"));
      if let Some(&(init_fv, _)) = init_key.as_deref().and_then(|k| ctx.user_func_ids.get(k)) {
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
    Expr::SafeCall(recv, method, args) => build_safe_call(
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
  pairs: &[(Spanned<Expr>, Spanned<Expr>)],
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
  recv: &Spanned<Expr>,
  method: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let Expr::Ident(recv_name) = &recv.node else {
    return Err(
      "codegen: method calls are only supported on a plain local-variable receiver".to_string(),
    );
  };

  // Plan 45's Decision log: `File` is a separate, hard-coded arm, not
  // plan 12's real module-dispatch mechanism (`File` is never a
  // `ModuleDef`, so it never populates `ctx.module_names`) — checked
  // first purely for arm-ordering clarity, since it can never actually
  // collide with the module check below.
  if recv_name == "File" {
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
    return match method {
      "read" => {
        let call = builder
          .build_call(ctx.file_read, &call_args, "filereadtmp")
          .map_err(|e| e.to_string())?;
        Ok((call_result(call)?, ValKind::Str))
      }
      "write" => {
        builder
          .build_call(ctx.file_write, &call_args, "filewritetmp")
          .map_err(|e| e.to_string())?;
        Ok((context.i64_type().const_int(0, false).into(), ValKind::Void))
      }
      other => Err(format!("codegen: unsupported File method `{other}`")),
    };
  }

  // Plan 45's Decision log: dispatched by checking `vars.get(recv_name)`'s
  // stored `ValKind` for `Str`, before falling through to `local_classes`'
  // class-name lookup below (which errors with "cannot determine the
  // class" for any receiver that isn't a registered class — a
  // String-typed local was always silently doomed to hit exactly that
  // error before this plan).
  if let Some((_, ValKind::Str)) = vars.get(recv_name) {
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
    let (fv, ret_kind) = match method {
      "length" => (ctx.string_length, ValKind::Int64),
      "upcase" => (ctx.string_upcase, ValKind::Str),
      "downcase" => (ctx.string_downcase, ValKind::Str),
      "strip" => (ctx.string_strip, ValKind::Str),
      "to_i" => (ctx.string_to_i, ValKind::Int64),
      "to_f" => (ctx.string_to_f, ValKind::Float64),
      "slice" => (ctx.string_slice, ValKind::Str),
      "split_count" => (ctx.string_split_count, ValKind::Int64),
      "split" => (ctx.string_split, ValKind::Ptr),
      other => return Err(format!("codegen: unsupported String method `{other}`")),
    };
    let call = builder
      .build_call(fv, &call_args, "strmethodtmp")
      .map_err(|e| e.to_string())?;
    return Ok((call_result(call)?, ret_kind));
  }

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
    if ret_kind == ValKind::Void {
      return Ok((context.i64_type().const_int(0, false).into(), ret_kind));
    }
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
    // Plan 32: resolve which ancestor actually *declares* `method` —
    // only the defining class has a compiled `{Class}_{method}` symbol
    // (`d.age` on a `Dog` that never declares `age` must call
    // `Animal_age`; `Dog_age` was never generated).
    let defining_class = ctx
      .method_owners
      .get(class_name.as_str())
      .and_then(|owners| owners.get(method))
      .ok_or_else(|| format!("codegen: unsupported method call `{class_name}.{method}`"))?;
    let key = format!("{defining_class}_{}", mangled_operator_symbol(method));
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
  if ret_kind == ValKind::Void {
    // `[]=`-style Void-returning operator methods are invoked from
    // `build_set_index` purely for their side effect (see
    // `build_call_expr`'s identical guard for the free-function case).
    return Ok((context.i64_type().const_int(0, false).into(), ret_kind));
  }
  Ok((call_result(call)?, ret_kind))
}

/// `obj&.method(args)` (plan 43's Decision log) — reuses `build_short_
/// circuit`'s own is-null-guarded-basic-blocks-plus-PHI pattern
/// wholesale: a real `is null` test on the receiver, `build_method_
/// call` invoked only on the non-null path (never on a null pointer),
/// merging both paths into one well-typed result via a real LLVM
/// `phi` — `null` on the nil path, the method's own return value on
/// the other. sema already restricts this to a class-typed nullable
/// receiver whose dispatched method's return type is itself pointer-
/// representable (`Class`/`String`/`Array`/`Hash`, all `ValKind::Ptr`/
/// `Str`, both backed by a real LLVM `ptr`), so the `phi`'s type is
/// always a bare `ptr` — codegen trusts that invariant, per every
/// prior plan's "codegen runs on already-checked input" contract.
#[allow(clippy::too_many_arguments)]
fn build_safe_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  method: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  if !matches!(&recv.node, Expr::Ident(_)) {
    return Err("codegen: `&.` is only supported on a plain local-variable receiver".to_string());
  }
  let (recv_val, recv_kind) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if recv_kind != ValKind::Ptr {
    return Err(format!(
      "codegen: `&.` requires a pointer-backed receiver, found {recv_kind:?}"
    ));
  }
  let is_null = builder
    .build_is_null(recv_val.into_pointer_value(), "isnil")
    .map_err(|e| e.to_string())?;

  let entry_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block")?;
  let func = entry_block
    .get_parent()
    .ok_or("codegen: internal error — block has no parent function")?;
  let call_block = context.append_basic_block(func, "safecall.call");
  let merge_block = context.append_basic_block(func, "safecall.merge");

  builder
    .build_conditional_branch(is_null, merge_block, call_block)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(call_block);
  let (call_val, call_kind) = build_method_call(
    context,
    builder,
    recv,
    method,
    args,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let call_end_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block after call")?;
  builder
    .build_unconditional_branch(merge_block)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(merge_block);
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let phi = builder
    .build_phi(ptr_ty, "safecallresult")
    .map_err(|e| e.to_string())?;
  let null_val = ptr_ty.const_null();
  phi.add_incoming(&[
    (&null_val, entry_block),
    (&call_val.into_pointer_value(), call_end_block),
  ]);
  Ok((phi.as_basic_value(), call_kind))
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
  elements: &[Spanned<Expr>],
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

/// Plan 39's Decision log: `Expr::Call`'s own positional argument-value
/// builder — fills any missing trailing arguments from the callee's
/// declared defaults, and — when the callee declares a splat parameter
/// — packs every argument beyond its ordinary parameter count into a
/// freshly allocated `Array[Elem]` (reusing `build_array_lit`'s own
/// alloc-and-store loop wholesale), appended as one final argument. The
/// compiled callee itself is always fixed-arity — this is purely a
/// call-site concern.
#[allow(clippy::too_many_arguments)]
fn build_call_arg_vals<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  name: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<Vec<BasicMetadataValueEnum<'ctx>>, String> {
  let f = ctx
    .func_defs
    .get(name)
    .ok_or_else(|| format!("codegen: internal error — `{name}` has no known declaration"))?;
  let ordinary_count = f.params.len();
  let mut arg_vals = Vec::with_capacity(ordinary_count + usize::from(f.splat_param.is_some()));
  for (i, param) in f.params.iter().enumerate() {
    let expr = if i < args.len() {
      &args[i]
    } else {
      param.default.as_ref().ok_or_else(|| {
        format!(
          "codegen: internal error — `{name}` is missing required argument `{}` (sema should have rejected this)",
          param.name
        )
      })?
    };
    let (v, _) = build_expr(
      context,
      builder,
      expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    arg_vals.push(v.into());
  }
  if f.splat_param.is_some() {
    let trailing: &[Spanned<Expr>] = if args.len() > ordinary_count {
      &args[ordinary_count..]
    } else {
      &[]
    };
    let packed = build_array_lit(
      context,
      builder,
      trailing,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    arg_vals.push(packed.into());
  }
  Ok(arg_vals)
}

/// Plan 39: builds a plain positional call — shared by `Expr::Call`
/// (which rejects a `Void` result, since it's being used as a value)
/// and `Stmt::Expr(Expr::Call(...))`'s own bare-statement dispatch
/// (which must NOT reject `Void`, the overwhelmingly common case for a
/// statement-position call). Deliberately does not itself decide
/// whether `Void` is acceptable — that's each caller's own call.
#[allow(clippy::too_many_arguments)]
fn build_call_expr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  name: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  // Plan 41's Decision log: a generic function's bare name is never
  // registered in `user_func_ids` at all — its call sites resolve their
  // own concrete argument type the same way `collect_generic_
  // specializations` did (a plain local via `local_classes`, or a
  // direct `ClassName.new(...)` literal), form the matching mangled
  // symbol, and emit a direct call against it, identical call-emission
  // code to every non-generic call below.
  if let Some(g) = ctx
    .func_defs
    .get(name)
    .filter(|f| !f.type_params.is_empty())
  {
    let mangled = mangled_generic_call_symbol(name, g, args, local_classes)?;
    let (fv, ret_kind) = *ctx.user_func_ids.get(&mangled).ok_or_else(|| {
      format!("codegen: no compiled specialization `{mangled}` for generic function `{name}`")
    })?;
    let arg_vals = build_call_arg_vals(
      context,
      builder,
      name,
      args,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let call = builder
      .build_call(fv, &arg_vals, "calltmp")
      .map_err(|e| e.to_string())?;
    if ret_kind == ValKind::Void {
      return Ok((context.i64_type().const_int(0, false).into(), ret_kind));
    }
    return Ok((call_result(call)?, ret_kind));
  }
  let (fv, ret_kind) = *ctx.user_func_ids.get(name).ok_or_else(|| {
    format!("codegen: unsupported call to `{name}` (not a compiled user function)")
  })?;
  let arg_vals = build_call_arg_vals(
    context,
    builder,
    name,
    args,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let call = builder
    .build_call(fv, &arg_vals, "calltmp")
    .map_err(|e| e.to_string())?;
  if ret_kind == ValKind::Void {
    // No value to extract — the caller (a bare-statement dispatch) is
    // only here for the call's side effects.
    return Ok((context.i64_type().const_int(0, false).into(), ret_kind));
  }
  let result = call_result(call)?;
  Ok((result, ret_kind))
}

/// Plan 39: `Expr::CallKw`'s own call builder — see `build_call_expr`'s
/// doc comment for why this doesn't reject `Void` itself either.
#[allow(clippy::too_many_arguments)]
fn build_call_kw_expr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  name: &str,
  kwargs: &[(String, Spanned<Expr>)],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (fv, ret_kind) = *ctx.user_func_ids.get(name).ok_or_else(|| {
    format!("codegen: unsupported call to `{name}` (not a compiled user function)")
  })?;
  let f = ctx
    .func_defs
    .get(name)
    .ok_or_else(|| format!("codegen: internal error — `{name}` has no known declaration"))?;
  let mut resolved: Vec<Option<&Spanned<Expr>>> = vec![None; f.params.len()];
  for (kw_name, kw_value) in kwargs {
    let pos = f
      .params
      .iter()
      .position(|p| &p.name == kw_name)
      .ok_or_else(|| {
        format!(
          "codegen: internal error — unrecognized keyword `{kw_name}` for `{name}` (sema should have rejected this)"
        )
      })?;
    resolved[pos] = Some(kw_value);
  }
  let mut arg_vals = Vec::with_capacity(resolved.len());
  for (i, slot) in resolved.iter().enumerate() {
    let expr = match slot {
      Some(e) => *e,
      None => f.params[i].default.as_ref().ok_or_else(|| {
        format!(
          "codegen: internal error — missing required keyword `{}` for `{name}` (sema should have rejected this)",
          f.params[i].name
        )
      })?,
    };
    let (v, _) = build_expr(
      context,
      builder,
      expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    arg_vals.push(v.into());
  }
  let call = builder
    .build_call(fv, &arg_vals, "callkwtmp")
    .map_err(|e| e.to_string())?;
  if ret_kind == ValKind::Void {
    return Ok((context.i64_type().const_int(0, false).into(), ret_kind));
  }
  let result = call_result(call)?;
  Ok((result, ret_kind))
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
  array: &Spanned<Expr>,
  index: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let Expr::Ident(arr_name) = &array.node else {
    return Err(
      "codegen: indexing is only supported on a plain local-variable receiver".to_string(),
    );
  };
  // Plan 40's Decision log: checked *before* `array`/`index` are built
  // — `local_classes` holds both real class names and `"Hash[K, V]"`
  // strings (plan 25's Decision log), so a genuine class name is one
  // `parse_hash_type` can't parse. Delegating this early avoids
  // double-evaluating `index` (which `build_method_call` independently
  // builds as its own argument).
  if let Some(class_name) = local_classes.get(arr_name) {
    if parse_hash_type(class_name).is_none() {
      return build_method_call(
        context,
        builder,
        array,
        "[]",
        std::slice::from_ref(index),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      );
    }
  }
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
  // Plan 45's Decision log: `str[i]` — a real one-character `String`
  // (`emerald_string_char_at`), not an `Int64` byte value. Checked
  // after `local_array_elem_types`/the Hash branch, alongside the
  // existing `vars`-`ValKind` check `build_method_call` already uses
  // for its own String dispatch.
  if let Some((_, ValKind::Str)) = vars.get(arr_name) {
    if idx_kind != ValKind::Int64 {
      return Err("codegen: String index must be Int64".to_string());
    }
    let call = builder
      .build_call(ctx.string_char_at, &[base.into(), idx.into()], "charattmp")
      .map_err(|e| e.to_string())?;
    return Ok((call_result(call)?, ValKind::Str));
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
  array: &Spanned<Expr>,
  index: &Spanned<Expr>,
  value: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let Expr::Ident(arr_name) = &array.node else {
    return Err(
      "codegen: indexed assignment is only supported on a plain local-variable receiver"
        .to_string(),
    );
  };
  // Plan 40's Decision log: see `build_index`'s identical early check
  // — this mirrors it for the write side, delegating to a `"[]="`
  // method with `[index, value]` as its two arguments. `build_method_
  // call` needs a real contiguous `&[Expr]` slice, so this clones the
  // two (non-adjacent in the original `Stmt::SetIndex`) expressions
  // into one owned, temporary `Vec` — a small, real cost, not a hack.
  if let Some(class_name) = local_classes.get(arr_name) {
    if parse_hash_type(class_name).is_none() {
      let call_args = vec![index.clone(), value.clone()];
      build_method_call(
        context,
        builder,
        array,
        "[]=",
        &call_args,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      return Ok(false);
    }
  }
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
  arg: &Spanned<Expr>,
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

/// `for var in [e1, e2, ...] body end` (plan 30's Decision log):
/// desugars to the same index-based `while` shape plan 09's array
/// traversal already proved — a fresh hidden `Int64` index counter, a
/// `for.cond`/`for.body`/`for.incr`/`for.after` block shape (four, not
/// three like `while`, because `next` must resume at the increment
/// step, not the condition check — otherwise a `next` would skip the
/// index bump entirely and infinite-loop), and a per-iteration
/// `Expr::Index`-shaped load bound to `var`.
#[allow(clippy::too_many_arguments)]
fn build_for<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  var: &str,
  elements: &[Spanned<Expr>],
  body: &'a [Spanned<Stmt>],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  // Built directly (not via `build_array_lit`) so the first element's
  // real `ValKind` is captured in the same pass — calling `build_expr`
  // on it a second time to discover the kind would double any side
  // effects a non-literal element expr has.
  let elem_count = elements.len() as u64;
  let size_val = context.i64_type().const_int(elem_count * 8, false);
  let call = builder
    .build_call(ctx.alloc, &[size_val.into()], "forarralloc")
    .map_err(|e| e.to_string())?;
  let arr_ptr = call_result(call)?.into_pointer_value();
  let mut elem_kind = ValKind::Int64;
  for (i, e) in elements.iter().enumerate() {
    let (v, k) = build_expr(
      context,
      builder,
      e,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    if i == 0 {
      elem_kind = k;
    }
    let elem_ptr = field_ptr(context, builder, arr_ptr, i as u64 * 8)?;
    builder
      .build_store(elem_ptr, v)
      .map_err(|e| e.to_string())?;
  }

  let idx_alloca = builder
    .build_alloca(context.i64_type(), "for.idx")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, context.i64_type().const_int(0, false))
    .map_err(|e| e.to_string())?;
  let var_alloca = builder
    .build_alloca(local_llvm_type(context, elem_kind), var)
    .map_err(|e| e.to_string())?;
  vars.insert(var.to_string(), (var_alloca, elem_kind));

  let cond_blk = context.append_basic_block(func, "for.cond");
  let body_blk = context.append_basic_block(func, "for.body");
  let incr_blk = context.append_basic_block(func, "for.incr");
  let exit_blk = context.append_basic_block(func, "for.after");

  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(cond_blk);
  let idx_val = builder
    .build_load(context.i64_type(), idx_alloca, "for.idx.val")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let len_val = context.i64_type().const_int(elem_count, false);
  let cond_val = builder
    .build_int_compare(IntPredicate::SLT, idx_val, len_val, "for.cond.cmp")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(cond_val, body_blk, exit_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(body_blk);
  let elem_llvm_ty = local_llvm_type(context, elem_kind);
  let elem_ptr = unsafe {
    builder
      .build_in_bounds_gep(elem_llvm_ty, arr_ptr, &[idx_val], "for.elemptr")
      .map_err(|e| e.to_string())?
  };
  let elem_val = builder
    .build_load(elem_llvm_ty, elem_ptr, "for.elem")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(var_alloca, elem_val)
    .map_err(|e| e.to_string())?;

  loop_stack.push(LoopTargets {
    header: incr_blk,
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
    ensure_stack,
    retry_stack,
    ret_kind,
    ctx,
  )?;
  loop_stack.pop();
  if !body_terminated {
    builder
      .build_unconditional_branch(incr_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(incr_blk);
  let idx_val = builder
    .build_load(context.i64_type(), idx_alloca, "for.idx.val")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let next_idx = builder
    .build_int_add(
      idx_val,
      context.i64_type().const_int(1, false),
      "for.idx.next",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, next_idx)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(exit_blk);
  Ok(false)
}

/// `for var in start..end body end` (`exclusive: false`) or
/// `start...end` (`exclusive: true`) — plan 37's Decision log: reuses
/// `build_for`'s exact five-block skeleton (`for.cond`/`for.body`/
/// `for.incr`/`for.after`, `LoopTargets` push/pop around `build_block`)
/// verbatim, with no array materialization and no per-iteration
/// GEP/load — a Range's "elements" *are* the loop index itself.
/// Unlike `build_for`, `var`'s `alloca` is NOT built here:
/// `collect_lets`/`prealloc_lets` already hoisted it (its `ValKind` is
/// statically `Int64` — see the Decision log), so this function looks
/// it up from `vars` instead of inserting a fresh one. `start`/`end`
/// are each evaluated exactly once, before the loop begins.
#[allow(clippy::too_many_arguments)]
fn build_for_range<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  var: &str,
  start: &Spanned<Expr>,
  end: &Spanned<Expr>,
  exclusive: bool,
  body: &'a [Spanned<Stmt>],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  let (start_val, start_kind) = build_expr(
    context,
    builder,
    start,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if start_kind != ValKind::Int64 {
    return Err(format!(
      "codegen: internal error — sema should have rejected a non-Int64 range start, found {start_kind:?}"
    ));
  }
  let (end_val, end_kind) = build_expr(
    context,
    builder,
    end,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  if end_kind != ValKind::Int64 {
    return Err(format!(
      "codegen: internal error — sema should have rejected a non-Int64 range end, found {end_kind:?}"
    ));
  }

  let idx_alloca = builder
    .build_alloca(context.i64_type(), "forrange.idx")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, start_val.into_int_value())
    .map_err(|e| e.to_string())?;

  let (var_alloca, _) = *vars
    .get(var)
    .expect("pre-allocated by prealloc_lets for every Stmt::ForRange loop var");

  let cond_blk = context.append_basic_block(func, "forrange.cond");
  let body_blk = context.append_basic_block(func, "forrange.body");
  let incr_blk = context.append_basic_block(func, "forrange.incr");
  let exit_blk = context.append_basic_block(func, "forrange.after");

  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(cond_blk);
  let idx_val = builder
    .build_load(context.i64_type(), idx_alloca, "forrange.idx.val")
    .map_err(|e| e.to_string())?
    .into_int_value();
  // `..` (inclusive, `exclusive: false`) keeps iterating through
  // `idx == end` (`SLE`); `...` (`exclusive: true`) stops one short of
  // it (`SLT`) — the real distinguishing bit this plan's worked example
  // (15 vs. 10) proves.
  let predicate = if exclusive {
    IntPredicate::SLT
  } else {
    IntPredicate::SLE
  };
  let cond_val = builder
    .build_int_compare(
      predicate,
      idx_val,
      end_val.into_int_value(),
      "forrange.cond.cmp",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(cond_val, body_blk, exit_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(body_blk);
  builder
    .build_store(var_alloca, idx_val)
    .map_err(|e| e.to_string())?;

  loop_stack.push(LoopTargets {
    header: incr_blk,
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
    ensure_stack,
    retry_stack,
    ret_kind,
    ctx,
  )?;
  loop_stack.pop();
  if !body_terminated {
    builder
      .build_unconditional_branch(incr_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(incr_blk);
  let idx_val = builder
    .build_load(context.i64_type(), idx_alloca, "forrange.idx.val")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let next_idx = builder
    .build_int_add(
      idx_val,
      context.i64_type().const_int(1, false),
      "forrange.idx.next",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, next_idx)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(exit_blk);
  Ok(false)
}

/// `name(args) { |params| body }` where `name` declares `block_param`
/// (plan 34's Decision log) — call-site specialization, not an ordinary
/// call: this compiles a fresh copy of `callee`'s body inline, directly
/// into the CALLER's current function, right here. There is no
/// first-class `Proc` value or indirect call in this backend (plan
/// 10's own constraint), so `yield` can only ever be lowered against
/// one specific, statically-known block literal — the one attached at
/// this exact call site. Reusing the same `vars`/`loop_stack` for the
/// inlined body is also what makes a block's free-variable capture
/// "just work" with zero closure-environment machinery: it's the same
/// stack frame, so a variable the block reads or the callee's own
/// locals are already in scope.
///
/// A real, disclosed limitation: `callee.body` is inlined verbatim, so
/// a `return` inside it would terminate the *caller's* function, not
/// just `callee`'s own invocation — this plan's worked example (and
/// every function callable this way) is `Void`-returning with no
/// `return` statement, so this is never exercised, but a `block_param`
/// function containing `return` would misbehave. Fixing that needs
/// either forbidding `return` in a `block_param` function's body
/// (sema's job) or a real synthesized-callee-as-its-own-function
/// design (this plan's own Decision log's heavier alternative) —
/// neither is attempted here.
#[allow(clippy::too_many_arguments)]
fn build_inline_block_call<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  callee: &'a AstFunction,
  args: &'a [Spanned<Expr>],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  let Some(Spanned {
    node: Expr::Lambda {
      params: blk_params,
      body: blk_body,
      ..
    },
    ..
  }) = args.last()
  else {
    return Err(format!(
      "codegen: `{}` requires a trailing block literal",
      callee.name
    ));
  };
  let positional_args = &args[..args.len() - 1];
  if positional_args.len() != callee.params.len() {
    return Err(format!(
      "codegen: `{}` expects {} argument(s), found {}",
      callee.name,
      callee.params.len(),
      positional_args.len()
    ));
  }

  // Evaluate the ordinary positional args in the CALLER's current scope
  // before binding anything under the callee's own param names.
  let mut evaluated = Vec::with_capacity(positional_args.len());
  for a in positional_args {
    let (v, k) = build_expr(
      context,
      builder,
      a,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    evaluated.push((v, k));
  }
  for (p, (v, k)) in callee.params.iter().zip(evaluated) {
    let alloca = builder
      .build_alloca(local_llvm_type(context, k), &p.name)
      .map_err(|e| e.to_string())?;
    builder.build_store(alloca, v).map_err(|e| e.to_string())?;
    vars.insert(p.name.clone(), (alloca, k));
  }

  // Pre-allocate the block's own param slots — every `yield` site
  // inside `callee`'s body stores its args here before the block body
  // (inlined fresh at each `yield`, since it can run any number of
  // times — e.g. once per loop iteration) reads them back out.
  for p in blk_params {
    let kind = value_kind_for_type(&p.ty);
    let alloca = builder
      .build_alloca(local_llvm_type(context, kind), &p.name)
      .map_err(|e| e.to_string())?;
    vars.insert(p.name.clone(), (alloca, kind));
  }

  // `callee.body` (and `blk_body`, in case the block itself declares a
  // fresh local) are being inlined straight into the CALLER's own
  // function — their own `Let`s need the exact same `collect_lets`/
  // `prealloc_lets` pre-pass an ordinary function body already gets in
  // `define_user_function`/`define_method`, or `Stmt::Let`'s codegen
  // (which expects every reachable `Let` to already be pre-allocated)
  // panics.
  let mut decls = Vec::new();
  collect_lets(&callee.body, &mut decls);
  collect_lets(blk_body, &mut decls);
  prealloc_lets(context, builder, &decls, vars)?;

  let inline_ctx = Ctx {
    yield_target: Some((blk_params.as_slice(), blk_body.as_slice())),
    ..*ctx
  };
  build_block(
    context,
    builder,
    func,
    &callee.body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ensure_stack,
    retry_stack,
    ret_kind,
    &inline_ctx,
  )
}

/// Plan 38: duplicate-emits every currently-active `begin`'s `ensure`
/// body, innermost first — called by every real exit out of a guarded
/// region (`Return`/`Break`/`Next`/`Raise`) right before that
/// statement's own terminator, since this backend has no
/// `invoke`/`landingpad`/unwind-table mechanism to attach a single
/// shared cleanup label to (Decision log). A snapshot of the current
/// `ensure_stack` is taken first (just copying the `&[Stmt]` pointers,
/// not the bodies themselves) so each duplicate-emit call can still
/// legitimately re-borrow `ensure_stack` mutably (needed because a
/// nested `begin` inside an ensure body is, in principle, still
/// well-formed code).
///
/// `ensure_stack`'s entries each carry a `raise_visible: bool` tag:
/// `true` only for an entry pushed while compiling a `rescue` clause's
/// own body (where a `Stmt::Raise` is a genuine re-raise, leaving this
/// `begin`), `false` while compiling its try `body` (where a
/// `Stmt::Raise` targets this SAME `begin`'s own handler and stays
/// within it — `build_begin`'s own 3 exit points already duplicate
/// this `begin`'s `ensure` correctly on every path a raise there can
/// actually take, so `Stmt::Raise` must not double it, hence
/// `raise_only` below). `Stmt::Return`/`Break`/`Next` ignore the tag
/// and always see every entry — those are unconditional jumps that
/// bypass the `setjmp`/`longjmp` mechanism entirely, genuinely leaving
/// every enclosing `begin` regardless of which region they're
/// lexically inside.
#[allow(clippy::too_many_arguments)]
fn emit_active_ensures<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
  // `true` only for `Stmt::Raise` — skips every entry whose own
  // `raise_visible` tag is `false` (see `ensure_stack`'s own doc
  // comment above). `false` for `Return`/`Break`/`Next`, which see
  // every entry regardless of its tag.
  raise_only: bool,
) -> Result<(), String> {
  let ensure_bodies: Vec<&'a [Spanned<Stmt>]> = ensure_stack
    .iter()
    .rev()
    .filter(|(_, raise_visible)| !raise_only || *raise_visible)
    .map(|(body, _)| *body)
    .collect();
  for ensure_body in ensure_bodies {
    build_block(
      context,
      builder,
      func,
      ensure_body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    )?;
  }
  Ok(())
}

/// Emits one statement. Returns `true` if the statement emitted a
/// block terminator (`return`/the loop-jump for `break`/`next`/
/// `raise`'s unreachable) — callers must not emit further instructions
/// into the current block afterward.
#[allow(clippy::too_many_arguments)]
fn build_stmt<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  stmt: &'a Spanned<Stmt>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Plan 38: the active `begin`'s `ensure` body slice(s), innermost
  // last — `Return`/`Break`/`Next`/`Raise` duplicate-emit every entry
  // here (innermost-first) before their own terminator, since this
  // backend has no `invoke`/`landingpad`/unwind-table mechanism to
  // attach a single shared cleanup label to (see plan 38's Decision
  // log). `retry` deliberately never consults this.
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  // Plan 38: the active `begin`'s `begin.retry` block, innermost last
  // — `Stmt::Retry` branches to `retry_stack.last()`.
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  match &stmt.node {
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::Lambda { .. },
        ..
      },
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
      value: Spanned {
        node: Expr::ArrayNew(size),
        ..
      },
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
      // Plan 43's Decision log: a `Greeter?`-typed local's storage is a
      // `ptr` slot (`value_kind_for_type` falls through any non-
      // primitive-named string, including `"Greeter?"`, to `ValKind::
      // Ptr`) — `Expr::Nil`'s generic `build_expr` arm unconditionally
      // emits a fixed `i64` `0` (plan 25's design), a real LLVM type
      // mismatch when stored into that slot. A literal `nil` value into
      // a pointer-backed declared type builds a real null pointer
      // constant directly instead.
      let expected_kind = value_kind_for_type(ty);
      let v = if matches!(value.node, Expr::Nil)
        && matches!(expected_kind, ValKind::Ptr | ValKind::Str)
      {
        context
          .ptr_type(AddressSpace::default())
          .const_null()
          .into()
      } else {
        let (v, _) = build_expr(
          context,
          builder,
          value,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        v
      };
      // Plan 43's Decision log: `local_classes` only ever needs a bare
      // class name, independent of nullability — strip a trailing `?`
      // before the lookup so a `Greeter?` local is still recorded as
      // class `Greeter`, exactly what `&.`'s dispatch (`build_method_
      // call`, reused by `build_safe_call`) needs to find.
      let bare_ty = ty.strip_suffix('?').unwrap_or(ty);
      if ctx.classes.contains_key(bare_ty) {
        local_classes.insert(name.clone(), bare_ty.to_string());
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
    // Plan 31: reuses the exact same existing-alloca/`build_store`
    // mechanism `Stmt::Let` above already uses for a loop counter's
    // repeated reassignment — `name` is never pre-allocated via
    // `collect_lets` for this variant (it's always an already-declared
    // outer name), so a genuinely-undefined target is a real `Err`
    // here, not a panic, unlike `Let`'s `.expect(...)` (sema guarantees
    // it in the normal pipeline, but codegen alone shouldn't assume
    // that).
    Stmt::Assign { name, value } => {
      let (ptr, target_kind) = *vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      // Plan 43's Decision log: same `Expr::Nil`-into-`ptr`-slot special
      // case as `Stmt::Let` above, driven by the target's already-
      // recorded `ValKind` in `vars` instead of a declared-type string.
      let v =
        if matches!(value.node, Expr::Nil) && matches!(target_kind, ValKind::Ptr | ValKind::Str) {
          context
            .ptr_type(AddressSpace::default())
            .const_null()
            .into()
        } else {
          let (v, _) = build_expr(
            context,
            builder,
            value,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?;
          v
        };
      builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 43's Decision log: a real is-nil-guarded conditional store —
    // two basic blocks plus a merge, no `phi` needed since (unlike
    // `build_safe_call`) this statement produces no value at all.
    // Assigns `default` only when `name`'s CURRENT value is nil.
    Stmt::OrAssign { name, default } => {
      let (ptr, kind) = *vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let current = builder
        .build_load(local_llvm_type(context, kind), ptr, name)
        .map_err(|e| e.to_string())?;
      let is_null = builder
        .build_is_null(current.into_pointer_value(), "orassign.isnil")
        .map_err(|e| e.to_string())?;
      let assign_block = context.append_basic_block(func, "orassign.assign");
      let merge_block = context.append_basic_block(func, "orassign.merge");
      builder
        .build_conditional_branch(is_null, assign_block, merge_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(assign_block);
      let v = if matches!(default.node, Expr::Nil) && matches!(kind, ValKind::Ptr | ValKind::Str) {
        context
          .ptr_type(AddressSpace::default())
          .const_null()
          .into()
      } else {
        let (v, _) = build_expr(
          context,
          builder,
          default,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        v
      };
      builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      builder
        .build_unconditional_branch(merge_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(merge_block);
      Ok(false)
    }
    // Plan 43's Decision log: the asymmetric twin of `OrAssign` above —
    // assigns `value` only when `name`'s CURRENT value is non-nil.
    Stmt::AndAssign { name, value } => {
      let (ptr, kind) = *vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let current = builder
        .build_load(local_llvm_type(context, kind), ptr, name)
        .map_err(|e| e.to_string())?;
      let is_null = builder
        .build_is_null(current.into_pointer_value(), "andassign.isnil")
        .map_err(|e| e.to_string())?;
      let assign_block = context.append_basic_block(func, "andassign.assign");
      let merge_block = context.append_basic_block(func, "andassign.merge");
      builder
        .build_conditional_branch(is_null, merge_block, assign_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(assign_block);
      let v = if matches!(value.node, Expr::Nil) && matches!(kind, ValKind::Ptr | ValKind::Str) {
        context
          .ptr_type(AddressSpace::default())
          .const_null()
          .into()
      } else {
        let (v, _) = build_expr(
          context,
          builder,
          value,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        v
      };
      builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      builder
        .build_unconditional_branch(merge_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(merge_block);
      Ok(false)
    }
    // Plan 31: every `values` expression is built into a temporary SSA
    // value BEFORE any `names` target is written — the entire point of
    // this leaf (Decision log): `a, b = b, a` must read both original
    // values before either alloca is overwritten, or the swap silently
    // corrupts (`a = b` then `b = a` would print the new `a` twice).
    Stmt::MultiAssign { names, values } => {
      let mut evaluated = Vec::with_capacity(values.len());
      for (name, v) in names.iter().zip(values) {
        // Plan 43's Decision log: same `Expr::Nil`-into-`ptr`-slot
        // special case, driven by each *target's* own already-recorded
        // `ValKind` — peeking at it here is a read, not a write, so it
        // doesn't disturb this statement's own "evaluate every value
        // before writing any target" ordering (the Decision log's own
        // reason `a, b = b, a` is a real swap).
        let (_, target_kind) = *vars
          .get(name)
          .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
        let val =
          if matches!(v.node, Expr::Nil) && matches!(target_kind, ValKind::Ptr | ValKind::Str) {
            context
              .ptr_type(AddressSpace::default())
              .const_null()
              .into()
          } else {
            let (val, _) = build_expr(
              context,
              builder,
              v,
              vars,
              local_classes,
              local_array_elem_types,
              ctx,
            )?;
            val
          };
        evaluated.push(val);
      }
      for (name, val) in names.iter().zip(evaluated) {
        let (ptr, _) = *vars
          .get(name)
          .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
        builder.build_store(ptr, val).map_err(|e| e.to_string())?;
      }
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
    // Plan 34: a call to a `block_param`-declaring function is never an
    // ordinary LLVM `call` — the callee was never declared as one (see
    // `declare_user_functions`). It's compiled fresh, inline, at this
    // one call site instead — see `build_inline_block_call`.
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) if ctx.block_funcs.contains_key(name.as_str()) => {
      let callee = ctx.block_funcs[name.as_str()];
      build_inline_block_call(
        context,
        builder,
        func,
        callee,
        args,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
      )
    }
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) if name == "puts" && args.len() == 1 => {
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
    // Plan 45: a bare-statement `gets()` (its return value discarded) —
    // `gets` is never in `ctx.user_func_ids`, so it could never reach
    // the generic `Expr::Call` arm below successfully; mirrors
    // `build_expr`'s own `gets` guard.
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) if name == "gets" => {
      if !args.is_empty() {
        return Err(format!(
          "codegen: `gets` expects 0 arguments, found {}",
          args.len()
        ));
      }
      builder
        .build_call(ctx.gets, &[], "getstmp")
        .map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 39: a bare-statement call to a `Void`-returning function
    // (`greet(name: "yo")` with no `puts`/assignment around it) must
    // NOT hit `Expr::Call`/`Expr::CallKw`'s own `Void`-rejection —
    // `build_expr` is a value-producing context, and this one isn't.
    // `build_call_expr`/`build_call_kw_expr` (unlike `build_expr`'s own
    // arms) never reject `Void`, exactly because they're shared with
    // this statement-position dispatch.
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) => {
      build_call_expr(
        context,
        builder,
        name,
        args,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok(false)
    }
    Stmt::Expr(Spanned {
      node: Expr::CallKw(name, kwargs),
      ..
    }) => {
      build_call_kw_expr(
        context,
        builder,
        name,
        kwargs,
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
      // Plan 43's Decision log: same `Expr::Nil`-into-`ptr`-slot special
      // case, driven by `ret_kind` (already a `build_stmt` parameter —
      // the enclosing function/method's own declared return kind).
      let v = if matches!(e.node, Expr::Nil) && matches!(ret_kind, ValKind::Ptr | ValKind::Str) {
        context
          .ptr_type(AddressSpace::default())
          .const_null()
          .into()
      } else {
        let (v, _) = build_expr(
          context,
          builder,
          e,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        v
      };
      emit_active_ensures(
        context,
        builder,
        func,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
        false,
      )?;
      builder.build_return(Some(&v)).map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Return(None) => {
      emit_active_ensures(
        context,
        builder,
        func,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
        false,
      )?;
      builder.build_return(None).map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Break => {
      let target = *loop_stack
        .last()
        .ok_or("codegen: `break` outside of a loop")?;
      emit_active_ensures(
        context,
        builder,
        func,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
        false,
      )?;
      builder
        .build_unconditional_branch(target.exit)
        .map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Next => {
      let target = *loop_stack
        .last()
        .ok_or("codegen: `next` outside of a loop")?;
      emit_active_ensures(
        context,
        builder,
        func,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
        false,
      )?;
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
        ensure_stack,
        retry_stack,
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
          ensure_stack,
          retry_stack,
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
        ensure_stack,
        retry_stack,
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
    // Plan 30: desugars to the same index-based `while` shape plan 09's
    // array traversal already proved. See `build_for`.
    Stmt::For {
      var,
      elements,
      body,
    } => build_for(
      context,
      builder,
      func,
      var,
      elements,
      body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    ),
    // Plan 37: no array materialization, no per-iteration GEP/load — a
    // Range's "elements" are the loop index itself. See `build_for_range`.
    Stmt::ForRange {
      var,
      start,
      end,
      exclusive,
      body,
    } => build_for_range(
      context,
      builder,
      func,
      var,
      start,
      end,
      *exclusive,
      body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    ),
    // Plan 38: a `raise` inside a TRY body targets this SAME `begin`'s
    // own handler and stays within it — `build_begin`'s own 3 exit
    // points already duplicate this `begin`'s `ensure` correctly on
    // every path such a raise can actually take, so this must not
    // double it (`raise_only: true` skips those entries). A `raise`
    // re-raising from inside a `rescue_body`, though, genuinely does
    // leave this `begin` (Decision log) — its own `ensure` entry is
    // tagged `raise_visible` for exactly that reason.
    Stmt::Raise(e) => {
      emit_active_ensures(
        context,
        builder,
        func,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
        true,
      )?;
      build_raise(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => build_begin(
      context,
      builder,
      func,
      body,
      rescues,
      ensure,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
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
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    ),
    // Plan 34: `ctx.yield_target` is `Some((block's params, block's
    // body))` exactly while this statement is reached via `build_
    // inline_block_call`'s own inline expansion of a `block_param`-
    // declaring callee's body (sema already guarantees `yield` never
    // appears anywhere else) — store `args` into the block's own
    // pre-allocated param slots, then build the block's body inline
    // right here, reusing the exact same `vars`/`loop_stack`, so a
    // block genuinely closes over the call site's surrounding scope
    // the same way a lambda literal already does.
    Stmt::Yield(args) => {
      let Some((blk_params, blk_body)) = ctx.yield_target else {
        return Err("codegen: `yield` reached codegen with no attached block".to_string());
      };
      if args.len() != blk_params.len() {
        return Err(format!(
          "codegen: `yield` passes {} argument(s), attached block declares {} parameter(s)",
          args.len(),
          blk_params.len()
        ));
      }
      for (a, p) in args.iter().zip(blk_params) {
        let (v, _) = build_expr(
          context,
          builder,
          a,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        let (ptr, _) = *vars
          .get(&p.name)
          .ok_or_else(|| format!("codegen: undefined variable `{}`", p.name))?;
        builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      }
      // A block's own body is never itself a `yield`-legal context —
      // sema already enforces this, but codegen clears the target
      // defensively too, matching the "not a panic" standard.
      let block_ctx = Ctx {
        yield_target: None,
        ..*ctx
      };
      build_block(
        context,
        builder,
        func,
        blk_body,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        &block_ctx,
      )?;
      Ok(false)
    }
    // Plan 38: branches straight to the innermost active `begin`'s
    // `begin.retry` block (pushed by `build_begin` for the duration of
    // compiling each of its `rescue` clauses) — a real `Err`, not a
    // panic, if reached with no active `begin` (defends a sema-
    // bypassing direct codegen call, same as every other function
    // here). Deliberately does NOT consult `ensure_stack` (Decision
    // log — `retry` doesn't exit the `begin` construct at all).
    Stmt::Retry => {
      let target = retry_stack
        .last()
        .ok_or("codegen: `retry` outside of a rescue body")?;
      builder
        .build_unconditional_branch(*target)
        .map_err(|e| e.to_string())?;
      Ok(true)
    }
  }
}

/// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan 20):
/// lowers to the same `icmp(Equal)` `Expr::Compare`'s `Eq` arm already
/// emits, chained across arms in source order (first matching arm
/// wins) — a multi-value arm's values are OR-ed together (bitwise
/// `or` over `i1`s, equivalent to logical or), falling through to
/// `else_body` (or nothing) if no arm matches.
#[allow(clippy::too_many_arguments)]
fn build_case<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  scrutinee: &Spanned<Expr>,
  arms: &'a [CaseArm],
  else_body: &'a Option<Vec<Spanned<Stmt>>>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
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
      ensure_stack,
      retry_stack,
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
      ensure_stack,
      retry_stack,
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
  cond: &Spanned<Expr>,
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
  e: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<bool, String> {
  let Expr::New(class_name, _) = &e.node else {
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

/// `begin body rescue T1 => e1 ... [rescue => eN ...] [ensure ...] end`
/// (plan 38's Decision log). Wraps the handler-push+`setjmp` sequence
/// in a dedicated `begin.retry` block, pushed onto `retry_stack` for
/// the duration of compiling every `rescue` clause's body, so `retry`
/// has something to re-attempt from. Chains `rescues` via the same
/// `arm_blk`/`next_check_blk` idiom `build_case` already uses for its
/// `when` arms (source order; subtype-aware — see
/// `Ctx::rescue_tag_sets`); a bare clause matches unconditionally,
/// binding nothing. `ensure`'s body (possibly empty — `build_block` on
/// `&[]` is a real no-op) is duplicate-emitted at all three real exit
/// points: normal fallthrough, each matched clause's own fallthrough,
/// and the mismatch-exhausted re-raise path.
#[allow(clippy::too_many_arguments)]
fn build_begin<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  body: &'a [Spanned<Stmt>],
  rescues: &'a [RescueClause],
  ensure: &'a Option<Vec<Spanned<Stmt>>>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  let ensure_body: &'a [Spanned<Stmt>] = ensure.as_deref().unwrap_or(&[]);

  let retry_blk = context.append_basic_block(func, "begin.retry");
  builder
    .build_unconditional_branch(retry_blk)
    .map_err(|e| e.to_string())?;
  builder.position_at_end(retry_blk);

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
  // `returns_twice` also needs attaching at the call site, not just the
  // callee declaration (LLVM keeps call-site and declaration-site
  // attribute lists separate) — see `declare_exception_runtime_funcs`'s
  // own comment for the full rationale.
  let returns_twice_id = Attribute::get_named_enum_kind_id("returns_twice");
  let returns_twice_attr = context.create_enum_attribute(returns_twice_id, 0);
  setjmp_call.add_attribute(AttributeLoc::Function, returns_twice_attr);
  let setjmp_result = call_result(setjmp_call)?.into_int_value();

  let try_blk = context.append_basic_block(func, "begin.try");
  let rescue_entry_blk = context.append_basic_block(func, "begin.rescue");
  let merge_blk = context.append_basic_block(func, "begin.merge");
  let zero = context.i32_type().const_int(0, false);
  let is_first_pass = builder
    .build_int_compare(IntPredicate::EQ, setjmp_result, zero, "isfirstpass")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(is_first_pass, try_blk, rescue_entry_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(try_blk);
  // `raise_visible: false` — a raise here targets this SAME `begin`'s
  // own handler (see `ensure_stack`'s doc comment).
  ensure_stack.push((ensure_body, false));
  let try_terminated = build_block(
    context,
    builder,
    func,
    body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ensure_stack,
    retry_stack,
    ret_kind,
    ctx,
  )?;
  ensure_stack.pop();
  if !try_terminated {
    builder
      .build_call(ctx.exc_funcs.pop_handler, &[], "pophandler")
      .map_err(|e| e.to_string())?;
    // Exit point 1/3: normal fallthrough.
    build_block(
      context,
      builder,
      func,
      ensure_body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    )?;
    builder
      .build_unconditional_branch(merge_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(rescue_entry_blk);
  let tag_call = builder
    .build_call(
      ctx.exc_funcs.handler_tag,
      &[handler_ptr.into()],
      "caughttag",
    )
    .map_err(|e| e.to_string())?;
  let caught_tag = call_result(tag_call)?.into_int_value();

  retry_stack.push(retry_blk);
  let mut next_check_blk = rescue_entry_blk;
  for rescue in rescues {
    builder.position_at_end(next_check_blk);
    let arm_blk = context.append_basic_block(func, "begin.match");
    let this_next_check_blk = context.append_basic_block(func, "begin.next");

    match &rescue.class_name {
      Some(class_name) => {
        let tags = ctx
          .rescue_tag_sets
          .get(class_name.as_str())
          .ok_or_else(|| format!("codegen: unknown class `{class_name}` in `rescue`"))?;
        let mut cond: Option<IntValue> = None;
        for &tag in tags {
          let expected = context.i64_type().const_int(tag as u64, true);
          let eq = builder
            .build_int_compare(IntPredicate::EQ, caught_tag, expected, "tagmatches")
            .map_err(|e| e.to_string())?;
          cond = Some(match cond {
            None => eq,
            Some(prev) => builder
              .build_or(prev, eq, "tagor")
              .map_err(|e| e.to_string())?,
          });
        }
        let cond =
          cond.expect("a class's own tag set always contains at least its own tag (itself)");
        builder
          .build_conditional_branch(cond, arm_blk, this_next_check_blk)
          .map_err(|e| e.to_string())?;
      }
      // A bare `rescue => e` matches unconditionally (Decision log).
      None => {
        builder
          .build_unconditional_branch(arm_blk)
          .map_err(|e| e.to_string())?;
      }
    }

    builder.position_at_end(arm_blk);
    let exc_ptr_call = builder
      .build_call(
        ctx.exc_funcs.handler_exception_ptr,
        &[handler_ptr.into()],
        "matchexc",
      )
      .map_err(|e| e.to_string())?;
    let match_exc_ptr = call_result(exc_ptr_call)?;
    builder
      .build_call(
        ctx.exc_funcs.free_handler,
        &[handler_ptr.into()],
        "freehandler",
      )
      .map_err(|e| e.to_string())?;

    // A bare clause's `var` is never stored into anything — per the
    // sema leaf, it was never bound in `env` either, and per
    // `collect_lets`, it was never allocated a slot (Decision log).
    if let Some(class_name) = &rescue.class_name {
      let (rescue_var_ptr, _) = *vars
        .get(&rescue.var)
        .expect("pre-allocated by prealloc_lets for every typed RescueClause's var");
      builder
        .build_store(rescue_var_ptr, match_exc_ptr)
        .map_err(|e| e.to_string())?;
      local_classes.insert(rescue.var.clone(), class_name.clone());
    }

    // `raise_visible: true` — a raise here is a genuine re-raise,
    // leaving this `begin` (see `ensure_stack`'s doc comment).
    ensure_stack.push((ensure_body, true));
    let rescue_terminated = build_block(
      context,
      builder,
      func,
      &rescue.body,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ensure_stack,
      retry_stack,
      ret_kind,
      ctx,
    )?;
    ensure_stack.pop();
    if !rescue_terminated {
      // Exit point 2/3: a matched clause's own fallthrough.
      build_block(
        context,
        builder,
        func,
        ensure_body,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ensure_stack,
        retry_stack,
        ret_kind,
        ctx,
      )?;
      builder
        .build_unconditional_branch(merge_blk)
        .map_err(|e| e.to_string())?;
    }

    next_check_blk = this_next_check_blk;
  }
  retry_stack.pop();

  // Caught, but it matched none of this `begin`'s clauses — free this
  // handler and propagate to the next-outer one. Exit point 3/3
  // (Decision log's own "easy exit to silently miss").
  builder.position_at_end(next_check_blk);
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
  build_block(
    context,
    builder,
    func,
    ensure_body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ensure_stack,
    retry_stack,
    ret_kind,
    ctx,
  )?;
  builder
    .build_call(
      ctx.exc_funcs.raise,
      &[caught_tag.into(), mismatch_exc_ptr.into()],
      "reraise",
    )
    .map_err(|e| e.to_string())?;
  builder.build_unreachable().map_err(|e| e.to_string())?;

  builder.position_at_end(merge_blk);
  Ok(false)
}

/// Emits a straight-line sequence of statements. Stops early (without
/// erroring) after any statement that terminates the current block —
/// anything syntactically after `return`/`break`/`next`/`raise` in the
/// same list is unreachable and must not be emitted into an
/// already-terminated LLVM block.
#[allow(clippy::too_many_arguments)]
fn build_block<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  stmts: &'a [Spanned<Stmt>],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  // Each entry's `bool` is `raise_visible` — see `emit_active_ensures`'s
  // own doc comment for the full rationale.
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
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
      ensure_stack,
      retry_stack,
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
fn build_function_body<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  body: &'a [Spanned<Stmt>],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<(), String> {
  let mut loop_stack = Vec::new();
  let mut ensure_stack: Vec<(&'a [Spanned<Stmt>], bool)> = Vec::new();
  let mut retry_stack = Vec::new();
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
    &mut ensure_stack,
    &mut retry_stack,
    ret_kind,
    ctx,
  )?;
  if terminated {
    return Ok(());
  }

  match &last.node {
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
    _ => {
      let other = last;
      let terminated = build_stmt(
        context,
        builder,
        func,
        other,
        vars,
        local_classes,
        local_array_elem_types,
        &mut loop_stack,
        &mut ensure_stack,
        &mut retry_stack,
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

/// Plan 40's Decision log: LLVM symbol names for operator methods go
/// through this defensive ASCII-safe mangling table, not the raw
/// operator characters — this session did not verify inkwell's/LLVM's
/// exact escaping behavior for symbol names containing `+`/`[`/`]`/`=`
/// end-to-end through `compile_to_object`'s object-emission path.
/// Consulted only at the handful of sites that build a class method's
/// mangled LLVM symbol/`user_func_ids` key (`declare_user_functions`,
/// its matching `define_method` lookup, and `build_method_call`'s own
/// class-method lookup) — every OTHER lookup (`ClassInfo.methods`,
/// `ctx.method_owners`) keeps using the literal operator-token string,
/// unaffected. Falls back to `name` unchanged for every ordinary
/// (non-operator) method name.
fn mangled_operator_symbol(name: &str) -> &str {
  match name {
    "+" => "op_add",
    "-" => "op_sub",
    "*" => "op_mul",
    "/" => "op_div",
    "==" => "op_eq",
    "<=>" => "op_cmp",
    "[]" => "op_index",
    "[]=" => "op_index_set",
    other => other,
  }
}

/// Plan 39's Decision log: the params list codegen actually uses for a
/// function's LLVM signature/binding — ordinary params plus, when
/// `f.splat_param` is declared, one synthetic trailing `Array[Elem]`-
/// typed `Param` appended. Reuses `bind_params`'s own existing
/// `"Array[...]"`-prefix detection for `local_array_elem_types`
/// bookkeeping, rather than a second, parallel binding path — the
/// compiled function itself stays fixed-arity, never a variadic LLVM
/// signature.
fn effective_params(f: &AstFunction) -> Vec<Param> {
  let mut params = f.params.clone();
  if let Some(splat) = &f.splat_param {
    params.push(Param {
      name: splat.name.clone(),
      ty: format!("Array[{}]", splat.ty),
      default: None,
    });
  }
  params
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
    &effective_params(f),
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
  body: &[Spanned<Stmt>],
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

/// Builds `main` (`extern "C" fn(i32, ptr) -> i32`): populates `ARGV`/
/// `ARGC` from `main`'s own real `argc`/`argv` params (plan 45's
/// Decision log) before evaluating the top-level statements (including
/// `puts` calls and lambda-creating `Let`s) via the already-declared
/// runtime/user functions, and returns 0.
fn define_main<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  main_fn: FunctionValue<'ctx>,
  program: &Program,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(main_fn, "entry");
  builder.position_at_end(entry);

  let top_stmts: Vec<Spanned<Stmt>> = program
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

  // Plan 45's Decision log: `ARGV`/`ARGC` populated before `build_block`
  // runs, from `main`'s own real `argc`/`argv` params — `emerald_
  // build_argv` skips `argv[0]` (the program name, matching Ruby's own
  // `ARGV`) and reuses the OS-owned string pointers directly, no copy.
  let argc_param = main_fn
    .get_nth_param(0)
    .expect("main declares argc as its first param")
    .into_int_value();
  let argv_param = main_fn
    .get_nth_param(1)
    .expect("main declares argv as its second param")
    .into_pointer_value();
  let argv_call = builder
    .build_call(
      gen_ctx.build_argv,
      &[argc_param.into(), argv_param.into()],
      "argvtmp",
    )
    .map_err(|e| e.to_string())?;
  let argv_val = call_result(argv_call)?;
  let argv_alloca = builder
    .build_alloca(local_llvm_type(context, ValKind::Ptr), "ARGV")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(argv_alloca, argv_val)
    .map_err(|e| e.to_string())?;
  vars.insert("ARGV".to_string(), (argv_alloca, ValKind::Ptr));
  local_array_elem_types.insert("ARGV".to_string(), ValKind::Str);

  let one = context.i32_type().const_int(1, false);
  let argc_minus_one = builder
    .build_int_sub(argc_param, one, "argcm1")
    .map_err(|e| e.to_string())?;
  let argc_i64 = builder
    .build_int_z_extend(argc_minus_one, context.i64_type(), "argc64")
    .map_err(|e| e.to_string())?;
  let argc_alloca = builder
    .build_alloca(local_llvm_type(context, ValKind::Int64), "ARGC")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(argc_alloca, argc_i64)
    .map_err(|e| e.to_string())?;
  vars.insert("ARGC".to_string(), (argc_alloca, ValKind::Int64));
  let mut decls = Vec::new();
  collect_lets(&top_stmts, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let mut loop_stack = Vec::new();
  let mut ensure_stack = Vec::new();
  let mut retry_stack = Vec::new();
  let terminated = build_block(
    context,
    builder,
    main_fn,
    &top_stmts,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    &mut loop_stack,
    &mut ensure_stack,
    &mut retry_stack,
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
      // Plan 34: a `block_param`-declaring function is never compiled
      // as an ordinary, reusable LLVM function at all — every call site
      // must attach a literal block (sema-enforced), and `yield` has no
      // first-class value to dispatch through generically, so it's only
      // ever compiled via call-site inline expansion (`build_inline_
      // block_call`). Declaring an LLVM symbol for it here would be
      // dead code with no valid body to give it (`yield` means nothing
      // outside a specific attachment).
      Item::Function(f) if f.block_param.is_some() => {}
      // Plan 41's Decision log: the bare generic name is never
      // registered as a callable LLVM symbol at all — `compile_to_
      // object`'s own monomorphization pass declares one mangled
      // symbol (`max$$Money`) per distinct concrete instantiation
      // actually called anywhere in the whole program instead.
      Item::Function(f) if !f.type_params.is_empty() => {}
      Item::Function(f) => {
        let ret_kind = value_kind_for_type(&f.return_type);
        let fn_ty = make_fn_type(context, &param_kinds(&effective_params(f)), ret_kind);
        let fv = module.add_function(&f.name, fn_ty, Some(Linkage::External));
        user_func_ids.insert(f.name.clone(), (fv, ret_kind));
      }
      Item::Class(c) => {
        for m in &c.methods {
          let ret_kind = value_kind_for_type(&m.return_type);
          let mut kinds = vec![ValKind::Ptr]; // self
          kinds.extend(param_kinds(&m.params));
          let fn_ty = make_fn_type(context, &kinds, ret_kind);
          let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
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
      // Plan 41: nothing to declare — an interface has no body of its
      // own to compile.
      Item::Interface(_) => {}
      // Plan 23: nothing to declare — `compile_to_object`'s own
      // per-item loop is where a `Program` that still contains an
      // unresolved `Item::Require` (meaning `emerald-cli`'s own
      // `require.rs`, plan 46, was skipped) produces a real,
      // descriptive `Err` instead of silently compiling an incomplete
      // program.
      Item::Require(_) => {}
      // Plan 47: never actually reached — `compile_to_object`'s own
      // prologue rejects any `Program` still containing an
      // `Item::Test` before this runs at all.
      Item::Test { .. } => {}
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
    let Item::Stmt(Spanned {
      node:
        Stmt::Let {
          name,
          ty,
          value:
            Spanned {
              node:
                Expr::Lambda {
                  params,
                  return_type,
                  ..
                },
              ..
            },
        },
      ..
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
  // Plan 47's Decision log: a `test "..." do ... end` block only ever
  // compiles through `compile_test_harness` (`emerald test`) — reaching
  // this, the ordinary `emerald <file>`/`emerald build` path, is a
  // real, described rejection, not a silent no-op or panic.
  if program.items.iter().any(|i| matches!(i, Item::Test { .. })) {
    return Err(
      "top-level test block only valid under `emerald test`, not an ordinary compile".to_string(),
    );
  }
  let mut items = program.items.clone();
  let uses_assertions = desugar_asserts_in_items(&mut items);
  if uses_assertions {
    ensure_assertion_error_class(&mut items);
  }
  let owned_program = Program { items };
  let program = &owned_program;

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
  // Plan 36 (string interpolation). `emerald_bool_to_string` takes a
  // plain `long long`, not a C `_Bool`/`i1` — matches every other
  // Int64-shaped runtime ABI boundary in this file, avoiding an i1-vs-C
  // calling-convention question; codegen zero-extends the `i1` value
  // before calling it (see `build_expr`'s `Expr::Interpolate` arm).
  let int64_to_string = module.add_function(
    "emerald_int64_to_string",
    ptr_ty.fn_type(&[i64_ty.into()], false),
    Some(Linkage::External),
  );
  let float64_to_string = module.add_function(
    "emerald_float64_to_string",
    ptr_ty.fn_type(&[f64_ty.into()], false),
    Some(Linkage::External),
  );
  let bool_to_string = module.add_function(
    "emerald_bool_to_string",
    ptr_ty.fn_type(&[i64_ty.into()], false),
    Some(Linkage::External),
  );
  // Plan 45 (stdlib strings and I/O).
  let string_length = module.add_function(
    "emerald_string_length",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_upcase = module.add_function(
    "emerald_string_upcase",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_downcase = module.add_function(
    "emerald_string_downcase",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_strip = module.add_function(
    "emerald_string_strip",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_to_i = module.add_function(
    "emerald_string_to_i",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_to_f = module.add_function(
    "emerald_string_to_f",
    f64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_char_at = module.add_function(
    "emerald_string_char_at",
    ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
    Some(Linkage::External),
  );
  let string_slice = module.add_function(
    "emerald_string_slice",
    ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into(), i64_ty.into()], false),
    Some(Linkage::External),
  );
  let string_split_count = module.add_function(
    "emerald_string_split_count",
    i64_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let string_split = module.add_function(
    "emerald_string_split",
    ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let file_read = module.add_function(
    "emerald_file_read",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let file_write = module.add_function(
    "emerald_file_write",
    void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let gets = module.add_function(
    "emerald_gets",
    ptr_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let build_argv = module.add_function(
    "emerald_build_argv",
    ptr_ty.fn_type(&[context.i32_type().into(), ptr_ty.into()], false),
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

  // Plan 32: raw `ClassDef`s keyed by name, so `build_class_layout`/
  // `build_method_owners` can walk any class's inheritance chain by
  // name lookup alone — independent of `program.items`' order.
  let mut class_defs: HashMap<String, &ClassDef> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      class_defs.insert(c.name.clone(), c);
    }
  }

  // Class layouts (field offsets/kinds, now chain-flattened — ancestor
  // fields first, see `build_class_layout`) and a stable per-class
  // integer tag (declaration order) for `rescue` matching.
  let mut classes: HashMap<String, ClassLayout> = HashMap::new();
  let mut class_tags: HashMap<String, i64> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      classes.insert(c.name.clone(), build_class_layout(&c.name, &class_defs)?);
      class_tags.insert(c.name.clone(), class_tags.len() as i64);
    }
  }
  let method_owners = build_method_owners(&class_defs)?;

  // Plan 44: see `Ctx::symbol_table`'s own doc comment — built once,
  // alongside `class_tags`, before any function body compiles.
  let symbol_table = collect_program_symbols(program);

  // Plan 38: see `Ctx::rescue_tag_sets`'s own doc comment.
  let mut rescue_tag_sets: HashMap<String, Vec<i64>> = HashMap::new();
  for c_name in class_defs.keys() {
    let chain = resolve_class_chain(c_name, &class_defs)?;
    let c_tag = class_tags[c_name];
    for ancestor in &chain {
      rescue_tag_sets
        .entry(ancestor.clone())
        .or_default()
        .push(c_tag);
    }
  }

  // Plan 34: every `block_param`-declaring free function, keyed by
  // name — see `Ctx::block_funcs`'s own doc comment.
  let mut block_funcs: HashMap<String, &AstFunction> = HashMap::new();
  // Plan 39: EVERY top-level free function, keyed by name — see
  // `Ctx::func_defs`'s own doc comment.
  let mut func_defs: HashMap<String, &AstFunction> = HashMap::new();
  // Plan 41: every top-level GENERIC free function, keyed by name — a
  // subset of `func_defs` (a generic function's name is always present
  // in both), consulted only by `collect_generic_specializations`'s
  // program-wide collection pass below.
  let mut generic_fns: HashMap<String, &AstFunction> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      if f.block_param.is_some() {
        block_funcs.insert(f.name.clone(), f);
      }
      func_defs.insert(f.name.clone(), f);
      if !f.type_params.is_empty() {
        generic_fns.insert(f.name.clone(), f);
      }
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
  let mut user_func_ids = declare_user_functions(&context, &module, program, &classes);
  let lambda_func_ids = declare_lambda_functions(&context, &module, program, &lambda_infos);

  // Plan 41's Decision log: one specialization cache entry per distinct
  // `(generic function, concrete type)` pair actually called anywhere in
  // the whole program — declared here, before `gen_ctx` borrows `user_
  // func_ids` immutably, so every call site's own lookup below finds its
  // mangled symbol already present.
  let generic_specializations = collect_generic_specializations(program, &generic_fns, &classes);
  for (fn_name, concrete_classes) in &generic_specializations {
    let f = generic_fns[fn_name.as_str()];
    let type_param = &f.type_params[0].name;
    for concrete_class in concrete_classes {
      let substituted = substitute_generic_function(f, type_param, concrete_class);
      let ret_kind = value_kind_for_type(&substituted.return_type);
      let fn_ty = make_fn_type(
        &context,
        &param_kinds(&effective_params(&substituted)),
        ret_kind,
      );
      let mangled = mangled_generic_symbol(fn_name, concrete_class);
      let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
      user_func_ids.insert(mangled, (fv, ret_kind));
    }
  }

  let gen_ctx = Ctx {
    user_func_ids: &user_func_ids,
    classes: &classes,
    print_i64,
    print_f64,
    alloc,
    print_str,
    string_concat,
    string_eq,
    int64_to_string,
    float64_to_string,
    bool_to_string,
    self_ctx: None,
    lambda_func_ids: &lambda_func_ids,
    lambda_infos: &lambda_infos,
    class_tags: &class_tags,
    rescue_tag_sets: &rescue_tag_sets,
    method_owners: &method_owners,
    exc_funcs,
    module_names: &module_names,
    alloc_zeroed,
    hash_key_not_found,
    block_funcs: &block_funcs,
    func_defs: &func_defs,
    yield_target: None,
    symbol_table: &symbol_table,
    string_length,
    string_upcase,
    string_downcase,
    string_strip,
    string_to_i,
    string_to_f,
    string_char_at,
    string_slice,
    string_split_count,
    string_split,
    file_read,
    file_write,
    gets,
    build_argv,
  };

  for item in &program.items {
    match item {
      // Plan 34: never declared in `user_func_ids` above — see
      // `declare_user_functions`'s matching arm.
      Item::Function(f) if f.block_param.is_some() => {}
      // Plan 41: never declared under its own bare name — see
      // `declare_user_functions`'s matching arm; each of its
      // specializations is defined separately, below.
      Item::Function(f) if !f.type_params.is_empty() => {}
      Item::Function(f) => {
        let (fv, _) = user_func_ids[&f.name];
        define_user_function(&context, &builder, f, fv, &gen_ctx)?;
      }
      Item::Class(c) => {
        let layout = &classes[&c.name];
        for m in &c.methods {
          let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
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
      Item::Stmt(Spanned {
        node:
          Stmt::Let {
            name,
            ty,
            value:
              Spanned {
                node:
                  Expr::Lambda {
                    params,
                    return_type,
                    body,
                  },
                ..
              },
          },
        ..
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
      // Plan 41: an interface declares one required method signature,
      // never a body — nothing here to compile.
      Item::Interface(_) => {}
      // Plan 23's Decision log: reaching codegen with an unresolved
      // `Item::Require` means `emerald-cli`'s own `require.rs` (plan
      // 46) was skipped or is missing — a real, descriptive `Err`, not
      // a silent partial compile.
      Item::Require(path) => {
        return Err(format!(
          "codegen: unresolved `require {path}` — internal driver bug (require.rs's resolution step should have stripped this before codegen)"
        ));
      }
      // Plan 47: never actually reached — `compile_to_object`'s own
      // prologue rejects any `Program` still containing an
      // `Item::Test` before this runs at all.
      Item::Test { .. } => {}
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
    }
  }

  // Plan 41's Decision log: each specialization compiles the *existing*,
  // unmodified single-function codegen path (`define_user_function`)
  // against a fully concrete, textually substituted `Function` — 100%
  // reuse of the non-generic machinery, just fed a synthesized AST
  // instead of one straight from the parser.
  for (fn_name, concrete_classes) in &generic_specializations {
    let f = generic_fns[fn_name.as_str()];
    let type_param = &f.type_params[0].name;
    for concrete_class in concrete_classes {
      let substituted = substitute_generic_function(f, type_param, concrete_class);
      let mangled = mangled_generic_symbol(fn_name, concrete_class);
      let (fv, _) = user_func_ids[&mangled];
      define_user_function(&context, &builder, &substituted, fv, &gen_ctx)?;
    }
  }

  // Plan 45's Decision log: `fn(i32, ptr) -> i32` — the ordinary C
  // `main(argc, argv)` ABI `cc`'s own linked `_start` already expects,
  // so no change to `emerald-cli`'s link step is needed. A program that
  // never references `ARGV`/`ARGC`/`gets()` is unaffected.
  let main_ty = context
    .i32_type()
    .fn_type(&[context.i32_type().into(), ptr_ty.into()], false);
  let main_fn = module.add_function("main", main_ty, Some(Linkage::External));
  define_main(&context, &builder, main_fn, program, &gen_ctx)?;

  module.verify().map_err(|e| e.to_string())?;

  // Plan 38: `setjmp`'s `returns_twice` attribute (see
  // `declare_exception_runtime_funcs`'s own comment) tells LLVM's
  // optimizer the call may resume execution a second time via
  // `longjmp` — but a `retry`-capable `begin` calls `setjmp` again
  // from a genuine control-flow LOOP back to it (`begin.retry`),
  // something no other construct in this compiler does. Verified this
  // session: even with `returns_twice` correctly attached, the full
  // `default<O3>` pipeline still miscompiles a `retry` loop into a
  // program that hangs at runtime (very likely `mem2reg`/SROA
  // promoting a local variable's `alloca` into a register value that
  // doesn't survive the `longjmp` correctly — the exact class of bug C
  // programmers avoid with `volatile`, which this backend has no
  // per-variable equivalent of). Skipping optimization entirely for a
  // program that uses `retry` anywhere is the safe, disclosed
  // workaround: `alloca`+`load`/`store`-based IR is always correct on
  // its own (mem2reg is a pure optimization, not required for
  // correctness), just slower — an acceptable, real tradeoff for a
  // rare, non-hot-path exception-recovery construct.
  if program_uses_retry(program) {
    return target_machine
      .write_to_file(&module, FileType::Object, out_path)
      .map_err(|e| e.to_string());
  }

  let pass_options = inkwell::passes::PassBuilderOptions::create();
  module
    .run_passes("default<O3>", &target_machine, pass_options)
    .map_err(|e| e.to_string())?;

  target_machine
    .write_to_file(&module, FileType::Object, out_path)
    .map_err(|e| e.to_string())
}

/// Plan 47's Decision log: `assert(cond, loc)`/`assert_eq(expected,
/// actual, loc)` are recognized-call-name intrinsics (like `puts`) —
/// this rewrites every such `Stmt::Expr(Expr::Call(...))` reachable
/// from `items` into a real `if !(...) { raise AssertionError.new(loc)
/// }` shape, entirely at the AST level, before any LLVM emission. They
/// can only ever appear as a bare statement (grammar.lalrpop only adds
/// them to `StmtPrimaryExpr`, never nested `PrimaryExpr`), so this only
/// needs to pattern-match at the `Stmt` level, never walk into
/// arbitrary sub-expressions. Returns whether it rewrote anything, so
/// the synthetic `AssertionError` class is only ever injected into a
/// program that actually needed it (never a plain `hello.em`).
/// Plan 22's Decision log: this whole desugar/synthesized-test-harness
/// section constructs brand-new `Expr`/`Stmt` nodes at codegen time —
/// none of them came from real source text, so every one wraps in
/// `Spanned::synthetic` (span `(0, 0)`) rather than fabricating a
/// position. `syn` is a short local alias, since this section
/// constructs a *lot* of them.
fn syn<T>(node: T) -> Spanned<T> {
  Spanned::synthetic(node)
}

fn desugar_asserts_in_items(items: &mut [Item]) -> bool {
  let mut rewrote = false;
  for item in items {
    match item {
      Item::Function(f) => desugar_asserts_in_stmts(&mut f.body, &mut rewrote),
      Item::Class(c) => {
        for m in &mut c.methods {
          desugar_asserts_in_stmts(&mut m.body, &mut rewrote);
        }
      }
      Item::Module(m) => {
        for f in &mut m.methods {
          desugar_asserts_in_stmts(&mut f.body, &mut rewrote);
        }
      }
      Item::Stmt(s) => desugar_asserts_in_stmt(s, &mut rewrote),
      Item::Test { body, .. } => desugar_asserts_in_stmts(body, &mut rewrote),
      Item::Interface(_) | Item::Require(_) | Item::Error => {}
    }
  }
  rewrote
}

fn desugar_asserts_in_stmts(stmts: &mut [Spanned<Stmt>], rewrote: &mut bool) {
  for s in stmts {
    desugar_asserts_in_stmt(s, rewrote);
  }
}

fn desugar_asserts_in_stmt(stmt: &mut Spanned<Stmt>, rewrote: &mut bool) {
  match &mut stmt.node {
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      desugar_asserts_in_stmts(then_branch, rewrote);
      if let Some(b) = else_branch {
        desugar_asserts_in_stmts(b, rewrote);
      }
    }
    Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
      desugar_asserts_in_stmts(body, rewrote)
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      desugar_asserts_in_stmts(body, rewrote);
      for r in rescues {
        desugar_asserts_in_stmts(&mut r.body, rewrote);
      }
      if let Some(e) = ensure {
        desugar_asserts_in_stmts(e, rewrote);
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        desugar_asserts_in_stmts(body, rewrote);
      }
      if let Some(b) = else_body {
        desugar_asserts_in_stmts(b, rewrote);
      }
    }
    _ => {}
  }
  let replacement = match &stmt.node {
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) if name == "assert" && args.len() == 2 => Some(desugar_assert(&args[0], &args[1])),
    Stmt::Expr(Spanned {
      node: Expr::Call(name, args),
      ..
    }) if name == "assert_eq" && args.len() == 3 => {
      Some(desugar_assert_eq(&args[0], &args[1], &args[2]))
    }
    _ => None,
  };
  if let Some(new_stmt) = replacement {
    // The original statement's own real span is kept — this replaces
    // exactly that source text, unlike every node synthesized *inside*
    // the new shape (which has no real source text to point at).
    stmt.node = new_stmt;
    *rewrote = true;
  }
}

fn desugar_assert(cond: &Spanned<Expr>, loc: &Spanned<Expr>) -> Stmt {
  Stmt::If {
    cond: syn(Expr::Not(Box::new(cond.clone()))),
    then_branch: vec![syn(Stmt::Raise(syn(Expr::New(
      "AssertionError".to_string(),
      vec![loc.clone()],
    ))))],
    else_branch: None,
  }
}

/// Plan 47's Decision log: the failure report concatenates only
/// `String + String` (the `"expected:"`/`"but got:"` labels) and
/// prints `expected`/`actual`'s own values via `puts` on their own
/// lines, exactly like `emerald test`'s summary counts — real,
/// disclosed limit: this only actually compiles for the Int64/Float64/
/// String operand types `puts` itself supports (see `build_puts`); a
/// `Boolean`/`Symbol` `assert_eq` (sema accepts both, matching `==`'s
/// own scope) fails at this point with `build_puts`'s own, unchanged
/// "does not support" codegen error.
fn desugar_assert_eq(
  expected: &Spanned<Expr>,
  actual: &Spanned<Expr>,
  loc: &Spanned<Expr>,
) -> Stmt {
  let compare = syn(Expr::Compare(
    Box::new(expected.clone()),
    CompareOp::Eq,
    Box::new(actual.clone()),
  ));
  Stmt::If {
    cond: syn(Expr::Not(Box::new(compare))),
    then_branch: vec![
      syn(Stmt::Expr(syn(Expr::Call(
        "puts".to_string(),
        vec![syn(Expr::StringLit("expected:".to_string()))],
      )))),
      syn(Stmt::Expr(syn(Expr::Call(
        "puts".to_string(),
        vec![expected.clone()],
      )))),
      syn(Stmt::Expr(syn(Expr::Call(
        "puts".to_string(),
        vec![syn(Expr::StringLit("but got:".to_string()))],
      )))),
      syn(Stmt::Expr(syn(Expr::Call(
        "puts".to_string(),
        vec![actual.clone()],
      )))),
      syn(Stmt::Raise(syn(Expr::New(
        "AssertionError".to_string(),
        vec![loc.clone()],
      )))),
    ],
    else_branch: None,
  }
}

/// A synthetic `class AssertionError; message: String; def initialize
/// (message: String) -> Void; @message = message; end; def message ->
/// String; @message; end; end` — the smallest real, catchable value
/// `raise`/`rescue` (plan 11, unchanged) already supports, mirroring
/// plan 33's own `read <name>: <Type>` field-accessor sugar's exact
/// codegen shape for the accessor. Inserted at index 0 so it's always
/// declared before anything that might reference it.
fn assertion_error_class_item() -> Item {
  Item::Class(ClassDef {
    name: "AssertionError".to_string(),
    superclass: None,
    implements: None,
    fields: vec![Param {
      name: "message".to_string(),
      ty: "String".to_string(),
      default: None,
    }],
    methods: vec![
      AstFunction {
        name: "initialize".to_string(),
        params: vec![Param {
          name: "message".to_string(),
          ty: "String".to_string(),
          default: None,
        }],
        return_type: "Void".to_string(),
        body: vec![syn(Stmt::SetField {
          name: "message".to_string(),
          value: syn(Expr::Ident("message".to_string())),
        })],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
      },
      AstFunction {
        name: "message".to_string(),
        params: Vec::new(),
        return_type: "String".to_string(),
        body: vec![syn(Stmt::Expr(syn(Expr::InstanceVar(
          "message".to_string(),
        ))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
      },
    ],
  })
}

fn ensure_assertion_error_class(items: &mut Vec<Item>) {
  let already_present = items
    .iter()
    .any(|i| matches!(i, Item::Class(c) if c.name == "AssertionError"));
  if !already_present {
    items.insert(0, assertion_error_class_item());
  }
}

/// Plan 47's `leaf-test-runner`: synthesizes a runnable program from
/// every `Item::Test` in `program.items` — each test's body becomes
/// its own top-level `Item::Function` (`__emerald_test_N`), called
/// from a synthesized top-level sequence that wraps each call in
/// `begin ... rescue AssertionError => e ... end`, tracks `passed`/
/// `failed` counts, prints `PASS:`/`FAIL:` lines and the final
/// `passed:`/`failed:` summary, then — since top-level Emerald code has
/// no mechanism of its own to set `main`'s exit code (see
/// `define_main`'s own doc comment) — deliberately `raise`s an
/// uncaught `AssertionError` when `failed > 0`, reusing
/// `emerald_raise`'s own already-tested uncaught-exception path
/// (`runtime/emerald_runtime.c`: a `stderr`-only message, `exit(1)`)
/// instead of adding any new LLVM-emitting code. The whole synthesized
/// program is then compiled by the ordinary, unmodified
/// `compile_to_object` — this function never touches LLVM directly.
pub fn compile_test_harness(program: &Program, out_path: &Path) -> Result<usize, String> {
  let tests: Vec<(String, Vec<Spanned<Stmt>>)> = program
    .items
    .iter()
    .filter_map(|it| match it {
      Item::Test { description, body } => Some((description.clone(), body.clone())),
      _ => None,
    })
    .collect();
  let num_tests = tests.len();

  let mut items: Vec<Item> = program
    .items
    .iter()
    .filter(|it| !matches!(it, Item::Test { .. }))
    .cloned()
    .collect();

  let mut harness_stmts = vec![
    syn(Stmt::Let {
      name: "passed".to_string(),
      ty: "Int64".to_string(),
      value: syn(Expr::Int(0)),
    }),
    syn(Stmt::Let {
      name: "failed".to_string(),
      ty: "Int64".to_string(),
      value: syn(Expr::Int(0)),
    }),
  ];

  for (i, (description, body)) in tests.into_iter().enumerate() {
    let fn_name = format!("__emerald_test_{i}");
    items.push(Item::Function(AstFunction {
      name: fn_name.clone(),
      params: Vec::new(),
      return_type: "Void".to_string(),
      body,
      block_param: None,
      splat_param: None,
      type_params: Vec::new(),
    }));

    harness_stmts.push(syn(Stmt::Begin {
      body: vec![
        syn(Stmt::Expr(syn(Expr::Call(fn_name, Vec::new())))),
        syn(Stmt::Expr(syn(Expr::Call(
          "puts".to_string(),
          vec![syn(Expr::StringLit(format!("PASS: {description}")))],
        )))),
        syn(Stmt::Assign {
          name: "passed".to_string(),
          value: syn(Expr::Add(
            Box::new(syn(Expr::Ident("passed".to_string()))),
            Box::new(syn(Expr::Int(1))),
          )),
        }),
      ],
      rescues: vec![RescueClause {
        class_name: Some("AssertionError".to_string()),
        var: "e".to_string(),
        body: vec![
          syn(Stmt::Expr(syn(Expr::Call(
            "puts".to_string(),
            vec![syn(Expr::Add(
              Box::new(syn(Expr::StringLit(format!("FAIL: {description}: ")))),
              Box::new(syn(Expr::MethodCall(
                Box::new(syn(Expr::Ident("e".to_string()))),
                "message".to_string(),
                Vec::new(),
              ))),
            ))],
          )))),
          syn(Stmt::Assign {
            name: "failed".to_string(),
            value: syn(Expr::Add(
              Box::new(syn(Expr::Ident("failed".to_string()))),
              Box::new(syn(Expr::Int(1))),
            )),
          }),
        ],
      }],
      ensure: None,
    }));
  }

  harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
    "puts".to_string(),
    vec![syn(Expr::StringLit("passed:".to_string()))],
  )))));
  harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
    "puts".to_string(),
    vec![syn(Expr::Ident("passed".to_string()))],
  )))));
  harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
    "puts".to_string(),
    vec![syn(Expr::StringLit("failed:".to_string()))],
  )))));
  harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
    "puts".to_string(),
    vec![syn(Expr::Ident("failed".to_string()))],
  )))));
  harness_stmts.push(syn(Stmt::If {
    cond: syn(Expr::Compare(
      Box::new(syn(Expr::Ident("failed".to_string()))),
      CompareOp::Gt,
      Box::new(syn(Expr::Int(0))),
    )),
    then_branch: vec![syn(Stmt::Raise(syn(Expr::New(
      "AssertionError".to_string(),
      vec![syn(Expr::StringLit(
        "emerald test: one or more tests failed".to_string(),
      ))],
    ))))],
    else_branch: None,
  }));

  items.extend(harness_stmts.into_iter().map(Item::Stmt));
  ensure_assertion_error_class(&mut items);

  let owned_program = Program { items };
  compile_to_object(&owned_program, out_path)?;
  Ok(num_tests)
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

  /// Plan 45's own general-purpose runner — `compile_link_run` above
  /// covers every prior plan's own no-args/no-stdin/cwd-agnostic case
  /// unchanged; this one adds exactly the three axes `ARGV`/`gets`/
  /// `File` need (fixed argv, fixed stdin, a chosen working directory),
  /// returning the full `Output` rather than just stdout so a negative
  /// (non-zero exit, disclosed abort) case can be asserted too.
  fn compile_link_run_full(
    src: &str,
    args: &[&str],
    stdin_input: Option<&str>,
    dir: Option<&std::path::Path>,
  ) -> std::process::Output {
    use std::io::Write;
    let program = emerald_parser::parse(src).expect("should parse");
    let temp_dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = temp_dir.join(format!("emerald_codegen_aot_{unique}.o"));
    let bin_path = temp_dir.join(format!("emerald_codegen_aot_bin_{unique}"));

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

    let mut cmd = Command::new(&bin_path);
    cmd.args(args);
    if let Some(d) = dir {
      cmd.current_dir(d);
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("failed to run compiled binary");
    if let Some(input) = stdin_input {
      child
        .stdin
        .take()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("should write to stdin");
    } else {
      drop(child.stdin.take());
    }
    let output = child
      .wait_with_output()
      .expect("failed to wait on compiled binary");

    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();

    output
  }

  fn compile_link_run_with_stdin(src: &str, input: &str) -> String {
    let output = compile_link_run_full(src, &[], Some(input), None);
    assert!(output.status.success(), "compiled binary should exit 0");
    String::from_utf8_lossy(&output.stdout).into_owned()
  }

  fn compile_link_run_with_args(src: &str, args: &[&str]) -> String {
    let output = compile_link_run_full(src, args, None, None);
    assert!(output.status.success(), "compiled binary should exit 0");
    String::from_utf8_lossy(&output.stdout).into_owned()
  }

  fn fresh_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald_codegen_{tag}_{}_{:?}",
      std::process::id(),
      std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("should create a fresh temp dir");
    dir
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
      items: vec![Item::Stmt(syn(Stmt::Expr(syn(Expr::Call(
        "puts".into(),
        vec![syn(Expr::Int(1)), syn(Expr::Int(2))],
      )))))],
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
      items: vec![Item::Stmt(syn(Stmt::Expr(syn(Expr::Call(
        "puts".into(),
        vec![syn(Expr::Rem(
          Box::new(syn(Expr::Float(1.5))),
          Box::new(syn(Expr::Float(2.0))),
        ))],
      )))))],
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
      items: vec![Item::Stmt(syn(Stmt::Expr(syn(Expr::Call(
        "puts".into(),
        vec![syn(Expr::Compare(
          Box::new(syn(Expr::StringLit("a".into()))),
          CompareOp::Lt,
          Box::new(syn(Expr::StringLit("b".into()))),
        ))],
      )))))],
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
      items: vec![Item::Stmt(syn(Stmt::Expr(syn(Expr::Call(
        "puts".into(),
        vec![syn(Expr::BitAnd(
          Box::new(syn(Expr::Float(1.5))),
          Box::new(syn(Expr::Int(1))),
        ))],
      )))))],
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

  // Plan 30 (for-in iteration).

  #[test]
  fn for_in_sums_a_literal_array() {
    let src = "sum: Int64 = 0\nfor x in [10, 20, 30]\n  sum: Int64 = sum + x\nend\nputs sum\n";
    assert_eq!(compile_link_run(src), "60\n");
  }

  #[test]
  fn for_in_break_stops_after_the_second_element() {
    let src = "for x in [10, 20, 30, 40]\n  if x == 30\n    break\n  end\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "10\n20\n");
  }

  #[test]
  fn for_in_next_skips_one_element() {
    let src = "for x in [10, 20, 30]\n  if x == 20\n    next\n  end\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "10\n30\n");
  }

  #[test]
  fn for_in_over_a_string_array_linked_and_run() {
    let src = "for x in [\"a\", \"b\", \"c\"]\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "a\nb\nc\n");
  }

  #[test]
  fn for_in_over_an_empty_element_list_does_not_panic() {
    // AC3's "unsupported shape" standard, adapted: the grammar bakes
    // literal-ness into `Stmt::For.elements` directly, so there is no
    // representable non-literal-scrutinee shape to construct here — the
    // one edge case codegen alone (bypassing sema's empty-literal
    // rejection) can actually receive is an empty `elements` list. It
    // must not panic; zero iterations is a coherent, defensible result.
    let program = Program {
      items: vec![Item::Stmt(syn(Stmt::For {
        var: "x".into(),
        elements: vec![],
        body: vec![syn(Stmt::Expr(syn(Expr::Call(
          "puts".into(),
          vec![syn(Expr::Ident("x".into()))],
        ))))],
      }))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_empty_for_in_should_not_exist.o");
    let result = compile_to_object(&program, &out);
    std::fs::remove_file(&out).ok();
    assert!(
      result.is_ok(),
      "must not panic on an empty for-in element list: {result:?}"
    );
  }

  // Plan 31 (compound and multiple assignment).

  #[test]
  fn bare_reassignment_linked_and_run() {
    let src = "x: Int64 = 1\nputs x\nx = 2\nputs x\n";
    assert_eq!(compile_link_run(src), "1\n2\n");
  }

  const COMPOUND_ASSIGN_EXAMPLE: &str =
    "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5\n  total += i\n  i += 1\nend\nputs total\n";

  #[test]
  fn compound_plus_assign_accumulator_linked_and_run() {
    assert_eq!(compile_link_run(COMPOUND_ASSIGN_EXAMPLE), "10\n");
  }

  #[test]
  fn all_compound_assign_operators_linked_and_run() {
    assert_eq!(compile_link_run("x: Int64 = 10\nx -= 3\nputs x\n"), "7\n");
    assert_eq!(compile_link_run("x: Int64 = 10\nx *= 3\nputs x\n"), "30\n");
    assert_eq!(compile_link_run("x: Int64 = 10\nx /= 3\nputs x\n"), "3\n");
    assert_eq!(compile_link_run("x: Int64 = 10\nx %= 3\nputs x\n"), "1\n");
  }

  const MULTI_ASSIGN_SWAP_EXAMPLE: &str =
    "a: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";

  #[test]
  fn multiple_assignment_swap_linked_and_run() {
    // The whole point of this leaf: a left-to-right, unbuffered
    // implementation would print `2\n2\n` (a real, easy-to-get-wrong
    // correctness pitfall, not just an implementation detail).
    assert_eq!(compile_link_run(MULTI_ASSIGN_SWAP_EXAMPLE), "2\n1\n");
  }

  #[test]
  fn plan_31_worked_example_linked_and_run() {
    let src = "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5\n  total += i\n  i += 1\nend\nputs total\n\na: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    assert_eq!(compile_link_run(src), "10\n2\n1\n");
  }

  // Plan 32 (class inheritance).

  const INHERITANCE_EXAMPLE: &str = "class Animal\n  age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def age -> Int64\n    @age\n  end\n\n  def describe -> Int64\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  def initialize(age: Int64, breed_code: Int64) -> Void\n    @age = age\n    @breed_code = breed_code\n  end\n\n  def describe -> Int64\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";

  #[test]
  fn inheritance_example_linked_and_run() {
    // Real executed proof inherited-field layout, inherited-method
    // resolution (`d.age` — Dog never declares `age`), and override
    // (`d.describe` — Dog's own wins and reads both the inherited
    // `@age` and Dog's own `@breed_code`) all work together without
    // corrupting each other's memory.
    assert_eq!(compile_link_run(INHERITANCE_EXAMPLE), "5\n3\n103\n");
  }

  #[test]
  fn class_with_no_superclass_still_compiles_and_runs_unchanged() {
    // Regression: an ordinary, non-inheriting class must behave exactly
    // as before this plan.
    let src = "class Counter\n  n: Int64\n\n  def initialize(n: Int64) -> Void\n    @n = n\n  end\n\n  def value -> Int64\n    @n\n  end\nend\n\nc: Counter = Counter.new(7)\nputs c.value\n";
    assert_eq!(compile_link_run(src), "7\n");
  }

  // Plan 33 (field-access sugar).

  #[test]
  fn read_field_accessor_linked_and_run() {
    // Real executed proof `p.x` dispatches through the synthesized
    // zero-arg accessor exactly like a hand-written method — no codegen
    // source changes needed for this plan at all.
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";
    assert_eq!(compile_link_run(src), "3\n");
  }

  // Plan 34 (blocks and yield).

  const BLOCKS_EXAMPLE: &str = "def repeat(n: Int64, &blk) -> Void\n  i: Int64 = 0\n  while i < n\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) { |i: Int64| puts i }\n";

  #[test]
  fn blocks_and_yield_example_linked_and_run() {
    // Real executed proof call-site specialization, the index-loop
    // control flow inlined straight into the caller, and `yield`'s
    // direct-call-style lowering into the attached block's body all
    // work together.
    assert_eq!(compile_link_run(BLOCKS_EXAMPLE), "0\n1\n2\n");
  }

  #[test]
  fn block_captures_an_outer_local_linked_and_run() {
    // AC2: the block reads a variable from its enclosing scope (an
    // accumulator), not just its own `yield`-bound parameter — real
    // proof capture flows through this call-site-specialization path
    // (reusing the same `vars` map as the call site itself), not just
    // plan 10's original top-level-`Let` lambda path.
    let src = "def repeat(n: Int64, &blk) -> Void\n  i: Int64 = 0\n  while i < n\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nmultiplier: Int64 = 10\nrepeat(3) { |i: Int64| puts i * multiplier }\n";
    assert_eq!(compile_link_run(src), "0\n10\n20\n");
  }

  #[test]
  fn block_param_call_missing_trailing_block_errors_not_panics() {
    // AC3: an unsupported shape (should already be rejected by sema in
    // the normal pipeline) defensively returns a descriptive `Err` if
    // it ever reaches codegen regardless, not a panic.
    let program = Program {
      items: vec![
        Item::Function(AstFunction {
          name: "repeat".into(),
          params: vec![Param {
            name: "n".into(),
            ty: "Int64".into(),
            default: None,
          }],
          return_type: "Void".into(),
          body: vec![syn(Stmt::Yield(vec![syn(Expr::Ident("n".into()))]))],
          block_param: Some("blk".into()),
          splat_param: None,
          type_params: Vec::new(),
        }),
        Item::Stmt(syn(Stmt::Expr(syn(Expr::Call(
          "repeat".into(),
          vec![syn(Expr::Int(3))],
        ))))),
      ],
    };
    let out = std::env::temp_dir().join("emerald_codegen_missing_block_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 23 (multi-file compilation).

  #[test]
  fn unresolved_require_errors_not_panics() {
    // `emerald-driver`'s resolution step (plan 17) isn't extracted in
    // this codebase yet — an `Item::Require` reaching codegen means
    // that step was skipped, a real internal-bug case codegen
    // defensively rejects rather than silently compiling an incomplete
    // program.
    let program = Program {
      items: vec![Item::Require("helpers".into())],
    };
    let out = std::env::temp_dir().join("emerald_codegen_unresolved_require_should_not_exist.o");
    let result = compile_to_object(&program, &out);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("require"));
  }

  // Plan 36 (string interpolation and heredocs).

  #[test]
  fn string_interpolation_worked_example_linked_and_run() {
    let src = "name: String = \"World\"\nage: Int64 = 30\nputs \"Hello, #{name}! You are #{age} years old.\"\n";
    assert_eq!(
      compile_link_run(src),
      "Hello, World! You are 30 years old.\n"
    );
  }

  #[test]
  fn string_interpolation_of_bool_and_float_linked_and_run() {
    let src = "puts \"#{true} and #{3.5}\"\n";
    assert_eq!(compile_link_run(src), "true and 3.5\n");
  }

  // Plan 37 (ranges and range-based iteration).

  const RANGE_WORKED_EXAMPLE: &str = "total: Int64 = 0\nfor i in 1..5\n  total += i\nend\nputs total\n\ntotal2: Int64 = 0\nfor i in 1...5\n  total2 += i\nend\nputs total2\n";

  #[test]
  fn range_worked_example_linked_and_run() {
    // The real distinguishing proof `..`/`...` bind their upper
    // endpoint differently: 1+2+3+4+5 = 15 vs. 1+2+3+4 = 10.
    assert_eq!(compile_link_run(RANGE_WORKED_EXAMPLE), "15\n10\n");
  }

  #[test]
  fn break_inside_a_range_for_in_stops_iteration_early() {
    let src = "for i in 1..5\n  if i == 3\n    break\n  end\n  puts i\nend\n";
    assert_eq!(compile_link_run(src), "1\n2\n");
  }

  #[test]
  fn next_inside_a_range_for_in_skips_one_element() {
    let src = "for i in 1..5\n  if i == 3\n    next\n  end\n  puts i\nend\n";
    assert_eq!(compile_link_run(src), "1\n2\n4\n5\n");
  }

  #[test]
  fn range_for_in_over_a_non_literal_endpoint_linked_and_run() {
    let src = "n: Int64 = 4\nfor i in 0..n\n  puts i\nend\n";
    assert_eq!(compile_link_run(src), "0\n1\n2\n3\n4\n");
  }

  #[test]
  fn reverse_range_for_in_is_a_well_typed_zero_iteration_loop() {
    let src = "for i in 5..1\n  puts i\nend\n";
    assert_eq!(compile_link_run(src), "");
  }

  // Plan 38 (full exception model).

  const FULL_EXCEPTION_WORKED_EXAMPLE: &str = "class NotFoundError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\nclass TimeoutError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise TimeoutError.new(7)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";

  #[test]
  fn full_exception_worked_example_linked_and_run() {
    // Real distinguishing proof: raising `TimeoutError` is NOT caught
    // by the first (`NotFoundError`) clause, IS caught by the second
    // (ordering), and `ensure` always runs afterward.
    assert_eq!(
      compile_link_run(FULL_EXCEPTION_WORKED_EXAMPLE),
      "7\ncleanup\n"
    );
  }

  #[test]
  fn ensure_runs_on_the_non_exceptional_path() {
    let src = "class Foo\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nbegin\n  puts 1\nrescue Foo => f\n  puts 0\nensure\n  puts 2\nend\n";
    assert_eq!(compile_link_run(src), "1\n2\n");
  }

  #[test]
  fn ensure_runs_on_the_mismatch_exhausted_reraise_path_of_an_inner_begin() {
    // The raised type matches neither the inner clause but does match
    // the outer one — proving `ensure` fires on the mismatch-exhausted
    // path, in the correct order (inner's `ensure` before the outer
    // clause's own output).
    let src = "class WrongType\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nclass RightType\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\nbegin\n  begin\n    raise RightType.new(5)\n  rescue WrongType => w\n    puts 0\n  ensure\n    puts \"inner\"\n  end\nrescue RightType => r\n  puts r.code\nensure\n  puts \"outer\"\nend\n";
    assert_eq!(compile_link_run(src), "inner\n5\nouter\n");
  }

  #[test]
  fn return_inside_a_matched_rescue_still_runs_ensure_first() {
    let src = "class Foo\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise Foo.new(1)\n  end\n  return x\nend\n\ndef f(x: Int64) -> Int64\n  begin\n    puts risky(x)\n  rescue Foo => e\n    return 9\n  ensure\n    puts \"cleanup\"\n  end\n  return 0\nend\n\nputs f(999)\n";
    assert_eq!(compile_link_run(src), "cleanup\n9\n");
  }

  #[test]
  fn subtype_aware_rescue_catches_a_raised_subclass_reusing_the_animal_dog_hierarchy() {
    let src = "class Animal\n  age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def age -> Int64\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  def initialize(age: Int64, breed_code: Int64) -> Void\n    @age = age\n    @breed_code = breed_code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise Dog.new(7, 1)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue Animal => a\n  puts a.age\nend\n";
    assert_eq!(compile_link_run(src), "7\n");
  }

  #[test]
  fn bare_rescue_after_a_typed_mismatch_still_catches_unconditionally() {
    let src = "class Foo\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nclass Bar\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise Foo.new(1)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue Bar => b\n  puts 0\nrescue => e\n  puts 1\nend\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  #[test]
  fn retry_re_attempts_the_begin_until_it_succeeds() {
    let src = "class Foo\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nattempts: Int64 = 0\n\nbegin\n  attempts = attempts + 1\n  if attempts < 3\n    raise Foo.new(1)\n  end\nrescue Foo => e\n  retry\nend\nputs attempts\n";
    assert_eq!(compile_link_run(src), "3\n");
  }

  #[test]
  fn retry_does_not_re_trigger_the_enclosing_ensure_per_attempt() {
    // `ensure` must print exactly once (after the third, successful
    // attempt), not once per attempt.
    let src = "class Foo\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\nend\n\nattempts: Int64 = 0\n\nbegin\n  attempts = attempts + 1\n  if attempts < 3\n    raise Foo.new(1)\n  end\nrescue Foo => e\n  retry\nensure\n  puts \"cleanup\"\nend\nputs attempts\n";
    assert_eq!(compile_link_run(src), "cleanup\n3\n");
  }

  #[test]
  fn retry_with_no_enclosing_rescue_body_errors_not_panics() {
    // Defensive-only: bypasses sema (which would already reject this)
    // to prove codegen alone rejects a `retry` with an empty
    // `retry_stack` with a descriptive `Err`, not a panic.
    let program = Program {
      items: vec![Item::Stmt(syn(Stmt::Retry))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_bare_retry_should_not_exist.o");
    let result = compile_to_object(&program, &out);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("retry"));
  }

  // Plan 39 (function signature completeness).

  #[test]
  fn default_param_omitted_uses_the_default_value() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n\nputs inc(5)\n";
    assert_eq!(compile_link_run(src), "6\n");
  }

  #[test]
  fn default_param_overridden_by_explicit_argument() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n\nputs inc(5, 10)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  const GREET_EXAMPLE: &str = "def greet(name: String, times: Int64 = 1) -> Void\n  i: Int64 = 0\n  while i < times\n    puts name\n    i += 1\n  end\nend\n\ngreet(name: \"yo\")\ngreet(name: \"hi\", times: 2)\n";

  #[test]
  fn greet_worked_example_keyword_calls_and_defaults_linked_and_run() {
    // Real distinguishing proof: keyword resolution and default-filling
    // compose correctly together (`greet(name: "yo")` uses `times`'s
    // default; `greet(name: "hi", times: 2)` overrides it).
    assert_eq!(compile_link_run(GREET_EXAMPLE), "yo\nhi\nhi\n");
  }

  #[test]
  fn positional_call_still_compiles_and_runs_unchanged() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  const SUM_ALL_EXAMPLE: &str = "def sum_all(*xs: Int64) -> Int64\n  total: Int64 = 0\n  i: Int64 = 0\n  while i < 4\n    total += xs[i]\n    i += 1\n  end\n  total\nend\n\nputs sum_all(1, 2, 3, 4)\n";

  #[test]
  fn splat_param_sums_a_packed_trailing_argument_list() {
    assert_eq!(compile_link_run(SUM_ALL_EXAMPLE), "10\n");
  }

  #[test]
  fn splat_param_with_zero_trailing_arguments_is_a_zero_length_capture() {
    let src = "def sum_all(*xs: Int64) -> Int64\n  0\nend\n\nputs sum_all()\n";
    assert_eq!(compile_link_run(src), "0\n");
  }

  // Plan 40 (operator overloading).

  const VECTOR2_EXAMPLE: &str = "class Vector2\n  read x: Float64\n  read y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def +(other: Vector2) -> Vector2\n    Vector2.new(@x + other.x, @y + other.y)\n  end\n\n  def ==(other: Vector2) -> Boolean\n    @x == other.x && @y == other.y\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0, 2.0)\nv2: Vector2 = Vector2.new(3.0, 4.0)\nv3: Vector2 = v1 + v2\nputs v3.x\nputs v3.y\nif v1 == v2\n  puts 1\nelse\n  puts 0\nend\nif v1 == v1\n  puts 1\nelse\n  puts 0\nend\n";

  #[test]
  fn vector2_worked_example_linked_and_run() {
    // Real executed proof `+` allocates a genuine new `Vector2` via a
    // real call into a compiled `Vector2_op_add` function, and `==`
    // genuinely calls into a compiled `Vector2_op_eq` function rather
    // than a raw pointer comparison (which would make `v1 == v1`
    // trivially true via identity but couldn't correctly print `0` for
    // `v1 == v2` based on field values alone).
    assert_eq!(compile_link_run(VECTOR2_EXAMPLE), "4\n6\n0\n1\n");
  }

  // `build_index`/`build_set_index` only support a plain local-variable
  // receiver (same restriction as `MethodCall`, plan 40's Decision log)
  // — `@data` itself isn't an `Expr::Ident`, so each method rebinds the
  // field into a local first, matching this compiler's existing
  // instance-var-to-local pattern used throughout the test suite.
  const BAG_EXAMPLE: &str = "class Bag\n  data: Array[Int64]\n\n  def initialize(a: Int64, b: Int64, c: Int64) -> Void\n    @data = [a, b, c]\n  end\n\n  def [](i: Int64) -> Int64\n    d: Array[Int64] = @data\n    d[i]\n  end\n\n  def []=(i: Int64, v: Int64) -> Void\n    d: Array[Int64] = @data\n    d[i] = v\n  end\nend\n\nb: Bag = Bag.new(10, 20, 30)\nputs b[0] + b[1] + b[2]\nb[1] = 99\nputs b[1]\n";

  #[test]
  fn bag_index_operator_example_read_then_write_then_read_back() {
    // Real proof `obj[i]`/`obj[i] = v` genuinely delegate to the
    // class's `[]`/`[]=` methods, analogous to plan 09's own
    // `ARRAY_EXAMPLE` read-then-write-then-read-back shape.
    assert_eq!(compile_link_run(BAG_EXAMPLE), "60\n99\n");
  }

  #[test]
  fn chained_operator_call_on_a_non_ident_receiver_errors_not_panics() {
    // `v1 + v2 + v3` — `+` parses left-associatively, so the outer
    // `+`'s LHS is the nested `Expr::Add(v1, v2)`, not a bare local,
    // inheriting `build_method_call`'s own pre-existing receiver
    // restriction verbatim (Decision log) — a descriptive `Err`, not a
    // panic. The grammar has no generic parenthesized-expression
    // grouping (only `Array.new(...)` uses parens), so this is written
    // without explicit parens.
    let src = "class Vector2\n  read x: Float64\n\n  def initialize(x: Float64) -> Void\n    @x = x\n  end\n\n  def +(other: Vector2) -> Vector2\n    Vector2.new(@x + other.x)\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nv3: Vector2 = Vector2.new(3.0)\nv4: Vector2 = v1 + v2 + v3\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let out = std::env::temp_dir().join("emerald_codegen_chained_operator_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  #[test]
  fn plan_08_point_example_still_compiles_and_runs_unchanged() {
    assert_eq!(compile_link_run(POINT_EXAMPLE), "5\n");
  }

  // Plan 41 (interfaces and generics).

  const INTERFACES_GENERICS_EXAMPLE: &str = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Int64\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  def initialize(meters: Int64) -> Void\n    @meters = meters\n  end\n\n  def compare_to(other: Distance) -> Int64\n    @meters - other.meters\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

  #[test]
  fn interfaces_generics_worked_example_linked_and_run() {
    // AC1: real executed proof both specializations (`max$$Money`,
    // `max$$Distance`) are independently correct and take *opposite*
    // control-flow branches inside what is, textually, one shared
    // function body — `m1.compare_to(m2) = -250 < 0` falls through to
    // `return b` (750 cents); `d1.compare_to(d2) = 58 >= 0` takes
    // `return a` (100 meters).
    assert_eq!(compile_link_run(INTERFACES_GENERICS_EXAMPLE), "750\n100\n");
  }

  #[test]
  fn exactly_two_specializations_are_emitted_no_bare_generic_symbol() {
    // AC2/AC3: real proof from the compiled object file's own raw
    // bytes — the mangled symbols `max$$Money`/`max$$Distance` are
    // present (each object-format symbol table entry is a
    // null-terminated string, so the exact-match search below can't be
    // fooled by `max$$Money` itself containing `max` as a
    // *non-null-terminated* prefix), and the bare `max` symbol is never
    // emitted at all — implicitly, since the worked-example test above
    // already proves each specialization independently produces the
    // correct result over a different field layout, every call resolves
    // to a direct, statically-resolved LLVM `call`, not an indirect one.
    let program = emerald_parser::parse(INTERFACES_GENERICS_EXAMPLE).expect("should parse");
    let out = std::env::temp_dir().join("emerald_codegen_interfaces_generics_symbols.o");
    compile_to_object(&program, &out).expect("should compile");
    let obj_bytes = std::fs::read(&out).expect("object file should exist");
    std::fs::remove_file(&out).ok();
    assert!(
      obj_bytes
        .windows(b"max$$Money\0".len())
        .any(|w| w == b"max$$Money\0"),
      "missing the `max$$Money` specialization symbol"
    );
    assert!(
      obj_bytes
        .windows(b"max$$Distance\0".len())
        .any(|w| w == b"max$$Distance\0"),
      "missing the `max$$Distance` specialization symbol"
    );
    assert!(
      !obj_bytes.windows(b"max\0".len()).any(|w| w == b"max\0"),
      "the bare generic name must never be a callable LLVM symbol"
    );
  }

  #[test]
  fn generic_call_with_an_unresolvable_concrete_type_errors_not_panics() {
    // AC4: an unsupported/malformed shape reaching codegen directly — no
    // argument position bound to `T` is a plain local/`ClassName.new(
    // ...)` literal `collect_generic_specializations`/`build_call_expr`
    // can resolve a concrete type from (both arguments are themselves
    // call results) — defensively returns a descriptive `Err`, not a
    // panic, same AC standard as every prior codegen plan.
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Int64\n    @cents - other.cents\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\ndef make_money(cents: Int64) -> Money\n  Money.new(cents)\nend\n\nboom: Money = max(make_money(500), make_money(750))\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let out =
      std::env::temp_dir().join("emerald_codegen_unresolvable_generic_call_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 43 (nullable types and safe navigation).

  const NULLABLE_WORKED_EXAMPLE: &str = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\ndef greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";

  #[test]
  fn nullable_worked_example_linked_and_run() {
    // Real executed proof, combining all three leaves: a real non-null
    // `Greeter` pointer (`greet(1)`) and a real null pointer
    // (`greet(2)`, `find_greeter`'s "not found" path) both round-trip
    // correctly through a `Greeter?`-typed local without crashing or
    // misreading the wrong bit pattern; `g&.shout` actually skips
    // calling `shout` on the null receiver and actually performs it on
    // the non-null one, merging both paths via a real LLVM `phi`; and
    // `||=` narrows `message` so `return message` type-checks and
    // prints the right string on both paths.
    assert_eq!(
      compile_link_run(NULLABLE_WORKED_EXAMPLE),
      "ada!\nnobody here\n"
    );
  }

  const AND_ASSIGN_UPGRADE_EXAMPLE: &str = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\ndef upgrade(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  g &&= Greeter.new(\"upgraded\")\n  message: String? = g&.shout\n  message ||= \"still nobody\"\n  return message\nend\n\nputs upgrade(1)\nputs upgrade(2)\n";

  #[test]
  fn and_assign_upgrade_worked_example_linked_and_run() {
    // Real executed proof of `&&=`'s both branches: `upgrade(1)`'s `g`
    // is non-nil, so `&&=` actually assigns (`g` becomes the
    // `"upgraded"` Greeter, whose `shout` produces `"upgraded!"`);
    // `upgrade(2)`'s `g` is nil, so `&&=` is genuinely skipped (`g`
    // stays nil — a real is-nil-guarded conditional store, not an
    // unconditional one), `g&.shout` short-circuits, and `||=` supplies
    // the default.
    assert_eq!(
      compile_link_run(AND_ASSIGN_UPGRADE_EXAMPLE),
      "upgraded!\nstill nobody\n"
    );
  }

  #[test]
  fn plan_25_nil_example_still_compiles_and_runs_unchanged() {
    // Regression: a bare `Nil`-typed variable's own `== nil` comparison
    // (plan 25, unrelated to `T?`) still flows through its own
    // `(ValKind::Nil, ValKind::Nil)` codegen case, not the new
    // `Ptr`/`Str`-vs-`Nil` null-pointer-test case this plan adds.
    assert_eq!(compile_link_run(NIL_EXAMPLE), "1\n");
  }

  // Plan 44 (symbols).

  const SYMBOLS_WORKED_EXAMPLE: &str = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar\n  puts 1\nelse\n  puts 0\nend\n";

  #[test]
  fn symbols_worked_example_linked_and_run() {
    // AC1: real executed proof — a `Hash[Symbol, Int64]` built, read,
    // and overwritten via symbol keys, plus symbol equality being a
    // real, cheap comparison (not a string compare).
    assert_eq!(compile_link_run(SYMBOLS_WORKED_EXAMPLE), "82\n100\n1\n0\n");
  }

  #[test]
  fn two_distinct_occurrences_of_the_same_spelling_produce_the_same_id() {
    // AC2: real proof the whole-program collector assigns one ID per
    // distinct spelling, not one per occurrence (the opposite,
    // disclosed choice from plan 19's deliberate *non*-dedup of String
    // literals) — one `:dup` inside a function body, one at top level;
    // `==` between them must be true.
    let src = "def make_dup -> Symbol\n  :dup\nend\n\nif make_dup() == :dup\n  puts 1\nelse\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  #[test]
  fn a_symbol_used_only_in_unreachable_code_still_compiles_cleanly() {
    // AC3: the collector walks every declared function/method
    // regardless of call reachability — `never_called` is declared but
    // never invoked from any top-level statement, yet the program must
    // still compile (no reachability analysis exists to skip it).
    let src = "def never_called -> Symbol\n  :dead_code\nend\n\nputs 42\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn ordering_comparison_on_two_symbols_errors_not_panics() {
    // AC4: reached only by directly constructing the AST (the parser
    // has no `<` grammar path that type-checks two `Symbol` operands
    // into codegen — sema's `Compare` arm doesn't special-case `Symbol`
    // at all, so this is a real codegen-level defensive check) —
    // returns a descriptive `Err`, not a panic.
    let program = Program {
      items: vec![Item::Stmt(syn(Stmt::Expr(syn(Expr::Compare(
        Box::new(syn(Expr::SymbolLit("foo".to_string()))),
        CompareOp::Lt,
        Box::new(syn(Expr::SymbolLit("bar".to_string()))),
      )))))],
    };
    let out = std::env::temp_dir().join("emerald_codegen_symbol_ordering_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 45 (stdlib strings and I/O).

  #[test]
  fn string_length_upcase_downcase_linked_and_run() {
    let src = "name: String = \"chicago\"\nputs name.length\nputs name.upcase\nshout: String = \"CHICAGO\"\nputs shout.downcase\n";
    assert_eq!(compile_link_run(src), "7\nCHICAGO\nchicago\n");
  }

  #[test]
  fn string_strip_linked_and_run() {
    let src = "raw: String = \"  padded  \"\nputs raw.strip\n";
    assert_eq!(compile_link_run(src), "padded\n");
  }

  #[test]
  fn string_to_i_and_to_f_linked_and_run() {
    let src = "digits: String = \"42abc\"\nputs digits.to_i\ndecimal: String = \"3.5\"\nputs decimal.to_f\n";
    assert_eq!(compile_link_run(src), "42\n3.5\n");
  }

  #[test]
  fn string_indexing_and_slicing_linked_and_run() {
    let src = "s: String = \"hello\"\nputs s[1]\nputs s.slice(1, 3)\n";
    assert_eq!(compile_link_run(src), "e\nell\n");
  }

  #[test]
  fn split_and_split_count_linked_and_run() {
    let src = "s: String = \"a b c\"\nputs s.split_count(\" \")\nwords: Array[String] = s.split(\" \")\nputs words[0]\nputs words[1]\nputs words[2]\n";
    assert_eq!(compile_link_run(src), "3\na\nb\nc\n");
  }

  #[test]
  fn empty_separator_split_is_a_disclosed_runtime_abort() {
    // AC4 of leaf-string-split: a documented, expected failure mode —
    // non-zero exit, a diagnostic on stderr — not silently looping
    // forever or producing garbage.
    let src = "s: String = \"abc\"\nn: Int64 = s.split_count(\"\")\nputs n\n";
    let output = compile_link_run_full(src, &[], None, None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("separator must not be empty"));
  }

  const PLAN_45_WORKED_EXAMPLE: &str = "input: String = \"hello world foo\"\nupper: String = input.upcase\nFile.write(\"plan45_demo.txt\", upper)\nreadback: String = File.read(\"plan45_demo.txt\")\nputs readback\nn: Int64 = readback.split_count(\" \")\nputs n\nwords: Array[String] = readback.split(\" \")\ni: Int64 = 0\nwhile i < n\n  puts words[i]\n  i += 1\nend\n";

  #[test]
  fn plan_45_worked_example_linked_and_run() {
    // Run from a fresh temporary working directory (`plan45_demo.txt`
    // is created, not pre-existing) — real proof `.upcase` transforms
    // the string, `File.write` persists it, `File.read` reads the same
    // bytes back, `.split_count`/`.split` genuinely tokenize the
    // reconstituted string, and existing `while`/indexing machinery
    // iterates the result correctly.
    let dir = fresh_temp_dir("plan45_worked_example");
    let output = compile_link_run_full(PLAN_45_WORKED_EXAMPLE, &[], None, Some(&dir));
    assert!(output.status.success());
    assert_eq!(
      String::from_utf8_lossy(&output.stdout),
      "HELLO WORLD FOO\n3\nHELLO\nWORLD\nFOO\n"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn file_read_on_a_nonexistent_path_is_a_disclosed_runtime_abort() {
    let dir = fresh_temp_dir("plan45_missing_file");
    let src = "content: String = File.read(\"/nonexistent/path/plan45.txt\")\nputs content\n";
    let output = compile_link_run_full(src, &[], None, Some(&dir));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not open"));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn argv_and_argc_with_two_fixed_arguments_linked_and_run() {
    let src = "puts ARGC\nputs ARGV[0]\nputs ARGV[1]\n";
    assert_eq!(
      compile_link_run_with_args(src, &["foo", "bar"]),
      "2\nfoo\nbar\n"
    );
  }

  #[test]
  fn argc_with_zero_extra_arguments_linked_and_run() {
    let src = "puts ARGC\n";
    assert_eq!(compile_link_run_with_args(src, &[]), "0\n");
  }

  #[test]
  fn gets_composes_with_strip_linked_and_run() {
    let src = "line: String = gets()\nputs line.strip\n";
    assert_eq!(compile_link_run_with_stdin(src, "hello\n"), "hello\n");
  }

  #[test]
  fn gets_at_immediate_eof_returns_empty_string_not_a_hang() {
    let src = "line: String = gets()\nputs line.length\n";
    assert_eq!(compile_link_run_with_stdin(src, ""), "0\n");
  }

  // Plan 47 (REPL and test framework).

  /// Like `compile_link_run`, but parses with a chosen source name —
  /// needed for `assert`'s own `"{name}:{line}"` location capture,
  /// which `compile_link_run`'s hardcoded `emerald_parser::parse` (name
  /// `"<source>"`) can't produce.
  fn compile_link_run_named(src: &str, name: &str) -> String {
    let program = emerald_parser::parse_named(src, name).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!(
      "{}_{:?}_named",
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
    assert!(output.status.success(), "compiled binary should exit 0");

    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();

    String::from_utf8_lossy(&output.stdout).into_owned()
  }

  #[test]
  fn assert_true_compiles_links_and_runs_with_no_output() {
    let src = "def f() -> Void\n  assert(true)\nend\n\nf()\n";
    assert_eq!(compile_link_run(src), "");
  }

  #[test]
  fn assert_false_raises_an_assertion_error_carrying_the_real_file_and_line() {
    // AC2: `assert(false)` on line 3 (1-based) of a file named `t.em`.
    let src = "def f() -> Void\n  begin\n    assert(false)\n  rescue AssertionError => e\n    puts e.message\n  end\nend\n\nf()\n";
    assert_eq!(compile_link_run_named(src, "t.em"), "t.em:3\n");
  }

  #[test]
  fn assert_eq_mismatch_prints_expected_then_actual_before_the_rescue() {
    // AC3.
    let src = "begin\n  assert_eq(3, 1 + 1)\nrescue AssertionError => e\n  puts 999\nend\n";
    assert_eq!(compile_link_run(src), "expected:\n3\nbut got:\n2\n999\n");
  }

  #[test]
  fn assert_eq_match_raises_nothing() {
    let src = "begin\n  assert_eq(2, 1 + 1)\n  puts 1\nrescue AssertionError => e\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  #[test]
  fn a_plain_hello_em_style_program_never_gets_an_assertionerror_symbol() {
    // AC5: `AssertionError` injection only fires for a program that
    // actually uses `test`/`assert`/`assert_eq` — real, checked proof
    // against the emitted object file's own symbol table, not just "it
    // still compiles".
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("no_assertion_symbol");
    let obj_path = dir.join("hello.o");
    compile_to_object(&program, &obj_path).expect("should compile");
    let nm = Command::new("nm")
      .arg(&obj_path)
      .output()
      .expect("failed to run nm");
    let symbols = String::from_utf8_lossy(&nm.stdout);
    assert!(
      !symbols.contains("AssertionError"),
      "a plain hello.em-style program's object file must carry no AssertionError symbol, found:\n{symbols}"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  /// Compiles `src` via `compile_test_harness` (not `compile_to_object`
  /// — the point of this whole leaf) and actually runs the produced
  /// binary. Returns the reported test count plus the full process
  /// `Output` (so a non-zero exit code, the disclosed AC1 negative
  /// case, can be asserted rather than forced to succeed).
  fn compile_test_harness_link_run(src: &str, name: &str) -> (usize, std::process::Output) {
    let program = emerald_parser::parse_named(src, name).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!(
      "{}_{:?}_testharness",
      std::process::id(),
      std::thread::current().id()
    );
    let obj_path = dir.join(format!("emerald_codegen_th_{unique}.o"));
    let bin_path = dir.join(format!("emerald_codegen_th_bin_{unique}"));

    let count = compile_test_harness(&program, &obj_path).expect("should compile the test harness");

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

    (count, output)
  }

  const MATH_TEST_EXAMPLE: &str = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n\ntest \"addition is broken on purpose\" do\n  assert_eq(3, 1 + 1)\nend\n";

  #[test]
  fn math_test_worked_example_prints_the_exact_worked_transcript_and_exits_1() {
    let (count, output) = compile_test_harness_link_run(MATH_TEST_EXAMPLE, "math_test.em");
    assert_eq!(count, 2);
    assert_eq!(
      String::from_utf8_lossy(&output.stdout),
      "PASS: addition works\nexpected:\n3\nbut got:\n2\nFAIL: addition is broken on purpose: math_test.em:6\npassed:\n1\nfailed:\n1\n"
    );
    assert_eq!(output.status.code(), Some(1));
  }

  #[test]
  fn a_file_where_every_test_passes_exits_0() {
    let src = "test \"one\" do\n  assert_eq(1, 1)\nend\n\ntest \"two\" do\n  assert(true)\nend\n";
    let (count, output) = compile_test_harness_link_run(src, "all_green_test.em");
    assert_eq!(count, 2);
    assert_eq!(
      String::from_utf8_lossy(&output.stdout),
      "PASS: one\nPASS: two\npassed:\n2\nfailed:\n0\n"
    );
    assert_eq!(output.status.code(), Some(0));
  }

  #[test]
  fn compiling_a_test_file_via_the_ordinary_path_is_a_described_rejection() {
    // AC4 (leaf-test-runner) / AC5 (leaf-test-intrinsics): the ordinary
    // `emerald <file>`-shaped `compile_to_object` path rejects a
    // `Program` containing a `test` block with a specific, named
    // message — not a panic, not a silent no-op compile.
    let program = emerald_parser::parse(MATH_TEST_EXAMPLE).expect("should parse");
    let dir = fresh_temp_dir("reject_test_block");
    let obj_path = dir.join("out.o");
    let err = compile_to_object(&program, &obj_path)
      .expect_err("a top-level test block must be rejected by the ordinary compile path");
    assert!(err.contains("only valid under `emerald test`"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
  }
}
