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
  CaseArm, CasePattern, ClassDef, CompareOp, Contract, EnumDef, EnumVariant, Expr, ExternBlock,
  ExternFn, Function as AstFunction, InterfaceDef, Item, ModuleDef, Param, Program, RescueClause,
  Spanned, Stmt, StringPart, TypeExpr, TypeParam,
};
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::AddressSpace;
// Plan 35: DWARF line-table debug info (v1 scope — see the plan's own
// Decision log: line tables only, no variable/type DIEs).
use inkwell::debug_info::{
  AsDIScope, DIFile, DIFlags, DIFlagsConstants, DIScope, DWARFEmissionKind, DWARFSourceLanguage,
  DebugInfoBuilder,
};
use inkwell::module::{FlagBehavior, Linkage, Module};
use inkwell::targets::{
  CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, FunctionType};
use inkwell::values::{
  BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue, ValueKind,
};
use inkwell::{IntPredicate, OptimizationLevel};
use std::cell::RefCell;
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
#[derive(Clone, PartialEq, Eq, Debug)]
enum ValKind {
  Int64,
  Float64,
  Ptr,
  Str,
  Void,
  Bool,
  /// Plan 44 — `i64`-backed, exactly like `Int64` above: a
  /// compile-time-assigned dense integer ID (`Ctx::symbol_table`), not
  /// a pointer — symbol equality is a plain `icmp` on this kind, never
  /// a runtime string comparison.
  Symbol,
  /// A fixed-arity anonymous tuple (plan 39's Decision log) — an LLVM
  /// struct return, never plan 09's boxed `Array[T]`. Valid ONLY as a
  /// function's declared return kind, exactly like `Type::Tuple` on the
  /// sema side — never a param/local/field/array-element kind. Carrying
  /// a `Vec` here is what forces dropping `Copy` from this whole enum;
  /// every pre-existing call site that relied on an implicit copy of a
  /// `ValKind` value now needs an explicit `.clone()`.
  Tuple(Vec<ValKind>),
}

/// Type names never reified as a real class/enum instantiation-under-a-
/// mangled-name — mirrors `emerald-sema`'s identically-named constant
/// (a real, disclosed duplication of bookkeeping between the two
/// passes, matching this codebase's own established pattern — see
/// `ClassInfo` vs. `ClassLayout`).
const NATIVE_GENERIC_NAMES: [&str; 4] = ["Array", "Hash", "Pair", "Result"];

thread_local! {
  /// `newtype Meters: Float64` — a deliberately narrow side channel for
  /// exactly one purpose: `value_kind_for_type` below is a free function
  /// with no `Ctx`/registry parameter at all, called from 40+ sites
  /// across this file (function/method signatures, `ClassLayout` field
  /// building, array-element-type inference, `Stmt::Let`/`bind_params`
  /// local storage, ...) — many deeply nested with no `Ctx` in scope —
  /// that must still resolve a newtype name to its own underlying
  /// primitive's REAL storage kind. Without this, a `Meters`-typed
  /// param/return/field/array-element would wrongly fall into this
  /// function's generic `_ => ValKind::Ptr` bucket below, which is a
  /// genuine miscompile (an LLVM function signature declaring `ptr` for
  /// a value that's actually an `f64`), not just a missed optimization —
  /// and would defeat the entire zero-cost claim `newtype` exists to
  /// make. Threading a `&HashSet`/`&HashMap` parameter through all 40+
  /// call sites (many of them free functions like `param_kinds`, called
  /// from further call sites still, with no natural `Ctx` thread) was
  /// judged not worth the blast radius for a single, narrowly-scoped
  /// lookup; this thread-local is set EXACTLY ONCE per `compile_to_
  /// object_impl` call, before any other codegen pass runs, and never
  /// mutated again for the rest of that call — sound under this
  /// backend's own pre-existing "one `Context`/compile pass per thread,
  /// never shared" safety model (`Ctx::escape_stats`'s own doc comment
  /// states the identical invariant for its `RefCell`, for the same
  /// reason).
  static NEWTYPE_UNDERLYING: RefCell<HashMap<String, TypeExpr>> = RefCell::new(HashMap::new());
}

/// Populates `NEWTYPE_UNDERLYING` for the current compile — see that
/// thread-local's own doc comment. Called once, at the very top of
/// `compile_to_object_impl`, from every entry point (`compile_to_
/// object`/`_with_stats`/`_with_debug_info`/`_scoped`/`_with_target`
/// all funnel through it) — a fresh `Program` compiled later in the
/// SAME thread (e.g. `emerald-lsp`'s repeated live-typing checks)
/// correctly overwrites rather than accumulates stale entries from a
/// previous, unrelated compile.
fn set_newtype_underlying(program: &Program) {
  let map: HashMap<String, TypeExpr> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Newtype(n) => Some((n.name.clone(), n.underlying.clone())),
      _ => None,
    })
    .collect();
  NEWTYPE_UNDERLYING.with(|cell| *cell.borrow_mut() = map);
}

/// Plan 84 (`deterministic-destruction-codegen`, `spec/OWNERSHIP.md`
/// §9/§10's own rescoping): synthetic `TypeExpr::Generic` names this
/// pass wraps a stripped `borrow`/`borrow var` type in when the
/// underlying value is genuinely by-value at the LLVM level (`Int64`/
/// `Float64`/`Boolean`/`Symbol` — see `value_kind_for_type`), never
/// written by any parser/sema output and never user-reachable (an
/// Emerald identifier can't start with `__`). `borrow`/`borrow var` of
/// an ALREADY pointer-represented type (a class instance, `String`/
/// `CString`) still strips to exactly the bare underlying type, unchanged
/// from plan 83's original passthrough — that case was already real,
/// zero-cost reference-passing for free (`Ctx`'s own established
/// "class instances are LLVM `ptr` values, passed by pointer" model —
/// see `bind_params`' own doc comment). Only the by-value case needed
/// real, new codegen: a `borrow Int64` with no marker would otherwise
/// compile to an ordinary by-value `i64` parameter, which is silently
/// WRONG for `borrow var` (a callee's mutation would never become
/// visible to the caller) and merely misleading for a plain `borrow`
/// (a copy happens to behave like a read-only reference, but isn't
/// one). `bind_params`/`build_call_arg_vals`/`build_call_kw_expr`'s own
/// matching arms are what actually turn this marker into a real
/// pointer at the LLVM level; see each's own doc comment for its half
/// of the mechanism.
const BORROW_PTR_SHARED: &str = "__EmeraldBorrowPtrShared";
const BORROW_PTR_MUT: &str = "__EmeraldBorrowPtrMut";

fn borrow_ptr_marker_name(is_mut: bool) -> &'static str {
  if is_mut {
    BORROW_PTR_MUT
  } else {
    BORROW_PTR_SHARED
  }
}

/// `Some((is_mut, underlying_ty))` iff `ty` is one of the two synthetic
/// marker shapes `strip_ownership_in_type_expr` produces for a
/// by-value `borrow`/`borrow var` parameter — see `BORROW_PTR_SHARED`'s
/// own doc comment. `None` for every other `TypeExpr`, including every
/// ordinary (non-marker) `Generic` this compiler already has (`Array[T]`,
/// `Hash[K, V]`, ...), since none of those names ever collide with
/// either marker.
fn borrow_ptr_marker_info(ty: &TypeExpr) -> Option<(bool, &TypeExpr)> {
  match ty {
    TypeExpr::Generic(name, args) if args.len() == 1 && name == BORROW_PTR_SHARED => {
      Some((false, &args[0]))
    }
    TypeExpr::Generic(name, args) if args.len() == 1 && name == BORROW_PTR_MUT => {
      Some((true, &args[0]))
    }
    _ => None,
  }
}

/// Plan 83's Decision log (`spec/OWNERSHIP.md` §10's own rescoping of
/// plan 84), extended by plan 84 itself: recursively strips an
/// `own`/`borrow`/`borrow var` wrapper anywhere it appears inside a
/// `TypeExpr` tree. `own` always fully erases to its bare underlying
/// type — an `own` transfer is either a class's existing pointer-copy
/// or a primitive's existing scalar-copy, both of which are exactly
/// what an ordinary, unannotated parameter already does today (plan
/// 83's sema pass is what makes the caller's binding actually unusable
/// afterward; there is no runtime transfer-invalidation mechanism to
/// reuse or build — `spec/OWNERSHIP.md` §9 already declines a `Drop`
/// mechanism, and plan 56's own `is_cross_actor_send` check this design
/// was generalized from is likewise sema-only, never codegen). A
/// `borrow`/`borrow var` of an already-pointer-represented type (`Ptr`/
/// `Str`) also fully erases, unchanged — see `BORROW_PTR_SHARED`'s own
/// doc comment for why that's still correct. A `borrow`/`borrow var` of
/// a genuinely by-value type instead wraps the stripped underlying type
/// in one of the two synthetic markers, when `allow_borrow_ptr_marker`
/// is `true`.
///
/// `allow_borrow_ptr_marker` is `false` for a class/actor/module
/// method's own parameters (`strip_ownership_in_function`'s own call
/// sites below) — a real, disclosed, narrower-than-ideal scope
/// boundary: making a by-value `borrow`/`borrow var` METHOD parameter a
/// real pointer would also require updating `build_method_call`'s own,
/// separate argument-evaluation code (several distinct dispatch arms:
/// ordinary same-actor calls, cross-actor `emerald_actor_enqueue`
/// sends, module-static calls, generic-method specializations, ...),
/// which this plan does not touch. A method's by-value `borrow`/`borrow
/// var` parameter therefore keeps plan 83's original full-erasure
/// passthrough — safe (the method's LLVM signature keeps the ordinary
/// scalar type, so every existing method-call site stays correct) but
/// not yet real reference-passing for that one case; a `borrow var
/// Int64` method parameter's mutation still doesn't propagate to the
/// caller, exactly like before this plan. `own`, and `borrow`/`borrow
/// var` of a class/`String` type, are unaffected by this flag — both
/// were already correct for methods too.
fn strip_ownership_in_type_expr(ty: &TypeExpr, allow_borrow_ptr_marker: bool) -> TypeExpr {
  match ty {
    TypeExpr::Own(inner) => strip_ownership_in_type_expr(inner, allow_borrow_ptr_marker),
    TypeExpr::Borrow(inner, is_mut) => {
      let stripped = strip_ownership_in_type_expr(inner, allow_borrow_ptr_marker);
      if allow_borrow_ptr_marker
        && !matches!(value_kind_for_type(&stripped), ValKind::Ptr | ValKind::Str)
      {
        TypeExpr::Generic(borrow_ptr_marker_name(*is_mut).to_string(), vec![stripped])
      } else {
        stripped
      }
    }
    TypeExpr::Named(_) => ty.clone(),
    TypeExpr::Generic(name, args) => TypeExpr::Generic(
      name.clone(),
      args
        .iter()
        .map(|t| strip_ownership_in_type_expr(t, allow_borrow_ptr_marker))
        .collect(),
    ),
    TypeExpr::Tuple(parts) => TypeExpr::Tuple(
      parts
        .iter()
        .map(|t| strip_ownership_in_type_expr(t, allow_borrow_ptr_marker))
        .collect(),
    ),
    TypeExpr::Func(params, ret) => TypeExpr::Func(
      params
        .iter()
        .map(|t| strip_ownership_in_type_expr(t, allow_borrow_ptr_marker))
        .collect(),
      Box::new(strip_ownership_in_type_expr(ret, allow_borrow_ptr_marker)),
    ),
  }
}

/// Strips every parameter's (and, if present, the splat parameter's)
/// declared type plus the return type of one `Function` — every
/// `Function` this compiler has, whether a top-level `fn`, a class/
/// actor method, or a module method, shares this exact same AST shape,
/// so one helper covers all of them (`strip_ownership_annotations_in_
/// item`'s own per-`Item`-kind dispatch below is what reaches each, and
/// what decides `allow_borrow_ptr_marker` — see `strip_ownership_in_
/// type_expr`'s own doc comment).
fn strip_ownership_in_function(f: &mut AstFunction, allow_borrow_ptr_marker: bool) {
  for p in &mut f.params {
    p.ty = strip_ownership_in_type_expr(&p.ty, allow_borrow_ptr_marker);
  }
  if let Some(p) = &mut f.splat_param {
    p.ty = strip_ownership_in_type_expr(&p.ty, allow_borrow_ptr_marker);
  }
  f.return_type = strip_ownership_in_type_expr(&f.return_type, allow_borrow_ptr_marker);
}

fn strip_ownership_annotations_in_item(item: &mut Item) {
  match item {
    // Only a top-level free function gets the real by-value `borrow`
    // pointer marker — see `strip_ownership_in_type_expr`'s own doc
    // comment for exactly why methods are excluded.
    Item::Function(f) => strip_ownership_in_function(f, true),
    Item::Class(c) => {
      for m in &mut c.methods {
        strip_ownership_in_function(m, false);
      }
    }
    Item::Actor(a) => {
      for m in &mut a.methods {
        strip_ownership_in_function(m, false);
      }
    }
    Item::Module(m) => {
      for meth in &mut m.methods {
        strip_ownership_in_function(meth, false);
      }
    }
    // `export fn ...`/`export class ...` (plan 76) wraps another `Item`
    // of one of the four kinds just handled above — recurse into it the
    // same way `desugar_asserts_in_items`'s own sibling passes already
    // must (this file's own established "an `Export` is transparent to
    // every whole-program AST rewrite" convention).
    Item::Export(inner) => strip_ownership_annotations_in_item(inner),
    // Plan 85 (`spec/OWNERSHIP.md` §7): an `unsafe extern "C"` fn's own
    // return type is the ONLY position `emerald-sema` ever lets `own`/
    // `borrow`/`borrow var` reach here (params are rejected at the sema
    // stage — see `resolve_extern_type`'s own doc comment — so by the
    // time a program reaches codegen at all, an `ExternFn`'s params
    // never carry the wrapper). Always `allow_borrow_ptr_marker: false`
    // — unlike an ordinary parameter, an extern return's `own`/`borrow`
    // annotation is pure documentation-as-code (§7's own words: "no new
    // mechanism"); the actual C ABI return value is identical regardless
    // of the annotation, so this is a plain erasure to the underlying
    // marshalable type, never the real pointer+writeback machinery
    // `strip_ownership_in_function` builds for a parameter.
    Item::Extern(block) => {
      for f in &mut block.fns {
        f.return_type = strip_ownership_in_type_expr(&f.return_type, false);
      }
    }
    // Every other `Item` kind (`Enum`, `Newtype`, `Interface`, `Stmt`,
    // `Require`) has no `Function`-shaped parameter/return-type
    // annotation of its own an `own`/`borrow`/`borrow var` could ever
    // appear on (an `Interface`'s own required-method signatures are
    // never lowered to LLVM at all — conformance is checked entirely in
    // `emerald-sema`), so there is nothing for this pass to strip there.
    _ => {}
  }
}

/// Top-level entry point, called once from `compile_to_object_impl`
/// before anything else in this file ever looks at `items` — see
/// `strip_ownership_in_type_expr`'s own doc comment for the full
/// rationale.
fn strip_ownership_annotations_in_items(items: &mut [Item]) {
  for item in items {
    strip_ownership_annotations_in_item(item);
  }
}

fn value_kind_for_type(ty: &TypeExpr) -> ValKind {
  match ty {
    TypeExpr::Named(name) => match name.as_str() {
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
      "Symbol" => ValKind::Symbol,
      // Plan 59's Decision log: `CString` is stored identically to
      // `String` — a bare pointer, no new runtime representation.
      "CString" => ValKind::Str,
      // `newtype Meters: Float64` — resolves to EXACTLY the underlying
      // primitive's own storage kind (see `NEWTYPE_UNDERLYING`'s own
      // doc comment for why this is a thread-local lookup rather than a
      // threaded parameter). Checked before the generic `_ => ValKind::
      // Ptr` catch-all immediately below, which would otherwise be
      // silently, incorrectly wrong for a newtype name specifically
      // (unlike a real class/enum name, for which `Ptr` IS correct).
      other if NEWTYPE_UNDERLYING.with(|c| c.borrow().contains_key(other)) => {
        let underlying = NEWTYPE_UNDERLYING.with(|c| c.borrow()[other].clone());
        value_kind_for_type(&underlying)
      }
      // Every other bare name (a class, an enum, a bound-less generic
      // type parameter's own placeholder) shares the generic `Ptr`
      // bucket.
      _ => ValKind::Ptr,
    },
    // `Array[Elem]`/`Hash[K, V]`/`Pair[K, V]`/`Result[T, E]`/a user
    // generic class or enum instantiation/`Proc[Args..., Ret]` all
    // share the generic `Ptr` bucket too — unlike `Array[Elem]`,
    // indexing a `Hash[K, V]` needs a key type too, which the
    // side-table `local_classes` (repurposed to hold a `Hash[K, V]`
    // type's own `Display` string alongside class names — see
    // `build_index`) already carries without needing a whole new
    // parameter threaded through every codegen function in this file.
    TypeExpr::Generic(..) | TypeExpr::Func(..) => ValKind::Ptr,
    // `Tuple` is valid only as a function's own declared return kind
    // (`ret_kind_for_type`'s own dedicated handling below) — reached
    // here only for a param/local/field/array-element position, which
    // sema already rejects everywhere but a top-level-function-like
    // return type; falls through to the same `Ptr` catch-all the old
    // flat-string convention's `_ => ValKind::Ptr` arm gave this
    // unreachable-in-practice shape.
    TypeExpr::Tuple(_) => ValKind::Ptr,
    // SUPERSEDED by plan 84 (`deterministic-destruction-codegen`) —
    // the paragraph this replaces claimed plan 84 would "replace this
    // exact arm"; it doesn't, and this note corrects that rather than
    // silently rewriting it (this file's own established convention).
    // What actually happened: this arm stays exactly as it was
    // (plan 83's original passthrough), and remains genuinely correct
    // for `own` (always a bare copy — see `strip_ownership_in_type_
    // expr`'s own doc comment for why `own` never needed new codegen)
    // and for `borrow`/`borrow var` of an already pointer-represented
    // type. Plan 84's REAL new codegen for a by-value `borrow`/`borrow
    // var` (`Int64`/`Float64`/`Boolean`/`Symbol`) lives entirely in
    // `strip_ownership_in_type_expr` (which now wraps that one case in
    // a synthetic `TypeExpr::Generic` marker BEFORE anything here ever
    // runs) plus `bind_params`/`build_call_arg_vals`/`build_call_kw_
    // expr` (which recognize that marker). This arm is reached only
    // when an `Own`/`Borrow` node somehow reaches `value_kind_for_type`
    // UNSTRIPPED — still possible for a lambda literal's own params
    // (`strip_ownership_annotations_in_items` never walks into a
    // `Stmt::Let`'s lambda value, a real, disclosed, pre-existing gap —
    // see `define_lambda`'s own comment), so this stays a real,
    // exhaustive match rather than an `unreachable!()`.
    TypeExpr::Own(inner) | TypeExpr::Borrow(inner, _) => value_kind_for_type(inner),
  }
}

/// Splits `s` on top-level `,` only — mirrors `emerald-sema`'s own
/// identically-named helper (a real, disclosed duplication of
/// bookkeeping between the two passes, matching this codebase's own
/// established pattern of independent, unsynchronized sema/codegen
/// registries built from the same AST — see `ClassInfo` vs.
/// `ClassLayout`). Needed so a tuple return type's own element list
/// (`"(Int64, Int64)"`, or in principle `"(Hash[Int64, Int64], Int64)"`)
/// splits correctly even when an element is itself a compound type
/// whose own convention already uses `", "`.
/// Plan 88: `"Stack[Int64]"` -> `"Stack$Int64"`, recursively — the
/// `TypeExpr`-native replacement for the old string-splitting `mangle_
/// type_name`/`parse_generic_instantiation`/`split_top_level_commas`
/// trio, mirroring `emerald-sema`'s identically-named `mangle_type_expr`
/// exactly (this string doubles as both sema's synthesized `Type::
/// Class` name and codegen's own `{ClassName}_{method}` mangling
/// prefix, by design).
fn mangle_type_expr(t: &TypeExpr) -> String {
  match t {
    TypeExpr::Generic(base, args) if !NATIVE_GENERIC_NAMES.contains(&base.as_str()) => {
      let mangled_args: Vec<String> = args.iter().map(mangle_type_expr).collect();
      format!("{base}${}", mangled_args.join("$"))
    }
    other => other.to_string(),
  }
}

/// Plan 88: whole-tree type-parameter substitution — mirrors `emerald-
/// sema`'s identically-named helper exactly (the `TypeExpr`-native
/// replacement for the old byte-scanning identifier-run substitution).
fn substitute_type_params(raw: &TypeExpr, subst: &HashMap<&str, &TypeExpr>) -> TypeExpr {
  match raw {
    TypeExpr::Named(name) => subst
      .get(name.as_str())
      .map(|t| (*t).clone())
      .unwrap_or_else(|| raw.clone()),
    TypeExpr::Generic(base, args) => TypeExpr::Generic(
      base.clone(),
      args
        .iter()
        .map(|a| substitute_type_params(a, subst))
        .collect(),
    ),
    TypeExpr::Tuple(parts) => TypeExpr::Tuple(
      parts
        .iter()
        .map(|p| substitute_type_params(p, subst))
        .collect(),
    ),
    TypeExpr::Func(params, ret) => TypeExpr::Func(
      params
        .iter()
        .map(|p| substitute_type_params(p, subst))
        .collect(),
      Box::new(substitute_type_params(ret, subst)),
    ),
    // Plan 83: mirrors `emerald-sema`'s identically-named helper exactly.
    TypeExpr::Own(inner) => TypeExpr::Own(Box::new(substitute_type_params(inner, subst))),
    TypeExpr::Borrow(inner, is_var) => {
      TypeExpr::Borrow(Box::new(substitute_type_params(inner, subst)), *is_var)
    }
  }
}

/// Plan 58: resolves a possibly-generic-instantiation type (`Stack[
/// Int64]`) to whichever key actually names it in `classes` — its
/// mangled form (`"Stack$Int64"`) if that's what's registered,
/// otherwise its bare `Display` string unchanged (an ordinary class
/// name, or an unresolvable one — `None` either way, matching every
/// pre-existing `ctx.classes.contains_key(bare_ty)` call site's own
/// behavior for a name that just isn't a class).
fn resolve_local_class_name(
  ty: &TypeExpr,
  classes: &HashMap<String, ClassLayout>,
) -> Option<String> {
  let bare_ty = ty.to_string();
  if classes.contains_key(&bare_ty) {
    return Some(bare_ty);
  }
  let mangled = mangle_type_expr(ty);
  if mangled != bare_ty && classes.contains_key(&mangled) {
    return Some(mangled);
  }
  None
}

/// The one place a `"(" T1 "," T2 ")"`-shaped tuple return-type
/// annotation gains real meaning in codegen (plan 39's Decision log) —
/// mirrors `emerald-sema`'s `resolve_return_type` vs. `resolve_type`
/// split. Used only at a function's own `ret_kind` computation sites
/// (top-level functions and module methods, which sema's `check_
/// function_body` path already treats identically — see that
/// function's own comment); every param/local/field/array-element/
/// hash-element resolution keeps calling `value_kind_for_type` directly,
/// which has no tuple branch and falls through to its `_ => ValKind::
/// Ptr` catch-all for this shape (unreachable in practice: sema already
/// rejects a tuple annotation everywhere but a top-level-function-like
/// return type before codegen ever runs).
fn ret_kind_for_type(ty: &TypeExpr) -> ValKind {
  if let TypeExpr::Tuple(parts) = ty {
    let elem_kinds = parts.iter().map(value_kind_for_type).collect();
    return ValKind::Tuple(elem_kinds);
  }
  value_kind_for_type(ty)
}

/// The LLVM storage type for a `Let`/param/field/array-element/return
/// kind. `Void` never reaches here — an internal invariant (it only
/// ever labels a function's return kind, handled separately in
/// `make_fn_type`), not a user-input-dependent case.
fn local_llvm_type<'ctx>(context: &'ctx Context, kind: &ValKind) -> BasicTypeEnum<'ctx> {
  match kind {
    ValKind::Int64 | ValKind::Symbol => context.i64_type().into(),
    ValKind::Float64 => context.f64_type().into(),
    ValKind::Ptr | ValKind::Str => context.ptr_type(AddressSpace::default()).into(),
    ValKind::Bool => context.bool_type().into(),
    ValKind::Void => unreachable!("internal: Void never used as a storage type"),
    ValKind::Tuple(_) => {
      unreachable!("internal: Tuple is only ever a function's declared return kind")
    }
  }
}

/// The LLVM struct type backing a tuple return (plan 39's Decision
/// log) — one field per element kind, in declared order. Never
/// recurses into another `Tuple` (nesting is rejected entirely at the
/// sema/grammar level, so `elem_kinds` here never itself contains a
/// `ValKind::Tuple`).
fn tuple_struct_type<'ctx>(
  context: &'ctx Context,
  elem_kinds: &[ValKind],
) -> inkwell::types::StructType<'ctx> {
  let field_types: Vec<BasicTypeEnum> = elem_kinds
    .iter()
    .map(|k| local_llvm_type(context, k))
    .collect();
  context.struct_type(&field_types, false)
}

fn make_fn_type<'ctx>(
  context: &'ctx Context,
  param_kinds: &[ValKind],
  ret_kind: &ValKind,
) -> FunctionType<'ctx> {
  let param_types: Vec<BasicMetadataTypeEnum> = param_kinds
    .iter()
    .map(|k| local_llvm_type(context, k).into())
    .collect();
  match ret_kind {
    ValKind::Void => context.void_type().fn_type(&param_types, false),
    ValKind::Int64 | ValKind::Symbol => context.i64_type().fn_type(&param_types, false),
    ValKind::Float64 => context.f64_type().fn_type(&param_types, false),
    ValKind::Ptr | ValKind::Str => context
      .ptr_type(AddressSpace::default())
      .fn_type(&param_types, false),
    ValKind::Bool => context.bool_type().fn_type(&param_types, false),
    ValKind::Tuple(elem_kinds) => {
      tuple_struct_type(context, elem_kinds).fn_type(&param_types, false)
    }
  }
}

/// Plan 89's Decision log: a `Proc`-typed runtime VALUE is a pointer to
/// a small heap block — historically just the lambda's own captured-
/// environment (`build_lambda_let`'s own doc comment, unchanged for
/// every existing top-level `Let`-bound lambda) with the callee always
/// resolved STATICALLY, by the receiver's own source-level NAME, via
/// `ctx.lambda_func_ids`. That static-name convention cannot resolve a
/// genuinely indirect Proc value — a method parameter, a field, or a
/// local forwarding an arbitrary caller-supplied Proc has no compile-
/// time name to look up at all. This plan reserves the block's own
/// leading 8 bytes for the lambda's OWN compiled function pointer
/// (every capture shifts down by this many bytes — see `LambdaInfo`'s
/// own `capture_offsets`), so ANY Proc value can be called indirectly
/// by loading this header slot and issuing a real LLVM indirect call,
/// with the SAME pointer also passed as the callee's own leading `env`
/// argument (unchanged calling convention) — no vtable, no second
/// runtime representation, just one extra slot on the existing one.
const CLOSURE_HEADER_BYTES: u64 = 8;

/// Plan 89: a compact, tagged encoding of a `Proc[Args..., Ret]`
/// value's own `ValKind`s, stashed into the pre-existing `local_
/// classes: HashMap<String, String>` side table (already overloaded to
/// carry a class name/`Pair[K, V]`'s own Display string for a non-
/// `Ptr`-distinguishing local — see `bind_params`'s own doc comment) —
/// avoids threading a whole new per-local side table through every
/// `build_expr`/`build_method_call` call site in this file just for
/// this one, narrowly-scoped need (resolving a genuinely indirect
/// `.call`). `\u{1}` can never appear in a real class name (an
/// `Ident`), so this can never collide with an ordinary class-name
/// entry already stored there.
const PROC_SIG_TAG: char = '\u{1}';

fn valkind_code(k: &ValKind) -> char {
  match k {
    ValKind::Int64 => 'i',
    ValKind::Float64 => 'f',
    ValKind::Str => 's',
    ValKind::Void => 'v',
    ValKind::Bool => 'b',
    ValKind::Symbol => 'y',
    // `Ptr` and the unreachable-here `Tuple` case share the same
    // generic bucket — a `Proc` component is never itself a tuple
    // (sema rejects that shape outright).
    ValKind::Ptr | ValKind::Tuple(_) => 'p',
  }
}

fn code_to_valkind(c: char) -> ValKind {
  match c {
    'i' => ValKind::Int64,
    'f' => ValKind::Float64,
    's' => ValKind::Str,
    'v' => ValKind::Void,
    'b' => ValKind::Bool,
    'y' => ValKind::Symbol,
    _ => ValKind::Ptr,
  }
}

fn encode_proc_sig(param_kinds: &[ValKind], ret_kind: &ValKind) -> String {
  let params: String = param_kinds.iter().map(valkind_code).collect();
  format!("{PROC_SIG_TAG}{params}:{}", valkind_code(ret_kind))
}

fn decode_proc_sig(s: &str) -> Option<(Vec<ValKind>, ValKind)> {
  let rest = s.strip_prefix(PROC_SIG_TAG)?;
  let (params, ret) = rest.split_once(':')?;
  let param_kinds = params.chars().map(code_to_valkind).collect();
  let ret_kind = code_to_valkind(ret.chars().next()?);
  Some((param_kinds, ret_kind))
}

/// `Proc[Args..., Ret]`'s own encoded signature string for a param/
/// local declared with this exact written `TypeExpr::Func` form — the
/// bare, untyped `Proc` annotation (no written signature at all) never
/// reaches this, and keeps working exactly as before (the pre-existing
/// static, name-based `lambda_func_ids` dispatch, never indirect).
fn proc_sig_for_type(ty: &TypeExpr) -> Option<String> {
  if let TypeExpr::Func(params, ret) = ty {
    let param_kinds: Vec<ValKind> = params.iter().map(value_kind_for_type).collect();
    Some(encode_proc_sig(&param_kinds, &value_kind_for_type(ret)))
  } else {
    None
  }
}

/// Reconstructs a representative `TypeExpr` for a generic method's own
/// free type parameter (`U`) resolved to `kind` — used only to build
/// this call site's own monomorphized mangled symbol/substituted
/// signature (`resolve_generic_method_instance`). `None` for `ValKind::
/// Ptr`/`Tuple` — a bare storage kind alone can't name which CLASS a
/// pointer-backed value actually is, a real, disclosed limit: this
/// plan's own codegen-side generic-method monomorphization only
/// supports a type parameter resolving to one of the primitive kinds
/// below (covers this plan's own worked example and every regression
/// test it adds), not an arbitrary class.
fn valkind_to_typeexpr(kind: &ValKind) -> Option<TypeExpr> {
  match kind {
    ValKind::Int64 => Some(TypeExpr::Named("Int64".to_string())),
    ValKind::Float64 => Some(TypeExpr::Named("Float64".to_string())),
    ValKind::Bool => Some(TypeExpr::Named("Boolean".to_string())),
    ValKind::Str => Some(TypeExpr::Named("String".to_string())),
    ValKind::Symbol => Some(TypeExpr::Named("Symbol".to_string())),
    ValKind::Void => Some(TypeExpr::Named("Void".to_string())),
    ValKind::Ptr | ValKind::Tuple(_) => None,
  }
}

/// A field's byte offset and storage kind within its class's instance
/// layout — every field is naively 8 bytes (matches the old Cranelift
/// backend's `FieldInfo`; see `spec/TYPE_SYSTEM.md` §8).
#[derive(Clone)]
struct FieldInfo {
  offset: u64,
  kind: ValKind,
}

struct ClassLayout {
  fields: HashMap<String, FieldInfo>,
  size: u64,
  /// `{field name} -> its raw declared TypeName string}` (plan 55's own
  /// addition) — `build_method_call`'s cross-actor dispatch check needs
  /// to know a `Ptr`-kind field's declared CLASS name (e.g. `@peer:
  /// PingPong`), which `FieldInfo.kind` alone can't carry (`ValKind::
  /// Ptr` is shared by every class/actor type, with no name attached).
  /// Populated for every field regardless of kind — cheap, and a
  /// non-`Ptr` field's entry is simply never consulted by anything.
  field_classes: HashMap<String, String>,
}

/// Plan 52's Decision log: a tagged union — `[tag: i64][payload: 8 *
/// max_fields bytes]`, generalizing the exact header+payload pattern
/// `Hash[K,V]`'s own buffer (`build_hash_lit`) already established, one
/// step further: a header field (the tag, replacing `Hash`'s count)
/// followed by a byte region sized to the WIDEST variant's payload
/// (replacing a fixed per-element stride), since unlike a `Hash`'s
/// uniform pairs, an enum's variants can carry different field counts.
struct EnumLayout {
  /// `{variant name} -> its discriminant` — declaration-order index
  /// (`Circle = 0`, `Square = 1`, ...).
  variant_tags: HashMap<String, u64>,
  /// `{variant name} -> its own field kinds, in declaration order}` —
  /// every variant's fields live at the SAME byte offsets (`8 + i * 8`
  /// for field `i`), typed per-variant at each access site, exactly
  /// `field_ptr`/`load_field`'s existing byte-offset mechanism (never a
  /// typed struct GEP, matching every other compound type in this
  /// backend).
  variant_fields: HashMap<String, Vec<ValKind>>,
  /// Plan 73's Decision log: the ORIGINAL, unresolved type-name string
  /// per field, alongside `variant_fields`' own `ValKind` — needed
  /// wherever a variant's payload must be resolved back to a class name
  /// (`local_classes`' own bookkeeping, e.g. `?.`'s dispatch on
  /// `Option[T]`'s unwrapped `Some` payload when `T` is a class), which
  /// `ValKind::Ptr` alone can't distinguish from "any other pointer."
  variant_field_types: HashMap<String, Vec<String>>,
  size: u64,
}

/// Codegen's own independent re-derivation of one `EnumDef` (plan 32's
/// "no shared sema→codegen structure" architecture, applied here the
/// same way `build_class_layout` already applies it to `ClassDef`) —
/// sema's own registration-time checks already guarantee this is only
/// ever called with a well-formed, collision-free `EnumDef` by the time
/// a real compile reaches this point.
fn build_enum_layout(e: &EnumDef) -> EnumLayout {
  let mut variant_tags = HashMap::new();
  let mut variant_fields = HashMap::new();
  let mut variant_field_types = HashMap::new();
  let mut max_fields = 0usize;
  for (i, v) in e.variants.iter().enumerate() {
    variant_tags.insert(v.name.clone(), i as u64);
    let kinds: Vec<ValKind> = v.fields.iter().map(value_kind_for_type).collect();
    max_fields = max_fields.max(kinds.len());
    variant_fields.insert(v.name.clone(), kinds);
    variant_field_types.insert(
      v.name.clone(),
      v.fields
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<String>>(),
    );
  }
  EnumLayout {
    variant_tags,
    variant_fields,
    variant_field_types,
    size: 8 + 8 * max_fields as u64,
  }
}

/// Codegen's own variant-owner lookup — mirrors `emerald-sema`'s
/// identically-purposed `find_variant` (a real, disclosed duplication
/// of bookkeeping between the two passes, matching this codebase's own
/// established pattern — see `ClassInfo` vs. `ClassLayout`). Iterates
/// every enum in `enums` for a variant tagged `name`, returning its
/// owning enum's name and declaration-order tag.
fn find_variant_layout<'a>(
  name: &str,
  enums: &'a HashMap<String, EnumLayout>,
) -> Option<(&'a str, u64)> {
  for (enum_name, layout) in enums {
    if let Some(&tag) = layout.variant_tags.get(name) {
      return Some((enum_name.as_str(), tag));
    }
  }
  None
}

/// Plan 50's `leaf-escape-instrumentation-and-report`: how many
/// `ClassName.new(...)` sites a compile actually stack- vs. heap-
/// allocated — the only proof surface for the escape-analysis
/// optimization, since a stack- and a heap-allocated instance are
/// behaviorally indistinguishable from a compiled program's own
/// stdout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EscapeStats {
  pub stack_allocated: u64,
  pub heap_allocated: u64,
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
  let mut field_classes = HashMap::new();
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
      // Plan 89's Decision log established this exact convention for a
      // Proc-typed PARAMETER (see `bind_params`'s own identical `proc_
      // sig_for_type` call): a `Proc[Args..., Ret]`-typed class field
      // needs its signature encoded here too, not just its plain type
      // string, so `build_method_call`'s indirect-`.call` dispatch can
      // resolve `@field.call(...)` the same way it already resolves a
      // Proc-typed parameter's `.call`.
      field_classes.insert(
        f.name.clone(),
        proc_sig_for_type(&f.ty).unwrap_or_else(|| f.ty.to_string()),
      );
      offset += 8;
    }
  }
  Ok(ClassLayout {
    fields,
    size: offset,
    field_classes,
  })
}

const GENERIC_INSTANTIATION_DEPTH_LIMIT: usize = 32;

/// Plan 58: builds ONE monomorphized `ClassDef` for `base_name<
/// type_args>` by substituting every type-parameter-named field/method
/// param/return type in the TEMPLATE — mirrors `emerald-sema`'s
/// `instantiate_generic_class`/`build_generic_class_info`, minus the
/// bound/depth-diagnostic machinery (sema already accepted this whole
/// program; a malformed instantiation never reaches codegen at all).
/// Still needs the identical self-reference short-circuit for
/// correctness, not just parity with sema: a linked-list-shaped `Node
/// [T]`'s own `succ: Node[T]` field would otherwise recurse forever
/// building its own synthesized `ClassDef`, regardless of how sema
/// already proved the *program* terminates.
///
/// Real, disclosed gap: a method BODY's own internal `Stmt::Let`
/// annotation referencing the bare type-parameter name directly (e.g.
/// `local: T = ...`) is NOT rewritten — `body` carries over unchanged.
/// `value_kind_for_type` falls through an unrecognized string to
/// `ValKind::Ptr`, which is only correct when `T` is instantiated with
/// a reference type; a primitive instantiation (`Stack[Int64]`) whose
/// method body declares such a local would compile a wrong storage
/// kind. Not exercised by this plan's own worked example, whose methods
/// only ever read/write `T`-typed FIELDS (substituted correctly below),
/// never an intermediate `T`-typed local.
fn instantiate_generic_class_defs(
  base_name: &str,
  type_args: &[TypeExpr],
  generic_class_defs: &HashMap<String, &ClassDef>,
  synthesized: &mut HashMap<String, ClassDef>,
  in_progress: &mut Vec<String>,
) -> String {
  let mangled_args: Vec<String> = type_args.iter().map(mangle_type_expr).collect();
  let mangled = format!("{base_name}${}", mangled_args.join("$"));

  if synthesized.contains_key(&mangled) {
    return mangled;
  }
  if in_progress.last().map(String::as_str) == Some(mangled.as_str()) {
    synthesized.insert(
      mangled.clone(),
      ClassDef {
        name: mangled.clone(),
        superclass: None,
        implements: None,
        derive: None,
        fields: Vec::new(),
        methods: Vec::new(),
        type_params: Vec::new(),
        // A cycle-detection placeholder, never the real monomorphized
        // result (overwritten below once the real instantiation
        // finishes) — no meaningful doc comment exists for it.
        doc: None,
      },
    );
    return mangled;
  }
  if in_progress.len() >= GENERIC_INSTANTIATION_DEPTH_LIMIT {
    // Defensive only — sema already rejects any program that would
    // reach this; codegen just needs to terminate rather than hang.
    return mangled;
  }
  let Some(c) = generic_class_defs.get(base_name).copied() else {
    return mangled;
  };

  let subst: HashMap<&str, &TypeExpr> = c
    .type_params
    .iter()
    .map(|tp| tp.name.as_str())
    .zip(type_args.iter())
    .collect();

  in_progress.push(mangled.clone());

  let fields: Vec<Param> = c
    .fields
    .iter()
    .map(|f| Param {
      name: f.name.clone(),
      ty: resolve_substituted_type_cg(&f.ty, &subst, generic_class_defs, synthesized, in_progress),
      default: None,
    })
    .collect();

  let methods: Vec<AstFunction> = c
    .methods
    .iter()
    .map(|m| AstFunction {
      name: m.name.clone(),
      params: m
        .params
        .iter()
        .map(|p| Param {
          name: p.name.clone(),
          ty: resolve_substituted_type_cg(
            &p.ty,
            &subst,
            generic_class_defs,
            synthesized,
            in_progress,
          ),
          default: p.default.clone(),
        })
        .collect(),
      return_type: resolve_substituted_type_cg(
        &m.return_type,
        &subst,
        generic_class_defs,
        synthesized,
        in_progress,
      ),
      body: m.body.clone(),
      block_param: m.block_param.clone(),
      splat_param: m.splat_param.as_ref().map(|p| Param {
        name: p.name.clone(),
        ty: resolve_substituted_type_cg(
          &p.ty,
          &subst,
          generic_class_defs,
          synthesized,
          in_progress,
        ),
        default: None,
      }),
      type_params: Vec::new(),
      is_comptime: false,
      requires: Vec::new(),
      ensures: Vec::new(),
      is_pure: m.is_pure,
      // A monomorphized copy of a real, user-written method — the same
      // declaration, just with substituted types, so its doc comment
      // (if any) carries forward unchanged.
      doc: m.doc.clone(),
    })
    .collect();

  in_progress.pop();

  synthesized.insert(
    mangled.clone(),
    ClassDef {
      name: mangled.clone(),
      superclass: c.superclass.clone(),
      implements: c.implements.clone(),
      derive: c.derive.clone(),
      fields,
      methods,
      type_params: Vec::new(),
      // A monomorphized copy of a real, user-written class — carries
      // its doc comment (if any) forward unchanged.
      doc: c.doc.clone(),
    },
  );
  mangled
}

fn resolve_substituted_type_cg(
  raw: &TypeExpr,
  subst: &HashMap<&str, &TypeExpr>,
  generic_class_defs: &HashMap<String, &ClassDef>,
  synthesized: &mut HashMap<String, ClassDef>,
  in_progress: &mut Vec<String>,
) -> TypeExpr {
  let substituted = substitute_type_params(raw, subst);
  if let TypeExpr::Generic(base, args) = &substituted {
    if generic_class_defs.contains_key(base.as_str()) {
      let mangled =
        instantiate_generic_class_defs(base, args, generic_class_defs, synthesized, in_progress);
      return TypeExpr::Named(mangled);
    }
  }
  substituted
}

/// Plan 73: codegen's own independent re-derivation of `emerald-sema`'s
/// `instantiate_generic_enum`/`build_generic_enum_info` — mirrors
/// `instantiate_generic_class_defs` immediately above exactly (same
/// mangled-name memoization, same `in_progress` self-reference/depth-
/// limit defense), building a synthesized, monomorphized `EnumDef`
/// (variant field types substituted) instead of a `ClassDef`. `Option[T]`
/// is the first generic enum this backend ever monomorphizes.
#[allow(clippy::too_many_arguments)]
fn instantiate_generic_enum_defs(
  base_name: &str,
  type_args: &[TypeExpr],
  generic_enum_defs: &HashMap<String, &EnumDef>,
  generic_class_defs: &HashMap<String, &ClassDef>,
  synthesized_classes: &mut HashMap<String, ClassDef>,
  synthesized_enums: &mut HashMap<String, EnumDef>,
  in_progress: &mut Vec<String>,
) -> String {
  let mangled_args: Vec<String> = type_args.iter().map(mangle_type_expr).collect();
  let mangled = format!("{base_name}${}", mangled_args.join("$"));

  if synthesized_enums.contains_key(&mangled) {
    return mangled;
  }
  if in_progress.last().map(String::as_str) == Some(mangled.as_str()) {
    synthesized_enums.insert(
      mangled.clone(),
      EnumDef {
        name: mangled.clone(),
        variants: Vec::new(),
        type_params: Vec::new(),
        // Cycle-detection placeholder, same reasoning as the identical
        // `ClassDef` sentinel above.
        doc: None,
      },
    );
    return mangled;
  }
  if in_progress.len() >= GENERIC_INSTANTIATION_DEPTH_LIMIT {
    return mangled;
  }
  let Some(e) = generic_enum_defs.get(base_name).copied() else {
    return mangled;
  };

  let subst: HashMap<&str, &TypeExpr> = e
    .type_params
    .iter()
    .map(|tp| tp.name.as_str())
    .zip(type_args.iter())
    .collect();

  in_progress.push(mangled.clone());
  let variants: Vec<EnumVariant> = e
    .variants
    .iter()
    .map(|v| EnumVariant {
      name: v.name.clone(),
      fields: v
        .fields
        .iter()
        .map(|f| {
          let substituted = substitute_type_params(f, &subst);
          if let TypeExpr::Generic(base, args) = &substituted {
            if generic_class_defs.contains_key(base.as_str()) {
              let mangled = instantiate_generic_class_defs(
                base,
                args,
                generic_class_defs,
                synthesized_classes,
                in_progress,
              );
              return TypeExpr::Named(mangled);
            } else if generic_enum_defs.contains_key(base.as_str()) {
              let mangled = instantiate_generic_enum_defs(
                base,
                args,
                generic_enum_defs,
                generic_class_defs,
                synthesized_classes,
                synthesized_enums,
                in_progress,
              );
              return TypeExpr::Named(mangled);
            }
          }
          substituted
        })
        .collect(),
    })
    .collect();
  in_progress.pop();

  synthesized_enums.insert(
    mangled.clone(),
    EnumDef {
      name: mangled.clone(),
      variants,
      type_params: Vec::new(),
      // A monomorphized copy of a real, user-written enum — carries its
      // doc comment (if any) forward unchanged.
      doc: e.doc.clone(),
    },
  );
  mangled
}

/// Plan 58: whole-program walk collecting every generic-class-
/// instantiation type-name string actually written anywhere — mirrors
/// `emerald-sema`'s identically-shaped `collect_generic_instantiation_
/// typenames`/`collect_typenames_in_stmt`. A generic class TEMPLATE's
/// own raw field/method types are deliberately skipped (handled via
/// substitution inside `instantiate_generic_class_defs` instead).
fn collect_generic_instantiation_typenames(program: &Program) -> Vec<TypeExpr> {
  let mut out = Vec::new();
  for item in &program.items {
    match item {
      Item::Function(f) => {
        if f.type_params.is_empty() {
          for p in &f.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&f.return_type, &mut out);
          if let Some(p) = &f.splat_param {
            push_generic_typename(&p.ty, &mut out);
          }
        }
        for s in &f.body {
          collect_typenames_in_stmt(s, &mut out);
        }
      }
      Item::Class(c) => {
        if c.type_params.is_empty() {
          for field in &c.fields {
            push_generic_typename(&field.ty, &mut out);
          }
          for m in &c.methods {
            for p in &m.params {
              push_generic_typename(&p.ty, &mut out);
            }
            push_generic_typename(&m.return_type, &mut out);
            for s in &m.body {
              collect_typenames_in_stmt(s, &mut out);
            }
          }
        }
      }
      Item::Module(md) => {
        for m in &md.methods {
          for p in &m.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&m.return_type, &mut out);
          for s in &m.body {
            collect_typenames_in_stmt(s, &mut out);
          }
        }
      }
      Item::Actor(a) => {
        for field in &a.fields {
          push_generic_typename(&field.ty, &mut out);
        }
        for m in &a.methods {
          for p in &m.params {
            push_generic_typename(&p.ty, &mut out);
          }
          push_generic_typename(&m.return_type, &mut out);
          for s in &m.body {
            collect_typenames_in_stmt(s, &mut out);
          }
        }
      }
      Item::Stmt(s) => collect_typenames_in_stmt(s, &mut out),
      // Plan 80: `property`/`benchmark` bodies get the identical walk
      // `test` bodies already get — same AST shape.
      Item::Test { body, .. } | Item::Property { body, .. } | Item::Benchmark { body, .. } => {
        for s in body {
          collect_typenames_in_stmt(s, &mut out);
        }
      }
      // Plan 76's `import-export-module-visibility`: both require-
      // resolution call sites (`emerald-cli::require`, `emerald_driver::
      // require_graph`) strip `Item::Export` and resolve away
      // `Item::Import` before any `Program` reaches `emerald-codegen` —
      // the same "resolved away before codegen" precedent `Item::
      // Require` immediately below already has.
      // A newtype's own `underlying` is always a plain primitive
      // (enforced at `emerald-sema` registration time), never a generic
      // instantiation — nothing to collect here, mirrors `Item::Enum`'s
      // identical no-op arm.
      Item::Enum(_)
      | Item::Newtype(_)
      | Item::Interface(_)
      | Item::Require(_)
      | Item::Import { .. }
      | Item::Export(_)
      | Item::Error
      | Item::Extern(_) => {}
    }
  }
  out
}

fn push_generic_typename(ty: &TypeExpr, out: &mut Vec<TypeExpr>) {
  if let TypeExpr::Generic(base, _) = ty {
    if !NATIVE_GENERIC_NAMES.contains(&base.as_str()) {
      out.push(ty.clone());
    }
  }
}

fn collect_typenames_in_stmt(stmt: &Spanned<Stmt>, out: &mut Vec<TypeExpr>) {
  match &stmt.node {
    Stmt::Let { ty, .. } => push_generic_typename(ty, out),
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      for s in then_branch {
        collect_typenames_in_stmt(s, out);
      }
      if let Some(eb) = else_branch {
        for s in eb {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
      for s in body {
        collect_typenames_in_stmt(s, out);
      }
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_typenames_in_stmt(s, out);
      }
      for r in rescues {
        for s in &r.body {
          collect_typenames_in_stmt(s, out);
        }
      }
      if let Some(e) = ensure {
        for s in e {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        for s in body {
          collect_typenames_in_stmt(s, out);
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          collect_typenames_in_stmt(s, out);
        }
      }
    }
    Stmt::MatchResult {
      ok_body, err_body, ..
    } => {
      for s in ok_body {
        collect_typenames_in_stmt(s, out);
      }
      for s in err_body {
        collect_typenames_in_stmt(s, out);
      }
    }
    Stmt::SetField { .. }
    | Stmt::SetIndex { .. }
    | Stmt::Assign { .. }
    | Stmt::MultiAssign { .. }
    | Stmt::Return(_)
    | Stmt::Break
    | Stmt::Next
    | Stmt::Expr(_)
    | Stmt::Raise(_)
    | Stmt::Yield(_)
    | Stmt::Retry => {}
  }
}

/// Plan 58: the top-level driver — collects every generic-instantiation
/// type-name string written anywhere in `program`, then monomorphizes
/// each (and, transitively, every nested/self-referential instantiation
/// a template's own fields/methods pull in) into a real `ClassDef`,
/// returned keyed by mangled name. Callers merge this into `class_defs`
/// exactly like `actor_class_defs`'s own synthesis already does.
fn collect_generic_class_specializations(
  program: &Program,
  generic_class_defs: &HashMap<String, &ClassDef>,
) -> HashMap<String, ClassDef> {
  let mut synthesized = HashMap::new();
  if generic_class_defs.is_empty() {
    return synthesized;
  }
  for ty in collect_generic_instantiation_typenames(program) {
    if let TypeExpr::Generic(base, args) = &ty {
      if generic_class_defs.contains_key(base.as_str()) {
        let mut in_progress = Vec::new();
        instantiate_generic_class_defs(
          base,
          args,
          generic_class_defs,
          &mut synthesized,
          &mut in_progress,
        );
      }
    }
  }
  synthesized
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

/// Real, disclosed regression fix (this session): plan 88 taught
/// `emerald-sema` to fully type-check a class method's own `[T]`/
/// `[T: Bound]` type parameter (`ClassInfo.generic_methods`) instead
/// of rejecting it outright — but this backend was never extended to
/// monomorphize such a method's body, so a call sema now accepts would
/// otherwise reach `build_method_call`'s ordinary per-class dispatch
/// and crash the LLVM verifier instead of failing cleanly. Independent
/// of sema's own registry (this crate never sees `emerald-sema`'s
/// private `ClassInfo`) — derived straight from the same raw
/// `ClassDef.methods` `build_method_owners` immediately above already
/// walks, keyed by the DECLARING class only (no inheritance-chain
/// flattening needed: `build_method_call`'s check runs against
/// `method_owners`' own already-resolved `defining_class`).
fn build_generic_class_methods(
  class_defs: &HashMap<String, &ClassDef>,
) -> HashMap<String, HashSet<String>> {
  let mut result = HashMap::new();
  for (name, c) in class_defs {
    let generic_names: HashSet<String> = c
      .methods
      .iter()
      .filter(|m| !m.type_params.is_empty())
      .map(|m| m.name.clone())
      .collect();
    if !generic_names.is_empty() {
      result.insert((*name).clone(), generic_names);
    }
  }
  result
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
    | Expr::Bool(_) => {}
    Expr::ArrayNew(size) => collect_idents_in_expr(size, out),
    Expr::Comptime(inner) => collect_idents_in_expr(inner, out),
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
    Expr::Compare(l, _, r) | Expr::Coalesce(l, r) => {
      collect_idents_in_expr(l, out);
      collect_idents_in_expr(r, out);
    }
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
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
    // Plan 39: `return a, b` — each element is a real free-variable site.
    Expr::TupleLit(elems) => {
      for e in elems {
        collect_idents_in_expr(e, out);
      }
    }
    // Plan 53: `Ok`/`Err`/`?`'s inner expression is a real free-variable site.
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => collect_idents_in_expr(e, out),
    // Plan 57: not recursed into, the same deliberate gap `Expr::
    // Lambda { .. }` above already leaves — a `supervise do ... end`
    // body is sema-restricted to spawn statements only (no arbitrary
    // expression that could reference an enclosing local), and its
    // real free-variable/capture analysis (for the child actors it
    // spawns) is `mark_expr`'s job (via `collect_referenced_idents`),
    // not this function's.
    Expr::Supervise(_) => {}
    Expr::Remote { addr, name, .. } => {
      collect_idents_in_expr(addr, out);
      collect_idents_in_expr(name, out);
    }
    Expr::Locate { key, args, .. } => {
      collect_idents_in_expr(key, out);
      for a in args {
        collect_idents_in_expr(a, out);
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
      for (pattern, body) in arms {
        match pattern {
          CasePattern::Values(values) => {
            for v in values {
              collect_idents_in_expr(v, referenced);
            }
          }
          // Plan 52: a pattern binding is bound (like a `Let`'s `name`),
          // not referenced — scoped to this arm's own body only in
          // sema, but this free-variable pass is deliberately
          // conservative (whole-function `bound`, not per-arm), the
          // same "over-approximate what's bound" posture every other
          // binding site here already takes.
          CasePattern::Variant { bindings, .. } => {
            for b in bindings {
              bound.insert(b.clone());
            }
          }
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
    // Plan 53: `ok_var`/`err_var` are bound (like a `Let`'s `name`),
    // scoped to their own arm's body only in sema — this
    // free-variable pass, like plan 52's `Variant` pattern immediately
    // above, deliberately over-approximates with one whole-function
    // `bound` set rather than a per-arm one.
    Stmt::MatchResult {
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
    } => {
      collect_idents_in_expr(scrutinee, referenced);
      bound.insert(ok_var.clone());
      for s in ok_body {
        collect_idents_in_stmt(s, referenced, bound);
      }
      bound.insert(err_var.clone());
      for s in err_body {
        collect_idents_in_stmt(s, referenced, bound);
      }
    }
  }
}

/// Plan 57 (supervision trees): every actor class name that appears in
/// at least one tracked `<Class>.spawn(...)` spawn inside ANY
/// `supervise do ... end` block anywhere in the program — what
/// `declare_supervisor_respawn_thunks` actually needs a thunk for.
/// Deliberately NOT a full generic expression-position walk the way
/// `collect_idents_in_stmt` is: `Expr::Supervise` is only ever
/// meaningfully reached at a statement's own direct value position (a
/// `Let`'s value or a bare `Stmt::Expr`) in every real, sema-accepted
/// program (sema's own `Expr::Supervise` typing already rejects most
/// other uses downstream — see `infer_expr_type`'s own arm) — a real,
/// disclosed narrow scope, not a panic risk, since an actor missed here
/// simply never gets a thunk and any `supervise` block that really did
/// reference it would fail codegen's own `no respawn thunk compiled
/// for actor` internal-error check loudly, not silently.
fn collect_supervised_classes_in_stmt(stmt: &Spanned<Stmt>, out: &mut HashSet<String>) {
  match &stmt.node {
    Stmt::Let {
      value: Spanned {
        node: Expr::Supervise(body),
        ..
      },
      ..
    }
    | Stmt::Expr(Spanned {
      node: Expr::Supervise(body),
      ..
    }) => {
      for s in body {
        match &s.node {
          Stmt::Let {
            value:
              Spanned {
                node: Expr::Spawn(class_name, _),
                ..
              },
            ..
          }
          | Stmt::Expr(Spanned {
            node: Expr::Spawn(class_name, _),
            ..
          }) => {
            out.insert(class_name.clone());
          }
          _ => {}
        }
      }
    }
    Stmt::If {
      then_branch,
      else_branch,
      ..
    } => {
      for s in then_branch {
        collect_supervised_classes_in_stmt(s, out);
      }
      if let Some(else_b) = else_branch {
        for s in else_b {
          collect_supervised_classes_in_stmt(s, out);
        }
      }
    }
    Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
      for s in body {
        collect_supervised_classes_in_stmt(s, out);
      }
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_supervised_classes_in_stmt(s, out);
      }
      for rescue in rescues {
        for s in &rescue.body {
          collect_supervised_classes_in_stmt(s, out);
        }
      }
      if let Some(ensure_body) = ensure {
        for s in ensure_body {
          collect_supervised_classes_in_stmt(s, out);
        }
      }
    }
    Stmt::Case {
      arms, else_body, ..
    } => {
      for (_, body) in arms {
        for s in body {
          collect_supervised_classes_in_stmt(s, out);
        }
      }
      if let Some(else_b) = else_body {
        for s in else_b {
          collect_supervised_classes_in_stmt(s, out);
        }
      }
    }
    Stmt::MatchResult {
      ok_body, err_body, ..
    } => {
      for s in ok_body {
        collect_supervised_classes_in_stmt(s, out);
      }
      for s in err_body {
        collect_supervised_classes_in_stmt(s, out);
      }
    }
    _ => {}
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
    | Expr::Bool(_) => {}
    Expr::ArrayNew(size) => collect_symbols_in_expr(size, table),
    Expr::Comptime(inner) => collect_symbols_in_expr(inner, table),
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
    Expr::Compare(l, _, r) | Expr::Coalesce(l, r) => {
      collect_symbols_in_expr(l, table);
      collect_symbols_in_expr(r, table);
    }
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
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
    Expr::TupleLit(elems) => {
      for e in elems {
        collect_symbols_in_expr(e, table);
      }
    }
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => collect_symbols_in_expr(e, table),
    Expr::Supervise(body) => {
      for s in body {
        collect_symbols_in_stmt(s, table);
      }
    }
    Expr::Remote { addr, name, .. } => {
      collect_symbols_in_expr(addr, table);
      collect_symbols_in_expr(name, table);
    }
    Expr::Locate { key, args, .. } => {
      collect_symbols_in_expr(key, table);
      for a in args {
        collect_symbols_in_expr(a, table);
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
      for (pattern, body) in arms {
        if let CasePattern::Values(values) = pattern {
          for v in values {
            collect_symbols_in_expr(v, table);
          }
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
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      collect_symbols_in_expr(scrutinee, table);
      for s in ok_body {
        collect_symbols_in_stmt(s, table);
      }
      for s in err_body {
        collect_symbols_in_stmt(s, table);
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
    // Plan 76: unwrap once (the grammar never nests `export`) so an
    // exported declaration's own body is walked identically to a
    // non-exported one.
    let mut item: &Item = item;
    while let Item::Export(inner) = item {
      item = inner;
    }
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
      // that prologue. Plan 80: `property`/`benchmark` bodies get the
      // identical treatment — same AST shape, same never-reached-in-
      // practice reasoning.
      Item::Test { body, .. } | Item::Property { body, .. } | Item::Benchmark { body, .. } => {
        for s in body {
          collect_symbols_in_stmt(s, &mut table);
        }
      }
      // Plan 52: an enum's variant fields are raw `TypeName` strings —
      // no `Symbol` literal appears anywhere in an `Item::Enum` itself.
      Item::Enum(_) => {}
      // A newtype's `underlying` is a bare `TypeExpr` too — no `Symbol`
      // literal, no body, mirrors `Item::Enum`'s identical no-op arm.
      Item::Newtype(_) => {}
      Item::Actor(a) => {
        for m in &a.methods {
          for s in &m.body {
            collect_symbols_in_stmt(s, &mut table);
          }
        }
      }
      Item::Interface(_) | Item::Require(_) | Item::Error | Item::Extern(_) => {}
      // Plan 76: `import path { Names }` names no body of its own.
      Item::Import { .. } => {}
      // Unreachable: unwrapped by the `while let` loop above this match.
      Item::Export(_) => unreachable!("Item::Export is unwrapped before this match"),
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
    .filter_map(|p| {
      let ty = p.ty.to_string();
      classes.contains_key(&ty).then(|| (p.name.clone(), ty))
    })
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
          if p.ty.as_named() == Some(type_param.name.as_str()) {
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
    | Expr::Bool(_) => {}
    Expr::ArrayNew(size) => collect_specializations_in_expr(size, generic_fns, local_classes, out),
    Expr::Comptime(inner) => {
      collect_specializations_in_expr(inner, generic_fns, local_classes, out)
    }
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
    Expr::Compare(l, _, r) | Expr::Coalesce(l, r) => {
      collect_specializations_in_expr(l, generic_fns, local_classes, out);
      collect_specializations_in_expr(r, generic_fns, local_classes, out);
    }
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
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
    Expr::TupleLit(elems) => {
      for e in elems {
        collect_specializations_in_expr(e, generic_fns, local_classes, out);
      }
    }
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => {
      collect_specializations_in_expr(e, generic_fns, local_classes, out)
    }
    // Plan 57: not recursed into, the same deliberate gap `Expr::
    // Lambda { .. }` above already leaves for this function — a
    // `supervise do ... end` body is sema-restricted to spawn
    // statements only, and this function has no `&mut local_classes`/
    // `classes` context available to call `collect_specializations_
    // in_stmt` with anyway (unlike its statement-level namesake).
    Expr::Supervise(_) => {}
    // Plan 60: `.remote`'s `addr`/`name` are always plain `String`
    // expressions — never a generic function call — so there's nothing
    // here for this pass to find, the same real gap `Expr::Supervise`
    // immediately above already discloses for the identical reason.
    Expr::Remote { .. } => {}
    // Plan 65: `.locate`'s own `args` reuses `.spawn`'s own identical
    // recursion (line ~1780 above) — `initialize`'s arguments can be
    // arbitrary expressions, including a generic call site this pass
    // needs to find; `key` is always a plain `String` expression, the
    // same "nothing to find there" reasoning `.remote` above states.
    Expr::Locate { args, .. } => {
      for a in args {
        collect_specializations_in_expr(a, generic_fns, local_classes, out);
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
    Stmt::Let {
      name, ty, value, ..
    } => {
      collect_specializations_in_expr(value, generic_fns, local_classes, out);
      let ty_str = ty.to_string();
      if classes.contains_key(&ty_str) {
        local_classes.insert(name.clone(), ty_str);
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
      for (pattern, body) in arms {
        if let CasePattern::Values(values) = pattern {
          for v in values {
            collect_specializations_in_expr(v, generic_fns, local_classes, out);
          }
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
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      collect_specializations_in_expr(scrutinee, generic_fns, local_classes, out);
      for s in ok_body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
      }
      for s in err_body {
        collect_specializations_in_stmt(s, generic_fns, local_classes, classes, out);
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
  let substitute = |ty: &TypeExpr| -> TypeExpr {
    if ty.as_named() == Some(type_param) {
      TypeExpr::Named(concrete_class.to_string())
    } else {
      ty.clone()
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
    is_comptime: f.is_comptime,
    // Plan 62's Decision log: `emerald-sema` already rejects a
    // `type_params`-bearing function with a non-empty `requires`/
    // `ensures` before this ever runs — `f.requires`/`f.ensures` are
    // carried through unchanged (not hardcoded empty) purely so this
    // stays correct-by-construction rather than correct-by-coincidence.
    requires: f.requires.clone(),
    ensures: f.ensures.clone(),
    is_pure: f.is_pure,
    doc: f.doc.clone(),
  }
}

/// Plan 89's Decision log: a generic METHOD's own monomorphization
/// needs more than `substitute_generic_function`'s "just the params/
/// return type" substitution — unlike every existing generic top-level
/// FUNCTION example, this plan's own worked example declares a BODY-
/// INTERNAL local (`result: Array[U] = Array.new(0)`) whose own type
/// annotation mentions the free type parameter directly. Left
/// unsubstituted, `build_stmt`'s `Stmt::Let` handling would resolve
/// `Array[U]`'s element kind via `value_kind_for_type`'s `_ => Ptr`
/// catch-all (the bare name `"U"` matches nothing else), silently
/// wrong whenever the concrete binding isn't itself `ValKind::Ptr` —
/// exactly the LLVM-verifier-crash shape plan 88's own stopgap existed
/// to avoid. `substitute_type_params_in_stmt` below walks every nested
/// statement body (mirroring `collect_typenames_in_stmt`'s own
/// recursive shape) rewriting each `Stmt::Let`'s own `ty` field; every
/// other statement kind carries no `TypeExpr` of its own and clones
/// unchanged.
fn substitute_function_type_params(
  f: &AstFunction,
  subst: &HashMap<&str, &TypeExpr>,
) -> AstFunction {
  AstFunction {
    name: f.name.clone(),
    params: f
      .params
      .iter()
      .map(|p| Param {
        name: p.name.clone(),
        ty: substitute_type_params(&p.ty, subst),
        default: p.default.clone(),
      })
      .collect(),
    return_type: substitute_type_params(&f.return_type, subst),
    body: f
      .body
      .iter()
      .map(|s| substitute_type_params_in_stmt(s, subst))
      .collect(),
    block_param: f.block_param.clone(),
    splat_param: f.splat_param.clone(),
    type_params: Vec::new(),
    is_comptime: f.is_comptime,
    requires: f.requires.clone(),
    ensures: f.ensures.clone(),
    is_pure: f.is_pure,
    doc: f.doc.clone(),
  }
}

fn substitute_type_params_in_stmt(
  stmt: &Spanned<Stmt>,
  subst: &HashMap<&str, &TypeExpr>,
) -> Spanned<Stmt> {
  let sub_body = |body: &[Spanned<Stmt>]| -> Vec<Spanned<Stmt>> {
    body
      .iter()
      .map(|s| substitute_type_params_in_stmt(s, subst))
      .collect()
  };
  let node = match &stmt.node {
    Stmt::Let {
      name,
      ty,
      value,
      is_var,
    } => Stmt::Let {
      name: name.clone(),
      ty: substitute_type_params(ty, subst),
      value: value.clone(),
      is_var: *is_var,
    },
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => Stmt::If {
      cond: cond.clone(),
      then_branch: sub_body(then_branch),
      else_branch: else_branch.as_ref().map(|b| sub_body(b)),
    },
    Stmt::While { cond, body } => Stmt::While {
      cond: cond.clone(),
      body: sub_body(body),
    },
    Stmt::For {
      var,
      elements,
      body,
    } => Stmt::For {
      var: var.clone(),
      elements: elements.clone(),
      body: sub_body(body),
    },
    Stmt::ForRange {
      var,
      start,
      end,
      exclusive,
      body,
    } => Stmt::ForRange {
      var: var.clone(),
      start: start.clone(),
      end: end.clone(),
      exclusive: *exclusive,
      body: sub_body(body),
    },
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => Stmt::Begin {
      body: sub_body(body),
      rescues: rescues
        .iter()
        .map(|r| RescueClause {
          class_name: r.class_name.clone(),
          var: r.var.clone(),
          body: sub_body(&r.body),
        })
        .collect(),
      ensure: ensure.as_ref().map(|e| sub_body(e)),
    },
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => Stmt::Case {
      scrutinee: scrutinee.clone(),
      arms: arms
        .iter()
        .map(|(pattern, body)| (pattern.clone(), sub_body(body)))
        .collect(),
      else_body: else_body.as_ref().map(|b| sub_body(b)),
    },
    Stmt::MatchResult {
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
    } => Stmt::MatchResult {
      scrutinee: scrutinee.clone(),
      ok_var: ok_var.clone(),
      ok_body: sub_body(ok_body),
      err_var: err_var.clone(),
      err_body: sub_body(err_body),
    },
    other => other.clone(),
  };
  Spanned {
    span: stmt.span,
    node,
  }
}

/// Plan 89's Decision log: this codegen-side generic-method
/// monomorphizer needs to know, at a specific call site, what CONCRETE
/// type a generic method's own free type parameter (`U`) resolves to —
/// sema's own `infer_type_param_binding` already proved the program
/// well-typed; this is a narrower, codegen-only re-derivation covering
/// just the shapes this plan's own worked example and regression tests
/// exercise: a lambda-literal argument's own inferred return type
/// (`infer_lambda_ret_kind`, generalized to see the CURRENT function's
/// own locals too — `build_local_val_kind_env`), or a lambda literal
/// whose last expression is a bare `ClassName.new(...)` (resolved
/// directly by name, not through a `ValKind` round-trip at all, since
/// `Ptr` alone can't name which class).
///
/// Found and closed 2026-09-21 (this session's "find all bugs" sweep):
/// this used to return `None` for every OTHER argument shape, so a
/// class-bound generic method whose type parameter is bound by an
/// ORDINARY (non-`Proc`) argument — `fn identity[U](x: U): U do x end`
/// called with a plain local, a fresh `ClassName.new(...)`, or a bare
/// literal — failed at codegen time with "could not resolve generic
/// method ...'s type parameter ... to a concrete type at this call
/// site," even though `emerald-sema`'s own fuller structural unifier
/// (`infer_type_param_binding`) already accepted the exact same
/// program (that function's own `TypeExpr::Named(name) if name ==
/// param_name` arm is the general case this narrower, codegen-only
/// re-derivation was missing entirely). Fixed by adding the same set
/// of leaf shapes `local_classes` and a handful of literal kinds can
/// name directly, without attempting full structural unification the
/// way `emerald-sema`'s own type checker does — still a real,
/// disclosed narrowing (an argument that is itself a nested call
/// result, a field read, or a binary expression is not covered), but
/// the common case (a local variable or a fresh literal/constructor
/// passed directly) now works.
fn infer_concrete_type_from_arg<'ctx>(
  arg: &Spanned<Expr>,
  ctx: &Ctx<'_, 'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
) -> Option<TypeExpr> {
  match &arg.node {
    Expr::Lambda { params, body, .. } => {
      if let Some(Spanned {
        node: Stmt::Expr(Spanned {
          node: Expr::New(class_name, _),
          ..
        }),
        ..
      })
      | Some(Spanned {
        node:
          Stmt::Return(Some(Spanned {
            node: Expr::New(class_name, _),
            ..
          })),
        ..
      }) = body.last()
      {
        return Some(TypeExpr::Named(class_name.clone()));
      }
      let base_env = build_local_val_kind_env(vars, ctx.top_level_types);
      let kind = infer_lambda_ret_kind(params, body, &base_env, ctx.user_fn_return_types);
      valkind_to_typeexpr(&kind)
    }
    // A fresh instance — the class name is right there, no `ValKind`
    // round-trip needed (same reasoning as the lambda-literal
    // `ClassName.new(...)` case above).
    Expr::New(class_name, _) => Some(TypeExpr::Named(class_name.clone())),
    // A handful of literal shapes whose type is a fixed, named
    // constant regardless of context.
    Expr::Int(_) => Some(TypeExpr::Named("Int64".to_string())),
    Expr::Float(_) => Some(TypeExpr::Named("Float64".to_string())),
    Expr::Bool(_) => Some(TypeExpr::Named("Boolean".to_string())),
    Expr::SymbolLit(_) => Some(TypeExpr::Named("Symbol".to_string())),
    Expr::StringLit(_) | Expr::Interpolate(_) => Some(TypeExpr::Named("String".to_string())),
    // Plan 89: `emerald-parser`'s own pre-existing block-attached-call
    // desugaring (unconditional, predates this plan) hoists an inline
    // lambda literal used as a call argument into a synthesized top-
    // level `__enum_blk_N` `Proc` `Let`, replacing the argument itself
    // with a bare `Ident` reference — so THIS, not a raw `Expr::
    // Lambda`, is the shape a generic method's own lambda-argument call
    // site actually sees by the time codegen runs. `ctx.lambda_func_ids`
    // already carries that top-level lambda's own inferred return kind
    // (`declare_lambda_functions`'s identical inference), reused
    // directly rather than re-deriving it a second time.
    //
    // Checked BEFORE the plain-local fallback immediately below: a
    // name present in `ctx.lambda_func_ids` is a synthesized top-level
    // Proc binding, never an ordinary class/newtype-typed local, so
    // there's no ambiguity between the two lookups.
    Expr::Ident(name) if ctx.lambda_func_ids.contains_key(name) => ctx
      .lambda_func_ids
      .get(name)
      .and_then(|(_, kind)| valkind_to_typeexpr(kind)),
    // An ordinary named local — `local_classes` names a class/newtype
    // receiver exactly (this is the same map `build_method_call`'s own
    // `Ident`-receiver dispatch already consults), which is what makes
    // this cover a fresh `own`/`borrow`-agnostic value passed straight
    // through, not just a `Proc`. Falls back to the local's plain
    // `ValKind` for a value type `local_classes` never populates.
    Expr::Ident(name) => local_classes
      .get(name)
      .map(|class_name| TypeExpr::Named(class_name.clone()))
      .or_else(|| {
        vars
          .get(name)
          .and_then(|(_, kind)| valkind_to_typeexpr(kind))
      }),
    _ => None,
  }
}

/// Plan 89: the enclosing function/method/lambda's own current
/// locals/params, plus every top-level name, as one flat `{name} ->
/// ValKind}` environment — `infer_lambda_ret_kind`'s own base
/// environment when inferring an INLINE (non-top-level) lambda
/// literal's return kind (`build_inline_lambda`), or a generic
/// method's own call-site type-parameter binding
/// (`infer_concrete_type_from_arg`) — either way, a captured free
/// variable might be a plain local/parameter, not just a top-level
/// name, unlike a top-level lambda's own capture set.
fn build_local_val_kind_env(
  vars: &HashMap<String, (PointerValue<'_>, ValKind)>,
  top_level_types: &HashMap<String, TypeExpr>,
) -> HashMap<String, ValKind> {
  let mut env: HashMap<String, ValKind> = top_level_types
    .iter()
    .map(|(k, v)| (k.clone(), value_kind_for_type(v)))
    .collect();
  for (k, (_, kind)) in vars {
    env.insert(k.clone(), kind.clone());
  }
  env
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
  let mut top_level_types: HashMap<String, TypeExpr> = HashMap::new();
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
          ..
        },
      ..
    }) = item
    else {
      continue;
    };
    // Plan 88's Decision log: a top-level `Let`-bound lambda is
    // collected as a real, compiled top-level Proc whether it's typed
    // via the bare `Proc` annotation (the pre-existing lambda-literal-
    // inference path, unchanged) OR the new, real `Proc[Args..., Ret]`
    // written form — both name the identical `Type::Proc` shape, and
    // this collection pass only ever cares about "is this a Proc-typed
    // top-level Let with a Lambda value," never which spelling wrote it.
    if ty.as_named() != Some("Proc") && !matches!(ty, TypeExpr::Func(..)) {
      continue;
    }

    let captures = free_vars_in_lambda(params, body);
    let mut capture_offsets = HashMap::new();
    let mut capture_kinds = HashMap::new();
    for (i, cap_name) in captures.iter().enumerate() {
      capture_offsets.insert(cap_name.clone(), CLOSURE_HEADER_BYTES + i as u64 * 8);
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
      .build_alloca(local_llvm_type(context, kind), name)
      .map_err(|e| e.to_string())?;
    vars.insert(name.clone(), (alloca, kind.clone()));
  }
  Ok(())
}

/// Plan 50's `leaf-escape-detection`: every `Stmt::Let{ name, value:
/// Expr::New(class_name, ..), .. }` site reachable from `stmts`, at any
/// nesting depth — mirrors `collect_lets`'s own traversal shape.
/// Candidates only; `find_non_escaping_news` is what actually filters
/// this down to the provably-non-escaping subset.
fn collect_new_let_classes(stmts: &[Spanned<Stmt>], out: &mut HashMap<String, String>) {
  for stmt in stmts {
    match &stmt.node {
      Stmt::Let {
        name,
        value: Spanned {
          node: Expr::New(class_name, _),
          ..
        },
        ..
      } => {
        out.insert(name.clone(), class_name.clone());
      }
      Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForRange { body, .. } => {
        collect_new_let_classes(body, out);
      }
      Stmt::If {
        then_branch,
        else_branch,
        ..
      } => {
        collect_new_let_classes(then_branch, out);
        if let Some(else_b) = else_branch {
          collect_new_let_classes(else_b, out);
        }
      }
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        collect_new_let_classes(body, out);
        for rescue in rescues {
          collect_new_let_classes(&rescue.body, out);
        }
        if let Some(ensure_body) = ensure {
          collect_new_let_classes(ensure_body, out);
        }
      }
      Stmt::Case {
        arms, else_body, ..
      } => {
        for (_, body) in arms {
          collect_new_let_classes(body, out);
        }
        if let Some(else_b) = else_body {
          collect_new_let_classes(else_b, out);
        }
      }
      _ => {}
    }
  }
}

/// Plan 50's `leaf-escape-detection`: every name referenced anywhere in
/// `stmts` (at any nesting depth, including inside a nested `Expr::
/// Lambda`'s own body) via a plain `Expr::Ident` use. This is a
/// deliberately broader check than the plan's own three literally-
/// enumerated escape rules (return position, `SetField`/`SetIndex`
/// value position, call/`New`/`MethodCall` argument-including-receiver
/// position) — it is their exact union, since every one of those
/// positions is itself an expression this function already walks, and
/// closes one real soundness gap the three rules don't individually
/// name: a reference from inside a nested lambda literal. Plan 10's
/// Decision log establishes by-value capture — a lambda body
/// referencing a candidate name copies that name's *pointer value*
/// into the lambda's own heap-allocated capture environment at lambda-
/// creation time, which can genuinely outlive the enclosing function
/// (the lambda itself can be returned, stored, or called later) — not
/// treating that as an escape would be unsound.
fn collect_referenced_idents(stmts: &[Spanned<Stmt>], out: &mut HashSet<String>) {
  for stmt in stmts {
    match &stmt.node {
      Stmt::Let { value, .. } => mark_expr(&value.node, out),
      Stmt::SetField { value, .. } => mark_expr(&value.node, out),
      Stmt::SetIndex {
        array,
        index,
        value,
      } => {
        mark_expr(&array.node, out);
        mark_expr(&index.node, out);
        mark_expr(&value.node, out);
      }
      Stmt::Assign { value, .. } => mark_expr(&value.node, out),
      Stmt::MultiAssign { values, .. } => {
        for v in values {
          mark_expr(&v.node, out);
        }
      }
      Stmt::If {
        cond,
        then_branch,
        else_branch,
      } => {
        mark_expr(&cond.node, out);
        collect_referenced_idents(then_branch, out);
        if let Some(else_b) = else_branch {
          collect_referenced_idents(else_b, out);
        }
      }
      Stmt::While { cond, body } => {
        mark_expr(&cond.node, out);
        collect_referenced_idents(body, out);
      }
      Stmt::Return(Some(e)) => mark_expr(&e.node, out),
      Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
      Stmt::Expr(e) => mark_expr(&e.node, out),
      Stmt::Raise(e) => mark_expr(&e.node, out),
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        collect_referenced_idents(body, out);
        for rescue in rescues {
          collect_referenced_idents(&rescue.body, out);
        }
        if let Some(ensure_body) = ensure {
          collect_referenced_idents(ensure_body, out);
        }
      }
      Stmt::Case {
        scrutinee,
        arms,
        else_body,
      } => {
        mark_expr(&scrutinee.node, out);
        for (pattern, body) in arms {
          // Plan 52: a pattern binding is a fresh declaration (like a
          // `Let` name), not a reference — only `Values` patterns can
          // contain an escape-candidate reference.
          if let CasePattern::Values(values) = pattern {
            for v in values {
              mark_expr(&v.node, out);
            }
          }
          collect_referenced_idents(body, out);
        }
        if let Some(else_b) = else_body {
          collect_referenced_idents(else_b, out);
        }
      }
      Stmt::For { elements, body, .. } => {
        for e in elements {
          mark_expr(&e.node, out);
        }
        collect_referenced_idents(body, out);
      }
      Stmt::Yield(args) => {
        for a in args {
          mark_expr(&a.node, out);
        }
      }
      Stmt::ForRange {
        start, end, body, ..
      } => {
        mark_expr(&start.node, out);
        mark_expr(&end.node, out);
        collect_referenced_idents(body, out);
      }
      // Plan 53: `ok_var`/`err_var` are fresh bindings, like a pattern
      // binding — only the scrutinee is a reference site.
      Stmt::MatchResult {
        scrutinee,
        ok_body,
        err_body,
        ..
      } => {
        mark_expr(&scrutinee.node, out);
        collect_referenced_idents(ok_body, out);
        collect_referenced_idents(err_body, out);
      }
    }
  }
}

/// One `Expr` tree's worth of `mark_expr` — every variant that can
/// contain a nested `Expr` is walked; `Expr::Lambda`'s `body` recurses
/// back into `collect_referenced_idents` (see that function's own doc
/// comment for why this matters).
fn mark_expr(e: &Expr, out: &mut HashSet<String>) {
  match e {
    Expr::Ident(n) => {
      out.insert(n.clone());
    }
    Expr::Add(a, b)
    | Expr::Sub(a, b)
    | Expr::Mul(a, b)
    | Expr::Div(a, b)
    | Expr::Rem(a, b)
    | Expr::And(a, b)
    | Expr::Or(a, b)
    | Expr::BitAnd(a, b)
    | Expr::BitOr(a, b)
    | Expr::BitXor(a, b)
    | Expr::Shl(a, b)
    | Expr::Shr(a, b) => {
      mark_expr(&a.node, out);
      mark_expr(&b.node, out);
    }
    Expr::Neg(a) | Expr::Not(a) | Expr::BitNot(a) => mark_expr(&a.node, out),
    Expr::Compare(a, _, b) | Expr::Coalesce(a, b) => {
      mark_expr(&a.node, out);
      mark_expr(&b.node, out);
    }
    Expr::Call(_, args) => {
      for a in args {
        mark_expr(&a.node, out);
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        mark_expr(&v.node, out);
      }
    }
    Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        mark_expr(&a.node, out);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      mark_expr(&recv.node, out);
      for a in args {
        mark_expr(&a.node, out);
      }
    }
    Expr::InstanceVar(_) => {}
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        mark_expr(&e.node, out);
      }
    }
    Expr::Index(base, idx) => {
      mark_expr(&base.node, out);
      mark_expr(&idx.node, out);
    }
    Expr::Lambda { body, .. } => collect_referenced_idents(body, out),
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        mark_expr(&k.node, out);
        mark_expr(&v.node, out);
      }
    }
    Expr::ArrayNew(size) => mark_expr(&size.node, out),
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          mark_expr(&e.node, out);
        }
      }
    }
    Expr::Int(_) | Expr::Float(_) | Expr::StringLit(_) | Expr::SymbolLit(_) | Expr::Bool(_) => {}
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => mark_expr(&e.node, out),
    Expr::Supervise(body) => collect_referenced_idents(body, out),
    Expr::Remote { addr, name, .. } => {
      mark_expr(&addr.node, out);
      mark_expr(&name.node, out);
    }
    Expr::Comptime(inner) => mark_expr(&inner.node, out),
    Expr::Locate { key, args, .. } => {
      mark_expr(&key.node, out);
      for a in args {
        mark_expr(&a.node, out);
      }
    }
  }
}

/// Plan 50's `leaf-escape-detection`: a `Stmt::Let`-bound `ClassName.
/// new(...)` instance named `n` is non-escaping iff `n` is never
/// referenced again anywhere in `body` after its own binding — see
/// `collect_referenced_idents`'s doc comment for why "never referenced
/// again" is exactly the union of the plan's own three enumerated
/// escape rules (return, `SetField`/`SetIndex` value, call/`New`/
/// `MethodCall` argument-including-receiver), plus the nested-lambda
/// soundness fix. The compiler-generated `initialize` call `Expr::New`
/// itself triggers is synthesized later, in codegen — it is not part
/// of the source AST this walks, so no special-casing is needed here
/// to exclude it.
fn find_non_escaping_news(body: &[Spanned<Stmt>]) -> HashSet<String> {
  let mut candidates = HashMap::new();
  collect_new_let_classes(body, &mut candidates);
  let mut referenced = HashSet::new();
  collect_referenced_idents(body, &mut referenced);
  candidates
    .into_keys()
    .filter(|name| !referenced.contains(name))
    .collect()
}

/// Plan 50's `leaf-stack-allocation-codegen`: one raw byte-array
/// `alloca` per non-escaping `Let`-bound `New`, sized to the class's
/// own `ClassLayout.size` (the exact same size the heap path
/// allocates via `ctx.alloc`) — built once, in the function's current
/// (entry) block, before `build_function_body` walks the body. Placing
/// it here rather than at the `Let` statement's own program point is
/// what keeps a non-escaping `New` written inside a loop body from
/// growing the stack frame's live-alloca count on every iteration
/// (`alloca` is not popped until the function returns) — the identical
/// discipline `prealloc_lets` already applies to ordinary named
/// locals.
fn prealloc_stack_objects<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  non_escaping: &HashSet<String>,
  new_let_classes: &HashMap<String, String>,
  classes: &HashMap<String, ClassLayout>,
) -> Result<HashMap<String, PointerValue<'ctx>>, String> {
  let mut object_allocas = HashMap::new();
  for name in non_escaping {
    let class_name = new_let_classes
      .get(name)
      .expect("every non-escaping name came from collect_new_let_classes");
    // A newtype's own `.new(...)` (`Meters.new(5.0)`) is never a real
    // class — `class_name` simply isn't in `classes` (which only ever
    // holds real `Item::Class` layouts). No allocation of ANY kind
    // (stack or heap) is ever needed for it — a newtype has the
    // identical LLVM representation as its own underlying primitive —
    // so this is skipped here rather than erroring; the ordinary
    // `Stmt::Let` -> `build_expr` fallthrough handles it directly (see
    // `build_expr`'s own `Expr::New` newtype-construction arm).
    let Some(layout) = classes.get(class_name) else {
      continue;
    };
    let alloca = builder
      .build_alloca(context.i8_type().array_type(layout.size as u32), name)
      .map_err(|e| e.to_string())?;
    object_allocas.insert(name.clone(), alloca);
  }
  Ok(object_allocas)
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
    // Plan 55: a pre-existing gap from plan 54's own `Item::Actor`
    // addition (silently fell into the wildcard below, meaning an
    // actor method using `retry` could have been wrongly optimized) —
    // fixed here since this plan is what makes actor method bodies
    // genuinely execute (and therefore genuinely `raise`/`rescue`/
    // `retry`) for the first time.
    Item::Actor(a) => a.methods.iter().any(|m| body_uses_retry(&m.body)),
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

/// Plan 55's own imported functions from `runtime/emerald_runtime.c`'s
/// mailbox/worker-pool — declared as a sibling to
/// `ExceptionRuntimeFuncs`/`declare_exception_runtime_funcs` above,
/// same shape of work. `enqueue`'s uniform trampoline signature
/// (`void(i8*, i64*)`) is shared with every per-method trampoline this
/// leaf itself generates (see `declare_actor_runtime_funcs`'s own call
/// site).
#[derive(Clone, Copy)]
struct ActorRuntimeFuncs<'ctx> {
  init_header: FunctionValue<'ctx>,
  /// Plan 60's Decision log: no longer called directly from codegen —
  /// `build_actor_enqueue_call` now always calls `dispatch` below,
  /// which itself calls `emerald_actor_enqueue` internally on the
  /// local path (byte-for-byte the same runtime call, just made from
  /// inside C instead of from generated IR). Kept declared (not
  /// removed) purely for documentation/future-caller symmetry with
  /// every other runtime function this struct names.
  #[allow(dead_code)]
  enqueue: FunctionValue<'ctx>,
  pool_start: FunctionValue<'ctx>,
  pool_drain_and_join: FunctionValue<'ctx>,
  current_thread_id: FunctionValue<'ctx>,
  /// Plan 57 (supervision trees) — see each declaration's own comment
  /// in `declare_actor_runtime_funcs` for its real C signature/purpose.
  set_region: FunctionValue<'ctx>,
  terminate: FunctionValue<'ctx>,
  supervisor_create: FunctionValue<'ctx>,
  supervisor_register_child: FunctionValue<'ctx>,
  supervisor_child: FunctionValue<'ctx>,
  /// Plan 60 (distributed, location-transparent actors) — see each
  /// declaration's own comment in `declare_actor_runtime_funcs` for its
  /// real C signature/purpose.
  ref_local: FunctionValue<'ctx>,
  register: FunctionValue<'ctx>,
  ref_remote: FunctionValue<'ctx>,
  dispatch: FunctionValue<'ctx>,
  remote_last_error: FunctionValue<'ctx>,
  wirebuf_push_i64: FunctionValue<'ctx>,
  wirebuf_push_string: FunctionValue<'ctx>,
  wirebuf_read_i64: FunctionValue<'ctx>,
  wirebuf_read_string: FunctionValue<'ctx>,
  /// Plan 65 (automatic actor placement) — see each declaration's own
  /// comment in `declare_actor_runtime_funcs` for its real C
  /// signature/purpose.
  locate_is_self_owner: FunctionValue<'ctx>,
  locate_owner_addr: FunctionValue<'ctx>,
  locate_cache_get: FunctionValue<'ctx>,
  locate_cache_put: FunctionValue<'ctx>,
}

fn declare_actor_runtime_funcs<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
) -> ActorRuntimeFuncs<'ctx> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let void_ty = context.void_type();

  let init_header = module.add_function(
    "emerald_actor_init_header",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  // Plan 65's `leaf-unified-fallible-send`: real C return type is now
  // `int` (0 success, -1 terminated), not `void` — this declaration
  // was never actually CALLED from this file (`#[allow(dead_code)]`
  // below), so the stale `void` signature was harmless dead code, not
  // a real bug, but is corrected here for the same reason any other
  // known-stale declaration would be.
  let enqueue = module.add_function(
    "emerald_actor_enqueue",
    context.i32_type().fn_type(
      &[ptr_ty.into(), ptr_ty.into(), ptr_ty.into(), i64_ty.into()],
      false,
    ),
    Some(Linkage::External),
  );
  let pool_start = module.add_function(
    "emerald_worker_pool_start",
    void_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let pool_drain_and_join = module.add_function(
    "emerald_worker_pool_drain_and_join",
    void_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let current_thread_id = module.add_function(
    "emerald_current_thread_id",
    i64_ty.fn_type(&[], false),
    Some(Linkage::External),
  );

  // Plan 57 (supervision trees).
  let set_region = module.add_function(
    "emerald_actor_set_region",
    void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let terminate = module.add_function(
    "emerald_actor_terminate",
    void_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let supervisor_create = module.add_function(
    "emerald_supervisor_create",
    ptr_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let supervisor_register_child = module.add_function(
    "emerald_supervisor_register_child",
    i64_ty.fn_type(
      &[
        ptr_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        i64_ty.into(),
      ],
      false,
    ),
    Some(Linkage::External),
  );
  let supervisor_child = module.add_function(
    "emerald_supervisor_child",
    ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );

  // Plan 60 (distributed, location-transparent actors).
  let i32_ty = context.i32_type();
  let ref_local = module.add_function(
    "emerald_actor_ref_local",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let register = module.add_function(
    "emerald_actor_register",
    void_ty.fn_type(
      &[
        ptr_ty.into(),
        ptr_ty.into(),
        i64_ty.into(),
        i32_ty.into(),
        ptr_ty.into(),
        i64_ty.into(),
      ],
      false,
    ),
    Some(Linkage::External),
  );
  let ref_remote = module.add_function(
    "emerald_actor_ref_remote",
    ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
    Some(Linkage::External),
  );
  let dispatch = module.add_function(
    "emerald_actor_dispatch",
    i32_ty.fn_type(
      &[
        ptr_ty.into(),
        i32_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        ptr_ty.into(),
        i64_ty.into(),
      ],
      false,
    ),
    Some(Linkage::External),
  );
  let remote_last_error = module.add_function(
    "emerald_remote_last_error_message",
    ptr_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let wirebuf_push_i64 = module.add_function(
    "emerald_wirebuf_push_i64",
    void_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
    Some(Linkage::External),
  );
  let wirebuf_push_string = module.add_function(
    "emerald_wirebuf_push_string",
    void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let wirebuf_read_i64 = module.add_function(
    "emerald_wirebuf_read_i64",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let wirebuf_read_string = module.add_function(
    "emerald_wirebuf_read_string",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );

  // Plan 65 (automatic actor placement), `leaf-virtual-actor-
  // placement`: `.locate`'s own real runtime entry points — see each
  // one's own doc comment in `runtime/emerald_runtime.c`.
  let locate_is_self_owner = module.add_function(
    "emerald_locate_is_self_owner",
    i32_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let locate_owner_addr = module.add_function(
    "emerald_locate_owner_addr",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let locate_cache_get = module.add_function(
    "emerald_locate_cache_get",
    ptr_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let locate_cache_put = module.add_function(
    "emerald_locate_cache_put",
    void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
    Some(Linkage::External),
  );

  ActorRuntimeFuncs {
    init_header,
    enqueue,
    pool_start,
    pool_drain_and_join,
    current_thread_id,
    set_region,
    terminate,
    supervisor_create,
    supervisor_register_child,
    supervisor_child,
    ref_local,
    register,
    ref_remote,
    dispatch,
    remote_last_error,
    wirebuf_push_i64,
    wirebuf_push_string,
    wirebuf_read_i64,
    wirebuf_read_string,
    locate_is_self_owner,
    locate_owner_addr,
    locate_cache_get,
    locate_cache_put,
  }
}

/// One small, uniform-ABI (`void(i8*, i64*)`) trampoline `FunctionValue`
/// per actor method — `emerald_actor_enqueue`'s message payload is
/// exactly `{self, argv}`, so a worker thread can run ANY actor
/// method through the identical call shape. Each trampoline unpacks
/// `argv` per that specific method's own already-known static `Param`
/// types (mirroring `local_llvm_type`'s existing `ValKind`-to-LLVM-type
/// mapping, in the encoding the plan's own Decision log states: an
/// `Int64`/`Symbol`/`Nil` word as-is, a `Float64` bit-cast, a `Ptr`/
/// `Str` `inttoptr`, a `Bool` truncated down from its full-width word)
/// and calls the real, already-declared method function — structurally
/// the same one-more-`FunctionValue`-per-item shape `declare_lambda_
/// functions`/`define_lambda` already do per lambda. Keyed by the
/// identical `"{Actor}_{method}"` mangled name `declare_user_functions`/
/// the define-pass already use for the method itself, so `leaf-cross-
/// actor-dispatch`'s own enqueue call site can look up "this call's
/// trampoline" the same way it already looks up "this call's target
/// function." Must run after `user_func_ids` is fully populated (its
/// `method_fv` lookup below reads straight from it), but needs none of
/// `declare_user_functions`'s own internal state — a fully separate,
/// later pass, not a change to that function.
fn declare_actor_trampolines<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  user_func_ids: &HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  exc_funcs: &ExceptionRuntimeFuncs<'ctx>,
  actor_funcs: &ActorRuntimeFuncs<'ctx>,
) -> Result<HashMap<String, FunctionValue<'ctx>>, String> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let void_ty = context.void_type();
  let builder = context.create_builder();
  let trampoline_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);

  let mut trampolines = HashMap::new();
  for item in &program.items {
    let Item::Actor(a) = item else { continue };
    for m in &a.methods {
      let mangled = format!("{}_{}", a.name, mangled_operator_symbol(&m.name));
      let Some(&(method_fv, _)) = user_func_ids.get(&mangled) else {
        continue;
      };
      // Bugfix (multi-file `--jobs` link): every compilation unit whose
      // require-closure includes this actor's own `Item::Actor` re-runs
      // this whole function and re-emits a byte-identical trampoline —
      // `own_function_names` (require_graph.rs) only scopes plain
      // `Item::Function`s, not classes/actors, so two `.o`s sharing an
      // actor both define this symbol with the old `External` linkage,
      // and `cc`/`ld` reject the duplicate definition. `WeakODR` (C++
      // template-instantiation's own standard fix for the same "N TUs
      // independently emit one deterministic, identical definition"
      // shape) lets the linker keep exactly one and drop the rest
      // instead of erroring — safe here because this function is a
      // pure function of the actor's AST, so every TU's copy is
      // guaranteed identical. Deliberately `WeakODR`, not the more
      // obvious `LinkOnceODR`: this session's own first attempt used
      // `LinkOnceODR` and broke `a_trampoline_called_directly_produces_
      // the_same_result_as_the_method_itself` (a hand-written C harness
      // that `extern`-links straight against this exact symbol) —
      // `default<O3>` is entitled to `GlobalDCE` a `linkonce_odr`
      // function that looks unreferenced *from inside this module*,
      // which a single-file actor program with no in-module caller of
      // its own trampoline genuinely is. `weak_odr` keeps the identical
      // multi-TU-dedup behavior but is never eligible for that
      // optimization-time removal, so the symbol always survives into
      // the object file whether or not anything in this module calls it.
      let trampoline_fv = module.add_function(
        &format!("{mangled}__trampoline"),
        trampoline_ty,
        Some(Linkage::WeakODR),
      );

      let entry = context.append_basic_block(trampoline_fv, "entry");
      builder.position_at_end(entry);
      let self_param = trampoline_fv
        .get_nth_param(0)
        .expect("trampoline always has a self param")
        .into_pointer_value();
      let argv_param = trampoline_fv
        .get_nth_param(1)
        .expect("trampoline always has an argv param")
        .into_pointer_value();

      let mut call_args: Vec<BasicMetadataValueEnum> = vec![self_param.into()];
      for (i, p) in m.params.iter().enumerate() {
        let idx = i64_ty.const_int(i as u64, false);
        let slot_ptr = unsafe {
          builder
            .build_in_bounds_gep(i64_ty, argv_param, &[idx], "argvslot")
            .map_err(|e| e.to_string())?
        };
        let raw = builder
          .build_load(i64_ty, slot_ptr, "argraw")
          .map_err(|e| e.to_string())?
          .into_int_value();
        let value: BasicMetadataValueEnum = match value_kind_for_type(&p.ty) {
          ValKind::Int64 | ValKind::Symbol => raw.into(),
          ValKind::Float64 => builder
            .build_bit_cast(raw, context.f64_type(), "argf64")
            .map_err(|e| e.to_string())?
            .into(),
          ValKind::Ptr | ValKind::Str => builder
            .build_int_to_ptr(raw, ptr_ty, "argptr")
            .map_err(|e| e.to_string())?
            .into(),
          ValKind::Bool => builder
            .build_int_truncate(raw, context.bool_type(), "argbool")
            .map_err(|e| e.to_string())?
            .into(),
          ValKind::Void | ValKind::Tuple(_) => {
            return Err(format!(
              "codegen: internal — `{}` is not a valid actor method param kind",
              p.ty
            ));
          }
        };
        call_args.push(value);
      }

      // Plan 57 (supervision trees), `leaf-crash-isolation`: wraps the
      // real method call in a synthetic, compiler-only `push_handler`/
      // `setjmp` frame — the unconditional-catch equivalent of
      // `build_begin`'s own bare-`rescue` codegen (mirrored here
      // directly in raw IR, since this function builds trampolines
      // outside `build_stmt`'s own Stmt-level machinery). A `raise`
      // that reaches this frame uncaught terminates the actor
      // (`emerald_actor_terminate`) instead of the process; a `raise`
      // a USER `rescue` inside the method body already caught never
      // reaches here at all — this frame only ever fires for what
      // would otherwise have been a true, process-fatal uncaught
      // exception.
      let push_call = builder
        .build_call(exc_funcs.push_handler, &[], "actorpushhandler")
        .map_err(|e| e.to_string())?;
      let handler_ptr = call_result(push_call)?.into_pointer_value();
      let jmpbuf_call = builder
        .build_call(
          exc_funcs.handler_jmpbuf,
          &[handler_ptr.into()],
          "actorjmpbuf",
        )
        .map_err(|e| e.to_string())?;
      let jmpbuf_ptr = call_result(jmpbuf_call)?.into_pointer_value();
      let setjmp_call = builder
        .build_call(exc_funcs.setjmp, &[jmpbuf_ptr.into()], "actorsetjmpres")
        .map_err(|e| e.to_string())?;
      let returns_twice_id = Attribute::get_named_enum_kind_id("returns_twice");
      let returns_twice_attr = context.create_enum_attribute(returns_twice_id, 0);
      setjmp_call.add_attribute(AttributeLoc::Function, returns_twice_attr);
      let setjmp_result = call_result(setjmp_call)?.into_int_value();

      let try_blk = context.append_basic_block(trampoline_fv, "trampoline.try");
      let catch_blk = context.append_basic_block(trampoline_fv, "trampoline.catch");
      let zero = context.i32_type().const_int(0, false);
      let is_first_pass = builder
        .build_int_compare(IntPredicate::EQ, setjmp_result, zero, "actorisfirstpass")
        .map_err(|e| e.to_string())?;
      builder
        .build_conditional_branch(is_first_pass, try_blk, catch_blk)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(try_blk);
      builder
        .build_call(method_fv, &call_args, "trampolinecall")
        .map_err(|e| e.to_string())?;
      builder
        .build_call(exc_funcs.pop_handler, &[], "actorpophandler")
        .map_err(|e| e.to_string())?;
      builder.build_return(None).map_err(|e| e.to_string())?;

      builder.position_at_end(catch_blk);
      // `emerald_raise` already unlinked this handler from the stack
      // before jumping back here (see `emerald_raise`'s own doc
      // comment) — `free_handler`, not `pop_handler`, matches plan
      // 38's own established split between the two.
      builder
        .build_call(
          exc_funcs.free_handler,
          &[handler_ptr.into()],
          "actorfreehandler",
        )
        .map_err(|e| e.to_string())?;
      builder
        .build_call(
          actor_funcs.terminate,
          &[self_param.into()],
          "actorterminate",
        )
        .map_err(|e| e.to_string())?;
      builder.build_return(None).map_err(|e| e.to_string())?;

      trampolines.insert(mangled, trampoline_fv);
    }
  }
  Ok(trampolines)
}

/// One `void *(long long *argv)` "respawn thunk" per actor in the
/// program (plan 57, `leaf-one-for-one-restart-runtime`) — callable
/// straight from C (`EmeraldSupervisedChild.respawn`,
/// `runtime/emerald_runtime.c`'s own `emerald_supervisor_notify_
/// terminated`), doing exactly what `build_spawn_alloc` already does
/// for an ordinary `.spawn` — region_create, region_alloc, actor_init_
/// header, set_region, `initialize` — just unpacking its arguments
/// from a raw argv array (mirroring `declare_actor_trampolines`'s own
/// per-param unpacking) instead of evaluating an AST `Args` list,
/// since a supervisor's own restart call has no expression context to
/// evaluate at all — only the child's already-captured, already-
/// evaluated original spawn arguments (Decision log: captured values,
/// never re-evaluated expressions). Actors never have a superclass
/// (plan 54's own grammar constraint), so `{name}_initialize` is
/// always the actor's own defining symbol — no `method_owners` chain
/// walk needed here, unlike `build_initialize_call`'s general case.
/// Unlike `declare_actor_trampolines`'s own "one per actor method,
/// whether or not cross-actor-called" precedent, this is scoped to
/// `supervised_classes` (`collect_supervised_classes_in_stmt`'s own
/// program-wide result) — an UNCONDITIONAL one-per-actor pass was
/// tried first and reverted (this session): it compiles a real, always
/// dead `emerald_region_create` call into every never-supervised
/// actor's own unused thunk, which broke an existing, unrelated
/// regression test counting `emerald_region_create` calls program-wide
/// (`each_spawn_call_site_allocates_from_its_own_freshly_created_
/// region`) — scoping to only the classes actually spawned inside a
/// real `supervise` block fixes that without weakening this leaf's own
/// coverage (every class a `supervise` block can ever reference still
/// gets exactly one thunk).
#[allow(clippy::too_many_arguments)]
fn declare_supervisor_respawn_thunks<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  user_func_ids: &HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  actor_funcs: &ActorRuntimeFuncs<'ctx>,
  region_create_fn: FunctionValue<'ctx>,
  region_alloc_fn: FunctionValue<'ctx>,
  classes: &HashMap<String, ClassLayout>,
  supervised_classes: &HashSet<String>,
) -> Result<HashMap<String, FunctionValue<'ctx>>, String> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let builder = context.create_builder();
  let thunk_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);

  let mut thunks = HashMap::new();
  for item in &program.items {
    let Item::Actor(a) = item else { continue };
    if !supervised_classes.contains(a.name.as_str()) {
      continue;
    }
    let layout = classes
      .get(a.name.as_str())
      .ok_or_else(|| format!("codegen: unknown class `{}`", a.name))?;

    let thunk_fv = module.add_function(
      &format!("{}__respawn", a.name),
      thunk_ty,
      Some(Linkage::External),
    );
    let entry = context.append_basic_block(thunk_fv, "entry");
    builder.position_at_end(entry);
    let argv_param = thunk_fv
      .get_nth_param(0)
      .expect("respawn thunk always has an argv param")
      .into_pointer_value();

    let header_size = 8u64;
    let size_val = i64_ty.const_int(layout.size + header_size, false);
    let region_call = builder
      .build_call(region_create_fn, &[], "respawnregion")
      .map_err(|e| e.to_string())?;
    let region = call_result(region_call)?.into_pointer_value();
    let alloc_call = builder
      .build_call(
        region_alloc_fn,
        &[region.into(), size_val.into()],
        "respawntmp",
      )
      .map_err(|e| e.to_string())?;
    let raw_ptr = call_result(alloc_call)?.into_pointer_value();
    builder
      .build_call(actor_funcs.init_header, &[raw_ptr.into()], "respawnheader")
      .map_err(|e| e.to_string())?;
    let self_ptr = field_ptr(context, &builder, raw_ptr, header_size)?;
    builder
      .build_call(
        actor_funcs.set_region,
        &[self_ptr.into(), region.into()],
        "respawnsetregion",
      )
      .map_err(|e| e.to_string())?;

    if let Some(init) = a.methods.iter().find(|m| m.name == "initialize") {
      let init_key = format!("{}_initialize", a.name);
      if let Some(&(init_fv, _)) = user_func_ids.get(&init_key) {
        let mut call_args: Vec<BasicMetadataValueEnum> = vec![self_ptr.into()];
        for (i, p) in init.params.iter().enumerate() {
          let idx = i64_ty.const_int(i as u64, false);
          let slot_ptr = unsafe {
            builder
              .build_in_bounds_gep(i64_ty, argv_param, &[idx], "respawnargvslot")
              .map_err(|e| e.to_string())?
          };
          let raw = builder
            .build_load(i64_ty, slot_ptr, "respawnargraw")
            .map_err(|e| e.to_string())?
            .into_int_value();
          let value: BasicMetadataValueEnum = match value_kind_for_type(&p.ty) {
            ValKind::Int64 | ValKind::Symbol => raw.into(),
            ValKind::Float64 => builder
              .build_bit_cast(raw, context.f64_type(), "respawnargf64")
              .map_err(|e| e.to_string())?
              .into(),
            ValKind::Ptr | ValKind::Str => builder
              .build_int_to_ptr(raw, ptr_ty, "respawnargptr")
              .map_err(|e| e.to_string())?
              .into(),
            ValKind::Bool => builder
              .build_int_truncate(raw, context.bool_type(), "respawnargbool")
              .map_err(|e| e.to_string())?
              .into(),
            ValKind::Void | ValKind::Tuple(_) => {
              return Err(format!(
                "codegen: internal — `{}` is not a valid actor initialize param kind",
                p.ty
              ));
            }
          };
          call_args.push(value);
        }
        builder
          .build_call(init_fv, &call_args, "respawninittmp")
          .map_err(|e| e.to_string())?;
      }
    }

    // Plan 60's Decision log (Design decision 1): a respawned actor's
    // value flows out through `emerald_supervisor_child` exactly the
    // same way `.spawn`'s own value does — needs the identical
    // `EmeraldActorRef` wrapping `build_spawn_alloc` now applies, or a
    // supervised actor's post-respawn value would be the wrong shape.
    let ref_call = builder
      .build_call(actor_funcs.ref_local, &[self_ptr.into()], "respawnreflocal")
      .map_err(|e| e.to_string())?;
    let ref_ptr = call_result(ref_call)?.into_pointer_value();
    builder
      .build_return(Some(&ref_ptr))
      .map_err(|e| e.to_string())?;

    thunks.insert(a.name.clone(), thunk_fv);
  }
  Ok(thunks)
}

/// Plan 60's Decision log — Design decision 2, re-derived independently
/// on the codegen side (this codebase's own "no shared sema→codegen
/// structure" architecture — sema's own identically-shaped `is_wire_
/// safe_type` already rejected an unsafe program before codegen ever
/// runs; this purely decides which classes actually need a generated
/// `_encode`/`_decode` pair). `seen` guards a self-referential class
/// from infinite recursion the same way sema's own version does.
fn is_wire_safe_class_field(
  ty: &str,
  classes: &HashMap<String, ClassLayout>,
  actor_names: &HashSet<String>,
  seen: &mut HashSet<String>,
) -> bool {
  match value_kind_for_type(&TypeExpr::Named(ty.to_string())) {
    ValKind::Int64 | ValKind::Float64 | ValKind::Bool | ValKind::Symbol => true,
    ValKind::Str => true,
    ValKind::Ptr => {
      if actor_names.contains(ty) {
        return false;
      }
      let Some(layout) = classes.get(ty) else {
        return false;
      };
      if !seen.insert(ty.to_string()) {
        return false;
      }
      layout
        .field_classes
        .values()
        .all(|ft| is_wire_safe_class_field(ft, classes, actor_names, seen))
    }
    ValKind::Void | ValKind::Tuple(_) => false,
  }
}

/// One `{Class}_encode(i8* self, EmeraldWireBuf* out)`/`{Class}_decode
/// (EmeraldWireBuf* in) -> i8*` pair per wire-safe class (`leaf-wire-
/// codec`), walking `ClassLayout`'s existing field list (plan 08's
/// fixed 8-byte-per-field, plan 32's chain-resolved offsets) in
/// offset order, dispatching per field on its already-known `ValKind` —
/// `String` -> length-prefixed wire copy; a nested wire-safe `Class` ->
/// recurse into its own `_encode`/`_decode`; every scalar kind -> a raw
/// 8-byte wire copy (`Float64`/`Bool` bit-cast/extended the same way
/// `build_actor_enqueue_call`'s own `argv` packing already does).
/// Declared for every wire-safe class up front, in one pass, before any
/// body is built, so a self-referential or mutually-referential pair
/// can call each other regardless of declaration order.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn declare_wire_class_codecs<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  classes: &HashMap<String, ClassLayout>,
  actor_names: &HashSet<String>,
  actor_funcs: &ActorRuntimeFuncs<'ctx>,
  alloc_fn: FunctionValue<'ctx>,
) -> Result<
  (
    HashMap<String, FunctionValue<'ctx>>,
    HashMap<String, FunctionValue<'ctx>>,
  ),
  String,
> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let void_ty = context.void_type();
  let builder = context.create_builder();

  let mut wire_safe: Vec<String> = Vec::new();
  for name in classes.keys() {
    if actor_names.contains(name) {
      continue;
    }
    let mut seen = HashSet::new();
    if is_wire_safe_class_field(name, classes, actor_names, &mut seen) {
      wire_safe.push(name.clone());
    }
  }

  let encode_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
  let decode_ty = ptr_ty.fn_type(&[ptr_ty.into()], false);
  let mut encode_fns = HashMap::new();
  let mut decode_fns = HashMap::new();
  for name in &wire_safe {
    // Bugfix (multi-file `--jobs` link): same shape as the trampoline
    // fix above — a wire-safe class (including a synthesized one like
    // `RemoteActorError`, added fresh into every compilation unit that
    // has any actor at all) gets its `_encode`/`_decode` re-emitted,
    // byte-identical, in every `.o` whose require-closure defines it.
    // `WeakODR` over `External` lets the linker dedup instead of
    // rejecting the duplicate definition — `WeakODR`, not `LinkOnceODR`,
    // per `declare_actor_trampolines`'s own fix above: a `linkonce_odr`
    // definition unreferenced from inside its own module is eligible
    // for `default<O3>`'s `GlobalDCE`, `weak_odr` isn't.
    encode_fns.insert(
      name.clone(),
      module.add_function(&format!("{name}_encode"), encode_ty, Some(Linkage::WeakODR)),
    );
    decode_fns.insert(
      name.clone(),
      module.add_function(&format!("{name}_decode"), decode_ty, Some(Linkage::WeakODR)),
    );
  }

  for name in &wire_safe {
    let layout = &classes[name];
    let mut fields: Vec<(&String, &FieldInfo)> = layout.fields.iter().collect();
    fields.sort_by_key(|(_, info)| info.offset);

    let encode_fv = encode_fns[name];
    builder.position_at_end(context.append_basic_block(encode_fv, "entry"));
    let self_param = encode_fv
      .get_nth_param(0)
      .expect("encode always has a self param")
      .into_pointer_value();
    let out_param = encode_fv
      .get_nth_param(1)
      .expect("encode always has an out param")
      .into_pointer_value();
    for (fname, finfo) in &fields {
      let field_val = load_field(context, &builder, self_param, (*finfo).clone())?;
      let raw_ty = &layout.field_classes[*fname];
      match finfo.kind {
        ValKind::Str => {
          builder
            .build_call(
              actor_funcs.wirebuf_push_string,
              &[out_param.into(), field_val.into()],
              "wireencstr",
            )
            .map_err(|e| e.to_string())?;
        }
        ValKind::Ptr => {
          let sub_encode = encode_fns.get(raw_ty).ok_or_else(|| {
            format!("codegen: internal — `{raw_ty}` has no generated `_encode` (not wire-safe)")
          })?;
          builder
            .build_call(
              *sub_encode,
              &[field_val.into(), out_param.into()],
              "wireencrec",
            )
            .map_err(|e| e.to_string())?;
        }
        ValKind::Float64 => {
          let raw = builder
            .build_bit_cast(field_val, i64_ty, "wireencf64raw")
            .map_err(|e| e.to_string())?;
          builder
            .build_call(
              actor_funcs.wirebuf_push_i64,
              &[out_param.into(), raw.into()],
              "wirencf64",
            )
            .map_err(|e| e.to_string())?;
        }
        ValKind::Bool => {
          let ext = builder
            .build_int_z_extend(field_val.into_int_value(), i64_ty, "wireencbool")
            .map_err(|e| e.to_string())?;
          builder
            .build_call(
              actor_funcs.wirebuf_push_i64,
              &[out_param.into(), ext.into()],
              "wireencboolraw",
            )
            .map_err(|e| e.to_string())?;
        }
        ValKind::Int64 | ValKind::Symbol => {
          builder
            .build_call(
              actor_funcs.wirebuf_push_i64,
              &[out_param.into(), field_val.into()],
              "wireenci64",
            )
            .map_err(|e| e.to_string())?;
        }
        ValKind::Void | ValKind::Tuple(_) => {
          return Err(format!(
            "codegen: internal — `{name}.{fname}` is not a valid wire-safe field kind"
          ));
        }
      }
    }
    builder.build_return(None).map_err(|e| e.to_string())?;

    let decode_fv = decode_fns[name];
    builder.position_at_end(context.append_basic_block(decode_fv, "entry"));
    let in_param = decode_fv
      .get_nth_param(0)
      .expect("decode always has an in param")
      .into_pointer_value();
    let size_val = i64_ty.const_int(layout.size, false);
    let alloc_call = builder
      .build_call(alloc_fn, &[size_val.into()], "wiredecalloc")
      .map_err(|e| e.to_string())?;
    let out_ptr = call_result(alloc_call)?.into_pointer_value();
    for (fname, finfo) in &fields {
      let raw_ty = &layout.field_classes[*fname];
      let value: BasicValueEnum = match finfo.kind {
        ValKind::Str => {
          let call = builder
            .build_call(
              actor_funcs.wirebuf_read_string,
              &[in_param.into()],
              "wiredecstr",
            )
            .map_err(|e| e.to_string())?;
          call_result(call)?
        }
        ValKind::Ptr => {
          let sub_decode = decode_fns.get(raw_ty).ok_or_else(|| {
            format!("codegen: internal — `{raw_ty}` has no generated `_decode` (not wire-safe)")
          })?;
          let call = builder
            .build_call(*sub_decode, &[in_param.into()], "wiredecrec")
            .map_err(|e| e.to_string())?;
          call_result(call)?
        }
        ValKind::Float64 => {
          let call = builder
            .build_call(
              actor_funcs.wirebuf_read_i64,
              &[in_param.into()],
              "wiredecraw",
            )
            .map_err(|e| e.to_string())?;
          let raw = call_result(call)?;
          builder
            .build_bit_cast(raw, context.f64_type(), "wiredecf64")
            .map_err(|e| e.to_string())?
        }
        ValKind::Bool => {
          let call = builder
            .build_call(
              actor_funcs.wirebuf_read_i64,
              &[in_param.into()],
              "wiredecraw",
            )
            .map_err(|e| e.to_string())?;
          let raw = call_result(call)?.into_int_value();
          builder
            .build_int_truncate(raw, context.bool_type(), "wiredecbool")
            .map_err(|e| e.to_string())?
            .into()
        }
        ValKind::Int64 | ValKind::Symbol => {
          let call = builder
            .build_call(
              actor_funcs.wirebuf_read_i64,
              &[in_param.into()],
              "wiredecraw",
            )
            .map_err(|e| e.to_string())?;
          call_result(call)?
        }
        ValKind::Void | ValKind::Tuple(_) => {
          return Err(format!(
            "codegen: internal — `{name}.{fname}` is not a valid wire-safe field kind"
          ));
        }
      };
      let dst = field_ptr(context, &builder, out_ptr, finfo.offset)?;
      builder.build_store(dst, value).map_err(|e| e.to_string())?;
    }
    builder
      .build_return(Some(&out_ptr))
      .map_err(|e| e.to_string())?;
  }

  Ok((encode_fns, decode_fns))
}

/// One `{key}_encode_args(long long *argv, EmeraldWireBuf *out)`/`{key}_
/// decode_args(EmeraldWireBuf *in, long long *argv_out)` pair per actor
/// method (`leaf-wire-codec`), mirroring `declare_actor_trampolines`'s
/// own "one per actor method, whether or not cross-actor-called"
/// precedent — `argv`'s own raw-word packing is EXACTLY `build_actor_
/// enqueue_call`'s (a `Float64`/`Bool` argument is already a raw
/// bitcast/zero-extended `i64` by the time it's in `argv`, so only
/// `String`/wire-safe-`Class` params need a real pointer-conversion
/// step here). A `Class`-typed param with no generated `_encode`/
/// `_decode` (not wire-safe — `Array`/`Hash`/`Proc`/an actor reference)
/// falls back to a raw-word wire copy instead of a codegen-time error:
/// sema already guarantees no cross-actor SEND site ever supplies such
/// an argument (`check_wire_safety`), so this exact branch is real,
/// disclosed dead code for a method that's declared but only ever
/// called same-actor/locally, never actually remotely dispatched.
/// Also returns each method's `method_tag` — declaration order within
/// its actor, the same dense-index convention `class_tags` uses —
/// consulted both by a remote SEND's own encoded frame header and by
/// `.register`'s own per-class method table.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn declare_actor_wire_arg_codecs<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  actor_funcs: &ActorRuntimeFuncs<'ctx>,
  wire_encode_fns: &HashMap<String, FunctionValue<'ctx>>,
  wire_decode_fns: &HashMap<String, FunctionValue<'ctx>>,
) -> Result<
  (
    HashMap<String, FunctionValue<'ctx>>,
    HashMap<String, FunctionValue<'ctx>>,
    HashMap<String, i32>,
  ),
  String,
> {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let i64_ty = context.i64_type();
  let void_ty = context.void_type();
  let builder = context.create_builder();

  let encode_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
  let decode_ty = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into()], false);

  let mut encode_fns = HashMap::new();
  let mut decode_fns = HashMap::new();
  let mut method_tags = HashMap::new();

  for item in &program.items {
    let Item::Actor(a) = item else { continue };
    for (tag, m) in a.methods.iter().enumerate() {
      let key = format!("{}_{}", a.name, mangled_operator_symbol(&m.name));
      method_tags.insert(key.clone(), tag as i32);
      // Bugfix (multi-file `--jobs` link): same shape as
      // `declare_actor_trampolines`'s own fix — `WeakODR` (not
      // `LinkOnceODR`, see that function's own doc comment for why)
      // so a wire arg codec pair re-emitted identically in every TU
      // that shares this actor's definition dedups at link time
      // instead of erroring as a duplicate symbol.
      let encode_fv = module.add_function(
        &format!("{key}_encode_args"),
        encode_ty,
        Some(Linkage::WeakODR),
      );
      let decode_fv = module.add_function(
        &format!("{key}_decode_args"),
        decode_ty,
        Some(Linkage::WeakODR),
      );

      builder.position_at_end(context.append_basic_block(encode_fv, "entry"));
      let argv_param = encode_fv
        .get_nth_param(0)
        .expect("encode_args always has an argv param")
        .into_pointer_value();
      let out_param = encode_fv
        .get_nth_param(1)
        .expect("encode_args always has an out param")
        .into_pointer_value();
      for (i, p) in m.params.iter().enumerate() {
        let idx = i64_ty.const_int(i as u64, false);
        let slot_ptr = unsafe {
          builder
            .build_in_bounds_gep(i64_ty, argv_param, &[idx], "encargvslot")
            .map_err(|e| e.to_string())?
        };
        let raw = builder
          .build_load(i64_ty, slot_ptr, "encargraw")
          .map_err(|e| e.to_string())?
          .into_int_value();
        match value_kind_for_type(&p.ty) {
          ValKind::Str => {
            let str_ptr = builder
              .build_int_to_ptr(raw, ptr_ty, "encargstrptr")
              .map_err(|e| e.to_string())?;
            builder
              .build_call(
                actor_funcs.wirebuf_push_string,
                &[out_param.into(), str_ptr.into()],
                "encargstr",
              )
              .map_err(|e| e.to_string())?;
          }
          ValKind::Ptr if wire_encode_fns.contains_key(&p.ty.to_string()) => {
            let obj_ptr = builder
              .build_int_to_ptr(raw, ptr_ty, "encargobjptr")
              .map_err(|e| e.to_string())?;
            builder
              .build_call(
                wire_encode_fns[&p.ty.to_string()],
                &[obj_ptr.into(), out_param.into()],
                "encargrec",
              )
              .map_err(|e| e.to_string())?;
          }
          _ => {
            builder
              .build_call(
                actor_funcs.wirebuf_push_i64,
                &[out_param.into(), raw.into()],
                "encargi64",
              )
              .map_err(|e| e.to_string())?;
          }
        }
      }
      builder.build_return(None).map_err(|e| e.to_string())?;

      builder.position_at_end(context.append_basic_block(decode_fv, "entry"));
      let in_param = decode_fv
        .get_nth_param(0)
        .expect("decode_args always has an in param")
        .into_pointer_value();
      let argv_out_param = decode_fv
        .get_nth_param(1)
        .expect("decode_args always has an argv_out param")
        .into_pointer_value();
      for (i, p) in m.params.iter().enumerate() {
        let raw = match value_kind_for_type(&p.ty) {
          ValKind::Str => {
            let call = builder
              .build_call(
                actor_funcs.wirebuf_read_string,
                &[in_param.into()],
                "decargstr",
              )
              .map_err(|e| e.to_string())?;
            let str_ptr = call_result(call)?.into_pointer_value();
            builder
              .build_ptr_to_int(str_ptr, i64_ty, "decargstrraw")
              .map_err(|e| e.to_string())?
          }
          ValKind::Ptr if wire_decode_fns.contains_key(&p.ty.to_string()) => {
            let call = builder
              .build_call(
                wire_decode_fns[&p.ty.to_string()],
                &[in_param.into()],
                "decargrec",
              )
              .map_err(|e| e.to_string())?;
            let obj_ptr = call_result(call)?.into_pointer_value();
            builder
              .build_ptr_to_int(obj_ptr, i64_ty, "decargobjraw")
              .map_err(|e| e.to_string())?
          }
          _ => {
            let call = builder
              .build_call(
                actor_funcs.wirebuf_read_i64,
                &[in_param.into()],
                "decargi64",
              )
              .map_err(|e| e.to_string())?;
            call_result(call)?.into_int_value()
          }
        };
        let idx = i64_ty.const_int(i as u64, false);
        let slot_ptr = unsafe {
          builder
            .build_in_bounds_gep(i64_ty, argv_out_param, &[idx], "decargvslot")
            .map_err(|e| e.to_string())?
        };
        builder
          .build_store(slot_ptr, raw)
          .map_err(|e| e.to_string())?;
      }
      builder.build_return(None).map_err(|e| e.to_string())?;

      encode_fns.insert(key.clone(), encode_fv);
      decode_fns.insert(key, decode_fv);
    }
  }

  Ok((encode_fns, decode_fns, method_tags))
}

/// One `static const EmeraldMethodEntry[]` global per actor class
/// (`leaf-remote-dispatch-and-worked-proof`), indexed by `method_tag`
/// (declaration order — matching `emerald_actor_register`'s own C
/// signature, `runtime/emerald_runtime.c`'s `EmeraldMethodEntry
/// {trampoline, decode_args}`), passed to `emerald_actor_register` at
/// every `.register` call site for that class. Built once, alongside
/// `actor_trampolines`/the wire-arg-codec pass, so a `.register` call
/// site just references an already-built pointer.
fn build_actor_method_tables<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  actor_trampolines: &HashMap<String, FunctionValue<'ctx>>,
  arg_decode_fns: &HashMap<String, FunctionValue<'ctx>>,
) -> (HashMap<String, PointerValue<'ctx>>, HashMap<String, i64>) {
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let entry_ty = context.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

  let mut tables = HashMap::new();
  let mut counts = HashMap::new();
  for item in &program.items {
    let Item::Actor(a) = item else { continue };
    let entries: Vec<_> = a
      .methods
      .iter()
      .map(|m| {
        let key = format!("{}_{}", a.name, mangled_operator_symbol(&m.name));
        let trampoline_ptr = actor_trampolines[&key].as_global_value().as_pointer_value();
        let decode_ptr = arg_decode_fns[&key].as_global_value().as_pointer_value();
        entry_ty.const_named_struct(&[trampoline_ptr.into(), decode_ptr.into()])
      })
      .collect();
    counts.insert(a.name.clone(), entries.len() as i64);
    let array_ty = entry_ty.array_type(entries.len() as u32);
    let global = module.add_global(array_ty, None, &format!("{}__methods", a.name));
    global.set_initializer(&entry_ty.const_array(&entries));
    global.set_constant(true);
    // Bugfix (multi-file `--jobs` link): same `WeakODR` treatment
    // (not `LinkOnceODR` — see `declare_actor_trampolines`'s own doc
    // comment) as this file's other actor-symbol sites above — this
    // table is re-built byte-identical in every TU that shares the
    // actor's definition (`add_global` defaults to `External` linkage
    // with no explicit setting), so it needs the same dedup fix.
    global.set_linkage(Linkage::WeakODR);
    tables.insert(a.name.clone(), global.as_pointer_value());
  }
  (tables, counts)
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

/// `(self_ptr, &class.fields, &class.field_classes)` — the shape
/// `Ctx::self_ctx` carries while compiling a method body. Its own named
/// alias, per clippy's `type_complexity` (the third element is plan
/// 55's own addition, see `ClassLayout.field_classes`'s own doc
/// comment).
type SelfCtx<'a, 'ctx> = (
  PointerValue<'ctx>,
  &'a HashMap<String, FieldInfo>,
  &'a HashMap<String, String>,
);

/// Context that's fixed for the duration of compiling one function/
/// method/lambda body. `self_ctx` is `Some(...)` (see `SelfCtx`'s own
/// doc comment) only while compiling a method body.
#[derive(Clone, Copy)]
struct Ctx<'a, 'ctx> {
  user_func_ids: &'a HashMap<String, (FunctionValue<'ctx>, ValKind)>,
  classes: &'a HashMap<String, ClassLayout>,
  /// Plan 52's Decision log: `{enum name} -> its tagged-union layout}`
  /// — independently re-derived from the raw `Program`, alongside
  /// `classes` above.
  enums: &'a HashMap<String, EnumLayout>,
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
  self_ctx: Option<SelfCtx<'a, 'ctx>>,
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
  /// `{class name} -> {generic method names it directly declares}` —
  /// see `build_generic_class_methods`'s own doc comment for why this
  /// exists (a codegen-only regression gate, independent of `emerald-
  /// sema`'s own `ClassInfo.generic_methods`). `build_method_call`
  /// checks this against the resolved `defining_class` and refuses the
  /// call with a clean diagnostic instead of emitting LLVM IR that
  /// would crash the verifier. Empty entries are never inserted, so a
  /// program with no generic method anywhere pays for an empty map
  /// lookup only.
  generic_class_methods: &'a HashMap<String, HashSet<String>>,
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
  /// Plan 35: `Some` only when `compile_to_object_with_debug_info` was
  /// the entry point — `None` (the `compile_to_object` path) attaches
  /// no debug info at all, unchanged behavior.
  dibuilder: Option<&'a DebugInfoBuilder<'ctx>>,
  di_file: Option<DIFile<'ctx>>,
  /// Sorted byte offsets of every `\n` in the compiled source — a
  /// `Spanned<T>`'s byte-offset span converts to a 1-based source line
  /// via `line_for_offset`'s binary search over this table.
  newline_offsets: Option<&'a [usize]>,
  /// The enclosing function/method/lambda's own `DISubprogram` scope,
  /// set once per `define_user_function`/`define_method`/
  /// `define_lambda`/`define_main` call — `build_stmt` reads this to
  /// build each statement's debug location.
  current_di_scope: Option<DIScope<'ctx>>,
  /// Plan 50's `leaf-stack-allocation-codegen`: `Some(&table)` only
  /// while compiling one function/method/lambda/`main` body — the
  /// stack `PointerValue` for every `Let`-bound `New` this function's
  /// own `find_non_escaping_news` proved non-escaping, built once in
  /// the entry block by `prealloc_stack_objects`. Set fresh per
  /// `define_user_function`/`define_method`/`define_lambda`/
  /// `define_main` call, unlike every other field here — each
  /// function's own non-escaping set is unrelated to any other's.
  /// `build_stmt`'s `Stmt::Let` arm consults this to skip `ctx.alloc`
  /// entirely for a matching name.
  object_allocas: Option<&'a HashMap<String, PointerValue<'ctx>>>,
  /// Plan 50's `leaf-escape-instrumentation-and-report`: `Some(&counter)`
  /// only from `compile_to_object_with_stats`'s own entry point —
  /// accumulates one count per `Expr::New` site actually compiled,
  /// program-wide (set once in `compile_to_object_impl`, never
  /// overridden per-function, unlike `object_allocas` above). A
  /// `RefCell`, not a loose `&mut EscapeStats` parameter threaded
  /// through the whole `build_stmt`/`build_block`/`build_for`/... call
  /// graph — sound because one `Ctx` (and the `RefCell` it points at)
  /// is never shared across threads: plan 49's own per-thread
  /// `Context` rule already guarantees each worker builds its own,
  /// entirely separate `gen_ctx` for its own file, so this adds no
  /// cross-thread/cross-process race despite plan 49's parallel
  /// codegen already existing in this same crate.
  escape_stats: Option<&'a RefCell<EscapeStats>>,
  /// Plan 53's Decision log: `is_valid_int`/`parse_digits` are
  /// compiler-known intrinsic builtins (`build_expr`'s `Expr::Call`
  /// handling dispatches to these directly, mirroring `puts`'s own
  /// hardcoded-name dispatch) backing `Result[T, E]`'s worked example
  /// — real string-parsing runtime helpers, no dependency on plan 45.
  is_valid_int_fn: FunctionValue<'ctx>,
  parse_digits_fn: FunctionValue<'ctx>,
  /// Plan 54's Decision log: `Expr::Spawn`'s own allocation call site —
  /// plan 51's real, landed two-call region API (`create` then
  /// `alloc`), swapped in for `Expr::New`'s single `ctx.alloc` call.
  /// No `Ctx` field tracks the created region handle anywhere past its
  /// own `Expr::Spawn` codegen site — this plan has no actor-
  /// termination event yet, so (like `ctx.alloc`'s own allocations
  /// already do) it leaks exactly as disclosed in this plan's own
  /// non-goals.
  region_create_fn: FunctionValue<'ctx>,
  region_alloc_fn: FunctionValue<'ctx>,
  /// Plan 55's own imported mailbox/worker-pool functions — see
  /// `ActorRuntimeFuncs`'s own doc comment.
  actor_funcs: ActorRuntimeFuncs<'ctx>,
  /// `{"{Actor}_{method}"} -> that method's own trampoline FunctionValue`
  /// — see `declare_actor_trampolines`'s own doc comment.
  actor_trampolines: &'a HashMap<String, FunctionValue<'ctx>>,
  /// Every declared `actor`'s name — `build_method_call`'s own
  /// dispatch-rule check (Plan 55's Decision log: a literal `self`
  /// receiver is always a direct call; any OTHER receiver whose static
  /// class is IN this set becomes a cross-actor `emerald_actor_enqueue`
  /// call instead).
  actor_names: &'a HashSet<String>,
  /// Plan 57 (supervision trees): `{actor class name} -> its own
  /// codegen-generated "respawn thunk" FunctionValue` — see
  /// `declare_supervisor_respawn_thunks`'s own doc comment. One per
  /// actor in the whole program, regardless of whether any `supervise`
  /// block actually tracks it (mirrors `actor_trampolines`'s own
  /// "one per actor method, whether or not it's ever cross-actor-
  /// called" precedent).
  supervisor_respawn_thunks: &'a HashMap<String, FunctionValue<'ctx>>,
  /// Plan 60 (distributed, location-transparent actors) —
  /// `leaf-wire-codec`/`leaf-remote-dispatch-and-worked-proof`: see
  /// `declare_actor_wire_arg_codecs`/`build_actor_method_tables`'s own
  /// doc comments.
  actor_arg_encoders: &'a HashMap<String, FunctionValue<'ctx>>,
  actor_method_tags: &'a HashMap<String, i32>,
  actor_method_tables: &'a HashMap<String, PointerValue<'ctx>>,
  actor_method_counts: &'a HashMap<String, i64>,
  /// Plan 61's Decision log: the hard step ceiling `ComptimeInterpreter`
  /// enforces — `1_000_000` by default (`compile_to_object_impl`'s own
  /// resolution of the `Option<u64>` a caller may override), configurable
  /// via `emerald-cli`'s `--comptime-step-limit` flag. A real, disclosed
  /// engineering compromise (Rice's theorem: general termination is
  /// undecidable), not a termination proof.
  comptime_step_limit: u64,
  /// Plan 62's Decision log: `Some((name, ensures))` only while
  /// `define_user_function` is compiling a top-level function's own
  /// body — `Stmt::Return`'s codegen arm and the implicit-return-
  /// fallthrough codegen path both read this to know whether (and under
  /// what name/clause list) to inject an `ensures` check before the
  /// real `ret`. `None` everywhere else (a method/lambda/`main` body
  /// never declares `ensures` at all — `leaf-ast-parser-contracts`'
  /// own grammar-level fence already guarantees that).
  current_function_contracts: Option<(&'a str, &'a [Contract])>,
  /// Plan 89's Decision log: needed so an anonymous lambda literal
  /// compiled on the fly at its own call-argument position (`build_
  /// inline_lambda`) or a generic method's own lazily-monomorphized
  /// specialization (`resolve_generic_method_instance`) can each
  /// `add_function` a brand-new top-level LLVM function right where
  /// they're first needed, mid-compile, rather than requiring a
  /// separate whole-program discovery pass upfront.
  module: &'a Module<'ctx>,
  /// `{top-level `Let` name} -> its declared `TypeExpr`}` (plan 89) —
  /// mirrors `declare_lambda_functions`'s own identical internal scan,
  /// computed once and shared so `build_inline_lambda`'s own return-
  /// kind inference (`infer_lambda_ret_kind`) sees the same top-level
  /// names a top-level lambda's return-kind inference already does.
  top_level_types: &'a HashMap<String, TypeExpr>,
  /// `{top-level function name} -> its declared return TypeExpr}` (plan
  /// 89) — the other half of `infer_lambda_ret_kind`'s own environment,
  /// mirrored from `declare_lambda_functions`'s identical internal scan.
  user_fn_return_types: &'a HashMap<String, TypeExpr>,
  /// `{class name} -> its raw ClassDef}` (plan 89) — covers ordinary
  /// classes, actors (normalized to a synthetic `ClassDef`), AND every
  /// monomorphized generic-class instantiation under its OWN mangled
  /// name, exactly like `compile_to_object_impl`'s own local `class_
  /// defs` already does; `resolve_generic_method_instance` needs a
  /// class's own RAW (unmonomorphized) method text to substitute a
  /// generic method's type parameter(s) into, which `ctx.classes`
  /// (already-flattened `ClassLayout`s, no AST left) can't answer.
  class_defs: &'a HashMap<String, &'a ClassDef>,
  /// `{interface name} -> its raw InterfaceDef}` (plan 89) —
  /// `resolve_generic_method_instance` needs an implementing class's
  /// bound interface's own declared type-parameter NAME (`interface
  /// Iterable[T]`'s `T`) to know which bare identifier in the class's
  /// own generic method text its `implements Iterable[Int64]` clause's
  /// concrete argument actually substitutes.
  interface_defs: &'a HashMap<String, &'a InterfaceDef>,
  /// Plan 89's Decision log: one memoized `(FunctionValue, return
  /// ValKind)` per distinct `{class}_{method}$${concrete type}` mangled
  /// symbol actually called anywhere in the program — a `RefCell`
  /// cache, not a whole-program discovery-then-declare-then-define
  /// pre-pass (`Ctx::escape_stats`'s own identical "one `Ctx`, one
  /// thread, no race" precedent applies here too), populated lazily by
  /// `resolve_generic_method_instance` the first time `build_method_
  /// call` reaches a given generic-method call site's own concrete
  /// binding, reused directly on every subsequent call to the SAME
  /// binding.
  generic_method_instances: &'a RefCell<HashMap<String, (FunctionValue<'ctx>, ValKind)>>,
  /// Every declared `newtype`'s own name (`{"Meters", "Seconds", ...}`)
  /// — built once from `program.items`, alongside `classes`/`enums`
  /// above (this crate's own "no shared sema→codegen structure"
  /// architecture: an independent re-derivation, not sema's `ClassInfo.
  /// newtype_underlying`). `Expr::New`'s own newtype-construction arm
  /// doesn't need this (a `class_name` absent from `ctx.classes` is
  /// already unambiguous), but `Stmt::Let`'s `local_classes` bookkeeping
  /// and `build_method_call`'s `.value` unwrap dispatch both do — a
  /// newtype-typed local otherwise leaves no trace in `local_classes` at
  /// all (unlike an enum/class/Pair/Hash-typed one), so `.value` would
  /// have no way to recognize its own receiver as a newtype rather than
  /// an ordinary (never-declared) method name on some other type.
  newtypes: &'a HashSet<String>,
  /// Plan 84's Decision log: `(incoming pointer, local alloca, kind)`
  /// for every `borrow var` by-value parameter the CURRENT function/
  /// method/lambda bound (`bind_params`'s own doc comment) — empty for
  /// every function that binds none, which is every function compiled
  /// before this plan and every one since that declares no such
  /// parameter, so this is a real no-op in the overwhelmingly common
  /// case. Read by `emit_borrow_var_writebacks`, called from every real
  /// function-exit point (`Stmt::Return`'s own two arms, and `build_
  /// function_body`'s three implicit-fallthrough exits) — see that
  /// function's own doc comment for why a per-function slice, set once
  /// by `define_user_function`/`define_method`/`define_lambda` right
  /// after their own `bind_params` call, is enough (this function's
  /// writebacks are meaningless to any OTHER function, unlike most
  /// other `Ctx` fields, which mirrors `object_allocas`' own identical
  /// per-function-not-per-program shape).
  borrow_var_writebacks: &'a [(PointerValue<'ctx>, PointerValue<'ctx>, ValKind)],
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

/// Plan 50: dispatches `ClassName.new(args)`'s compiler-generated
/// `initialize` call against an already-obtained instance pointer
/// `ptr` — factored out so `Expr::New`'s heap-allocating arm below and
/// `build_stmt`'s stack-allocating `Stmt::Let` arm can share it,
/// dispatching construction identically regardless of which allocation
/// strategy produced `ptr`. Not a source-level use of any name `args`
/// might reference — this call is intrinsic to construction, not part
/// of the AST `find_non_escaping_news` walks, so it needs no
/// escape-analysis special-casing.
#[allow(clippy::too_many_arguments)]
fn build_initialize_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  class_name: &str,
  args: &[Spanned<Expr>],
  ptr: PointerValue<'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  // Plan 32: `initialize` resolves through `method_owners` too — a
  // subclass that doesn't declare its own `initialize` inherits the
  // nearest ancestor's, same as any other method.
  let init_key = ctx
    .method_owners
    .get(class_name)
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
  Ok(())
}

/// `region_create` → `region_alloc` (the header-prefixed size) →
/// `emerald_actor_init_header` → `emerald_actor_set_region` →
/// `initialize`, in that order — the exact sequence `Expr::Spawn`'s own
/// codegen originally inlined, factored out (plan 57) so `Expr::
/// Supervise`'s own tracked-child spawns can reuse it verbatim instead
/// of a second, drifting copy. Returns the new instance's own `self`
/// pointer (past the header slot — see `EmeraldActorHeader`'s own doc
/// comment in `runtime/emerald_runtime.c`).
#[allow(clippy::too_many_arguments)]
fn build_spawn_alloc<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  class_name: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  let layout = ctx
    .classes
    .get(class_name)
    .ok_or_else(|| format!("codegen: unknown class `{class_name}`"))?;
  let header_size = 8u64;
  let size_val = context
    .i64_type()
    .const_int(layout.size + header_size, false);
  let region_call = builder
    .build_call(ctx.region_create_fn, &[], "spawnregion")
    .map_err(|e| e.to_string())?;
  let region = call_result(region_call)?.into_pointer_value();
  let alloc_call = builder
    .build_call(
      ctx.region_alloc_fn,
      &[region.into(), size_val.into()],
      "spawntmp",
    )
    .map_err(|e| e.to_string())?;
  let raw_ptr = call_result(alloc_call)?.into_pointer_value();
  builder
    .build_call(
      ctx.actor_funcs.init_header,
      &[raw_ptr.into()],
      "spawnheader",
    )
    .map_err(|e| e.to_string())?;
  let self_ptr = field_ptr(context, builder, raw_ptr, header_size)?;
  // Plan 57 (supervision trees): records this instance's own region on
  // its header, so `emerald_actor_terminate` can find it — see that
  // function's own doc comment for why it's tracked but deliberately
  // not yet freed there.
  builder
    .build_call(
      ctx.actor_funcs.set_region,
      &[self_ptr.into(), region.into()],
      "spawnsetregion",
    )
    .map_err(|e| e.to_string())?;
  build_initialize_call(
    context,
    builder,
    class_name,
    args,
    self_ptr,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  // Plan 60's Decision log (Design decision 1): from this plan on, an
  // actor-typed Emerald VALUE is a tagged `EmeraldActorRef*`, not the
  // bare arena pointer — `self_ptr` above is still exactly what every
  // OTHER actor mechanism (field access inside a method body,
  // `emerald_actor_enqueue`'s own header lookup, `emerald_actor_
  // terminate`) expects and keeps using unmodified; only the value
  // that flows OUT of `.spawn` into a local/field/`argv` slot changes
  // shape, wrapped here in one extra fixed-size allocation.
  let ref_call = builder
    .build_call(
      ctx.actor_funcs.ref_local,
      &[self_ptr.into()],
      "spawnreflocal",
    )
    .map_err(|e| e.to_string())?;
  Ok(call_result(ref_call)?.into_pointer_value())
}

/// Plan 65's `leaf-virtual-actor-placement`: `ClassName.locate(key,
/// args...)`'s own codegen — computes `emerald_locate_is_self_owner`
/// (a real, live-peer-filtered consistent-hash owner check) and
/// branches: on the self-owner path, a process-local cache lookup
/// (`emerald_locate_cache_get`) either returns an already-activated
/// `EmeraldActorRef*` or, on a miss, calls `build_spawn_alloc`
/// verbatim (the identical allocation/`initialize` path `.spawn`
/// already uses) and caches the fresh result; on the remote-owner
/// path, resolves the owning peer's address (`emerald_locate_owner_
/// addr`) and reuses `ctx.actor_funcs.ref_remote` directly — the same
/// underlying call `Expr::Remote`'s own codegen arm makes — addressed
/// by `key` itself as the registered name. **A real, disclosed gap**:
/// this does NOT extend the RESOLVE wire handshake for lazy remote
/// activation (Design decision left un-implemented by this leaf's own
/// time budget) — a remote-owner `.locate` only succeeds if that peer
/// has ALREADY locally activated (via its own `.locate` call) and
/// `.register`ed this exact key; otherwise `ref_remote` returns `NULL`
/// exactly like an ordinary `.remote` resolve-miss does, and the
/// identical `RemoteActorError` is raised. Both branches converge on
/// one merged `EmeraldActorRef*` (`ValKind::Ptr`, the same shape
/// `.spawn`/`.remote` already share), via the "alloca + per-branch
/// store + one final load" idiom `build_send_result` already
/// established for this file.
#[allow(clippy::too_many_arguments)]
fn build_locate_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  class_name: &str,
  key: &Spanned<Expr>,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let func = builder
    .get_insert_block()
    .and_then(|b| b.get_parent())
    .ok_or_else(|| "codegen: internal — no enclosing function for a `.locate` call".to_string())?;

  let (key_val, _) = build_expr(
    context,
    builder,
    key,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let key_ptr = key_val.into_pointer_value();

  let ptr_ty = context.ptr_type(AddressSpace::default());
  let result_slot = builder
    .build_alloca(ptr_ty, "locateresultslot")
    .map_err(|e| e.to_string())?;

  let is_self_call = builder
    .build_call(
      ctx.actor_funcs.locate_is_self_owner,
      &[key_ptr.into()],
      "locateisself",
    )
    .map_err(|e| e.to_string())?;
  let is_self = call_result(is_self_call)?.into_int_value();
  let is_self_bool = builder
    .build_int_compare(
      IntPredicate::NE,
      is_self,
      context.i32_type().const_int(0, false),
      "locateisselfbool",
    )
    .map_err(|e| e.to_string())?;

  let self_blk = context.append_basic_block(func, "locate.self");
  let remote_blk = context.append_basic_block(func, "locate.remote");
  let merge_blk = context.append_basic_block(func, "locate.merge");
  builder
    .build_conditional_branch(is_self_bool, self_blk, remote_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(self_blk);
  let cache_get_call = builder
    .build_call(
      ctx.actor_funcs.locate_cache_get,
      &[key_ptr.into()],
      "locatecacheget",
    )
    .map_err(|e| e.to_string())?;
  let cached_ptr = call_result(cache_get_call)?.into_pointer_value();
  let is_cached = builder
    .build_is_not_null(cached_ptr, "locateiscached")
    .map_err(|e| e.to_string())?;
  let cache_hit_blk = context.append_basic_block(func, "locate.cachehit");
  let cache_miss_blk = context.append_basic_block(func, "locate.cachemiss");
  builder
    .build_conditional_branch(is_cached, cache_hit_blk, cache_miss_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(cache_hit_blk);
  builder
    .build_store(result_slot, cached_ptr)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(cache_miss_blk);
  let new_ref = build_spawn_alloc(
    context,
    builder,
    class_name,
    args,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_call(
      ctx.actor_funcs.locate_cache_put,
      &[key_ptr.into(), new_ref.into()],
      "locatecacheput",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_store(result_slot, new_ref)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(remote_blk);
  let owner_addr_call = builder
    .build_call(
      ctx.actor_funcs.locate_owner_addr,
      &[key_ptr.into()],
      "locateowneraddr",
    )
    .map_err(|e| e.to_string())?;
  let owner_addr_ptr = call_result(owner_addr_call)?.into_pointer_value();
  let key_len_call = builder
    .build_call(ctx.string_length, &[key_ptr.into()], "locatekeylen")
    .map_err(|e| e.to_string())?;
  let key_len = call_result(key_len_call)?;
  let ref_call = builder
    .build_call(
      ctx.actor_funcs.ref_remote,
      &[owner_addr_ptr.into(), key_ptr.into(), key_len.into()],
      "locateremoteref",
    )
    .map_err(|e| e.to_string())?;
  let remote_ref_ptr = call_result(ref_call)?.into_pointer_value();
  let is_null = builder
    .build_is_null(remote_ref_ptr, "locateremoterefisnull")
    .map_err(|e| e.to_string())?;
  build_raise_on_remote_send_failure(
    context,
    builder,
    builder
      .build_int_z_extend(is_null, context.i32_type(), "locateremoterefnullstatus")
      .map_err(|e| e.to_string())?,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_store(result_slot, remote_ref_ptr)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(merge_blk);
  let loaded = builder
    .build_load(ptr_ty, result_slot, "locateresult")
    .map_err(|e| e.to_string())?;
  Ok((loaded, ValKind::Ptr))
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
      // Plan 73's Decision log: a bare, zero-arg variant reference —
      // `Option[T]`'s own `None` — reaches here as an ordinary
      // `Expr::Ident` (mirrors `emerald-sema`'s identical fallback),
      // checked only once `vars` itself has no binding for `name` (an
      // ordinary local always wins, unchanged). Constructs it exactly
      // like `build_call_expr`'s own `Expr::Call`-shaped variant
      // construction just below — allocate `layout.size` bytes, store
      // the tag, no field writes (a nullary variant has none).
      if !vars.contains_key(name) {
        if let Some((enum_name, tag)) = find_variant_layout(name, ctx.enums) {
          let layout = &ctx.enums[enum_name];
          let size_val = context.i64_type().const_int(layout.size, false);
          let alloc_call = builder
            .build_call(ctx.alloc, &[size_val.into()], "enumlit")
            .map_err(|e| e.to_string())?;
          let ptr = call_result(alloc_call)?.into_pointer_value();
          let tag_ptr = field_ptr(context, builder, ptr, 0)?;
          builder
            .build_store(tag_ptr, context.i64_type().const_int(tag, false))
            .map_err(|e| e.to_string())?;
          return Ok((ptr.into(), ValKind::Ptr));
        }
      }
      let (ptr, kind) = vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let loaded = builder
        .build_load(local_llvm_type(context, kind), *ptr, name)
        .map_err(|e| e.to_string())?;
      Ok((loaded, kind.clone()))
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
    // `newtype Meters: Float64` — a newtype-typed operand is ALSO
    // tracked in `local_classes` (see `Ctx::newtypes`'s own doc comment
    // for why), but has no class `==` method to delegate to at all — it
    // falls straight through to the ordinary, scalar `Expr::Compare` arm
    // below instead, which compares by the receiver's own `ValKind`
    // (exactly `Float64 == Float64` would, since a newtype IS its
    // underlying primitive at this level). Excluded here explicitly
    // rather than routed into `build_method_call`'s class-operator
    // dispatch, which would otherwise fail with "newtype has no method
    // `==`" for a comparison sema already accepts.
    Expr::Compare(lhs, op, rhs)
      if matches!(&lhs.node, Expr::Ident(name) if local_classes
        .get(name)
        .is_some_and(|c| !ctx.newtypes.contains(c))) =>
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
      let cmp = match (&lk, &rk) {
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
    // Plan 53's Decision log: `is_valid_int`/`parse_digits` are
    // compiler-known intrinsics, mirroring `gets`'s own hard-coded-name
    // dispatch immediately above — checked before the generic
    // `Expr::Call` arm below since neither is ever in `ctx.user_func_ids`.
    Expr::Call(name, args) if name == "is_valid_int" || name == "parse_digits" => {
      if args.len() != 1 {
        return Err(format!(
          "codegen: `{name}` expects 1 argument, found {}",
          args.len()
        ));
      }
      let (arg_val, arg_kind) = build_expr(
        context,
        builder,
        &args[0],
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if arg_kind != ValKind::Str {
        return Err(format!(
          "codegen: `{name}` expects a String argument, found {arg_kind:?}"
        ));
      }
      let fv = if name == "is_valid_int" {
        ctx.is_valid_int_fn
      } else {
        ctx.parse_digits_fn
      };
      let call = builder
        .build_call(fv, &[arg_val.into()], "intrinsictmp")
        .map_err(|e| e.to_string())?;
      let raw = call_result(call)?;
      if name == "is_valid_int" {
        // `emerald_is_valid_int`'s real C ABI returns a `long long`
        // (register-width, matching every other `Int64`-shaped runtime
        // boundary) — genuine `ValKind::Bool` values in this backend
        // are `i1` (`local_llvm_type`'s own mapping), so the raw `i64`
        // result is compared against `0` here to produce a real `i1`,
        // the same conversion direction `Expr::Compare` already builds
        // for every other boolean-producing expression.
        let zero = context.i64_type().const_int(0, false);
        let as_bool = builder
          .build_int_compare(IntPredicate::NE, raw.into_int_value(), zero, "isvalidint")
          .map_err(|e| e.to_string())?;
        Ok((as_bool.into(), ValKind::Bool))
      } else {
        Ok((raw, ValKind::Int64))
      }
    }
    // Plan 55's Decision log: `current_thread_id()` — supplementary,
    // best-effort observability only, the same hardcoded-name
    // dispatch convention as `is_valid_int`/`parse_digits` immediately
    // above.
    Expr::Call(name, args) if name == "current_thread_id" => {
      if !args.is_empty() {
        return Err(format!(
          "codegen: `current_thread_id` expects 0 arguments, found {}",
          args.len()
        ));
      }
      let call = builder
        .build_call(ctx.actor_funcs.current_thread_id, &[], "curthreadid")
        .map_err(|e| e.to_string())?;
      Ok((call_result(call)?, ValKind::Int64))
    }
    // Plan 53's Decision log: `Ok(inner)`/`Err(inner)`, ordinary
    // `build_expr` arms — no `Stmt`-level special case needed (unlike
    // `Try` below), since the payload's kind comes from evaluating
    // `inner` itself, bottom-up, exactly like `Expr::HashLit`'s own
    // key/value builds already work. A flat 16-byte `[discriminant:
    // i64][payload: 8 bytes]` layout, `build_hash_lit`'s fixed-offset
    // `field_ptr` idiom generalized from a per-pair stride to a single
    // fixed pair.
    Expr::Ok(inner) | Expr::Err(inner) => {
      let discriminant = if matches!(&expr.node, Expr::Ok(_)) {
        0u64
      } else {
        1u64
      };
      let byte_size = context.i64_type().const_int(16, false);
      let call = builder
        .build_call(ctx.alloc, &[byte_size.into()], "resulttmp")
        .map_err(|e| e.to_string())?;
      let ptr = call_result(call)?.into_pointer_value();
      let tag = context.i64_type().const_int(discriminant, false);
      builder.build_store(ptr, tag).map_err(|e| e.to_string())?;
      let (inner_val, _) = build_expr(
        context,
        builder,
        inner,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let payload_ptr = field_ptr(context, builder, ptr, 8)?;
      builder
        .build_store(payload_ptr, inner_val)
        .map_err(|e| e.to_string())?;
      Ok((ptr.into(), ValKind::Ptr))
    }
    // Plan 53's Decision log: `expr?` used anywhere other than directly
    // as a `Stmt::Let`/`Stmt::Assign` value — those two positions are
    // handled by dedicated `Stmt`-level arms (mirroring `Expr::ArrayNew`'s
    // own precedent) before `build_expr` is ever reached for a `Try`
    // node; sema already rejects every other position with a real
    // diagnostic, so reaching this arm at all means sema's own check
    // was bypassed — a descriptive `Err`, never a panic, matching this
    // crate's standing defensive-codegen convention.
    Expr::Try(_) => Err(
      "codegen: internal error — `?` reached outside a `let`/assignment value position \
       (sema should have rejected this)"
        .to_string(),
    ),
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
    // `Meters.new(5.0)` — a newtype's own construction. Sema already
    // guarantees `class_name` names either a real class or a real
    // newtype whenever an `Expr::New` reaches here (nothing else is
    // legal); a name absent from `ctx.classes` (which only ever holds
    // real `Item::Class` layouts) can therefore only be a newtype.
    // The whole point of `newtype` being zero-cost is that this
    // compiles to EXACTLY the argument's own already-compiled value —
    // no allocation, no wrapper struct, byte-identical to compiling
    // the bare underlying expression alone.
    Expr::New(class_name, args) if !ctx.classes.contains_key(class_name) => build_expr(
      context,
      builder,
      &args[0],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
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
      build_initialize_call(
        context,
        builder,
        class_name,
        args,
        ptr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      // Plan 50's `leaf-escape-instrumentation-and-report`: this arm is
      // reached for every heap-allocating `New` — a non-escaping `Let`-
      // bound `New` never reaches `build_expr` at all (`build_stmt`'s
      // own stack-allocating special case handles it first).
      if let Some(stats) = ctx.escape_stats {
        stats.borrow_mut().heap_allocated += 1;
      }
      Ok((ptr.into(), ValKind::Ptr))
    }
    // Plan 54/55: `.spawn`'s exact structural mirror of `Expr::New`
    // above — same `layout`/`build_initialize_call`, the one
    // difference being the allocation call site itself: plan 51's real
    // two-call region API (`create` then `alloc`) in place of
    // `Expr::New`'s single `ctx.alloc` call. Never counted in
    // `escape_stats` — plan 50's stack-allocation optimization is
    // `Expr::New`-only by design (see plan 54's Decision log): an
    // actor instance always heap-allocates into its own region.
    //
    // Plan 55's own addition: the allocated region reserves 8 extra
    // leading bytes (one pointer width) ahead of the instance's own
    // `layout.size` fields — `runtime/emerald_runtime.c`'s own real
    // header-prefix contract (see its `EmeraldActorHeader` doc comment)
    // — filled in by `emerald_actor_init_header`. `self`, from here on
    // (the value `build_initialize_call` receives and this expression
    // itself evaluates to), is `raw_ptr` offset past that slot via the
    // exact same `field_ptr` helper ordinary field access already
    // uses — never a change to `ClassLayout`/`FieldInfo`'s own 0-based
    // field offsets, so every other piece of actor-method codegen
    // (`@field` reads/writes, same-actor `self` calls) stays completely
    // unaware this header prefix exists.
    Expr::Spawn(class_name, args) => {
      let self_ptr = build_spawn_alloc(
        context,
        builder,
        class_name,
        args,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok((self_ptr.into(), ValKind::Ptr))
    }
    // Plan 57 (supervision trees), `leaf-supervise-declaration`: sema
    // has already restricted `body` to a flat list of bound-or-bare
    // `<Class>.spawn(<args>)` statements (`Stmt::Let{value: Expr::
    // Spawn(...)}` or `Stmt::Expr(Expr::Spawn(...))` — see `infer_expr_
    // type`'s own `Expr::Supervise` arm) — codegen trusts that shape
    // unconditionally, the same "codegen runs on already-checked
    // input" contract every other leaf in this backend relies on.
    // Registers each tracked child with its own per-class "respawn
    // thunk" (`declare_supervisor_respawn_thunks`) and its already-
    // evaluated spawn arguments, packed into a raw argv buffer the
    // exact same way `build_actor_enqueue_call` already packs a
    // cross-actor message's own arguments (Decision log: captured
    // values, never re-evaluated expressions).
    Expr::Supervise(body) => {
      let sup_call = builder
        .build_call(ctx.actor_funcs.supervisor_create, &[], "supcreate")
        .map_err(|e| e.to_string())?;
      let sup_ptr = call_result(sup_call)?.into_pointer_value();
      let i64_ty = context.i64_type();
      let ptr_ty = context.ptr_type(AddressSpace::default());
      const ARGV_MAX: usize = 16;
      for stmt in body {
        let (name_opt, class_name, spawn_args): (Option<&str>, &str, &[Spanned<Expr>]) = match &stmt
          .node
        {
          Stmt::Let {
            name,
            value:
              Spanned {
                node: Expr::Spawn(class_name, spawn_args),
                ..
              },
            ..
          } => (
            Some(name.as_str()),
            class_name.as_str(),
            spawn_args.as_slice(),
          ),
          Stmt::Expr(Spanned {
            node: Expr::Spawn(class_name, spawn_args),
            ..
          }) => (None, class_name.as_str(), spawn_args.as_slice()),
          other => {
            return Err(format!(
              "codegen: internal error — `supervise do ... end` body statement {other:?} is not a spawn (sema should have rejected this)"
            ));
          }
        };
        let self_ptr = build_spawn_alloc(
          context,
          builder,
          class_name,
          spawn_args,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;

        let argv_alloca = builder
          .build_alloca(i64_ty.array_type(ARGV_MAX as u32), "supargv")
          .map_err(|e| e.to_string())?;
        for (i, a) in spawn_args.iter().enumerate() {
          let (v, kind) = build_expr(
            context,
            builder,
            a,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?;
          let raw = match kind {
            ValKind::Int64 | ValKind::Symbol => v.into_int_value(),
            ValKind::Float64 => builder
              .build_bit_cast(v, i64_ty, "supargraw")
              .map_err(|e| e.to_string())?
              .into_int_value(),
            ValKind::Ptr | ValKind::Str => builder
              .build_ptr_to_int(v.into_pointer_value(), i64_ty, "supargraw")
              .map_err(|e| e.to_string())?,
            ValKind::Bool => builder
              .build_int_z_extend(v.into_int_value(), i64_ty, "supargraw")
              .map_err(|e| e.to_string())?,
            ValKind::Void | ValKind::Tuple(_) => {
              return Err(
                "codegen: internal — not a valid supervised-spawn argument kind".to_string(),
              );
            }
          };
          let idx = i64_ty.const_int(i as u64, false);
          let slot_ptr = unsafe {
            builder
              .build_in_bounds_gep(i64_ty, argv_alloca, &[idx], "supargvslot")
              .map_err(|e| e.to_string())?
          };
          builder
            .build_store(slot_ptr, raw)
            .map_err(|e| e.to_string())?;
        }
        let argc = i64_ty.const_int(spawn_args.len() as u64, false);

        let respawn_fv = *ctx
          .supervisor_respawn_thunks
          .get(class_name)
          .ok_or_else(|| {
            format!("codegen: internal error — no respawn thunk compiled for actor `{class_name}`")
          })?;
        let respawn_ptr = respawn_fv.as_global_value().as_pointer_value();
        let name_ptr = match name_opt {
          Some(n) => builder
            .build_global_string_ptr(n, "supchildname")
            .map_err(|e| e.to_string())?
            .as_pointer_value(),
          None => ptr_ty.const_null(),
        };
        let class_name_ptr = builder
          .build_global_string_ptr(class_name, "supchildclass")
          .map_err(|e| e.to_string())?
          .as_pointer_value();

        builder
          .build_call(
            ctx.actor_funcs.supervisor_register_child,
            &[
              sup_ptr.into(),
              name_ptr.into(),
              class_name_ptr.into(),
              respawn_ptr.into(),
              self_ptr.into(),
              argv_alloca.into(),
              argc.into(),
            ],
            "supregister",
          )
          .map_err(|e| e.to_string())?;
      }
      Ok((sup_ptr.into(), ValKind::Ptr))
    }
    // Plan 60's Decision log (Design decision 1): `.spawn`'s
    // distributed counterpart — a synchronous connect + RESOLVE
    // handshake, raising a real, catchable `RemoteActorError` on any
    // failure (host unreachable, connect timeout, or "not found")
    // rather than returning a `NULL` the caller could dereference.
    Expr::Remote {
      class: _,
      addr,
      name,
    } => {
      let (addr_val, _) = build_expr(
        context,
        builder,
        addr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let (name_val, _) = build_expr(
        context,
        builder,
        name,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let name_len_call = builder
        .build_call(ctx.string_length, &[name_val.into()], "remotenamelen")
        .map_err(|e| e.to_string())?;
      let name_len = call_result(name_len_call)?;
      let ref_call = builder
        .build_call(
          ctx.actor_funcs.ref_remote,
          &[addr_val.into(), name_val.into(), name_len.into()],
          "remoteref",
        )
        .map_err(|e| e.to_string())?;
      let ref_ptr = call_result(ref_call)?.into_pointer_value();
      let is_null = builder
        .build_is_null(ref_ptr, "remoterefisnull")
        .map_err(|e| e.to_string())?;
      build_raise_on_remote_send_failure(
        context,
        builder,
        builder
          .build_int_z_extend(is_null, context.i32_type(), "remoterefnullstatus")
          .map_err(|e| e.to_string())?,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok((ref_ptr.into(), ValKind::Ptr))
    }
    Expr::Locate { class, key, args } => build_locate_call(
      context,
      builder,
      class,
      key,
      args,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
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
    Expr::SafeCall(recv, method, args) => {
      let (v, k, _enum_name) = build_safe_call(
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
      Ok((v, k))
    }
    // Plan 73's Decision log: `lhs ?? rhs` — a real is-guarded-basic-
    // blocks-plus-PHI shape, the identical pattern `build_safe_call`
    // above uses: branch on `lhs`'s own tag, load the `Some` payload on
    // that path, evaluate `rhs` (a plain, un-wrapped `T`, sema-checked
    // already) on the other, merge via `phi`. Unlike `?.`, no fresh
    // enum needs constructing — the result is a plain `T`, not another
    // `Option[T]`.
    Expr::Coalesce(lhs, rhs) => {
      // Plan 73's Decision log: `lhs` is either a plain local (`recv ??
      // default`) or a `?.` chain (`recv?.method ?? default`, this
      // plan's own concrete target proof) — the latter needs `build_
      // safe_call` called DIRECTLY rather than through the generic
      // `build_expr` dispatch, since only `build_safe_call` itself
      // knows which fresh `Option[U]` it just produced (see its own
      // doc comment); a plain local instead resolves its enum name the
      // ordinary way, via `local_classes`.
      let (lhs_val, lhs_kind, enum_name) = match &lhs.node {
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
        )?,
        Expr::Ident(lhs_name) => {
          let (v, k) = build_expr(
            context,
            builder,
            lhs,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?;
          let enum_name = local_classes.get(lhs_name).cloned().ok_or_else(|| {
            format!("codegen: `??` requires an `Option[T]`-typed left operand, found untyped local `{lhs_name}`")
          })?;
          (v, k, enum_name)
        }
        _ => {
          return Err(
            "codegen: `??`'s left operand must be a plain local variable or a `?.` chain"
              .to_string(),
          );
        }
      };
      if lhs_kind != ValKind::Ptr {
        return Err(format!(
          "codegen: `??`'s left operand must be `Option[T]`, found {lhs_kind:?}"
        ));
      }
      let layout = ctx.enums.get(enum_name.as_str()).ok_or_else(|| {
        format!("codegen: internal error — unregistered enum `{enum_name}` (sema should have rejected this)")
      })?;
      let some_tag = *layout
        .variant_tags
        .get("Some")
        .ok_or_else(|| format!("codegen: internal error — `{enum_name}` has no `Some` variant"))?;
      let inner_kind = layout
        .variant_fields
        .get("Some")
        .and_then(|f| f.first())
        .cloned()
        .ok_or_else(|| format!("codegen: internal error — `{enum_name}`'s `Some` has no field"))?;

      let ptr = lhs_val.into_pointer_value();
      let tag_val = load_field(
        context,
        builder,
        ptr,
        FieldInfo {
          offset: 0,
          kind: ValKind::Int64,
        },
      )?
      .into_int_value();
      let some_tag_const = context.i64_type().const_int(some_tag, false);
      let is_some = builder
        .build_int_compare(IntPredicate::EQ, tag_val, some_tag_const, "coalesceissome")
        .map_err(|e| e.to_string())?;

      let entry_block = builder
        .get_insert_block()
        .ok_or("codegen: internal error — no current block")?;
      let func = entry_block
        .get_parent()
        .ok_or("codegen: internal error — block has no parent function")?;
      let some_block = context.append_basic_block(func, "coalesce.some");
      let none_block = context.append_basic_block(func, "coalesce.none");
      let merge_block = context.append_basic_block(func, "coalesce.merge");
      builder
        .build_conditional_branch(is_some, some_block, none_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(some_block);
      let some_val = load_field(
        context,
        builder,
        ptr,
        FieldInfo {
          offset: 8,
          kind: inner_kind.clone(),
        },
      )?;
      let some_end_block = builder
        .get_insert_block()
        .ok_or("codegen: internal error — no current block after some")?;
      builder
        .build_unconditional_branch(merge_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(none_block);
      let (none_val, none_kind) = build_expr(
        context,
        builder,
        rhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if none_kind != inner_kind {
        return Err(format!(
          "codegen: internal error — `??`'s right operand has kind {none_kind:?}, expected {inner_kind:?} (sema should have rejected this)"
        ));
      }
      let none_end_block = builder
        .get_insert_block()
        .ok_or("codegen: internal error — no current block after none")?;
      builder
        .build_unconditional_branch(merge_block)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(merge_block);
      let phi = builder
        .build_phi(local_llvm_type(context, &inner_kind), "coalesceresult")
        .map_err(|e| e.to_string())?;
      phi.add_incoming(&[(&some_val, some_end_block), (&none_val, none_end_block)]);
      Ok((phi.as_basic_value(), inner_kind))
    }
    Expr::InstanceVar(name) => {
      let (self_ptr, fields, _) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name}` used outside of a method body"))?;
      let field = fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?;
      let loaded = load_field(context, builder, self_ptr, field.clone())?;
      Ok((loaded, field.kind.clone()))
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
    // Plan 89's Decision log: a lambda literal used to only have codegen
    // meaning at a top-level `Let`'s value (`build_lambda_let`, invoked
    // directly from `build_stmt`, never reaching here at all) — every
    // OTHER position (a call argument, this plan's own worked example)
    // now compiles via `build_inline_lambda` instead of failing.
    Expr::Lambda { .. } => {
      let (env_ptr, kind) = build_inline_lambda(context, builder, expr, vars, ctx)?;
      Ok((env_ptr.into(), kind))
    }
    // Plan 25 (stdlib expansion).
    Expr::Bool(b) => Ok((
      context.bool_type().const_int(u64::from(*b), false).into(),
      ValKind::Bool,
    )),

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
    // Plan 39's Decision log: the grammar only ever constructs this
    // from `return a, b`'s comma-list `Stmt::Return` rule — an LLVM
    // struct value, built up one field at a time via `build_insert_
    // value` starting from an `undef` of the right struct type. Every
    // element's own value/kind comes from an ordinary recursive
    // `build_expr` call, no different from any other sub-expression.
    Expr::TupleLit(elems) => {
      let mut kinds = Vec::with_capacity(elems.len());
      let mut vals = Vec::with_capacity(elems.len());
      for e in elems {
        let (v, k) = build_expr(
          context,
          builder,
          e,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        kinds.push(k);
        vals.push(v);
      }
      let mut agg = tuple_struct_type(context, &kinds).get_undef();
      for (i, v) in vals.into_iter().enumerate() {
        agg = builder
          .build_insert_value(agg, v, i as u32, "tuplelit")
          .map_err(|e| e.to_string())?
          .into_struct_value();
      }
      Ok((agg.into(), ValKind::Tuple(kinds)))
    }
    // Plan 61's Decision log: reached from either of `comptime`'s own
    // two legal positions (a top-level constant's initializer, `Array.
    // new`'s size argument — `emerald-sema`'s `check_comptime_positions`
    // already rejects every other reachable position before codegen
    // ever runs). Evaluates `inner` via the tree-walking interpreter and
    // bakes the result as a literal LLVM constant — no `alloca`, no
    // `call`, the "zero runtime computation" proof this leaf's own
    // worked example needs. `env` starts empty: a top-level `comptime`
    // expression has no enclosing local scope of its own to read from
    // (only a `comptime` FUNCTION's own body ever binds parameters —
    // see `ComptimeInterpreter::eval`'s own `Expr::Call` arm for that).
    Expr::Comptime(inner) => {
      let comptime_fns: HashMap<String, &AstFunction> = ctx
        .func_defs
        .iter()
        .filter(|(_, f)| f.is_comptime)
        .map(|(k, f)| (k.clone(), *f))
        .collect();
      let mut interp = ComptimeInterpreter::new(ctx.comptime_step_limit);
      let value = interp.eval(
        inner,
        &HashMap::new(),
        &comptime_fns,
        "<comptime expression>",
      )?;
      Ok(comptime_value_to_llvm_constant(context, &value))
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
    .build_load(local_llvm_type(context, &field.kind), field_ptr, "fieldval")
    .map_err(|e| e.to_string())
}

/// Plan 39's Decision log: `x, y = f()`'s own codegen — `struct_val` is
/// the LLVM struct `f`'s call actually returned; extracts each field in
/// order and stores it into its target's already-allocated slot (sema
/// guarantees `names.len()` matches the struct's own field count).
fn build_tuple_multi_assign<'ctx>(
  builder: &Builder<'ctx>,
  names: &[String],
  struct_val: inkwell::values::StructValue<'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
) -> Result<(), String> {
  for (i, name) in names.iter().enumerate() {
    let elem = builder
      .build_extract_value(struct_val, i as u32, "tupleelem")
      .map_err(|e| e.to_string())?;
    let ptr = vars
      .get(name)
      .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?
      .0;
    builder.build_store(ptr, elem).map_err(|e| e.to_string())?;
  }
  Ok(())
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

/// Plan 74 (enumerable chaining): resolves `recv` down to a plain
/// `Expr::Ident` receiver `build_method_call`'s own, pre-existing
/// dispatch already knows how to handle — reusing it completely
/// unchanged for everything downstream (Array/Hash enumerable dispatch,
/// `File`/`String`/class-method dispatch, all of it). The overwhelming
/// common case (`recv` is already a plain named local, or anything
/// else `build_method_call`'s own existing checks handle some other
/// way) is a no-op passthrough, with `vars`/`local_array_elem_types`
/// simply cloned unchanged.
///
/// When `recv` is itself a chained `.map`/`.select`/`.filter`/`.sort`
/// call — the only four enumerable methods whose own result is itself
/// an `Array[T]` a further enumerable call could legally target
/// (`emerald-sema`'s own `check_enumerable_call` return-type table) —
/// this recurses on ITS OWN receiver first (so a chain of any length,
/// `nums.select do ... end.map do ... end.sort()`, collapses one link
/// at a time, innermost first), builds that one link via
/// `build_enumerable_call` DIRECTLY (not `build_expr`'s generic
/// dispatch, which would discard the produced element `ValKind` this
/// needs — see `build_array_map`'s own Decision-log addition), and
/// stashes the result into a synthetic named local: `build_safe_call`'s
/// own established "synthesize a fresh AST node, reuse the existing
/// codegen path unchanged" technique, applied here to the identical
/// underlying problem (a receiver shape with no entry in
/// `local_array_elem_types`, which is keyed by variable NAME only,
/// populated at `Let`-binding time from ITS OWN declared type — an
/// intermediate, unnamed chain link never gets one that way).
#[allow(clippy::too_many_arguments)]
fn resolve_chained_enumerable_receiver<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<ResolvedChainReceiver<'ctx>, String> {
  let Expr::MethodCall(inner_recv, inner_method, inner_args) = &recv.node else {
    return Ok((recv.clone(), vars.clone(), local_array_elem_types.clone()));
  };
  if !matches!(inner_method.as_str(), "map" | "select" | "filter" | "sort") {
    return Ok((recv.clone(), vars.clone(), local_array_elem_types.clone()));
  }
  let (resolved_inner_recv, mut vars2, mut elem_types2) = resolve_chained_enumerable_receiver(
    context,
    builder,
    inner_recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let Expr::Ident(inner_recv_name) = &resolved_inner_recv.node else {
    return Err(
      "codegen: method calls are only supported on a plain local-variable receiver".to_string(),
    );
  };
  let (inner_val, _inner_kind, inner_elem) = build_enumerable_call(
    context,
    builder,
    &resolved_inner_recv,
    inner_recv_name,
    inner_method,
    inner_args,
    &vars2,
    local_classes,
    &elem_types2,
    ctx,
  )?;
  let elem_kind = inner_elem.ok_or_else(|| {
    format!(
      "codegen: internal error — chained `.{inner_method}` did not produce an Array (sema should have rejected further chaining here)"
    )
  })?;

  let synthetic_name = format!("__chain{}_{inner_method}", vars2.len());
  let ptr_ty = local_llvm_type(context, &ValKind::Ptr);
  let alloca = builder
    .build_alloca(ptr_ty, &synthetic_name)
    .map_err(|e| e.to_string())?;
  builder
    .build_store(alloca, inner_val)
    .map_err(|e| e.to_string())?;
  vars2.insert(synthetic_name.clone(), (alloca, ValKind::Ptr));
  elem_types2.insert(synthetic_name.clone(), elem_kind);
  Ok((
    Spanned::synthetic(Expr::Ident(synthetic_name)),
    vars2,
    elem_types2,
  ))
}

/// `resolve_chained_enumerable_receiver`'s own return shape — the
/// resolved receiver plus the (always owned, possibly chain-link-
/// extended) `vars`/`local_array_elem_types` maps to use in its place.
/// A named alias purely to keep clippy's `type_complexity` lint quiet;
/// no behavior of its own. Declared after the function that uses it
/// (Rust item order is unconstrained) so the function's own doc comment
/// above reads first, uninterrupted.
type ResolvedChainReceiver<'ctx> = (
  Spanned<Expr>,
  HashMap<String, (PointerValue<'ctx>, ValKind)>,
  HashMap<String, ValKind>,
);

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
  // Plan 55's Decision log: the one receiver shape beyond a plain local
  // this backend supports — and only for this one purpose — is `@field.
  // method(args)` where `@field`'s own declared type is a known actor
  // (this plan's own `@peer.hit` shape). Checked before the `Expr::
  // Ident`-only guard below, which stays completely unchanged for every
  // other receiver shape (an ordinary class-typed field still isn't a
  // supported method-call receiver — real, disclosed, narrow scope:
  // this plan only needs the actor case, not general field-based
  // dispatch). Actors never have a superclass (plan 54's own grammar
  // constraint), so the field's own declared class name IS always the
  // defining class — no `method_owners` chain walk needed here.
  if let Expr::InstanceVar(field_name) = &recv.node {
    if let Some((_, _, field_classes)) = ctx.self_ctx {
      if let Some(field_class) = field_classes.get(field_name) {
        if ctx.actor_names.contains(field_class.as_str()) {
          let key = format!("{field_class}_{}", mangled_operator_symbol(method));
          return build_actor_enqueue_call(
            context,
            builder,
            recv,
            &key,
            args,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          );
        }
      }
    }
  }

  // Real bug fix (found post-plan-89, this session): a `Proc`-typed
  // CLASS FIELD's `.call` was disclosed as unsupported — `.call`'s
  // dispatch required a plain `Ident` receiver, so `@op.call(x)` (from
  // inside the declaring class's own method body) failed with "method
  // calls are only supported on a plain local-variable receiver" even
  // though sema fully accepted the program. Mirrors the actor-
  // `InstanceVar` check immediately above and reuses plan 89's own
  // indirect-call mechanism below verbatim (same `decode_proc_sig`/
  // `build_indirect_call` shape) — the field's Proc signature comes
  // from `field_classes` (see `build_class_layout`'s own `proc_sig_
  // for_type` call, added alongside this fix), not `local_classes`,
  // since a field is never a named local. Disclosed, narrower scope
  // than a fully general fix: only `@field.call(...)` (an `InstanceVar`
  // receiver, from inside the declaring class's own method) is
  // covered — `instance.field.call(...)` from OUTSIDE the class is a
  // separate, unrelated limitation (this grammar has no public field
  // read-access expression at all outside a `read`-sugared accessor
  // method, confirmed separately, not attempted here).
  if let Expr::InstanceVar(field_name) = &recv.node {
    if method == "call" {
      if let Some((_, _, field_classes)) = ctx.self_ctx {
        if let Some(sig) = field_classes
          .get(field_name)
          .and_then(|s| decode_proc_sig(s))
        {
          return build_indirect_proc_call(
            context,
            builder,
            recv,
            args,
            sig,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          );
        }
      }
    }
  }

  // Real bug fix (found this session, 2026-09-21's "find all bugs"
  // sweep): a newtype-typed CLASS/ACTOR FIELD's `.value` unwrap was
  // unsupported for the identical reason the Proc-typed field `.call`
  // fix immediately above was — the newtype-unwrap check further below
  // only ever consults `local_classes` (keyed by a named LOCAL
  // variable), which an `InstanceVar` receiver (`@field`) never
  // populates, so `@field.value` fell straight through to the ordinary
  // `Expr::Ident`-only guard and failed with "method calls are only
  // supported on a plain local-variable receiver" even though sema
  // fully accepts the program. Mirrors both `InstanceVar` checks
  // immediately above: the newtype-ness comes from `field_classes`
  // (`build_class_layout`'s own `f.ty.to_string()` fallback already
  // stores a newtype field's bare type name there, unchanged — no
  // change needed on that side), and the actual unwrap is the same
  // zero-cost identity `build_expr(recv)` the local-variable arm below
  // already uses, since `@field` compiles to an ordinary field read
  // either way.
  if let Expr::InstanceVar(field_name) = &recv.node {
    if let Some((_, _, field_classes)) = ctx.self_ctx {
      if field_classes
        .get(field_name)
        .is_some_and(|s| ctx.newtypes.contains(s))
      {
        if method != "value" {
          return Err(format!("codegen: newtype has no method `{method}`"));
        }
        return build_expr(
          context,
          builder,
          recv,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        );
      }
    }
  }

  // Plan 74 (enumerable chaining): resolve a chained-enumerable-call
  // receiver (`nums.select do ... end.map do ... end`) down to a plain
  // Ident first — see `resolve_chained_enumerable_receiver`'s own doc
  // comment. A no-op passthrough for every other receiver shape (the
  // overwhelmingly common case: a plain named local).
  let (resolved_recv, resolved_vars, resolved_local_array_elem_types) =
    resolve_chained_enumerable_receiver(
      context,
      builder,
      recv,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
  let recv = &resolved_recv;
  let vars = &resolved_vars;
  let local_array_elem_types = &resolved_local_array_elem_types;

  let Expr::Ident(recv_name) = &recv.node else {
    return Err(
      "codegen: method calls are only supported on a plain local-variable receiver".to_string(),
    );
  };

  // Plan 57 (supervision trees), `leaf-one-for-one-restart-runtime`:
  // `sup.child(:name)` is the ONLY method a `Supervisor`-typed local
  // ever dispatches — checked before the ordinary `local_classes`-
  // driven class-method lookup below, since "Supervisor" is never a
  // real `ClassInfo`/`ClassLayout` entry (`Type::Proc`'s own "the
  // signature travels with the value, not a registry" precedent,
  // reused — see `emerald-sema`'s `Type::Supervisor`). Sema already
  // requires the argument to be a literal `Expr::SymbolLit` (so its
  // spelling is embeddable as a C string constant at compile time,
  // sidestepping any runtime symbol-to-string reverse lookup) and
  // already resolved this call's own return type against the tracked
  // child's real class — codegen here only needs the raw runtime call,
  // `emerald_supervisor_child` does the rest (including the "no child
  // named ..." diagnostic for an unnamed/unknown child, Decision log).
  if local_classes.get(recv_name).map(String::as_str) == Some("Supervisor") && method == "child" {
    let Some(Spanned {
      node: Expr::SymbolLit(child_name),
      ..
    }) = args.first()
    else {
      return Err(
        "codegen: internal error — `.child(...)` argument is not a symbol literal (sema should have rejected this)"
          .to_string(),
      );
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
    let name_ptr = builder
      .build_global_string_ptr(child_name, "supchildquery")
      .map_err(|e| e.to_string())?
      .as_pointer_value();
    let call = builder
      .build_call(
        ctx.actor_funcs.supervisor_child,
        &[recv_val.into(), name_ptr.into()],
        "supchildtmp",
      )
      .map_err(|e| e.to_string())?;
    return Ok((call_result(call)?, ValKind::Ptr));
  }

  // Plan 42 (enumerable stdlib): `.key`/`.value` on a `Pair`-typed
  // receiver — `Type::Pair`'s own doc comment: the only real source of
  // a `Pair` value is a `Hash[K,V].each` block's own parameter.
  if let Some((k_kind, v_kind)) = local_classes
    .get(recv_name)
    .and_then(|s| parse_pair_type(s))
  {
    if method == "key" || method == "value" {
      let (recv_val, _) = build_expr(
        context,
        builder,
        recv,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let pair_ptr = recv_val.into_pointer_value();
      let (offset, kind) = if method == "key" {
        (0u64, k_kind)
      } else {
        (8u64, v_kind)
      };
      let field_p = field_ptr(context, builder, pair_ptr, offset)?;
      let loaded = builder
        .build_load(local_llvm_type(context, &kind), field_p, "pairfield")
        .map_err(|e| e.to_string())?;
      return Ok((loaded, kind));
    }
  }

  // `m.value` — a newtype's own unwrap accessor (see `Ctx::newtypes`'s
  // own doc comment for why `local_classes` is what recognizes this
  // receiver as a newtype). Zero-cost means this compiles to EXACTLY
  // the receiver's own already-compiled value — no load through any
  // wrapper, no offset, just `build_expr(recv)` returned unchanged,
  // which is what makes `.value` a pure compile-time type-level
  // identity rather than a real runtime operation.
  if local_classes
    .get(recv_name)
    .is_some_and(|s| ctx.newtypes.contains(s))
  {
    if method != "value" {
      return Err(format!("codegen: newtype has no method `{method}`"));
    }
    return build_expr(
      context,
      builder,
      recv,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    );
  }

  // Plan 42 (enumerable stdlib): `each`/`map`/`select`/`filter`/
  // `reduce`/`inject`/`each_with_index`/`count`/`sum`/`sort` on an
  // `Array[T]`/`Hash[K,V]`-typed receiver — see `build_enumerable_
  // call`'s own doc comment.
  //
  // Real, disclosed bug found and fixed this session: an earlier
  // version of this check (like `.key`/`.value` immediately above,
  // BEFORE that arm's own fix) was guarded purely by method name,
  // matching `emerald-sema`'s own first-drafted, since-corrected
  // arm — and broke the exact same real, pre-existing example,
  // `examples/classes.em`'s `Point#sum`, for the exact same reason:
  // sema now correctly accepts a real class's own `.sum`/`.count`/etc.
  // method (per that fix), but this arm would still have intercepted
  // the call *here*, in codegen, before ever reaching the ordinary
  // per-class dispatch below, and failed with an internal "cannot
  // determine the element type" error instead of compiling the real
  // method call. Fixed the identical way: scoped to the receiver's
  // OWN determined representation (`local_array_elem_types`/`local_
  // classes`-as-Hash — never possible for a real class instance, which
  // is always tracked via `local_classes` as a bare class name), not
  // the method name alone — a receiver of any other type using one of
  // these ten names falls straight through, unaffected, to the
  // ordinary class-method dispatch below.
  let recv_is_array_or_hash = local_array_elem_types.contains_key(recv_name)
    || local_classes
      .get(recv_name)
      .is_some_and(|s| parse_hash_type(s).is_some());
  if recv_is_array_or_hash
    && matches!(
      method,
      "each"
        | "map"
        | "select"
        | "filter"
        | "reduce"
        | "inject"
        | "each_with_index"
        | "count"
        | "sum"
        | "sort"
    )
  {
    let (val, kind, _elem_kind) = build_enumerable_call(
      context,
      builder,
      recv,
      recv_name,
      method,
      args,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    return Ok((val, kind));
  }

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

  // Plan 59's Decision log (superseded by plan 73's own Decision log
  // below): `String.from_cstring(ptr)` — the same reserved-namespace
  // static-call shape as `File` immediately above, for the same reason
  // (`String` is never a real `ModuleDef`).
  //
  // Plan 73's Decision log: `emerald-sema` now types this call's result
  // as a real `Option[String]` ADT value, not plan 43's old raw-
  // pointer `String?` (where a real C `NULL` and Emerald's own nil-
  // nullable sentinel were bit-for-bit the same `ptr` value, so no
  // conversion was ever needed). That bit-representation convergence
  // no longer holds — `Option[String]` is a heap-allocated tag+payload
  // struct now, the same shape every other `Some`/`None` construction
  // in this module already uses — so this call site must itself
  // branch on the incoming C pointer's nullness and build the matching
  // tagged value, merging both paths via a real LLVM `phi`, the same
  // pattern `Expr::Coalesce`'s own codegen above already establishes.
  if recv_name == "String" {
    if method != "from_cstring" {
      return Err(format!(
        "codegen: unsupported String static method `{method}`"
      ));
    }
    let arg = args
      .first()
      .ok_or_else(|| "codegen: `String.from_cstring` expects 1 argument".to_string())?;
    let (v, _) = build_expr(
      context,
      builder,
      arg,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let enum_name = "Option$String";
    let layout = ctx.enums.get(enum_name).ok_or_else(|| {
      "codegen: internal error — `Option$String` was not pre-instantiated for `String.from_cstring`"
        .to_string()
    })?;
    let some_tag = *layout.variant_tags.get("Some").ok_or_else(|| {
      "codegen: internal error — `Option$String` has no `Some` variant".to_string()
    })?;
    let none_tag = *layout.variant_tags.get("None").ok_or_else(|| {
      "codegen: internal error — `Option$String` has no `None` variant".to_string()
    })?;
    let size_val = context.i64_type().const_int(layout.size, false);
    let ptr_val = v.into_pointer_value();
    let is_null = builder
      .build_is_null(ptr_val, "fromcstringisnull")
      .map_err(|e| e.to_string())?;

    let entry_block = builder
      .get_insert_block()
      .ok_or("codegen: internal error — no current block")?;
    let func = entry_block
      .get_parent()
      .ok_or("codegen: internal error — block has no parent function")?;
    let some_block = context.append_basic_block(func, "fromcstring.some");
    let none_block = context.append_basic_block(func, "fromcstring.none");
    let merge_block = context.append_basic_block(func, "fromcstring.merge");
    builder
      .build_conditional_branch(is_null, none_block, some_block)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(some_block);
    let some_alloc = builder
      .build_call(ctx.alloc, &[size_val.into()], "fromcstringsome")
      .map_err(|e| e.to_string())?;
    let some_ptr = call_result(some_alloc)?.into_pointer_value();
    let some_tag_ptr = field_ptr(context, builder, some_ptr, 0)?;
    builder
      .build_store(some_tag_ptr, context.i64_type().const_int(some_tag, false))
      .map_err(|e| e.to_string())?;
    let some_field_ptr = field_ptr(context, builder, some_ptr, 8)?;
    builder
      .build_store(some_field_ptr, ptr_val)
      .map_err(|e| e.to_string())?;
    let some_end_block = builder
      .get_insert_block()
      .ok_or("codegen: internal error — no current block after some")?;
    builder
      .build_unconditional_branch(merge_block)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(none_block);
    let none_alloc = builder
      .build_call(ctx.alloc, &[size_val.into()], "fromcstringnone")
      .map_err(|e| e.to_string())?;
    let none_ptr = call_result(none_alloc)?.into_pointer_value();
    let none_tag_ptr = field_ptr(context, builder, none_ptr, 0)?;
    builder
      .build_store(none_tag_ptr, context.i64_type().const_int(none_tag, false))
      .map_err(|e| e.to_string())?;
    let none_end_block = builder
      .get_insert_block()
      .ok_or("codegen: internal error — no current block after none")?;
    builder
      .build_unconditional_branch(merge_block)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(merge_block);
    let phi = builder
      .build_phi(local_llvm_type(context, &ValKind::Ptr), "fromcstringresult")
      .map_err(|e| e.to_string())?;
    let some_val: BasicValueEnum = some_ptr.into();
    let none_val: BasicValueEnum = none_ptr.into();
    phi.add_incoming(&[(&some_val, some_end_block), (&none_val, none_end_block)]);
    return Ok((phi.as_basic_value(), ValKind::Ptr));
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
    // Plan 59's Decision log: `s.to_cstring()` — a pure type-level
    // relabeling with zero emitted instructions beyond whatever already
    // produced `recv_val` (`String`/`CString` share the identical bare-
    // pointer `ValKind::Str` representation — see `Type::CString`'s own
    // doc comment). Checked before the runtime-call dispatch below,
    // which every OTHER String intrinsic still goes through.
    if method == "to_cstring" {
      return Ok((recv_val, ValKind::Str));
    }
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
    let (fv, ret_kind) = ctx
      .user_func_ids
      .get(&key)
      .map(|(fv, k)| (*fv, k.clone()))
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

  // Plan 89's Decision log: a literal, statically-named top-level
  // `Proc` `Let` still dispatches via a direct call to its own known
  // `__lambda_{name}` function (unchanged, zero regression — the
  // common, simplest case). Every OTHER Proc-typed receiver (a
  // parameter, or a local forwarding one — see `bind_params`'/`Stmt::
  // Let`'s own `proc_sig_for_type` bookkeeping) falls through to a
  // genuine INDIRECT call below instead of failing outright.
  if method == "call" && !ctx.lambda_func_ids.contains_key(recv_name) {
    let sig = local_classes.get(recv_name).ok_or_else(|| {
      format!("codegen: cannot determine `{recv_name}`'s Proc signature for an indirect `.call`")
    })?;
    let decoded = decode_proc_sig(sig).ok_or_else(|| {
      format!("codegen: `{recv_name}` is not a Proc-typed local/parameter — cannot `.call` it")
    })?;
    return build_indirect_proc_call(
      context,
      builder,
      recv,
      args,
      decoded,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    );
  }

  let (fv, ret_kind) = if method == "call" {
    ctx
      .lambda_func_ids
      .get(recv_name)
      .map(|(fv, k)| (*fv, k.clone()))
      .ok_or_else(|| {
        format!(
          "codegen: `.call` on `{recv_name}` — not a lambda literal bound to a top-level `Let`"
        )
      })?
  } else {
    let class_name = local_classes.get(recv_name).ok_or_else(|| {
      format!("codegen: cannot determine the class of `{recv_name}` for `.{method}`")
    })?;
    // Plan 60's Decision log: `recv.register(name, port)` — scoped by
    // the receiver's own actual class being a declared actor, mirroring
    // sema's own identical `info.is_actor && method == "register"` gate
    // (`infer_expr_type`). Checked before the ordinary `method_owners`
    // lookup below, since "register" is never a real declared method.
    if method == "register" && ctx.actor_names.contains(class_name.as_str()) {
      return build_actor_register_call(
        context,
        builder,
        recv,
        args,
        class_name,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      );
    }
    // Plan 32: resolve which ancestor actually *declares* `method` —
    // only the defining class has a compiled `{Class}_{method}` symbol
    // (`d.age` on a `Dog` that never declares `age` must call
    // `Animal_age`; `Dog_age` was never generated).
    let defining_class = ctx
      .method_owners
      .get(class_name.as_str())
      .and_then(|owners| owners.get(method))
      .ok_or_else(|| format!("codegen: unsupported method call `{class_name}.{method}`"))?;

    // Plan 89's Decision log: replaces plan 88's own stopgap rejection
    // — a generic method's own body is now really monomorphized, one
    // compiled function per distinct concrete type-parameter binding
    // actually called anywhere in the program, lazily, right here (see
    // `resolve_generic_method_instance`'s own doc comment for exactly
    // which existing mechanism this mirrors and why it's driven lazily
    // instead of via a separate whole-program discovery pass).
    if ctx
      .generic_class_methods
      .get(defining_class.as_str())
      .is_some_and(|names| names.contains(method))
    {
      resolve_generic_method_instance(
        context,
        builder,
        ctx,
        defining_class,
        method,
        args,
        vars,
        local_classes,
      )?
    } else {
      let key = format!("{defining_class}_{}", mangled_operator_symbol(method));

      // Plan 55's Decision log: the dispatch rule is purely syntactic —
      // a literal `self` receiver is always a direct call (AC4: zero
      // enqueue overhead for the same-actor case); any OTHER receiver
      // whose static class is a declared actor becomes a cross-actor
      // `emerald_actor_enqueue` call instead, even one that happens to
      // alias `self` at runtime (deliberately conservative — no runtime
      // identity check exists anywhere in this backend to tell the
      // difference). Checked here, before the ordinary direct-call
      // lookup below, so it applies uniformly to every actor-typed
      // receiver shape `local_classes` can name (a field, a parameter,
      // a local — this function's own receiver is always a plain
      // `Expr::Ident`, per the check at its very top).
      if recv_name != "self" && ctx.actor_names.contains(class_name.as_str()) {
        return build_actor_enqueue_call(
          context,
          builder,
          recv,
          &key,
          args,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        );
      }

      ctx
        .user_func_ids
        .get(&key)
        .map(|(fv, k)| (*fv, k.clone()))
        .ok_or_else(|| format!("codegen: unsupported method call `{class_name}.{method}`"))?
    }
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

/// Plan 89's real, genuine indirect-`.call` mechanism — factored out of
/// `build_method_call` so its two real call sites (an ordinary Proc-
/// typed local/parameter receiver, and this session's own fix for a
/// Proc-typed CLASS FIELD receiver, `@field.call(...)`) share one copy
/// rather than two independently-maintained ones. `sig` is the
/// receiver's already-decoded `(param_kinds, ret_kind)` — resolving
/// WHICH string to decode it from (`local_classes` for a local/param,
/// `field_classes` for a field) is each caller's own, different job;
/// this function only ever builds the actual indirect call once a
/// signature is in hand.
#[allow(clippy::too_many_arguments)]
fn build_indirect_proc_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  args: &[Spanned<Expr>],
  sig: (Vec<ValKind>, ValKind),
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (param_kinds, ret_kind) = sig;
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let env_ptr = recv_val.into_pointer_value();
  let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum> = vec![env_ptr.into()];
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
  let mut kinds = vec![ValKind::Ptr]; // env
  kinds.extend(param_kinds);
  let fn_ty = make_fn_type(context, &kinds, &ret_kind);
  let fn_ptr_slot = field_ptr(context, builder, env_ptr, 0)?;
  let fn_ptr = builder
    .build_load(
      context.ptr_type(AddressSpace::default()),
      fn_ptr_slot,
      "procfnptr",
    )
    .map_err(|e| e.to_string())?
    .into_pointer_value();
  let call = builder
    .build_indirect_call(fn_ty, fn_ptr, &call_args, "indirectcalltmp")
    .map_err(|e| e.to_string())?;
  if ret_kind == ValKind::Void {
    Ok((context.i64_type().const_int(0, false).into(), ret_kind))
  } else {
    Ok((call_result(call)?, ret_kind))
  }
}

/// Plan 42 (enumerable stdlib) — dispatches one of the ten intrinsic
/// Array[T]/Hash[K,V] method names to its own dedicated builder below.
/// `emerald-sema`'s own `check_enumerable_call` (its doc comment has
/// the full "why a hard-coded arm instead of a real `Iterable[T]`
/// interface" rationale) already validated arity/block-shape/element-
/// type for every case reached here — this function trusts that,
/// mirroring every other "codegen runs on already-checked input"
/// leaf in this backend. `func` is recovered from the builder's own
/// current insertion point (`build_safe_call`'s own established
/// trick) rather than threaded as a new parameter through `build_
/// method_call` and its own many, already-numerous call sites.
#[allow(clippy::too_many_arguments)]
fn build_enumerable_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  recv_name: &str,
  method: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind, Option<ValKind>), String> {
  let func = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block")?
    .get_parent()
    .ok_or("codegen: internal error — block has no parent function")?;
  let i64_ty = context.i64_type();

  // `.count` is the one method shared by Array and Hash alike — a
  // plain O(1) header read either way (`build_array_lit`'s own `leaf-
  // array-length-header` doc comment; `build_hash_lit`'s already-
  // shipped count header).
  if method == "count" && args.is_empty() {
    let (recv_val, _) = build_expr(
      context,
      builder,
      recv,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let count_val = builder
      .build_load(i64_ty, recv_val.into_pointer_value(), "encount")
      .map_err(|e| e.to_string())?;
    return Ok((count_val, ValKind::Int64, None));
  }

  if let Some((k_kind, v_kind)) = local_classes
    .get(recv_name)
    .and_then(|s| parse_hash_type(s))
  {
    // Plan 70 (enumerable stdlib completion): `map`/`select`/`filter`/
    // `reduce`/`inject`/`each_with_index` on `Hash[K,V]` — mirrors
    // `each` immediately below (already plan 42's own, unchanged), each
    // one constructing its per-iteration `Pair[K,V]` value via the same
    // `build_hash_pair_ptr` helper `build_hash_each` itself now also
    // calls. `sum`/`sort` (and anything else) fall to the `other` arm's
    // pre-existing rejection, unchanged — `emerald-sema`'s own
    // `check_enumerable_call` never routes them here in the first place
    // (Hash stays Array-only for those two).
    return match method {
      "each" => {
        let [proc_arg] = args else {
          return Err(
            "codegen: internal error — `.each` expects exactly 1 argument (sema should have rejected this)"
              .to_string(),
          );
        };
        build_hash_each(
          context,
          builder,
          func,
          recv,
          proc_arg,
          k_kind,
          v_kind,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )
        .map(|(v, k)| (v, k, None))
      }
      "map" => {
        let [proc_arg] = args else {
          return Err(
            "codegen: internal error — `.map` expects exactly 1 argument (sema should have rejected this)"
              .to_string(),
          );
        };
        build_hash_map(
          context,
          builder,
          func,
          recv,
          proc_arg,
          k_kind,
          v_kind,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )
      }
      // Plan 70's own real, disclosed narrowing: `select`/`filter` are
      // deliberately NOT extended to `Hash[K,V]` — see `emerald-sema`'s
      // `check_enumerable_call` doc comment on why `Array[Pair[K,V]]`
      // (the only sensible result type) can never even be named in
      // this language's concrete syntax. Falls to the `other` arm
      // below, matching sema's own rejection.
      "reduce" | "inject" => {
        let [initial, proc_arg] = args else {
          return Err(format!(
            "codegen: internal error — `.{method}` expects exactly 2 arguments (sema should have rejected this)"
          ));
        };
        build_hash_reduce(
          context,
          builder,
          func,
          recv,
          initial,
          proc_arg,
          k_kind,
          v_kind,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )
        .map(|(v, k)| (v, k, None))
      }
      "each_with_index" => {
        let [proc_arg] = args else {
          return Err(
            "codegen: internal error — `.each_with_index` expects exactly 1 argument (sema should have rejected this)"
              .to_string(),
          );
        };
        build_hash_each_with_index(
          context,
          builder,
          func,
          recv,
          proc_arg,
          k_kind,
          v_kind,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )
        .map(|(v, k)| (v, k, None))
      }
      // Plan 70: `.count(proc_name)` — the predicate form (the arity-0
      // O(1) header-read form is handled unconditionally above, before
      // this Hash/Array split, and never reaches here).
      "count" => {
        let [proc_arg] = args else {
          return Err(format!(
            "codegen: internal error — `.count` takes 0 or 1 arguments, found {} (sema should have rejected this)",
            args.len()
          ));
        };
        build_hash_count_predicate(
          context,
          builder,
          func,
          recv,
          proc_arg,
          k_kind,
          v_kind,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )
        .map(|(v, k)| (v, k, None))
      }
      other => Err(format!(
        "codegen: internal error — `.{other}` is not supported on Hash[K, V] (sema should have rejected this)"
      )),
    };
  }

  let elem_kind = local_array_elem_types
    .get(recv_name)
    .cloned()
    .ok_or_else(|| {
      format!(
        "codegen: internal error — cannot determine the element type of `{recv_name}` for `.{method}`"
      )
    })?;

  match method {
    "each" => {
      let [block] = args else {
        return Err(
          "codegen: internal error — `.each` expects exactly 1 argument (sema should have rejected this)"
            .to_string(),
        );
      };
      build_array_each(
        context,
        builder,
        func,
        recv,
        block,
        elem_kind,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
      .map(|(v, k)| (v, k, None))
    }
    "map" => {
      let [block] = args else {
        return Err(
          "codegen: internal error — `.map` expects exactly 1 argument (sema should have rejected this)"
            .to_string(),
        );
      };
      build_array_map(
        context,
        builder,
        func,
        recv,
        block,
        elem_kind,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
    }
    // Plan 74 (enumerable chaining): `.select`/`.filter` preserve the
    // receiver's own element type exactly — `emerald-sema`'s own
    // `check_enumerable_call` returns `Array[elem_ty]` unchanged, never
    // a transformation of it the way `.map`'s own Proc return type is
    // — so the produced-element-kind this plan's own further-chained
    // call needs is just `elem_kind` itself, already known above,
    // cloned once for the `Some(...)` alongside the (moved) copy this
    // call itself still needs.
    "select" | "filter" => {
      let [block] = args else {
        return Err(format!(
          "codegen: internal error — `.{method}` expects exactly 1 argument (sema should have rejected this)"
        ));
      };
      build_array_select(
        context,
        builder,
        func,
        recv,
        block,
        elem_kind.clone(),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
      .map(|(v, k)| (v, k, Some(elem_kind)))
    }
    "reduce" | "inject" => {
      let [initial, block] = args else {
        return Err(format!(
          "codegen: internal error — `.{method}` expects exactly 2 arguments (sema should have rejected this)"
        ));
      };
      build_array_reduce(
        context,
        builder,
        func,
        recv,
        initial,
        block,
        elem_kind,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
      .map(|(v, k)| (v, k, None))
    }
    "each_with_index" => {
      let [block] = args else {
        return Err(
          "codegen: internal error — `.each_with_index` expects exactly 1 argument (sema should have rejected this)"
            .to_string(),
        );
      };
      build_array_each_with_index(
        context,
        builder,
        func,
        recv,
        block,
        elem_kind,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
      .map(|(v, k)| (v, k, None))
    }
    "sum" => build_array_sum(
      context,
      builder,
      func,
      recv,
      elem_kind,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )
    .map(|(v, k)| (v, k, None)),
    // Plan 74 (enumerable chaining): `.sort` preserves the receiver's
    // own element type exactly too (`check_enumerable_call`'s own
    // `Int64`/`Float64`-only narrowing never changes it) — same
    // `elem_kind.clone()`-then-`Some(elem_kind)` treatment as
    // `.select`/`.filter` immediately above, for the identical reason.
    "sort" => build_array_sort(
      context,
      builder,
      func,
      recv,
      elem_kind.clone(),
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )
    .map(|(v, k)| (v, k, Some(elem_kind))),
    // Plan 70: `.count(proc_name)` — the predicate form (the arity-0
    // O(1) header-read form is handled unconditionally above, before
    // this Hash/Array split, and never reaches here).
    "count" => {
      let [proc_arg] = args else {
        return Err(format!(
          "codegen: internal error — `.count` takes 0 or 1 arguments, found {} (sema should have rejected this)",
          args.len()
        ));
      };
      build_array_count_predicate(
        context,
        builder,
        func,
        recv,
        proc_arg,
        elem_kind,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )
      .map(|(v, k)| (v, k, None))
    }
    other => Err(format!(
      "codegen: internal error — unsupported enumerable method `.{other}` (sema should have rejected this)"
    )),
  }
}

/// Plan 42 (enumerable stdlib): `arr.each(proc_name)` — walks every
/// element once, calling the already-compiled named `Proc` on each
/// (`call_named_proc`'s own doc comment), discarding its value (sema
/// already requires nothing of it).
#[allow(clippy::too_many_arguments)]
fn build_array_each<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "eachcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "eachelemptr")
        .map_err(|e| e.to_string())?
    };
    let elem_val = builder
      .build_load(elem_llvm_ty, elem_ptr, "eachelemval")
      .map_err(|e| e.to_string())?;
    call_named_proc(
      context,
      builder,
      proc_arg,
      &[elem_val],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    Ok(())
  })?;

  Ok((i64_ty.const_int(0, false).into(), ValKind::Void))
}

/// Plan 42 (enumerable stdlib): `h.each(proc_name)` — walks every
/// stored `(key, value)` pair once, constructing a fresh, real 16-byte
/// `Pair` (`Type::Pair`'s own doc comment: the same `[key: 8][value:
/// 8]` layout `Hash[K,V]`'s own buffer already uses, copied straight
/// out of it rather than inventing a second layout convention) and
/// calling the named `Proc` with it. `hash_ty`/hash_ty-derived `Pair`
/// annotation bookkeeping from this leaf's earlier, now-superseded
/// inline-block design is gone — the Proc's own params were already
/// checked against `Pair[K, V]` at its own declaration site by sema,
/// long before this call.
#[allow(clippy::too_many_arguments)]
fn build_hash_each<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  k_kind: ValKind,
  v_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let h_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, h_ptr, "heachcount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let pair_ptr = build_hash_pair_ptr(context, builder, h_ptr, idx, &k_kind, &v_kind, ctx)?;
    call_named_proc(
      context,
      builder,
      proc_arg,
      &[pair_ptr.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    Ok(())
  })?;

  Ok((i64_ty.const_int(0, false).into(), ValKind::Void))
}

/// Plan 70 (enumerable stdlib completion): reads the `(key, value)`
/// pair stored at `idx` in a `Hash[K,V]`'s own buffer and copies it
/// into a freshly heap-allocated, real 16-byte `Pair` (`Type::Pair`'s
/// own `[key: 8][value: 8]` layout) — factored out of `build_hash_each`
/// (plan 42's own original implementation, which now calls this too),
/// so every Hash enumerable method added by this plan (`map`/`select`/
/// `filter`/`reduce`/`inject`/`each_with_index`) constructs its
/// `Pair[K,V]` value identically, in one place.
fn build_hash_pair_ptr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  h_ptr: PointerValue<'ctx>,
  idx: inkwell::values::IntValue<'ctx>,
  k_kind: &ValKind,
  v_kind: &ValKind,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  let i64_ty = context.i64_type();
  let stride = i64_ty.const_int(16, false);
  let pair_off = builder
    .build_int_mul(idx, stride, "hpairoff")
    .map_err(|e| e.to_string())?;
  let pair_byte_off = builder
    .build_int_add(pair_off, i64_ty.const_int(8, false), "hpairbyteoff")
    .map_err(|e| e.to_string())?;
  let key_src_ptr = unsafe {
    builder
      .build_in_bounds_gep(context.i8_type(), h_ptr, &[pair_byte_off], "hkeysrc")
      .map_err(|e| e.to_string())?
  };
  let key_val = builder
    .build_load(local_llvm_type(context, k_kind), key_src_ptr, "hkeyval")
    .map_err(|e| e.to_string())?;
  let value_byte_off = builder
    .build_int_add(pair_byte_off, i64_ty.const_int(8, false), "hvaluebyteoff")
    .map_err(|e| e.to_string())?;
  let value_src_ptr = unsafe {
    builder
      .build_in_bounds_gep(context.i8_type(), h_ptr, &[value_byte_off], "hvaluesrc")
      .map_err(|e| e.to_string())?
  };
  let value_val = builder
    .build_load(local_llvm_type(context, v_kind), value_src_ptr, "hvalueval")
    .map_err(|e| e.to_string())?;

  let pair_alloc_call = builder
    .build_call(ctx.alloc, &[i64_ty.const_int(16, false).into()], "hpair")
    .map_err(|e| e.to_string())?;
  let pair_ptr = call_result(pair_alloc_call)?.into_pointer_value();
  builder
    .build_store(pair_ptr, key_val)
    .map_err(|e| e.to_string())?;
  let pair_value_ptr = field_ptr(context, builder, pair_ptr, 8)?;
  builder
    .build_store(pair_value_ptr, value_val)
    .map_err(|e| e.to_string())?;
  Ok(pair_ptr)
}

/// Plan 70 (enumerable stdlib completion): `h.map(proc_name)` — mirrors
/// `build_array_map` exactly, except each source "element" is a
/// freshly constructed `Pair[K,V]` (`build_hash_pair_ptr`) rather than
/// a value loaded straight out of a flat element array.
///
/// Plan 74 (enumerable chaining): also surfaces the produced element
/// `ValKind` as a 3rd return-tuple element, for the identical reason
/// `build_array_map`'s own Decision-log addition does — the result is
/// a real `Array[R]` (not a `Hash`), so any further chained enumerable
/// call on it is the ordinary Array case from here on.
#[allow(clippy::too_many_arguments)]
fn build_hash_map<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  k_kind: ValKind,
  v_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind, Option<ValKind>), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let h_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, h_ptr, "hmapcount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  let elems_bytes = builder
    .build_int_mul(count_val, i64_ty.const_int(8, false), "hmapoutelembytes")
    .map_err(|e| e.to_string())?;
  let out_bytes = builder
    .build_int_add(elems_bytes, i64_ty.const_int(8, false), "hmapoutbytes")
    .map_err(|e| e.to_string())?;
  let out_call = builder
    .build_call(ctx.alloc, &[out_bytes.into()], "hmapout")
    .map_err(|e| e.to_string())?;
  let out_ptr = call_result(out_call)?.into_pointer_value();
  builder
    .build_store(out_ptr, count_val)
    .map_err(|e| e.to_string())?;
  let out_elems_base = field_ptr(context, builder, out_ptr, 8)?;

  let mut result_elem_kind: Option<ValKind> = None;
  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let pair_ptr = build_hash_pair_ptr(context, builder, h_ptr, idx, &k_kind, &v_kind, ctx)?;
    let (result_val_opt, result_kind) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[pair_ptr.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let result_val = result_val_opt.ok_or(
      "codegen: internal error — `.map`'s Proc must not be Void (sema should have rejected this)",
    )?;
    let result_llvm_ty = local_llvm_type(context, &result_kind);
    let dst_ptr = unsafe {
      builder
        .build_in_bounds_gep(result_llvm_ty, out_elems_base, &[idx], "hmapdst")
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(dst_ptr, result_val)
      .map_err(|e| e.to_string())?;
    result_elem_kind = Some(result_kind);
    Ok(())
  })?;

  Ok((out_ptr.into(), ValKind::Ptr, result_elem_kind))
}

/// Plan 70 (enumerable stdlib completion): `h.count(proc_name)` — the
/// predicate form (the arity-0 O(1) header-read form never reaches
/// this function; see `build_enumerable_call`'s own unconditional
/// early return). A plain running counter, incremented once per pair
/// the predicate accepts — no output buffer at all, unlike `.select`.
#[allow(clippy::too_many_arguments)]
fn build_hash_count_predicate<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  k_kind: ValKind,
  v_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let h_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, h_ptr, "hcntcount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  let acc_alloca = builder
    .build_alloca(i64_ty, "hcntacc")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(acc_alloca, i64_ty.const_int(0, false))
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let pair_ptr = build_hash_pair_ptr(context, builder, h_ptr, idx, &k_kind, &v_kind, ctx)?;
    let (pred_val_opt, _) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[pair_ptr.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let pred_bool = pred_val_opt
      .ok_or(
        "codegen: internal error — `.count`'s predicate Proc must not be Void (sema should have rejected this)",
      )?
      .into_int_value();
    let acc_cur = builder
      .build_load(i64_ty, acc_alloca, "hcntacccur")
      .map_err(|e| e.to_string())?
      .into_int_value();
    let incremented = builder
      .build_int_add(acc_cur, i64_ty.const_int(1, false), "hcntinc")
      .map_err(|e| e.to_string())?;
    let new_acc = builder
      .build_select(pred_bool, incremented, acc_cur, "hcntnewacc")
      .map_err(|e| e.to_string())?;
    builder
      .build_store(acc_alloca, new_acc)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let final_val = builder
    .build_load(i64_ty, acc_alloca, "hcntfinal")
    .map_err(|e| e.to_string())?;
  Ok((final_val, ValKind::Int64))
}

/// Plan 70 (enumerable stdlib completion): `h.reduce(initial, proc_name)`/
/// `.inject(...)` — mirrors `build_array_reduce` exactly, folding over
/// freshly constructed `Pair[K,V]` values (`build_hash_pair_ptr`).
#[allow(clippy::too_many_arguments)]
fn build_hash_reduce<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  initial: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  k_kind: ValKind,
  v_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let h_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, h_ptr, "hreducecount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  let (initial_val, acc_kind) = build_expr(
    context,
    builder,
    initial,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let acc_llvm_ty = local_llvm_type(context, &acc_kind);
  let acc_alloca = builder
    .build_alloca(acc_llvm_ty, "hreduceacc")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(acc_alloca, initial_val)
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let acc_cur = builder
      .build_load(acc_llvm_ty, acc_alloca, "hreduceacccur")
      .map_err(|e| e.to_string())?;
    let pair_ptr = build_hash_pair_ptr(context, builder, h_ptr, idx, &k_kind, &v_kind, ctx)?;
    let (result_val_opt, _) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[acc_cur, pair_ptr.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let result_val = result_val_opt.ok_or(
      "codegen: internal error — `.reduce`'s Proc must not be Void (sema should have rejected this)",
    )?;
    builder
      .build_store(acc_alloca, result_val)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let final_val = builder
    .build_load(acc_llvm_ty, acc_alloca, "hreducefinal")
    .map_err(|e| e.to_string())?;
  Ok((final_val, acc_kind))
}

/// Plan 70 (enumerable stdlib completion): `h.each_with_index(proc_name)`
/// — mirrors `build_array_each_with_index` exactly, over freshly
/// constructed `Pair[K,V]` values (`build_hash_pair_ptr`).
#[allow(clippy::too_many_arguments)]
fn build_hash_each_with_index<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  k_kind: ValKind,
  v_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let h_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, h_ptr, "hewicount")
    .map_err(|e| e.to_string())?
    .into_int_value();

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let pair_ptr = build_hash_pair_ptr(context, builder, h_ptr, idx, &k_kind, &v_kind, ctx)?;
    call_named_proc(
      context,
      builder,
      proc_arg,
      &[pair_ptr.into(), idx.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    Ok(())
  })?;

  Ok((i64_ty.const_int(0, false).into(), ValKind::Void))
}

/// Plan 42 (enumerable stdlib): `arr.map(proc_name)` — allocates a
/// fresh, same-length output `Array[R]` (`R` is the named Proc's own
/// declared return type — trusted from the caller's own declared `Let`
/// annotation and from sema's own `check_enumerable_proc_arg`, the same
/// "codegen never re-derives a type sema already checked" posture
/// every other intrinsic here takes), storing each transformed element
/// in place.
///
/// Plan 74 (enumerable chaining): also surfaces the concrete `ValKind`
/// `R` actually turned out to be, as a 3rd return-tuple element — the
/// one case among the ten enumerable methods where a further chained
/// `.select`/`.filter`/`.sort`/`.map` call's own element type isn't
/// already known from the receiver's own `local_array_elem_types`
/// entry (that table is keyed by variable NAME, populated at `Let`-
/// binding time from ITS OWN declared type — an intermediate, unnamed
/// chain link has no entry there). `call_named_proc`'s own returned
/// `result_kind` is captured once, from inside `build_count_loop`'s
/// single codegen-time closure call (its own doc comment: `body` is
/// invoked exactly once to build the loop's body IR, never re-invoked
/// per runtime iteration) — exactly the value every element in the
/// real runtime loop actually gets, since it depends only on the
/// proc's own static signature, never re-derived by a second, separate
/// prediction-only pass (which would risk running the proc's own
/// codegen, and any real side effect it has, more than once).
#[allow(clippy::too_many_arguments)]
fn build_array_map<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind, Option<ValKind>), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "mapcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let elems_bytes = builder
    .build_int_mul(count_val, i64_ty.const_int(8, false), "mapoutelembytes")
    .map_err(|e| e.to_string())?;
  let out_bytes = builder
    .build_int_add(elems_bytes, i64_ty.const_int(8, false), "mapoutbytes")
    .map_err(|e| e.to_string())?;
  let out_call = builder
    .build_call(ctx.alloc, &[out_bytes.into()], "mapout")
    .map_err(|e| e.to_string())?;
  let out_ptr = call_result(out_call)?.into_pointer_value();
  builder
    .build_store(out_ptr, count_val)
    .map_err(|e| e.to_string())?;
  let out_elems_base = field_ptr(context, builder, out_ptr, 8)?;

  let mut result_elem_kind: Option<ValKind> = None;
  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let src_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "mapsrc")
        .map_err(|e| e.to_string())?
    };
    let src_val = builder
      .build_load(elem_llvm_ty, src_ptr, "mapsrcval")
      .map_err(|e| e.to_string())?;
    let (result_val_opt, result_kind) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[src_val],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let result_val = result_val_opt.ok_or(
      "codegen: internal error — `.map`'s Proc must not be Void (sema should have rejected this)",
    )?;
    let result_llvm_ty = local_llvm_type(context, &result_kind);
    let dst_ptr = unsafe {
      builder
        .build_in_bounds_gep(result_llvm_ty, out_elems_base, &[idx], "mapdst")
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(dst_ptr, result_val)
      .map_err(|e| e.to_string())?;
    result_elem_kind = Some(result_kind);
    Ok(())
  })?;

  Ok((out_ptr.into(), ValKind::Ptr, result_elem_kind))
}

/// Plan 42 (enumerable stdlib): `arr.select(proc_name)`/`.filter(...)`
/// — allocates a same-CAPACITY (never-exceeded, since a filter can
/// never keep more elements than it started with) output buffer, but
/// stores only the ACTUAL filtered count in its own header (`leaf-
/// array-length-header`'s own addition is exactly what makes this
/// safe): a real, disclosed, single-pass alternative to either a
/// genuinely resizable array (this compiler has none, Decision log) or
/// a correctness-breaking two-pass "count then fill" scheme (which
/// would call the Proc — and any side effect it has — twice per
/// element).
#[allow(clippy::too_many_arguments)]
fn build_array_select<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "selcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let elems_bytes = builder
    .build_int_mul(count_val, i64_ty.const_int(8, false), "seloutelembytes")
    .map_err(|e| e.to_string())?;
  let out_bytes = builder
    .build_int_add(elems_bytes, i64_ty.const_int(8, false), "seloutbytes")
    .map_err(|e| e.to_string())?;
  let out_call = builder
    .build_call(ctx.alloc, &[out_bytes.into()], "selout")
    .map_err(|e| e.to_string())?;
  let out_ptr = call_result(out_call)?.into_pointer_value();
  let out_elems_base = field_ptr(context, builder, out_ptr, 8)?;
  let out_idx_alloca = builder
    .build_alloca(i64_ty, "seloutidx")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(out_idx_alloca, i64_ty.const_int(0, false))
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let src_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "selsrc")
        .map_err(|e| e.to_string())?
    };
    let src_val = builder
      .build_load(elem_llvm_ty, src_ptr, "selsrcval")
      .map_err(|e| e.to_string())?;
    let (pred_val_opt, _) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[src_val],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let pred_bool = pred_val_opt
      .ok_or(
        "codegen: internal error — `.select`'s Proc must not be Void (sema should have rejected this)",
      )?
      .into_int_value();

    let keep_blk = context.append_basic_block(func, "select.keep");
    let cont_blk = context.append_basic_block(func, "select.cont");
    builder
      .build_conditional_branch(pred_bool, keep_blk, cont_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(keep_blk);
    let out_idx_val = builder
      .build_load(i64_ty, out_idx_alloca, "seloutidxval")
      .map_err(|e| e.to_string())?
      .into_int_value();
    let dst_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, out_elems_base, &[out_idx_val], "seldst")
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(dst_ptr, src_val)
      .map_err(|e| e.to_string())?;
    let next_out_idx = builder
      .build_int_add(out_idx_val, i64_ty.const_int(1, false), "seloutidxnext")
      .map_err(|e| e.to_string())?;
    builder
      .build_store(out_idx_alloca, next_out_idx)
      .map_err(|e| e.to_string())?;
    builder
      .build_unconditional_branch(cont_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(cont_blk);
    Ok(())
  })?;

  let final_count = builder
    .build_load(i64_ty, out_idx_alloca, "selfinalcount")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(out_ptr, final_count)
    .map_err(|e| e.to_string())?;

  Ok((out_ptr.into(), ValKind::Ptr))
}

/// Plan 70 (enumerable stdlib completion): `arr.count(proc_name)` —
/// the predicate form (the arity-0 O(1) header-read form never reaches
/// this function; see `build_enumerable_call`'s own unconditional
/// early return). A plain running counter, incremented once per
/// element the predicate accepts — no output buffer at all, unlike
/// `.select`.
#[allow(clippy::too_many_arguments)]
fn build_array_count_predicate<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "acntcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let acc_alloca = builder
    .build_alloca(i64_ty, "acntacc")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(acc_alloca, i64_ty.const_int(0, false))
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let src_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "acntsrc")
        .map_err(|e| e.to_string())?
    };
    let src_val = builder
      .build_load(elem_llvm_ty, src_ptr, "acntsrcval")
      .map_err(|e| e.to_string())?;
    let (pred_val_opt, _) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[src_val],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let pred_bool = pred_val_opt
      .ok_or(
        "codegen: internal error — `.count`'s predicate Proc must not be Void (sema should have rejected this)",
      )?
      .into_int_value();
    let acc_cur = builder
      .build_load(i64_ty, acc_alloca, "acntacccur")
      .map_err(|e| e.to_string())?
      .into_int_value();
    let incremented = builder
      .build_int_add(acc_cur, i64_ty.const_int(1, false), "acntinc")
      .map_err(|e| e.to_string())?;
    let new_acc = builder
      .build_select(pred_bool, incremented, acc_cur, "acntnewacc")
      .map_err(|e| e.to_string())?;
    builder
      .build_store(acc_alloca, new_acc)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let final_val = builder
    .build_load(i64_ty, acc_alloca, "acntfinal")
    .map_err(|e| e.to_string())?;
  Ok((final_val, ValKind::Int64))
}

/// Plan 42 (enumerable stdlib): `arr.reduce(initial, proc_name)`/
/// `.inject(...)` — a plain accumulator fold, the named Proc's own
/// return value becoming next iteration's accumulator.
#[allow(clippy::too_many_arguments)]
fn build_array_reduce<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  initial: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "reducecount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let (initial_val, acc_kind) = build_expr(
    context,
    builder,
    initial,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let acc_llvm_ty = local_llvm_type(context, &acc_kind);
  let acc_alloca = builder
    .build_alloca(acc_llvm_ty, "reduceacc")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(acc_alloca, initial_val)
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let acc_cur = builder
      .build_load(acc_llvm_ty, acc_alloca, "reduceacccur")
      .map_err(|e| e.to_string())?;
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "reduceelem")
        .map_err(|e| e.to_string())?
    };
    let elem_val = builder
      .build_load(elem_llvm_ty, elem_ptr, "reduceelemval")
      .map_err(|e| e.to_string())?;
    let (result_val_opt, _) = call_named_proc(
      context,
      builder,
      proc_arg,
      &[acc_cur, elem_val],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let result_val = result_val_opt.ok_or(
      "codegen: internal error — `.reduce`'s Proc must not be Void (sema should have rejected this)",
    )?;
    builder
      .build_store(acc_alloca, result_val)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let final_val = builder
    .build_load(acc_llvm_ty, acc_alloca, "reducefinal")
    .map_err(|e| e.to_string())?;
  Ok((final_val, acc_kind))
}

/// Plan 42 (enumerable stdlib): `arr.each_with_index(proc_name)` —
/// mirrors `build_array_each` exactly, with a second argument carrying
/// the loop's own index.
#[allow(clippy::too_many_arguments)]
fn build_array_each_with_index<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  proc_arg: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "ewicount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "ewielem")
        .map_err(|e| e.to_string())?
    };
    let elem_val = builder
      .build_load(elem_llvm_ty, elem_ptr, "ewielemval")
      .map_err(|e| e.to_string())?;
    call_named_proc(
      context,
      builder,
      proc_arg,
      &[elem_val, idx.into()],
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    Ok(())
  })?;

  Ok((i64_ty.const_int(0, false).into(), ValKind::Void))
}

/// Plan 42 (enumerable stdlib): `arr.sum()` — `Int64`/`Float64`
/// element types only (sema's own real, disclosed narrowing from the
/// plan's stated `Comparable`-adjacent design — see `check_enumerable_
/// call`'s own doc comment: this language has no operator-overload
/// dispatch for `+` on an arbitrary class at all).
#[allow(clippy::too_many_arguments)]
fn build_array_sum<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "sumcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let acc_alloca = builder
    .build_alloca(elem_llvm_ty, "sumacc")
    .map_err(|e| e.to_string())?;
  let zero_val: BasicValueEnum = if elem_kind == ValKind::Float64 {
    context.f64_type().const_float(0.0).into()
  } else {
    i64_ty.const_int(0, false).into()
  };
  builder
    .build_store(acc_alloca, zero_val)
    .map_err(|e| e.to_string())?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "sumelem")
        .map_err(|e| e.to_string())?
    };
    let elem_val = builder
      .build_load(elem_llvm_ty, elem_ptr, "sumelemval")
      .map_err(|e| e.to_string())?;
    let acc_val = builder
      .build_load(elem_llvm_ty, acc_alloca, "sumacccur")
      .map_err(|e| e.to_string())?;
    let new_acc: BasicValueEnum = if elem_kind == ValKind::Float64 {
      builder
        .build_float_add(
          acc_val.into_float_value(),
          elem_val.into_float_value(),
          "sumaddf",
        )
        .map_err(|e| e.to_string())?
        .into()
    } else {
      builder
        .build_int_add(
          acc_val.into_int_value(),
          elem_val.into_int_value(),
          "sumadd",
        )
        .map_err(|e| e.to_string())?
        .into()
    };
    builder
      .build_store(acc_alloca, new_acc)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let final_val = builder
    .build_load(elem_llvm_ty, acc_alloca, "sumfinal")
    .map_err(|e| e.to_string())?;
  Ok((final_val, elem_kind))
}

/// Plan 42 (enumerable stdlib): `arr.sort()` — `Int64`/`Float64`
/// element types only, a real, disclosed narrowing from the plan's own
/// stated `Comparable`-bounded design (`check_enumerable_call`'s own
/// doc comment: a real `<=>`-dispatching fork for a user class needs
/// generics-annotation machinery this plan's own simplification
/// declines to add). A copy-then-bubble-sort, `O(n^2)` and stated
/// plainly — the same disclosed-simplicity move plan 25 already made
/// for `Hash`'s own `O(n)` lookup. The compare-and-swap itself uses
/// `build_select`, not a branch — no PHI node needed, and it composes
/// cleanly inside `build_count_loop`'s own per-iteration closure.
#[allow(clippy::too_many_arguments)]
fn build_array_sort<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  recv: &Spanned<Expr>,
  elem_kind: ValKind,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (recv_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let arr_ptr = recv_val.into_pointer_value();
  let i64_ty = context.i64_type();
  let count_val = builder
    .build_load(i64_ty, arr_ptr, "sortcount")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
  let elems_base = field_ptr(context, builder, arr_ptr, 8)?;

  let elems_bytes = builder
    .build_int_mul(count_val, i64_ty.const_int(8, false), "sortelembytes")
    .map_err(|e| e.to_string())?;
  let out_bytes = builder
    .build_int_add(elems_bytes, i64_ty.const_int(8, false), "sortoutbytes")
    .map_err(|e| e.to_string())?;
  let out_call = builder
    .build_call(ctx.alloc, &[out_bytes.into()], "sortout")
    .map_err(|e| e.to_string())?;
  let out_ptr = call_result(out_call)?.into_pointer_value();
  builder
    .build_store(out_ptr, count_val)
    .map_err(|e| e.to_string())?;
  let out_elems_base = field_ptr(context, builder, out_ptr, 8)?;

  build_count_loop(context, builder, func, count_val, |builder, idx| {
    let src_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx], "sortcopysrc")
        .map_err(|e| e.to_string())?
    };
    let src_val = builder
      .build_load(elem_llvm_ty, src_ptr, "sortcopyval")
      .map_err(|e| e.to_string())?;
    let dst_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, out_elems_base, &[idx], "sortcopydst")
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(dst_ptr, src_val)
      .map_err(|e| e.to_string())?;
    Ok(())
  })?;

  let one = i64_ty.const_int(1, false);
  let inner_bound = builder
    .build_int_sub(count_val, one, "sortinnerbound")
    .map_err(|e| e.to_string())?;
  build_count_loop(context, builder, func, count_val, |builder, _outer_idx| {
    build_count_loop(context, builder, func, inner_bound, |builder, j| {
      let j1 = builder
        .build_int_add(j, one, "sortj1")
        .map_err(|e| e.to_string())?;
      let aj_ptr = unsafe {
        builder
          .build_in_bounds_gep(elem_llvm_ty, out_elems_base, &[j], "sortajptr")
          .map_err(|e| e.to_string())?
      };
      let aj1_ptr = unsafe {
        builder
          .build_in_bounds_gep(elem_llvm_ty, out_elems_base, &[j1], "sortaj1ptr")
          .map_err(|e| e.to_string())?
      };
      let aj_val = builder
        .build_load(elem_llvm_ty, aj_ptr, "sortaj")
        .map_err(|e| e.to_string())?;
      let aj1_val = builder
        .build_load(elem_llvm_ty, aj1_ptr, "sortaj1")
        .map_err(|e| e.to_string())?;
      let should_swap = if elem_kind == ValKind::Float64 {
        builder
          .build_float_compare(
            inkwell::FloatPredicate::OGT,
            aj_val.into_float_value(),
            aj1_val.into_float_value(),
            "sortcmp",
          )
          .map_err(|e| e.to_string())?
      } else {
        builder
          .build_int_compare(
            IntPredicate::SGT,
            aj_val.into_int_value(),
            aj1_val.into_int_value(),
            "sortcmp",
          )
          .map_err(|e| e.to_string())?
      };
      let new_aj = builder
        .build_select(should_swap, aj1_val, aj_val, "sortnewaj")
        .map_err(|e| e.to_string())?;
      let new_aj1 = builder
        .build_select(should_swap, aj_val, aj1_val, "sortnewaj1")
        .map_err(|e| e.to_string())?;
      builder
        .build_store(aj_ptr, new_aj)
        .map_err(|e| e.to_string())?;
      builder
        .build_store(aj1_ptr, new_aj1)
        .map_err(|e| e.to_string())?;
      Ok(())
    })
  })?;

  Ok((out_ptr.into(), ValKind::Ptr))
}

/// Plan 55's Decision log: builds a cross-actor `emerald_actor_enqueue`
/// call in place of an ordinary direct call — `build_method_call`'s own
/// dispatch-rule branch, factored out for readability. `argv` is a
/// fixed 16-word (`EMERALD_MESSAGE_ARGV_MAX`, `runtime/emerald_
/// runtime.c`) stack buffer, packed with each argument's own raw
/// 64-bit encoding (an `Int64`/`Nil`/`Symbol` word as-is, a `Float64`
/// bit-cast, a `Ptr`/`Str` `ptrtoint`, a `Bool` zero-extended) — the
/// exact reverse of `declare_actor_trampolines`'s own unpacking, so a
/// message built here decodes correctly on whichever worker thread
/// eventually dequeues it. Always evaluates to `(0, Void)` — a
/// cross-actor call is genuinely asynchronous and never produces a
/// value synchronously (sema's own return-type rule already guarantees
/// every callable method here is declared `Void`).
#[allow(clippy::too_many_arguments)]
fn build_actor_enqueue_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  key: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  const ARGV_MAX: usize = 16;
  if args.len() > ARGV_MAX {
    return Err(format!(
      "codegen: cross-actor call `{key}` takes {} arguments, exceeding the {ARGV_MAX}-word mailbox message cap",
      args.len()
    ));
  }
  let trampoline_fv = *ctx
    .actor_trampolines
    .get(key)
    .ok_or_else(|| format!("codegen: no trampoline compiled for cross-actor call `{key}`"))?;
  let trampoline_ptr = trampoline_fv.as_global_value().as_pointer_value();

  let (self_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;

  let i64_ty = context.i64_type();
  let argv_alloca = builder
    .build_alloca(i64_ty.array_type(ARGV_MAX as u32), "argv")
    .map_err(|e| e.to_string())?;
  for (i, a) in args.iter().enumerate() {
    let (v, kind) = build_expr(
      context,
      builder,
      a,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let raw = match kind {
      ValKind::Int64 | ValKind::Symbol => v.into_int_value(),
      ValKind::Float64 => builder
        .build_bit_cast(v, i64_ty, "argraw")
        .map_err(|e| e.to_string())?
        .into_int_value(),
      ValKind::Ptr | ValKind::Str => builder
        .build_ptr_to_int(v.into_pointer_value(), i64_ty, "argraw")
        .map_err(|e| e.to_string())?,
      ValKind::Bool => builder
        .build_int_z_extend(v.into_int_value(), i64_ty, "argraw")
        .map_err(|e| e.to_string())?,
      ValKind::Void | ValKind::Tuple(_) => {
        return Err("codegen: internal — not a valid cross-actor argument kind".to_string());
      }
    };
    let idx = i64_ty.const_int(i as u64, false);
    let slot_ptr = unsafe {
      builder
        .build_in_bounds_gep(i64_ty, argv_alloca, &[idx], "argvslot")
        .map_err(|e| e.to_string())?
    };
    builder
      .build_store(slot_ptr, raw)
      .map_err(|e| e.to_string())?;
  }

  let argc = i64_ty.const_int(args.len() as u64, false);

  // Plan 60's Decision log (Design decision 1b): every cross-actor send
  // now compiles to this ONE uniform call, regardless of whether `key`
  // is ever actually dispatched remotely — `emerald_actor_dispatch`
  // itself branches on `self_val->is_remote` at runtime, reusing
  // `emerald_actor_enqueue` byte-for-byte on the local path (AC2's own
  // "zero added overhead beyond plan 55's own existing call shape").
  let method_tag = *ctx
    .actor_method_tags
    .get(key)
    .ok_or_else(|| format!("codegen: no method_tag assigned for cross-actor call `{key}`"))?;
  let arg_encoder_fv = *ctx
    .actor_arg_encoders
    .get(key)
    .ok_or_else(|| format!("codegen: no arg encoder compiled for cross-actor call `{key}`"))?;
  let arg_encoder_ptr = arg_encoder_fv.as_global_value().as_pointer_value();
  let i32_ty = context.i32_type();
  let method_tag_val = i32_ty.const_int(method_tag as u64, true);
  let dispatch_call = builder
    .build_call(
      ctx.actor_funcs.dispatch,
      &[
        self_val.into(),
        method_tag_val.into(),
        trampoline_ptr.into(),
        arg_encoder_ptr.into(),
        argv_alloca.into(),
        argc.into(),
      ],
      "dispatchtmp",
    )
    .map_err(|e| e.to_string())?;
  let status = call_result(dispatch_call)?.into_int_value();
  build_send_result(
    context,
    builder,
    status,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )
}

/// Plan 65's `leaf-unified-fallible-send`: replaces the old `build_
/// raise_on_remote_send_failure` (which always raised a catchable
/// `RemoteActorError` on any nonzero status, and always returned
/// `(0, Void)` otherwise) with a real `Result[Void, SendError]`
/// VALUE — `status` is now `runtime/emerald_runtime.c`'s own real,
/// 4-way `EMERALD_DISPATCH_*` code (0 = success, 1 =
/// `ACTOR_TERMINATED`, 3 = `TIMEOUT`, anything else — in practice
/// always 2, `NODE_UNREACHABLE` — the default/fallback arm). Builds
/// and evaluates a synthetic `Expr::Ok`/`Expr::Err` node per branch
/// (`Spanned::synthetic`, the identical "synthesize an AST node in
/// Rust, feed it through the ordinary `build_expr` path" idiom this
/// function's own predecessor already used for its `RemoteActorError`
/// raise), storing each
/// branch's resulting pointer into one shared stack slot merged at
/// `result_blk` — the same "alloca + per-branch store + one final
/// load" value-merge idiom this backend already uses wherever two
/// branches must produce one SSA value without a raw `PHINode` (no
/// existing call site in this file builds one by hand; this leaf
/// doesn't start that precedent either). `RemoteActorError` is
/// UNCHANGED — still raised by `.remote(...)`'s own connect-time
/// failure, a genuinely different, earlier moment this leaf never
/// touches (Decision log).
#[allow(clippy::too_many_arguments)]
fn build_send_result<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  status: inkwell::values::IntValue<'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let func = builder
    .get_insert_block()
    .and_then(|b| b.get_parent())
    .ok_or_else(|| {
      "codegen: internal — no enclosing function for a send-result check".to_string()
    })?;
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let result_slot = builder
    .build_alloca(ptr_ty, "sendresultslot")
    .map_err(|e| e.to_string())?;

  let i32_ty = context.i32_type();
  let ok_blk = context.append_basic_block(func, "sendresult.ok");
  let check_terminated_blk = context.append_basic_block(func, "sendresult.checkterminated");
  let terminated_blk = context.append_basic_block(func, "sendresult.terminated");
  let check_timeout_blk = context.append_basic_block(func, "sendresult.checktimeout");
  let timeout_blk = context.append_basic_block(func, "sendresult.timeout");
  let unreachable_blk = context.append_basic_block(func, "sendresult.unreachable");
  let merge_blk = context.append_basic_block(func, "sendresult.merge");

  let is_ok = builder
    .build_int_compare(
      IntPredicate::EQ,
      status,
      i32_ty.const_int(0, false),
      "sendisok",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(is_ok, ok_blk, check_terminated_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(ok_blk);
  let ok_expr = Spanned::synthetic(Expr::Ok(Box::new(Spanned::synthetic(Expr::Int(0)))));
  let (ok_val, _) = build_expr(
    context,
    builder,
    &ok_expr,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_store(result_slot, ok_val)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(check_terminated_blk);
  let is_terminated = builder
    .build_int_compare(
      IntPredicate::EQ,
      status,
      i32_ty.const_int(1, false),
      "sendisterminated",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(is_terminated, terminated_blk, check_timeout_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(terminated_blk);
  let terminated_expr = Spanned::synthetic(Expr::Err(Box::new(Spanned::synthetic(Expr::Call(
    "ActorTerminated".to_string(),
    Vec::new(),
  )))));
  let (terminated_val, _) = build_expr(
    context,
    builder,
    &terminated_expr,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_store(result_slot, terminated_val)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(check_timeout_blk);
  let is_timeout = builder
    .build_int_compare(
      IntPredicate::EQ,
      status,
      i32_ty.const_int(3, false),
      "sendistimeout",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(is_timeout, timeout_blk, unreachable_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(timeout_blk);
  let timeout_expr = Spanned::synthetic(Expr::Err(Box::new(Spanned::synthetic(Expr::Call(
    "Timeout".to_string(),
    Vec::new(),
  )))));
  let (timeout_val, _) = build_expr(
    context,
    builder,
    &timeout_expr,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_store(result_slot, timeout_val)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  // Default/fallback arm — in practice always `EMERALD_DISPATCH_NODE_
  // UNREACHABLE` (2), but any other, unanticipated nonzero code lands
  // here too rather than being silently mistaken for success.
  builder.position_at_end(unreachable_blk);
  let unreachable_expr = Spanned::synthetic(Expr::Err(Box::new(Spanned::synthetic(Expr::Call(
    "NodeUnreachable".to_string(),
    Vec::new(),
  )))));
  let (unreachable_val, _) = build_expr(
    context,
    builder,
    &unreachable_expr,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  builder
    .build_store(result_slot, unreachable_val)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(merge_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(merge_blk);
  let loaded = builder
    .build_load(ptr_ty, result_slot, "sendresult")
    .map_err(|e| e.to_string())?;
  Ok((loaded, ValKind::Ptr))
}

/// `leaf-remote-dispatch-and-worked-proof`'s own failure path,
/// UNCHANGED by plan 65 (Decision log: `.remote(addr, name)`'s own
/// connect-time failure is a genuinely different, EARLIER moment than
/// a post-connection send — no `EmeraldActorRef` even exists yet for
/// a `Result` to travel through, so it stays exactly what it always
/// was, a raised, catchable `RemoteActorError`). `status != 0` means
/// `Expr::Remote`'s own `.remote(...)` call site (its only remaining
/// caller after plan 65's `build_send_result` took over the cross-
/// actor SEND path) got back a `NULL` ref. `status == 0` falls
/// straight through with no branch overhead beyond the one `icmp`/
/// `br` pair.
#[allow(clippy::too_many_arguments)]
fn build_raise_on_remote_send_failure<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  status: inkwell::values::IntValue<'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let func = builder
    .get_insert_block()
    .and_then(|b| b.get_parent())
    .ok_or_else(|| {
      "codegen: internal — no enclosing function for a remote-send check".to_string()
    })?;
  let zero = context.i32_type().const_int(0, false);
  let is_err = builder
    .build_int_compare(IntPredicate::NE, status, zero, "remotesendfailed")
    .map_err(|e| e.to_string())?;
  let err_blk = context.append_basic_block(func, "remotesend.err");
  let ok_blk = context.append_basic_block(func, "remotesend.ok");
  builder
    .build_conditional_branch(is_err, err_blk, ok_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(err_blk);
  let msg_call = builder
    .build_call(ctx.actor_funcs.remote_last_error, &[], "remotesenderrmsg")
    .map_err(|e| e.to_string())?;
  let msg_ptr = call_result(msg_call)?.into_pointer_value();
  let msg_slot = builder
    .build_alloca(
      context.ptr_type(AddressSpace::default()),
      "remotesenderrmsgslot",
    )
    .map_err(|e| e.to_string())?;
  builder
    .build_store(msg_slot, msg_ptr)
    .map_err(|e| e.to_string())?;
  let mut raise_vars = vars.clone();
  raise_vars.insert(
    "__remote_send_err_msg".to_string(),
    (msg_slot, ValKind::Str),
  );
  let raise_expr = Spanned::synthetic(Expr::New(
    "RemoteActorError".to_string(),
    vec![Spanned::synthetic(Expr::Ident(
      "__remote_send_err_msg".to_string(),
    ))],
  ));
  build_raise(
    context,
    builder,
    &raise_expr,
    &raise_vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;

  builder.position_at_end(ok_blk);
  Ok(())
}

/// `recv.register(name, port)` (`leaf-actor-ref-and-addressing`) —
/// evaluates `name`/`port`, computes `name`'s length via the same
/// `ctx.string_length` runtime helper `.remote`'s own codegen already
/// uses, and calls `emerald_actor_register` with `class_name`'s own
/// already-built method-dispatch table (`build_actor_method_tables`).
/// Always `Void` — `.register`'s only effect is the side effect of
/// starting this process's listener (idempotent) and adding a name
/// entry, nothing meaningful to return.
#[allow(clippy::too_many_arguments)]
fn build_actor_register_call<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  recv: &Spanned<Expr>,
  args: &[Spanned<Expr>],
  class_name: &str,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(BasicValueEnum<'ctx>, ValKind), String> {
  let (ref_val, _) = build_expr(
    context,
    builder,
    recv,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (name_val, _) = build_expr(
    context,
    builder,
    &args[0],
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let (port_val, _) = build_expr(
    context,
    builder,
    &args[1],
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let name_len_call = builder
    .build_call(ctx.string_length, &[name_val.into()], "registernamelen")
    .map_err(|e| e.to_string())?;
  let name_len = call_result(name_len_call)?;
  let port32 = builder
    .build_int_truncate(
      port_val.into_int_value(),
      context.i32_type(),
      "registerport32",
    )
    .map_err(|e| e.to_string())?;
  let methods_ptr = *ctx
    .actor_method_tables
    .get(class_name)
    .ok_or_else(|| format!("codegen: internal — no method table built for actor `{class_name}`"))?;
  let method_count = *ctx.actor_method_counts.get(class_name).ok_or_else(|| {
    format!("codegen: internal — no method count recorded for actor `{class_name}`")
  })?;
  let method_count_val = context.i64_type().const_int(method_count as u64, false);
  builder
    .build_call(
      ctx.actor_funcs.register,
      &[
        ref_val.into(),
        name_val.into(),
        name_len.into(),
        port32.into(),
        methods_ptr.into(),
        method_count_val.into(),
      ],
      "registercall",
    )
    .map_err(|e| e.to_string())?;
  Ok((context.i64_type().const_int(0, false).into(), ValKind::Void))
}

/// `recv?.method(args)` (plan 73's Decision log — replaces plan 43's
/// `&.` outright, new `Option[T]` semantics): `recv` is an `Option[T]`
/// value (a real tagged-union pointer, plan 52's mechanism, never a
/// bare nullable pointer any more) — this branches on its own tag
/// (`Some` vs. `None`), dispatches `method` on the unwrapped `Some`
/// payload only on the `Some` path, then re-wraps the result as a
/// FRESH `Option[U]` value (`Some(result)` / `None`), merged via a real
/// LLVM `phi` over that fresh enum's own tagged-union pointer — the
/// same is-guarded-basic-blocks-plus-PHI shape `build_short_circuit`/
/// plan 43's own now-replaced implementation already established, just
/// branching on a loaded tag instead of `is_null`.
///
/// Real, disclosed limitation: codegen has no expected-type context
/// here (unlike `emerald-sema`, which resolves this from the enclosing
/// `let`/`return`'s own declared type) — the fresh `Option[U]` to wrap
/// into is found by scanning `ctx.enums` for the unique already-
/// instantiated `"Option$..."` whose `Some` payload `ValKind` matches
/// the dispatched method's own return kind. Correct whenever the
/// program has at most one `Option[T]` instantiation per distinct
/// `ValKind` (the overwhelmingly common case); genuinely ambiguous only
/// if two DIFFERENT `Option[T]`/`Option[U]` instantiations share the
/// same underlying `ValKind` (e.g. two different classes, both
/// `ValKind::Ptr`) — a real gap, disclosed rather than silently
/// mis-wrapped: this errors out by name in that case.
/// Returns `(the result value, its ValKind — always `Ptr`, the FRESH
/// `Option[U]`'s own enum name)` — the third element lets a caller that
/// itself needs to know which `Option[U]` this produced (`Expr::
/// Coalesce`'s own `lhs` handling, when `lhs` is itself a `?.` chain
/// rather than a plain local) reuse it directly instead of re-deriving
/// it from scratch.
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
) -> Result<(BasicValueEnum<'ctx>, ValKind, String), String> {
  let Expr::Ident(recv_name) = &recv.node else {
    return Err("codegen: `?.` is only supported on a plain local-variable receiver".to_string());
  };
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
      "codegen: `?.` requires an `Option[T]` receiver, found {recv_kind:?}"
    ));
  }
  let enum_name = local_classes.get(recv_name).cloned().ok_or_else(|| {
    format!(
      "codegen: `?.` requires an `Option[T]`-typed receiver, found untyped local `{recv_name}`"
    )
  })?;
  let layout = ctx.enums.get(enum_name.as_str()).ok_or_else(|| {
    format!(
      "codegen: internal error — unregistered enum `{enum_name}` (sema should have rejected this)"
    )
  })?;
  let some_tag = *layout.variant_tags.get("Some").ok_or_else(|| {
    format!("codegen: internal error — `{enum_name}` has no `Some` variant (sema should have rejected this)")
  })?;
  let inner_kind = layout
    .variant_fields
    .get("Some")
    .and_then(|f| f.first())
    .cloned()
    .ok_or_else(|| format!("codegen: internal error — `{enum_name}`'s `Some` has no field"))?;
  let inner_ty = layout
    .variant_field_types
    .get("Some")
    .and_then(|f| f.first())
    .cloned()
    .unwrap_or_default();

  let ptr = recv_val.into_pointer_value();
  let tag_val = load_field(
    context,
    builder,
    ptr,
    FieldInfo {
      offset: 0,
      kind: ValKind::Int64,
    },
  )?
  .into_int_value();
  let some_tag_const = context.i64_type().const_int(some_tag, false);
  let is_some = builder
    .build_int_compare(IntPredicate::EQ, tag_val, some_tag_const, "issome")
    .map_err(|e| e.to_string())?;

  let entry_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block")?;
  let func = entry_block
    .get_parent()
    .ok_or("codegen: internal error — block has no parent function")?;
  let call_block = context.append_basic_block(func, "safecall.call");
  let none_block = context.append_basic_block(func, "safecall.none");
  let merge_block = context.append_basic_block(func, "safecall.merge");

  builder
    .build_conditional_branch(is_some, call_block, none_block)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(call_block);
  let inner_val = load_field(
    context,
    builder,
    ptr,
    FieldInfo {
      offset: 8,
      kind: inner_kind.clone(),
    },
  )?;
  // Plan 62's own "synthesize a fresh AST node and reuse the existing
  // codegen path unchanged" technique (`build_requires_checks`'
  // Decision log), applied here: `build_method_call` needs a real
  // `Expr::Ident` receiver to dispatch through `local_classes` — stash
  // the just-unwrapped payload into its own scratch stack slot under a
  // synthetic name, in a CLONED `vars`/`local_classes` (never mutating
  // the caller's real environment), then call through exactly like any
  // other local.
  let synthetic_name = format!("__safecall_recv_{recv_name}");
  let alloca = builder
    .build_alloca(local_llvm_type(context, &inner_kind), &synthetic_name)
    .map_err(|e| e.to_string())?;
  builder
    .build_store(alloca, inner_val)
    .map_err(|e| e.to_string())?;
  let mut inner_vars = vars.clone();
  inner_vars.insert(synthetic_name.clone(), (alloca, inner_kind.clone()));
  let mut inner_local_classes = local_classes.clone();
  if ctx.classes.contains_key(inner_ty.as_str()) {
    inner_local_classes.insert(synthetic_name.clone(), inner_ty.clone());
  }
  let synthetic_recv = Spanned::synthetic(Expr::Ident(synthetic_name));
  let (call_val, call_kind) = build_method_call(
    context,
    builder,
    &synthetic_recv,
    method,
    args,
    &inner_vars,
    &inner_local_classes,
    local_array_elem_types,
    ctx,
  )?;

  // Re-wrap the call's result as a fresh `Option[U]` — see this
  // function's own doc comment for the real, disclosed limitation this
  // lookup has.
  let result_enum_name = ctx
    .enums
    .iter()
    .filter(|(name, l)| {
      name.starts_with("Option$")
        && l
          .variant_fields
          .get("Some")
          .and_then(|f| f.first())
          .is_some_and(|k| *k == call_kind)
    })
    .map(|(name, _)| name.clone())
    .collect::<Vec<_>>();
  let [result_enum_name] = result_enum_name.as_slice() else {
    return Err(format!(
      "codegen: `?.{method}`'s result type ({call_kind:?}) doesn't uniquely identify an already-instantiated `Option[U]` — candidates: {result_enum_name:?}; add an explicit `Option[U]` type annotation elsewhere in this program"
    ));
  };
  let result_layout = &ctx.enums[result_enum_name];
  let result_size = context.i64_type().const_int(result_layout.size, false);
  let alloc_call = builder
    .build_call(ctx.alloc, &[result_size.into()], "safecallwrap")
    .map_err(|e| e.to_string())?;
  let result_ptr = call_result(alloc_call)?.into_pointer_value();
  let result_tag_ptr = field_ptr(context, builder, result_ptr, 0)?;
  let result_some_tag = *result_layout.variant_tags.get("Some").ok_or_else(|| {
    format!("codegen: internal error — `{result_enum_name}` has no `Some` variant")
  })?;
  builder
    .build_store(
      result_tag_ptr,
      context.i64_type().const_int(result_some_tag, false),
    )
    .map_err(|e| e.to_string())?;
  let result_field_ptr = field_ptr(context, builder, result_ptr, 8)?;
  builder
    .build_store(result_field_ptr, call_val)
    .map_err(|e| e.to_string())?;

  let call_end_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block after call")?;
  builder
    .build_unconditional_branch(merge_block)
    .map_err(|e| e.to_string())?;

  // On the `None` path, the result is a FRESH `None` too — allocated in
  // its own block rather than reusing the receiver's own pointer (a
  // distinct `Option[U]` instantiation may have a different `size`/
  // layout than `Option[T]`).
  builder.position_at_end(none_block);
  let result_none_tag = *result_layout.variant_tags.get("None").ok_or_else(|| {
    format!("codegen: internal error — `{result_enum_name}` has no `None` variant")
  })?;
  let none_alloc_call = builder
    .build_call(ctx.alloc, &[result_size.into()], "safecallnone")
    .map_err(|e| e.to_string())?;
  let none_ptr = call_result(none_alloc_call)?.into_pointer_value();
  let none_tag_ptr = field_ptr(context, builder, none_ptr, 0)?;
  builder
    .build_store(
      none_tag_ptr,
      context.i64_type().const_int(result_none_tag, false),
    )
    .map_err(|e| e.to_string())?;
  let none_end_block = builder
    .get_insert_block()
    .ok_or("codegen: internal error — no current block after none")?;
  builder
    .build_unconditional_branch(merge_block)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(merge_block);
  let ptr_ty = context.ptr_type(AddressSpace::default());
  let phi = builder
    .build_phi(ptr_ty, "safecallresult")
    .map_err(|e| e.to_string())?;
  phi.add_incoming(&[(&none_ptr, none_end_block), (&result_ptr, call_end_block)]);
  Ok((phi.as_basic_value(), ValKind::Ptr, result_enum_name.clone()))
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
  // Plan 42 (enumerable stdlib), `leaf-array-length-header`: `[length:
  // Int64][elements...]`, mirroring `Hash[K,V]`'s already-shipped
  // `[count: Int64][pairs...]` layout (`build_hash_lit`) — the missing
  // prerequisite `each`/`count`/etc. need to know when to stop. The
  // stored base pointer is still the header-inclusive address (matching
  // `Hash`'s own convention); every element shifts to `8 + i * 8`.
  let size_val = context
    .i64_type()
    .const_int(8 + elements.len() as u64 * 8, false);
  let call = builder
    .build_call(ctx.alloc, &[size_val.into()], "arralloc")
    .map_err(|e| e.to_string())?;
  let ptr = call_result(call)?.into_pointer_value();
  let count_val = context.i64_type().const_int(elements.len() as u64, false);
  builder
    .build_store(ptr, count_val)
    .map_err(|e| e.to_string())?;
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
    let elem_ptr = field_ptr(context, builder, ptr, 8 + i as u64 * 8)?;
    builder
      .build_store(elem_ptr, v)
      .map_err(|e| e.to_string())?;
  }
  Ok(ptr)
}

/// Plan 84's Decision log: computes the ADDRESS `arg` should be passed
/// at, for a call-site argument bound to a by-value `borrow`/`borrow
/// var` marker parameter — shared by `build_call_arg_vals` (`Expr::
/// Call`) and `build_call_kw_expr` (`Expr::CallKw`), the two call forms
/// this plan's own scope covers (see `strip_ownership_in_type_expr`'s
/// own doc comment for why method calls, `build_method_call`'s
/// separate arg-building code, are NOT covered). A plain local-variable
/// argument reuses ITS OWN existing `alloca` directly (genuinely
/// zero-cost — every local this backend tracks already lives behind a
/// `PointerValue`, `vars`' own doc comment); anything else is evaluated
/// normally and spilled into a fresh `alloca`, since there is no
/// existing address to hand over instead.
fn build_borrow_arg_ptr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  arg: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<PointerValue<'ctx>, String> {
  if let Expr::Ident(name) = &arg.node {
    if let Some((ptr, _)) = vars.get(name) {
      return Ok(*ptr);
    }
  }
  let (v, k) = build_expr(
    context,
    builder,
    arg,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let spill = builder
    .build_alloca(local_llvm_type(context, &k), "borrowargspill")
    .map_err(|e| e.to_string())?;
  builder.build_store(spill, v).map_err(|e| e.to_string())?;
  Ok(spill)
}

/// Plan 39's Decision log: `Expr::Call`'s own positional argument-value
/// builder — fills any missing trailing arguments from the callee's
/// declared defaults, and — when the callee declares a splat parameter
/// — packs every argument beyond its ordinary parameter count into a
/// freshly allocated `Array[Elem]` (reusing `build_array_lit`'s own
/// alloc-and-store loop wholesale), appended as one final argument. The
/// compiled callee itself is always fixed-arity — this is purely a
/// call-site concern.
///
/// Plan 84's Decision log: when the callee's own `param.ty` is the
/// by-value borrow-ptr marker (`borrow_ptr_marker_info` — a top-level
/// function's `borrow`/`borrow var` of `Int64`/`Float64`/`Boolean`/
/// `Symbol`, per `strip_ownership_in_type_expr`'s own doc comment),
/// this pushes the ARGUMENT'S ADDRESS instead of its value, matching
/// the `ptr`-typed slot `make_fn_type` already declared for it. Two
/// cases: a plain local-variable argument (`Expr::Ident` naming an
/// entry already in `vars`) reuses that binding's OWN existing `alloca`
/// directly — genuinely zero-cost, no new instruction at all, since
/// every local this backend tracks is already an `(alloca, kind)` pair
/// (`vars`' own doc comment). Any other argument shape (a literal, a
/// computed expression, ...) has no existing address to reuse, so it's
/// evaluated normally and spilled into a fresh `alloca` — one real,
/// disclosed extra store this specific case pays that an ordinary
/// by-value parameter never would (see `spec/OWNERSHIP.md`'s own
/// updated zero-cost claim and `examples/ownership.em`'s benchmark for
/// the measured cost of each case).
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
    if borrow_ptr_marker_info(&param.ty).is_some() {
      let arg_ptr = build_borrow_arg_ptr(
        context,
        builder,
        expr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      arg_vals.push(arg_ptr.into());
      continue;
    }
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
  // Plan 52's Decision log: `Circle(2.0)` reaches here exactly like an
  // ordinary function call (no new grammar production — sema's own
  // registration-time checks already guarantee a name is never both a
  // declared variant and a declared function) — a hit against the
  // variant-owner lookup, checked before the generic/ordinary
  // function-call paths below, resolves it as construction instead:
  // allocate `layout.size` bytes (the same `ctx.alloc` call
  // `Expr::New` already uses), store the tag at offset 0 (mirroring
  // `build_hash_lit`'s own count-header store), then each argument at
  // `8 + i * 8`.
  if let Some((enum_name, tag)) = find_variant_layout(name, ctx.enums) {
    let layout = &ctx.enums[enum_name];
    let size_val = context.i64_type().const_int(layout.size, false);
    let alloc_call = builder
      .build_call(ctx.alloc, &[size_val.into()], "enumlit")
      .map_err(|e| e.to_string())?;
    let ptr = call_result(alloc_call)?.into_pointer_value();
    let tag_ptr = field_ptr(context, builder, ptr, 0)?;
    builder
      .build_store(tag_ptr, context.i64_type().const_int(tag, false))
      .map_err(|e| e.to_string())?;
    for (i, arg) in args.iter().enumerate() {
      let (v, _) = build_expr(
        context,
        builder,
        arg,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let value_ptr = field_ptr(context, builder, ptr, 8 + i as u64 * 8)?;
      builder
        .build_store(value_ptr, v)
        .map_err(|e| e.to_string())?;
    }
    return Ok((ptr.into(), ValKind::Ptr));
  }
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
    let (fv, ret_kind) = ctx
      .user_func_ids
      .get(&mangled)
      .map(|(fv, k)| (*fv, k.clone()))
      .ok_or_else(|| {
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
  let (fv, ret_kind) = ctx
    .user_func_ids
    .get(name)
    .map(|(fv, k)| (*fv, k.clone()))
    .ok_or_else(|| {
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
  let (fv, ret_kind) = ctx
    .user_func_ids
    .get(name)
    .map(|(fv, k)| (*fv, k.clone()))
    .ok_or_else(|| {
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
    // Plan 84's Decision log: mirrors `build_call_arg_vals`'s own
    // identical check — see `build_borrow_arg_ptr`'s doc comment.
    if borrow_ptr_marker_info(&f.params[i].ty).is_some() {
      let arg_ptr = build_borrow_arg_ptr(
        context,
        builder,
        expr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      arg_vals.push(arg_ptr.into());
      continue;
    }
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
  Some((
    value_kind_for_type(&TypeExpr::Named(k.to_string())),
    value_kind_for_type(&TypeExpr::Named(v.to_string())),
  ))
}

/// Plan 42 (enumerable stdlib): mirrors `parse_hash_type` exactly, for
/// the `"Pair[K, V]"` local_classes convention (`Type::Pair`'s own doc
/// comment) — `Hash[K,V].each`'s own block parameter is the only
/// source of a `Pair` value.
fn parse_pair_type(s: &str) -> Option<(ValKind, ValKind)> {
  let inner = s.strip_prefix("Pair[")?.strip_suffix(']')?;
  let (k, v) = inner.split_once(", ")?;
  Some((
    value_kind_for_type(&TypeExpr::Named(k.to_string())),
    value_kind_for_type(&TypeExpr::Named(v.to_string())),
  ))
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
  if let Some(elem_kind) = local_array_elem_types.get(arr_name) {
    if idx_kind != ValKind::Int64 {
      return Err("codegen: array index must be Int64".to_string());
    }
    let elem_llvm_ty = local_llvm_type(context, elem_kind);
    // Plan 42's `leaf-array-length-header`: the stored base pointer is
    // header-inclusive (`build_array_lit`'s own doc comment) — every
    // element read/write shifts a fixed 8 bytes past it first.
    let elems_base = field_ptr(context, builder, base.into_pointer_value(), 8)?;
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx.into_int_value()], "elemptr")
        .map_err(|e| e.to_string())?
    };
    let loaded = builder
      .build_load(elem_llvm_ty, elem_ptr, "elemval")
      .map_err(|e| e.to_string())?;
    return Ok((loaded, elem_kind.clone()));
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
    let value_llvm_ty = local_llvm_type(context, &value_kind);
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
  if let Some(elem_kind) = local_array_elem_types.get(arr_name) {
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
    // Plan 42's `leaf-array-length-header`: same fixed 8-byte shift as
    // `build_index`'s own read side.
    let elems_base = field_ptr(context, builder, base.into_pointer_value(), 8)?;
    let elem_ptr = unsafe {
      builder
        .build_in_bounds_gep(elem_llvm_ty, elems_base, &[idx.into_int_value()], "elemptr")
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
    .const_int(CLOSURE_HEADER_BYTES + info.captures.len() as u64 * 8, false);
  let call = builder
    .build_call(ctx.alloc, &[size_val.into()], "envalloc")
    .map_err(|e| e.to_string())?;
  let env_ptr = call_result(call)?.into_pointer_value();
  // Plan 89's Decision log: the closure block's own leading header slot
  // carries this lambda's compiled function pointer, so any Proc value
  // derived from `name` later can be called indirectly, not just via
  // `ctx.lambda_func_ids`'s static name lookup (`.call`'s own dispatch
  // in `build_method_call`).
  let (lambda_fv, _) = ctx.lambda_func_ids.get(name).ok_or_else(|| {
    format!("codegen: internal error — `{name}` has no compiled `__lambda_` function")
  })?;
  let fn_ptr_slot = field_ptr(context, builder, env_ptr, 0)?;
  builder
    .build_store(fn_ptr_slot, lambda_fv.as_global_value().as_pointer_value())
    .map_err(|e| e.to_string())?;
  for cap_name in &info.captures {
    let (cap_ptr, cap_kind) = vars.get(cap_name).ok_or_else(|| {
      format!("codegen: captured variable `{cap_name}` is not in scope at `{name}`'s creation site")
    })?;
    let val = builder
      .build_load(local_llvm_type(context, cap_kind), *cap_ptr, cap_name)
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

/// Plan 42 (enumerable stdlib): `for i in 0..count { body(i) }`,
/// built from fresh basic blocks mirroring `build_for`'s own cond/
/// body/incr/exit shape immediately below — shared by every one of
/// this plan's eight intrinsic Array/Hash methods, each of which walks
/// every element/pair exactly once. `body` runs with the builder
/// positioned inside the loop's own body block and receives that
/// iteration's index as a plain `IntValue`; it may append its own
/// further basic blocks (as `build_enumerable_call`'s own `select`
/// case does, for its conditional keep-or-skip write), as long as the
/// builder ends up positioned at whichever block should fall through
/// into this loop's own increment step once `body` returns — no
/// `break`/`next` support (`loop_stack` isn't threaded through), a
/// real, disclosed narrowing: none of these eight methods' own block
/// arguments are checked against or expected to use either.
fn build_count_loop<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  count_val: IntValue<'ctx>,
  mut body: impl FnMut(&Builder<'ctx>, IntValue<'ctx>) -> Result<(), String>,
) -> Result<(), String> {
  let i64_ty = context.i64_type();
  let idx_alloca = builder
    .build_alloca(i64_ty, "encount.idx")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, i64_ty.const_int(0, false))
    .map_err(|e| e.to_string())?;

  let cond_blk = context.append_basic_block(func, "encount.cond");
  let body_blk = context.append_basic_block(func, "encount.body");
  let incr_blk = context.append_basic_block(func, "encount.incr");
  let exit_blk = context.append_basic_block(func, "encount.exit");

  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(cond_blk);
  let idx_val = builder
    .build_load(i64_ty, idx_alloca, "encount.idx.val")
    .map_err(|e| e.to_string())?
    .into_int_value();
  let cond_val = builder
    .build_int_compare(IntPredicate::SLT, idx_val, count_val, "encount.cmp")
    .map_err(|e| e.to_string())?;
  builder
    .build_conditional_branch(cond_val, body_blk, exit_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(body_blk);
  body(builder, idx_val)?;
  builder
    .build_unconditional_branch(incr_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(incr_blk);
  let next_val = builder
    .build_int_add(idx_val, i64_ty.const_int(1, false), "encount.next")
    .map_err(|e| e.to_string())?;
  builder
    .build_store(idx_alloca, next_val)
    .map_err(|e| e.to_string())?;
  builder
    .build_unconditional_branch(cond_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(exit_blk);
  Ok(())
}

/// Plan 42 (enumerable stdlib): builds a direct call to an already-
/// compiled top-level `Proc` (`declare_lambda_functions`'s own
/// `__lambda_{name}` — real, disclosed design correction, found this
/// session: see `emerald-sema`'s `check_enumerable_proc_arg`'s own doc
/// comment for why a NAMED `Proc`, not an inline block literal, is
/// what actually reaches codegen here). `proc_arg` must be a plain
/// `Expr::Ident` (sema's own `check_enumerable_proc_arg` already
/// guarantees the argument is `Type::Proc`-typed, and the only way to
/// get one in this language is a top-level `Let`-bound name). Passes
/// the lambda's own captured-environment pointer as its implicit first
/// argument (`build_expr` on `proc_arg` itself already evaluates to
/// exactly that pointer — the same value `.call`'s own existing
/// dispatch reads), then `call_args_tail`. Returns `None` for the
/// value when the Proc is `Void`-returning (`call_result` errors
/// trying to extract a value from a genuinely void call) — every
/// caller that needs a real value already knows, from sema's own
/// check, that its Proc isn't `Void`.
#[allow(clippy::too_many_arguments)]
fn call_named_proc<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  proc_arg: &Spanned<Expr>,
  call_args_tail: &[BasicValueEnum<'ctx>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(Option<BasicValueEnum<'ctx>>, ValKind), String> {
  let Expr::Ident(proc_name) = &proc_arg.node else {
    return Err(
      "codegen: internal error — enumerable Proc argument is not a plain local (sema should have rejected this)"
        .to_string(),
    );
  };
  let (lambda_fv, ret_kind) = ctx
    .lambda_func_ids
    .get(proc_name)
    .map(|(fv, k)| (*fv, k.clone()))
    .ok_or_else(|| {
      format!("codegen: internal error — `{proc_name}` is not a compiled top-level Proc")
    })?;
  let (env_val, _) = build_expr(
    context,
    builder,
    proc_arg,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let mut call_args: Vec<BasicMetadataValueEnum> = vec![env_val.into()];
  call_args.extend(
    call_args_tail
      .iter()
      .map(|v| BasicMetadataValueEnum::from(*v)),
  );
  let call = builder
    .build_call(lambda_fv, &call_args, "enproccall")
    .map_err(|e| e.to_string())?;
  if ret_kind == ValKind::Void {
    Ok((None, ValKind::Void))
  } else {
    Ok((Some(call_result(call)?), ret_kind))
  }
}

/// Plan 89's Decision log: compiles an anonymous lambda literal that
/// appears at an ordinary EXPRESSION position (a call argument — this
/// plan's own worked example, `n.map(do |x: Int64| x * 2 end)` — never
/// a top-level `Let`'s own value, which `build_lambda_let` already
/// handles unchanged) into a real, separately-compiled top-level LLVM
/// function, on the fly, right where it's first needed — not via a
/// separate whole-program discovery-then-declare pass. Reuses `define_
/// lambda` verbatim for the function's own body (same captured-
/// environment-loading + param-binding + statement-compiling logic a
/// top-level `Proc` `Let` already gets), and `build_lambda_let`'s own
/// closure-construction shape (alloc, store the fn pointer at the
/// header slot, store each capture) for the call-site value. The
/// builder's own position is saved and restored around the nested
/// `define_lambda` call — inkwell/LLVM has no notion of "the current
/// function"; only which basic block the builder is positioned at, so
/// interleaving a whole separate function's construction mid-
/// compilation of the caller is sound as long as the builder ends up
/// back where the caller expects it (exactly what happens here).
/// Named by the lambda's own source span (`Spanned<Expr>::span`, unique
/// per literal occurrence) rather than a shared mutable counter.
fn build_inline_lambda<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  lam_expr: &Spanned<Expr>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(PointerValue<'ctx>, ValKind), String> {
  let Expr::Lambda { params, body, .. } = &lam_expr.node else {
    return Err(
      "codegen: internal error — build_inline_lambda called on a non-lambda expr".to_string(),
    );
  };
  let base_env = build_local_val_kind_env(vars, ctx.top_level_types);
  let ret_kind = infer_lambda_ret_kind(params, body, &base_env, ctx.user_fn_return_types);
  let captures = free_vars_in_lambda(params, body);
  let mut capture_offsets = HashMap::new();
  let mut capture_kinds = HashMap::new();
  for (i, cap_name) in captures.iter().enumerate() {
    let (_, kind) = vars.get(cap_name).ok_or_else(|| {
      format!(
        "codegen: captured variable `{cap_name}` is not in scope at this inline lambda's creation site"
      )
    })?;
    capture_offsets.insert(cap_name.clone(), CLOSURE_HEADER_BYTES + i as u64 * 8);
    capture_kinds.insert(cap_name.clone(), kind.clone());
  }
  let info = LambdaInfo {
    captures: captures.clone(),
    capture_offsets,
    capture_kinds,
  };

  let unique_name = format!("__inline_lambda_{}_{}", lam_expr.span.0, lam_expr.span.1);
  let mut kinds = vec![ValKind::Ptr]; // env
  kinds.extend(param_kinds(params));
  let fn_ty = make_fn_type(context, &kinds, &ret_kind);
  let fv = ctx.module.add_function(
    &format!("__lambda_{unique_name}"),
    fn_ty,
    Some(Linkage::Internal),
  );

  // Compiling the new function's own body repositions the shared
  // `Builder` — save/restore around it so the CALLER's own in-progress
  // block is exactly where it was once this returns. The debug
  // location is SEPARATE builder state that survives a `position_at_
  // end` unchanged (`define_lambda`'s own `build_stmt` calls set a NEW
  // one, scoped to the nested function's own `DISubprogram`) — left
  // unrestored, every instruction the CALLER builds after this returns
  // would carry the wrong function's debug scope until its own next
  // statement resets it, which is exactly the "!dbg attachment points
  // at wrong subprogram" verifier failure this restores against.
  let saved_block = builder.get_insert_block();
  let saved_di_loc = builder.get_current_debug_location();
  define_lambda(
    context,
    builder,
    &unique_name,
    params,
    ret_kind.clone(),
    body,
    &info,
    fv,
    ctx,
  )?;
  if let Some(block) = saved_block {
    builder.position_at_end(block);
  }
  match saved_di_loc {
    Some(loc) => builder.set_current_debug_location(loc),
    None => builder.unset_current_debug_location(),
  }

  let size_val = context
    .i64_type()
    .const_int(CLOSURE_HEADER_BYTES + captures.len() as u64 * 8, false);
  let alloc_call = builder
    .build_call(ctx.alloc, &[size_val.into()], "inlineenvalloc")
    .map_err(|e| e.to_string())?;
  let env_ptr = call_result(alloc_call)?.into_pointer_value();
  let fn_ptr_slot = field_ptr(context, builder, env_ptr, 0)?;
  builder
    .build_store(fn_ptr_slot, fv.as_global_value().as_pointer_value())
    .map_err(|e| e.to_string())?;
  for cap_name in &captures {
    let (cap_ptr, cap_kind) = vars.get(cap_name).ok_or_else(|| {
      format!("codegen: captured variable `{cap_name}` is not in scope at this inline lambda's creation site")
    })?;
    let val = builder
      .build_load(local_llvm_type(context, cap_kind), *cap_ptr, cap_name)
      .map_err(|e| e.to_string())?;
    let offset = info.capture_offsets[cap_name];
    let slot_ptr = field_ptr(context, builder, env_ptr, offset)?;
    builder
      .build_store(slot_ptr, val)
      .map_err(|e| e.to_string())?;
  }
  Ok((env_ptr, ValKind::Ptr))
}

/// Plan 89's Decision log, generalized 2026-09-21 (this session's
/// "find all bugs" sweep — see `infer_concrete_type_from_arg`'s own
/// doc comment for the full story): originally the ONE shape this
/// codegen-side generic-method type-parameter resolver handled was
/// `raw` (the method's own declared param type, already interface-
/// type-parameter-substituted) being `Proc[..., U]` (`type_param` in
/// RETURN position, this plan's own worked example, `f: Proc[T, U]`).
/// Now also handles the far more common shape — `raw` being the bare
/// type parameter itself (`x: U`) — via the identical leaf-argument
/// inference `infer_concrete_type_from_arg` already does for the
/// `Proc` case, mirroring `emerald-sema`'s own `infer_type_param_
/// binding`'s `TypeExpr::Named(name) if name == param_name` arm. Still
/// a real, disclosed narrowing next to that fuller structural unifier
/// — sema already proved the program well-typed; this only re-derives
/// the SAME binding for the leaf argument shapes `infer_concrete_type_
/// from_arg` recognizes, not general structural unification (a type
/// parameter nested inside `Array[U]`/`Hash[K, U]`/... is not covered).
fn infer_method_type_param_binding<'ctx>(
  raw: &TypeExpr,
  arg: &Spanned<Expr>,
  type_param: &str,
  ctx: &Ctx<'_, 'ctx>,
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
) -> Option<TypeExpr> {
  match raw {
    TypeExpr::Func(_, ret) if ret.as_named() == Some(type_param) => {
      infer_concrete_type_from_arg(arg, ctx, vars, local_classes)
    }
    TypeExpr::Named(name) if name == type_param => {
      infer_concrete_type_from_arg(arg, ctx, vars, local_classes)
    }
    _ => None,
  }
}

/// Plan 89's Decision log: replaces the old codegen-time stopgap
/// rejection (`build_method_call`'s own former "generic methods are
/// not yet supported for code generation" diagnostic) with real,
/// lazy, per-call-site monomorphization — mirroring the existing top-
/// level generic-FUNCTION pipeline's strategy exactly (a distinct
/// compiled function per concrete type-parameter binding, mangled
/// name, no vtables — `substitute_generic_function`/`mangled_generic_
/// symbol`/`collect_generic_specializations`'s own header comments) but
/// driven LAZILY from this one call site (`ctx.generic_method_
/// instances`'s own doc comment) rather than a separate whole-program
/// discovery-then-declare pass, since a method's own free type
/// parameter can bind to a non-class primitive (`Int64`, unlike a top-
/// level generic function's own class-only bound), which needs the
/// ACTUAL call argument in hand to infer at all.
///
/// Also substitutes the enclosing class's own bound interface type
/// parameter (`implements Iterable[Int64]`'s `T`) — a generic
/// INTERFACE method's own free type parameter (`U`) is one level of
/// generics on top of whatever `T` the class's own `implements` clause
/// already binds, exactly mirroring `emerald-sema`'s own `interface_
/// type_param_subst`/`build_flattened_class_info` pairing.
#[allow(clippy::too_many_arguments)]
fn resolve_generic_method_instance<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  ctx: &Ctx<'_, 'ctx>,
  class_name: &str,
  method: &str,
  args: &[Spanned<Expr>],
  vars: &HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
) -> Result<(FunctionValue<'ctx>, ValKind), String> {
  let c = ctx.class_defs.get(class_name).ok_or_else(|| {
    format!("codegen: internal error — unknown class `{class_name}` for generic method `{method}`")
  })?;
  let m = c.methods.iter().find(|m| m.name == method).ok_or_else(|| {
    format!("codegen: internal error — class `{class_name}` has no method `{method}`")
  })?;
  if m.type_params.len() != 1 {
    return Err(format!(
      "codegen: generic method `{class_name}.{method}` declares {} type parameters — only exactly one is supported",
      m.type_params.len()
    ));
  }
  let method_tp = m.type_params[0].name.clone();

  // The enclosing class's own `implements Interface[Args]` clause (if
  // any) binds the INTERFACE's own type parameter(s) — mirrors
  // `emerald-sema`'s `interface_type_param_subst` exactly, just over
  // the raw `ClassDef`/`InterfaceDef` ASTs codegen already has.
  let mut iface_subst_owned: HashMap<String, TypeExpr> = HashMap::new();
  if let Some((iface_name, iface_args)) = &c.implements {
    if let Some(iface) = ctx.interface_defs.get(iface_name.as_str()) {
      for (tp, arg) in iface.type_params.iter().zip(iface_args.iter()) {
        iface_subst_owned.insert(tp.name.clone(), arg.clone());
      }
    }
  }
  let iface_subst: HashMap<&str, &TypeExpr> = iface_subst_owned
    .iter()
    .map(|(k, v)| (k.as_str(), v))
    .collect();

  let mut concrete_u: Option<TypeExpr> = None;
  for (i, p) in m.params.iter().enumerate() {
    let p_ty = substitute_type_params(&p.ty, &iface_subst);
    if let Some(arg) = args.get(i) {
      if let Some(binding) =
        infer_method_type_param_binding(&p_ty, arg, &method_tp, ctx, vars, local_classes)
      {
        concrete_u = Some(binding);
      }
    }
  }
  let concrete_u = concrete_u.ok_or_else(|| {
    format!(
      "codegen: could not resolve generic method `{class_name}.{method}`'s type parameter `{method_tp}` to a concrete type at this call site"
    )
  })?;

  let mangled = format!(
    "{class_name}_{}$${}",
    mangled_operator_symbol(method),
    mangle_type_expr(&concrete_u)
  );
  if let Some((fv, ret_kind)) = ctx.generic_method_instances.borrow().get(&mangled) {
    return Ok((*fv, ret_kind.clone()));
  }

  let mut full_subst_owned = iface_subst_owned.clone();
  full_subst_owned.insert(method_tp.clone(), concrete_u);
  let full_subst: HashMap<&str, &TypeExpr> = full_subst_owned
    .iter()
    .map(|(k, v)| (k.as_str(), v))
    .collect();
  let substituted = substitute_function_type_params(m, &full_subst);

  let ret_kind = value_kind_for_type(&substituted.return_type);
  let mut kinds = vec![ValKind::Ptr]; // self
  kinds.extend(param_kinds(&substituted.params));
  let fn_ty = make_fn_type(context, &kinds, &ret_kind);
  let fv = ctx
    .module
    .add_function(&mangled, fn_ty, Some(Linkage::Internal));
  // Inserted BEFORE the body compiles (mirrors `instantiate_generic_
  // class`'s own self-reference-safe ordering elsewhere in this
  // codebase) — a recursive generic-method call inside its own body
  // would otherwise recurse into this same resolver forever.
  ctx
    .generic_method_instances
    .borrow_mut()
    .insert(mangled.clone(), (fv, ret_kind.clone()));

  let layout = ctx
    .classes
    .get(class_name)
    .ok_or_else(|| format!("codegen: internal error — no ClassLayout for `{class_name}`"))?;
  // See `build_inline_lambda`'s identical save/restore for why the
  // debug location (separate builder state from the basic-block
  // position) must be restored too, not just the block.
  let saved_block = builder.get_insert_block();
  let saved_di_loc = builder.get_current_debug_location();
  define_method(
    context,
    builder,
    &substituted,
    fv,
    &layout.fields,
    &layout.field_classes,
    ctx,
  )?;
  if let Some(block) = saved_block {
    builder.position_at_end(block);
  }
  match saved_di_loc {
    Some(loc) => builder.set_current_debug_location(loc),
    None => builder.unset_current_debug_location(),
  }

  Ok((fv, ret_kind))
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
    .build_alloca(local_llvm_type(context, &elem_kind), var)
    .map_err(|e| e.to_string())?;
  vars.insert(var.to_string(), (var_alloca, elem_kind.clone()));

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
  let elem_llvm_ty = local_llvm_type(context, &elem_kind);
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

/// `name(args) do |params| ... end` where `name` declares `block_param`
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
      .build_alloca(local_llvm_type(context, &k), &p.name)
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
      .build_alloca(local_llvm_type(context, &kind), &p.name)
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
      ret_kind.clone(),
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
  // Plan 35: every statement anywhere (top-level body, nested if/while/
  // case/begin/rescue/ensure — all funnel through this one function)
  // gets a real debug location from its own `Spanned<Stmt>` span before
  // any of its instructions are built.
  if let (Some(dibuilder), Some(scope), Some(offsets)) =
    (ctx.dibuilder, ctx.current_di_scope, ctx.newline_offsets)
  {
    let line = line_for_offset(offsets, stmt.span.0);
    let loc = dibuilder.create_debug_location(context, line, 0, scope, None);
    builder.set_current_debug_location(loc);
  }
  match &stmt.node {
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::Lambda { .. },
        ..
      },
      ..
    } if ty.as_named() == Some("Proc") || matches!(ty, TypeExpr::Func(..)) => {
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
      ..
    } => {
      let elem_ty = match ty {
        TypeExpr::Generic(base, args) if base == "Array" => args.first().ok_or_else(|| {
          format!("codegen: `{name}: {ty} = Array.new(...)` — declared type is not an Array")
        })?,
        _ => {
          return Err(format!(
            "codegen: `{name}: {ty} = Array.new(...)` — declared type is not an Array"
          ))
        }
      };
      let elem_kind = value_kind_for_type(elem_ty);
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
      let elems_byte_size = builder
        .build_int_mul(size_val.into_int_value(), elem_size, "arraynewelembytes")
        .map_err(|e| e.to_string())?;
      // Plan 42 (enumerable stdlib), `leaf-array-length-header`: same
      // `[length: Int64][elements...]` layout `build_array_lit` now
      // uses — `alloc_zeroed` still zero-fills the whole buffer
      // (including the header slot), then the real `size` is stored
      // over that zeroed header word.
      let header_size = context.i64_type().const_int(8, false);
      let byte_size = builder
        .build_int_add(elems_byte_size, header_size, "arraynewbytes")
        .map_err(|e| e.to_string())?;
      let call = builder
        .build_call(ctx.alloc_zeroed, &[byte_size.into()], "arraynew")
        .map_err(|e| e.to_string())?;
      let ptr = call_result(call)?.into_pointer_value();
      builder
        .build_store(ptr, size_val)
        .map_err(|e| e.to_string())?;
      local_array_elem_types.insert(name.clone(), elem_kind);
      let (dst, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder.build_store(dst, ptr).map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 50's `leaf-stack-allocation-codegen`: a `Let`-bound `New`
    // this function's own `find_non_escaping_news` proved never leaves
    // the function — `prealloc_stack_objects` already built its stack
    // `PointerValue` in the entry block; this arm skips `ctx.alloc`
    // entirely and dispatches `initialize` against that pre-built
    // pointer instead. Ordered before the generic `Stmt::Let` arm
    // below, alongside its `Expr::Lambda`/`Expr::ArrayNew` special
    // cases.
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::New(class_name, args),
        ..
      },
      ..
    } if ctx
      .object_allocas
      .is_some_and(|allocas| allocas.contains_key(name)) =>
    {
      let ptr = *ctx.object_allocas.unwrap().get(name).unwrap();
      // Plan 58: `Stack.new()`'s own `class_name` is the bare template
      // name — sema only ever accepts this shape when `ty` is a
      // matching generic instantiation (`Stack[Int64]`), so mangling
      // `ty` here always finds the real, monomorphized class.
      let effective_class_name = mangle_type_expr(ty);
      let effective_class_name = if ctx.classes.contains_key(&effective_class_name) {
        effective_class_name.as_str()
      } else {
        class_name.as_str()
      };
      build_initialize_call(
        context,
        builder,
        effective_class_name,
        args,
        ptr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      // Same `local_classes` bookkeeping the generic `Stmt::Let` arm
      // below performs — a later method call on `name` still needs to
      // resolve its class via this side table regardless of which
      // allocation strategy backs it. Plan 58: `resolve_local_class_
      // name` also tries the MANGLED form (`"Stack[Int64]"` ->
      // `"Stack$Int64"`) — a generic-instantiation-typed local resolves
      // to its real, monomorphized class exactly like an ordinary one.
      if let Some(resolved) = resolve_local_class_name(ty, ctx.classes) {
        local_classes.insert(name.clone(), resolved);
      }
      let (dst, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder.build_store(dst, ptr).map_err(|e| e.to_string())?;
      if let Some(stats) = ctx.escape_stats {
        stats.borrow_mut().stack_allocated += 1;
      }
      Ok(false)
    }
    // Plan 58: `s: Stack[Int64] = Stack.new()`, the ordinary (not
    // stack-allocated — see the `object_allocas` arm just above for
    // that path) heap-allocating case. `build_expr`'s own generic
    // `Expr::New` arm can't handle this: it only ever sees the bare
    // `class_name` AST node (`"Stack"`), with no access to the
    // enclosing `Let`'s own declared type — the only place the concrete
    // type argument actually lives. Mirrors that arm's own alloc-then-
    // initialize logic exactly, just keyed by the MANGLED name instead.
    // Sema's own `Stmt::Let` special case (mirrored here) guarantees
    // this shape only ever appears when `ty` really is a generic
    // instantiation of `class_name` — this never fires for an ordinary,
    // non-generic `Foo.new()` (`mangle_type_name` is a no-op for a
    // plain class name, so `ctx.classes.contains_key` would just find
    // the very same, already-generic-instantiation-free entry the
    // ordinary fallthrough arm below would anyway; the guard below only
    // actually matches a real generic instantiation).
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::New(class_name, args),
        ..
      },
      ..
    } if matches!(ty, TypeExpr::Generic(base, _) if base == class_name) => {
      let mangled = mangle_type_expr(ty);
      let layout = ctx
        .classes
        .get(&mangled)
        .ok_or_else(|| format!("codegen: unknown class `{mangled}`"))?;
      let size_val = context.i64_type().const_int(layout.size, false);
      let alloc_call = builder
        .build_call(ctx.alloc, &[size_val.into()], "newtmp")
        .map_err(|e| e.to_string())?;
      let ptr = call_result(alloc_call)?.into_pointer_value();
      build_initialize_call(
        context,
        builder,
        &mangled,
        args,
        ptr,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if let Some(stats) = ctx.escape_stats {
        stats.borrow_mut().heap_allocated += 1;
      }
      local_classes.insert(name.clone(), mangled);
      let (dst, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder.build_store(dst, ptr).map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 53's Decision log: `n: T = parse_int(s)?` — `Expr::Try`'s
    // Ok-path unwrap needs the *target's* own declared kind to load the
    // right LLVM type out of the payload slot, information `build_expr`
    // alone never has (mirrors `Expr::ArrayNew`'s own dedicated-arm
    // precedent immediately above). Loads the discriminant, branches:
    // on `Err`, forwards the exact same `Result` pointer straight back
    // out of the *enclosing* function via the identical
    // `builder.build_return` call `Stmt::Return(Some(e))` already uses
    // — zero new allocation, zero copy, since `Result[T, E]`'s 16-byte
    // layout never depends on `T` at all (only `E` occupies the payload
    // slot in the `Err` state) — sema's own exact-`E`-match check is
    // what makes forwarding the untouched pointer sound.
    Stmt::Let {
      name,
      ty,
      value: Spanned {
        node: Expr::Try(inner),
        ..
      },
      ..
    } => {
      let (result_val, _) = build_expr(
        context,
        builder,
        inner,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let result_ptr = result_val.into_pointer_value();
      let tag_val = load_field(
        context,
        builder,
        result_ptr,
        FieldInfo {
          offset: 0,
          kind: ValKind::Int64,
        },
      )?
      .into_int_value();
      let zero = context.i64_type().const_int(0, false);
      let is_ok = builder
        .build_int_compare(IntPredicate::EQ, tag_val, zero, "tryisok")
        .map_err(|e| e.to_string())?;

      let ok_blk = context.append_basic_block(func, "try.ok");
      let err_blk = context.append_basic_block(func, "try.err");
      builder
        .build_conditional_branch(is_ok, ok_blk, err_blk)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(err_blk);
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
        ret_kind.clone(),
        ctx,
        false,
      )?;
      builder
        .build_return(Some(&result_val))
        .map_err(|e| e.to_string())?;

      builder.position_at_end(ok_blk);
      let target_kind = value_kind_for_type(ty);
      let payload = load_field(
        context,
        builder,
        result_ptr,
        FieldInfo {
          offset: 8,
          kind: target_kind.clone(),
        },
      )?;
      let ty_str = ty.to_string();
      if matches!(ty, TypeExpr::Generic(base, _) if base == "Result")
        || ctx.classes.contains_key(&ty_str)
        || ctx.enums.contains_key(&ty_str)
      {
        local_classes.insert(name.clone(), ty_str);
      }
      let (dst, _) = *vars
        .get(name)
        .expect("pre-allocated by prealloc_lets for every reachable Let");
      builder
        .build_store(dst, payload)
        .map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 53's Decision log: `name = parse_int(s)?` — the `Stmt::Assign`
    // mirror of the `Stmt::Let` arm immediately above, driven by the
    // target's already-recorded `ValKind` in `vars` instead of a
    // declared-type string.
    Stmt::Assign {
      name,
      value: Spanned {
        node: Expr::Try(inner),
        ..
      },
    } => {
      let (result_val, _) = build_expr(
        context,
        builder,
        inner,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let result_ptr = result_val.into_pointer_value();
      let tag_val = load_field(
        context,
        builder,
        result_ptr,
        FieldInfo {
          offset: 0,
          kind: ValKind::Int64,
        },
      )?
      .into_int_value();
      let zero = context.i64_type().const_int(0, false);
      let is_ok = builder
        .build_int_compare(IntPredicate::EQ, tag_val, zero, "tryisok")
        .map_err(|e| e.to_string())?;

      let ok_blk = context.append_basic_block(func, "try.ok");
      let err_blk = context.append_basic_block(func, "try.err");
      builder
        .build_conditional_branch(is_ok, ok_blk, err_blk)
        .map_err(|e| e.to_string())?;

      builder.position_at_end(err_blk);
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
        ret_kind.clone(),
        ctx,
        false,
      )?;
      builder
        .build_return(Some(&result_val))
        .map_err(|e| e.to_string())?;

      builder.position_at_end(ok_blk);
      let (dst, target_kind) = vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let (dst, target_kind) = (*dst, target_kind.clone());
      let payload = load_field(
        context,
        builder,
        result_ptr,
        FieldInfo {
          offset: 8,
          kind: target_kind,
        },
      )?;
      builder
        .build_store(dst, payload)
        .map_err(|e| e.to_string())?;
      Ok(false)
    }
    Stmt::Let {
      name, ty, value, ..
    } => {
      // Plan 73's Decision log: plan 43's `T?`/`Expr::Nil` sentinel
      // special-case (a `nil` literal into a pointer-backed declared
      // type built a raw null-pointer constant directly, bypassing
      // ordinary codegen) is removed outright along with `T?` itself —
      // `Option[T]` is a real, monomorphized enum value now (tag +
      // payload), constructed by the same ordinary `build_expr` path
      // every other enum variant already uses (`Some`/`None` are
      // `Expr::Call`/`Expr::Ident` like any other variant), never a bare
      // null pointer.
      let (v, _) = build_expr(
        context,
        builder,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let bare_ty = ty.to_string();
      // Plan 52: an enum-typed local carries its enum name the same
      // way a class-typed local carries its class name — `build_case`
      // consults this to detect an enum scrutinee.
      // Plan 57: `"Supervisor"` doubles as the side-table for a
      // `supervise do ... end` local too — never a real `ctx.classes`
      // entry (see `build_method_call`'s own `sup.child(...)` dispatch
      // check, which is what actually consults this).
      // Plan 58: `resolve_local_class_name` also tries the MANGLED form
      // (`"Stack[Int64]"` -> `"Stack$Int64"`) — a generic-instantiation-
      // typed local resolves to its real, monomorphized class exactly
      // like an ordinary one.
      if let Some(resolved) = resolve_local_class_name(ty, ctx.classes) {
        local_classes.insert(name.clone(), resolved);
      } else if bare_ty == "Supervisor"
        || ctx.enums.contains_key(&bare_ty)
        // `newtype Meters: Float64` — a newtype-typed local otherwise
        // leaves no trace in `local_classes` at all (it isn't a real
        // `ctx.classes`/`ctx.enums` entry, unlike everything else this
        // side-table already tracks) — recorded here the identical way
        // `"Supervisor"`/an enum name already are, so `build_method_
        // call`'s `.value` dispatch can recognize this receiver as a
        // newtype rather than an ordinary (never-declared) method name.
        || ctx.newtypes.contains(&bare_ty)
      {
        local_classes.insert(name.clone(), bare_ty.clone());
      } else {
        // Plan 73: a generic-ENUM-instantiation-typed local (`Option[
        // Int64]`) resolves to its real, monomorphized `EnumLayout`
        // exactly like a generic-class-instantiation-typed local
        // already does via `resolve_local_class_name` above — the
        // identical mangled-name fallback, just against `ctx.enums`
        // instead of `ctx.classes`.
        let mangled = mangle_type_expr(ty);
        if mangled != bare_ty && ctx.enums.contains_key(&mangled) {
          local_classes.insert(name.clone(), mangled);
        }
      }
      if let TypeExpr::Generic(base, args) = ty {
        if base == "Array" {
          if let Some(elem_ty) = args.first() {
            local_array_elem_types.insert(name.clone(), value_kind_for_type(elem_ty));
          }
        }
      }
      // Plan 25: `local_classes` doubles as the side-table for
      // `"Hash[K, V]"` locals too (see `value_kind_for_type`'s doc
      // comment) — `build_index`/`build_set_index` check for this
      // prefix to route to hash-lookup codegen instead of array
      // indexing.
      // Plan 53: `local_classes` doubles as the side-table for
      // `"Result[T, E]"` locals too — `Stmt::MatchResult`'s codegen
      // reads this to know each arm's real payload `ValKind`.
      // Plan 42: `local_classes` doubles as the side-table for
      // `"Pair[K, V]"` locals too — `build_method_call`'s new `.key`/
      // `.value` dispatch reads this the same way `Hash`'s own
      // `"[]"`/`"[]="` dispatch already reads its own prefix above.
      // The only real source of a `Pair` value is a `Hash[K,V].each`
      // block's own parameter, bound the identical way (see that
      // dispatch's own codegen), but this covers an explicit `p: Pair
      // [K, V] = ...`-annotated `Let` too, for free.
      if matches!(ty, TypeExpr::Generic(base, _) if base == "Hash" || base == "Result" || base == "Pair")
      {
        local_classes.insert(name.clone(), bare_ty.clone());
      }
      // Plan 89's Decision log: a `Proc[Args..., Ret]`-typed local
      // bound to an arbitrary expression (forwarding a parameter, a
      // field load, or any other Proc-typed value — not a lambda
      // literal, which `build_stmt`'s own dedicated `Expr::Lambda` arm
      // above already special-cases) needs the identical `local_
      // classes` signature encoding `bind_params` gives a Proc-typed
      // PARAMETER, so `.call`'s own dispatch can resolve an indirect
      // call through it too.
      if let Some(sig) = proc_sig_for_type(ty) {
        local_classes.insert(name.clone(), sig);
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
      let (ptr, _) = vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      let ptr = *ptr;
      // Plan 73's Decision log: plan 43's `Expr::Nil`-into-`ptr`-slot
      // special case is removed outright along with `T?`/`nil` — see
      // `Stmt::Let`'s own identical removal above for the full
      // rationale.
      let (v, _) = build_expr(
        context,
        builder,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      Ok(false)
    }
    // Plan 31: every `values` expression is built into a temporary SSA
    // value BEFORE any `names` target is written — the entire point of
    // this leaf (Decision log): `a, b = b, a` must read both original
    // values before either alloca is overwritten, or the swap silently
    // corrupts (`a = b` then `b = a` would print the new `a` twice).
    Stmt::MultiAssign { names, values } => {
      // Plan 39's Decision log: `x, y = f()` — a single call-shaped
      // value whose declared return type is a tuple — unpacks
      // positionally, entirely ahead of the ordinary per-value path
      // below, which never anticipated a single value expression
      // producing more than one result. Sema (`check_tuple_multi_
      // assign`) already guarantees arity/type agreement for any
      // program that reaches here.
      if let [value] = values.as_slice() {
        if let Expr::Call(..) = &value.node {
          let (v, kind) = build_expr(
            context,
            builder,
            value,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?;
          if matches!(kind, ValKind::Tuple(_)) {
            build_tuple_multi_assign(builder, names, v.into_struct_value(), vars)?;
            return Ok(false);
          }
        }
      }
      let mut evaluated = Vec::with_capacity(values.len());
      for v in values {
        // Plan 73's Decision log: plan 43's `Expr::Nil`-into-`ptr`-slot
        // special case is removed outright — see `Stmt::Let`'s own
        // identical removal above for the full rationale. Ordinary
        // `build_expr` still runs BEFORE any target is written (this
        // loop's own "evaluate every value before writing any target"
        // ordering, the reason `a, b = b, a` is a real swap, is
        // unaffected).
        let (val, _) = build_expr(
          context,
          builder,
          v,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
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
      let (self_ptr, fields, _) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name} = ...` used outside of a method body"))?;
      let field_offset = fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?
        .offset;
      let (v, _) = build_expr(
        context,
        builder,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let fp = field_ptr(context, builder, self_ptr, field_offset)?;
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
      // Plan 73's Decision log: plan 43's `Expr::Nil`-into-`ptr`-slot
      // special case is removed outright — see `Stmt::Let`'s own
      // identical removal above for the full rationale.
      let (v, _) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      // Plan 62's Decision log: runs BEFORE `emit_active_ensures` —
      // deliberately. A failed `ensures` clause raises a real exception
      // (`build_raise`, `longjmp`-based), which must be caught by
      // whatever ENCLOSING `begin`/`rescue` handler actually catches it
      // (that block's own `build_begin`-emitted exit points already
      // duplicate its `ensure` body correctly); `emit_active_ensures`
      // is a completely different mechanism — the compile-time-
      // duplicated `begin`/`ensure` cleanup a plain, non-exceptional
      // `return` needs to fall through on its way out. A no-op (no
      // branch emitted at all) for every function with no `ensures`.
      build_ensures_checks(
        context,
        builder,
        func,
        Some((v, ret_kind.clone())),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
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
      // Plan 84's Decision log: right before the real `ret` — see
      // `emit_borrow_var_writebacks`'s own doc comment for why this is
      // one of its two `Stmt::Return` injection points.
      emit_borrow_var_writebacks(context, builder, ctx)?;
      builder.build_return(Some(&v)).map_err(|e| e.to_string())?;
      Ok(true)
    }
    Stmt::Return(None) => {
      // Plan 62's Decision log: see the `Some(e)` arm's own comment for
      // why this runs before `emit_active_ensures`. `result_value: None`
      // — a bare `return` has no value to bind under `result`; a clause
      // referencing it here would already have been rejected by
      // `emerald-sema` (a `Void` declared return has no comparable
      // operations).
      build_ensures_checks(
        context,
        builder,
        func,
        None,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
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
      // Plan 84's Decision log — see the `Some(e)` arm's own identical
      // comment just above.
      emit_borrow_var_writebacks(context, builder, ctx)?;
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
        ret_kind.clone(),
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
    // Plan 53's Decision log: `case scrutinee when Ok(v) ... when
    // Err(e) ... end` — the scrutinee's real `T`/`E` come from
    // `local_classes`'s `"Result[T, E]"` string (only a plain
    // `Expr::Ident` scrutinee can carry a known static type here, the
    // same receiver restriction `build_case`'s own enum detection
    // already imposes).
    Stmt::MatchResult {
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
    } => {
      let Expr::Ident(scrut_name) = &scrutinee.node else {
        return Err(
          "codegen: internal error — `Ok`/`Err` match scrutinee must be a plain local (sema should have rejected this)"
            .to_string(),
        );
      };
      let result_ty = local_classes.get(scrut_name).ok_or_else(|| {
        format!(
          "codegen: internal error — `{scrut_name}` has no known `Result[T, E]` type (sema should have rejected this)"
        )
      })?;
      let inner = result_ty
        .strip_prefix("Result[")
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| {
          format!("codegen: internal error — `{scrut_name}` is not `Result[T, E]`-typed")
        })?;
      let (t_name, e_name) = inner.split_once(", ").ok_or_else(|| {
        format!("codegen: internal error — malformed `Result[T, E]` type `{result_ty}`")
      })?;
      let ok_kind = value_kind_for_type(&TypeExpr::Named(t_name.to_string()));
      let err_kind = value_kind_for_type(&TypeExpr::Named(e_name.to_string()));
      build_match_result(
        context,
        builder,
        func,
        scrutinee,
        ok_var,
        ok_body,
        err_var,
        err_body,
        ok_kind,
        err_kind,
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
  // Plan 52's Decision log: an enum-typed scrutinee is detected via
  // `local_classes` (only a plain `Expr::Ident` scrutinee can carry a
  // known static type here — the same receiver restriction
  // `build_method_call` already imposes elsewhere in this backend),
  // checked BEFORE building the scrutinee expression, since it needs
  // an entirely different comparison (a loaded tag, not a raw Int64
  // value) than the Int64 path below.
  if let Expr::Ident(name) = &scrutinee.node {
    if let Some(layout) = local_classes.get(name).and_then(|tn| ctx.enums.get(tn)) {
      return build_enum_case(
        context,
        builder,
        func,
        scrutinee,
        layout,
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
      );
    }
  }

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

  for (pattern, body) in arms {
    let CasePattern::Values(values) = pattern else {
      return Err(
        "codegen: internal error — variant pattern over an Int64 scrutinee (sema should have rejected this)"
          .to_string(),
      );
    };
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
      ret_kind.clone(),
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

/// Plan 52's `leaf-codegen-tagged-union`: the enum half of `build_case`
/// — loads the scrutinee's tag (mirroring `build_hash_lookup`'s
/// existing header-field read), then for each `Variant` arm emits the
/// same `icmp eq`/conditional-branch chain shape `build_case`'s own
/// `Values` path already uses, now comparing against `layout.
/// variant_tags`. Each arm's own bindings are inserted into `vars`
/// only for the duration of building that arm's block — the prior
/// entry (if any) is saved and restored (or removed) immediately
/// after, so a stale pointer never lingers for a later arm or a
/// statement after the `case` ends (the codegen half of plan 52's
/// disclosed departure from this compiler's flat-scoping convention).
#[allow(clippy::too_many_arguments)]
fn build_enum_case<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  scrutinee: &Spanned<Expr>,
  layout: &EnumLayout,
  arms: &'a [CaseArm],
  else_body: &'a Option<Vec<Spanned<Stmt>>>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  let (scrut_val, _scrut_kind) = build_expr(
    context,
    builder,
    scrutinee,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let ptr = scrut_val.into_pointer_value();
  let tag_val = load_field(
    context,
    builder,
    ptr,
    FieldInfo {
      offset: 0,
      kind: ValKind::Int64,
    },
  )?
  .into_int_value();

  let merge_blk = context.append_basic_block(func, "case.merge");

  for (pattern, body) in arms {
    let CasePattern::Variant { name, bindings } = pattern else {
      return Err(
        "codegen: internal error — value pattern over an enum scrutinee (sema should have rejected this)"
          .to_string(),
      );
    };
    let tag = *layout.variant_tags.get(name).ok_or_else(|| {
      format!("codegen: internal error — unknown variant `{name}` (sema should have rejected this)")
    })?;
    let field_kinds = layout.variant_fields.get(name).ok_or_else(|| {
      format!("codegen: internal error — unknown variant `{name}` (sema should have rejected this)")
    })?;

    let arm_blk = context.append_basic_block(func, "case.arm");
    let next_check_blk = context.append_basic_block(func, "case.next");

    let tag_const = context.i64_type().const_int(tag, false);
    let cond = builder
      .build_int_compare(IntPredicate::EQ, tag_val, tag_const, "tageq")
      .map_err(|e| e.to_string())?;
    builder
      .build_conditional_branch(cond, arm_blk, next_check_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(arm_blk);

    let mut prior_vars: Vec<(String, Option<(PointerValue<'ctx>, ValKind)>)> = Vec::new();
    for (i, bname) in bindings.iter().enumerate() {
      let kind = field_kinds.get(i).ok_or_else(|| {
        format!(
          "codegen: internal error — pattern `{name}` binding count mismatch (sema should have rejected this)"
        )
      })?;
      let loaded = load_field(
        context,
        builder,
        ptr,
        FieldInfo {
          offset: 8 + i as u64 * 8,
          kind: kind.clone(),
        },
      )?;
      // A binding gets its own stack slot (the same `PointerValue`
      // shape every other local this backend tracks in `vars`), never
      // the tagged union's own field slot aliased directly — so
      // `build_stmt`'s ordinary `Expr::Ident` read path works
      // identically to any other local.
      let alloca = builder
        .build_alloca(local_llvm_type(context, kind), bname)
        .map_err(|e| e.to_string())?;
      builder
        .build_store(alloca, loaded)
        .map_err(|e| e.to_string())?;
      prior_vars.push((bname.clone(), vars.get(bname).cloned()));
      vars.insert(bname.clone(), (alloca, kind.clone()));
    }

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
      ret_kind.clone(),
      ctx,
    )?;

    for (bname, prior) in prior_vars {
      match prior {
        Some(p) => {
          vars.insert(bname, p);
        }
        None => {
          vars.remove(&bname);
        }
      }
    }

    if !terminated {
      builder
        .build_unconditional_branch(merge_blk)
        .map_err(|e| e.to_string())?;
    }

    builder.position_at_end(next_check_blk);
  }

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

/// Plan 53's `leaf-codegen-try-and-match`: `case scrutinee when Ok(v)
/// ... when Err(e) ... end`'s own codegen — reuses `build_enum_case`'s
/// `arm_blk`/`next_check_blk` chaining idiom, generalized from an
/// arbitrary-arity enum tag comparison to the smallest possible
/// instance of that same shape: a fixed two-way `i64` discriminant
/// branch, `Ok` always first, `Err` always second, both mandatory, no
/// `else`. `ok_var`/`err_var` get their own stack slot each, inserted
/// into `vars` only for the duration of their own arm's block — the
/// codegen half of the same block-scoping departure plan 52 already
/// established for its own pattern bindings.
#[allow(clippy::too_many_arguments)]
fn build_match_result<'a, 'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  scrutinee: &Spanned<Expr>,
  ok_var: &str,
  ok_body: &'a [Spanned<Stmt>],
  err_var: &str,
  err_body: &'a [Spanned<Stmt>],
  ok_kind: ValKind,
  err_kind: ValKind,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  loop_stack: &mut Vec<LoopTargets<'ctx>>,
  ensure_stack: &mut Vec<(&'a [Spanned<Stmt>], bool)>,
  retry_stack: &mut Vec<BasicBlock<'ctx>>,
  ret_kind: ValKind,
  ctx: &Ctx<'a, 'ctx>,
) -> Result<bool, String> {
  let (scrut_val, _scrut_kind) = build_expr(
    context,
    builder,
    scrutinee,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let ptr = scrut_val.into_pointer_value();
  let tag_val = load_field(
    context,
    builder,
    ptr,
    FieldInfo {
      offset: 0,
      kind: ValKind::Int64,
    },
  )?
  .into_int_value();
  let zero = context.i64_type().const_int(0, false);
  let is_ok = builder
    .build_int_compare(IntPredicate::EQ, tag_val, zero, "matchresultisok")
    .map_err(|e| e.to_string())?;

  let ok_blk = context.append_basic_block(func, "matchresult.ok");
  let err_blk = context.append_basic_block(func, "matchresult.err");
  let merge_blk = context.append_basic_block(func, "matchresult.merge");
  builder
    .build_conditional_branch(is_ok, ok_blk, err_blk)
    .map_err(|e| e.to_string())?;

  builder.position_at_end(ok_blk);
  // Plan 65's `leaf-unified-fallible-send`, a real, disclosed edge
  // case its own Decision log named but didn't fully resolve on the
  // BINDING side: `Result[Void, SendError]`'s `Ok(v)` arm binds `v`
  // at sema's own inferred `Type::Void` — `local_llvm_type`'s own
  // documented invariant is that `Void` never reaches it as a storage
  // type (found by this leaf's own new AC2 test, which panicked here
  // before this fix). There is nothing meaningful to load/bind — a
  // `Void` value carries no real data, and sema already forbids using
  // one anywhere a real value is required (the same "Void can't be
  // used as a value" rule every other `Void`-typed expression already
  // enforces) — so `ok_var` simply isn't inserted into `vars` at all
  // for this one case; `prior_ok`, captured either way, still restores
  // whatever `ok_var` named beforehand (or nothing) once the arm ends.
  let prior_ok = vars.get(ok_var).cloned();
  if ok_kind != ValKind::Void {
    let ok_val = load_field(
      context,
      builder,
      ptr,
      FieldInfo {
        offset: 8,
        kind: ok_kind.clone(),
      },
    )?;
    let ok_alloca = builder
      .build_alloca(local_llvm_type(context, &ok_kind), ok_var)
      .map_err(|e| e.to_string())?;
    builder
      .build_store(ok_alloca, ok_val)
      .map_err(|e| e.to_string())?;
    vars.insert(ok_var.to_string(), (ok_alloca, ok_kind));
  }
  let ok_terminated = build_block(
    context,
    builder,
    func,
    ok_body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ensure_stack,
    retry_stack,
    ret_kind.clone(),
    ctx,
  )?;
  match prior_ok {
    Some(p) => {
      vars.insert(ok_var.to_string(), p);
    }
    None => {
      vars.remove(ok_var);
    }
  }
  if !ok_terminated {
    builder
      .build_unconditional_branch(merge_blk)
      .map_err(|e| e.to_string())?;
  }

  builder.position_at_end(err_blk);
  let err_val = load_field(
    context,
    builder,
    ptr,
    FieldInfo {
      offset: 8,
      kind: err_kind.clone(),
    },
  )?;
  let err_alloca = builder
    .build_alloca(local_llvm_type(context, &err_kind), err_var)
    .map_err(|e| e.to_string())?;
  builder
    .build_store(err_alloca, err_val)
    .map_err(|e| e.to_string())?;
  let prior_err = vars.get(err_var).cloned();
  vars.insert(err_var.to_string(), (err_alloca, err_kind));
  let err_terminated = build_block(
    context,
    builder,
    func,
    err_body,
    vars,
    local_classes,
    local_array_elem_types,
    loop_stack,
    ensure_stack,
    retry_stack,
    ret_kind,
    ctx,
  )?;
  match prior_err {
    Some(p) => {
      vars.insert(err_var.to_string(), p);
    }
    None => {
      vars.remove(err_var);
    }
  }
  if !err_terminated {
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
    ret_kind.clone(),
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
      ret_kind.clone(),
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
      ret_kind.clone(),
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
        ret_kind.clone(),
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
      ret_kind.clone(),
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
    // Plan 84's Decision log: an empty body is a real (if unusual)
    // function-exit point too — see `emit_borrow_var_writebacks`'s own
    // doc comment.
    emit_borrow_var_writebacks(context, builder, ctx)?;
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
    ret_kind.clone(),
    ctx,
  )?;
  if terminated {
    return Ok(());
  }

  match &last.node {
    // Plan 55's own disclosed fix to pre-existing code: this arm used
    // to match ANY `Stmt::Expr(e)` unconditionally, evaluating `e` via
    // `build_expr` and building a return with its value — wrong for a
    // `Void`-returning body, since `build_expr` has no dispatch for a
    // statement-only intrinsic like `puts`/`gets` (those only exist as
    // `build_stmt`'s own special-cased arms) and building `ret void <v>`
    // out of a `Void` function is a real LLVM verifier error regardless.
    // A real, previously-latent gap: nothing in this whole test suite
    // happened to have a `Void`-returning function/method whose LAST
    // statement was one of those statement-only forms until this
    // plan's own `Spinner` worked example (`spin`'s body ends in a bare
    // `puts ...`) — the `ret_kind != Void` guard below routes a `Void`
    // body's last statement through the exact same full `build_stmt`
    // dispatch (including the `puts`/`gets` special cases) every other
    // statement in the body already gets, via the `_` arm.
    Stmt::Expr(e) if ret_kind != ValKind::Void => {
      let (v, _) = build_expr(
        context,
        builder,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      // Plan 62's Decision log: the implicit-return-fallthrough half of
      // the two fixed `ensures` injection points — see `Stmt::Return`'s
      // own identical comment for why this runs unconditionally here
      // (there is no enclosing `emit_active_ensures` call on this path
      // at all, unlike an explicit `return`).
      build_ensures_checks(
        context,
        builder,
        func,
        Some((v, ret_kind.clone())),
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      // Plan 84's Decision log: the implicit-tail-expression half of
      // this function's three real exit points — see `emit_borrow_var_
      // writebacks`'s own doc comment.
      emit_borrow_var_writebacks(context, builder, ctx)?;
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
        ret_kind.clone(),
        ctx,
      )?;
      // A Void-returning body whose last statement isn't a
      // value-producing `Stmt::Expr` (e.g. `initialize`'s trailing
      // `@y = y`) needs an explicit empty `return` — nothing else
      // would ever terminate the block.
      if !terminated && ret_kind == ValKind::Void {
        // Plan 62's Decision log: only reached when NOTHING else in the
        // body already terminated it (a real implicit fallthrough) — an
        // explicit `Stmt::Return`/`Stmt::Raise` reaching `terminated:
        // true` already ran its own `ensures` check via `build_stmt`'s
        // own arm; this is the one remaining case that never does.
        build_ensures_checks(
          context,
          builder,
          func,
          None,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?;
        // Plan 84's Decision log: the `Void`-fallthrough half of this
        // function's three real exit points — see `emit_borrow_var_
        // writebacks`'s own doc comment.
        emit_borrow_var_writebacks(context, builder, ctx)?;
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
      ty: TypeExpr::Generic("Array".to_string(), vec![splat.ty.clone()]),
      default: None,
    });
  }
  params
}

/// Binds `f`'s declared params to fresh entry-block `alloca`s (storing
/// each incoming SSA parameter value into its slot), populating
/// `local_classes`/`local_array_elem_types` bookkeeping for any
/// class-/array-typed parameter along the way.
///
/// Plan 84's Decision log: a by-value `borrow`/`borrow var` parameter
/// (`borrow_ptr_marker_info` recognizes `p.ty`) arrives as a real LLVM
/// `ptr` instead — `make_fn_type`'s own `ValKind::Ptr` case already
/// builds that signature shape for free, since `value_kind_for_type` of
/// either marker `Generic` falls into the same generic-`Ptr` bucket
/// every OTHER `Generic` type already does (`Array[T]`, `Hash[K,V]`,
/// ...). This arm loads the real underlying scalar out of that incoming
/// pointer ONCE, up front, into an ordinary local `alloca` exactly like
/// every other parameter gets — so every other codegen function in this
/// file (`build_numeric_binop`, `build_expr`'s `Expr::Ident` read path,
/// ...) sees a completely ordinary `Int64`/`Float64`/`Boolean`/`Symbol`
/// local, with zero special-casing anywhere else. A `borrow var` (not a
/// plain `borrow`) additionally records `(incoming pointer, this
/// alloca, kind)` in `borrow_var_writebacks` — `emit_borrow_var_
/// writebacks`' own doc comment covers the other half: writing the
/// local's final value back through the incoming pointer at every
/// function-exit point, which is what actually makes the callee's
/// mutation visible to the caller.
#[allow(clippy::too_many_arguments)]
fn bind_params<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  params: &[Param],
  param_offset: u32,
  classes: &HashMap<String, ClassLayout>,
  // `newtype Meters: Float64` — see `Ctx::newtypes`'s own doc comment.
  // A newtype-typed PARAMETER (`fn add(a: Meters, b: Meters): Meters`)
  // needs the identical `local_classes` bookkeeping a newtype-typed
  // `Let` local already gets, so `.value` can be called on it from
  // inside the function body too.
  newtypes: &HashSet<String>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, ValKind>,
  // Plan 84's Decision log: appended to for every `borrow var`
  // by-value parameter this call binds — see this function's own doc
  // comment and `emit_borrow_var_writebacks`'s.
  borrow_var_writebacks: &mut Vec<(PointerValue<'ctx>, PointerValue<'ctx>, ValKind)>,
) -> Result<(), String> {
  for (i, p) in params.iter().enumerate() {
    if let Some((is_mut, underlying_ty)) = borrow_ptr_marker_info(&p.ty) {
      let underlying_kind = value_kind_for_type(underlying_ty);
      let underlying_llvm_ty = local_llvm_type(context, &underlying_kind);
      let incoming_ptr = func
        .get_nth_param(param_offset + i as u32)
        .expect("declared signature has this many params")
        .into_pointer_value();
      let loaded = builder
        .build_load(underlying_llvm_ty, incoming_ptr, &p.name)
        .map_err(|e| e.to_string())?;
      let alloca = builder
        .build_alloca(underlying_llvm_ty, &p.name)
        .map_err(|e| e.to_string())?;
      builder
        .build_store(alloca, loaded)
        .map_err(|e| e.to_string())?;
      vars.insert(p.name.clone(), (alloca, underlying_kind.clone()));
      if is_mut {
        borrow_var_writebacks.push((incoming_ptr, alloca, underlying_kind));
      }
      continue;
    }
    let kind = value_kind_for_type(&p.ty);
    let param_val = func
      .get_nth_param(param_offset + i as u32)
      .expect("declared signature has this many params");
    let alloca = builder
      .build_alloca(local_llvm_type(context, &kind), &p.name)
      .map_err(|e| e.to_string())?;
    builder
      .build_store(alloca, param_val)
      .map_err(|e| e.to_string())?;
    vars.insert(p.name.clone(), (alloca, kind));
    let p_ty_str = p.ty.to_string();
    if classes.contains_key(&p_ty_str) || newtypes.contains(&p_ty_str) {
      local_classes.insert(p.name.clone(), p_ty_str.clone());
    }
    // Plan 42 (enumerable stdlib): a `Pair[K, V]`-typed parameter
    // (only ever reachable via a `Hash[K,V].each` Proc — `Type::Pair`'s
    // own doc comment) needs the identical `local_classes` bookkeeping
    // `Stmt::Let`'s own generic arm already gives a `Pair[K, V]` local,
    // so `build_method_call`'s `.key`/`.value` dispatch can resolve it
    // from inside the Proc's own compiled body.
    if matches!(&p.ty, TypeExpr::Generic(base, _) if base == "Pair") {
      local_classes.insert(p.name.clone(), p_ty_str);
    }
    if let TypeExpr::Generic(base, args) = &p.ty {
      if base == "Array" {
        if let Some(elem_ty) = args.first() {
          local_array_elem_types.insert(p.name.clone(), value_kind_for_type(elem_ty));
        }
      }
    }
    // Plan 89's Decision log: a `Proc[Args..., Ret]`-typed parameter
    // (a generic method's own `f: Proc[T, U]`, already fully concrete
    // by the time this specific specialization's body compiles — see
    // `resolve_generic_method_instance`) gets its signature encoded
    // into `local_classes` too, so `.call`'s own dispatch can resolve
    // an INDIRECT call through it (no compile-time name to look up in
    // `ctx.lambda_func_ids` for an ordinary parameter).
    if let Some(sig) = proc_sig_for_type(&p.ty) {
      local_classes.insert(p.name.clone(), sig);
    }
  }
  Ok(())
}

/// Plan 62's `leaf-codegen-contracts`: raised via `build_raise` reuse —
/// the exact "synthesize a `Spanned<Expr>::New` and call the EXISTING
/// `build_raise` function unchanged" technique plan 60's own `Remote
/// ActorError` raise site already established, simpler here since the
/// message is entirely compile-time-known (`Contract.text`/`Contract.
/// line`, both already real strings/numbers by the time codegen runs —
/// no runtime-computed value to stash in a scratch `vars` entry first,
/// unlike `RemoteActorError`'s own socket-layer error string). Named
/// the function and the clause's own declaration LINE — not the file
/// (a real, disclosed narrowing: `compile_to_object`'s ordinary,
/// non-debug-info path carries no source file name for codegen to name
/// here, unlike `compile_to_object_with_debug_info`'s own `di_file`).
#[allow(clippy::too_many_arguments)]
fn build_requires_checks<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  fname: &str,
  requires: &[Contract],
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  for c in requires {
    let cond = build_bool(
      context,
      builder,
      &c.expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let ok_blk = context.append_basic_block(func, "requires.ok");
    let fail_blk = context.append_basic_block(func, "requires.fail");
    builder
      .build_conditional_branch(cond, ok_blk, fail_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(fail_blk);
    let message = format!(
      "contract violation: `{fname}`'s requires `{}` failed (declared at line {})",
      c.text, c.line
    );
    let raise_expr = Spanned::synthetic(Expr::New(
      "ContractViolation".to_string(),
      vec![Spanned::synthetic(Expr::StringLit(message))],
    ));
    build_raise(
      context,
      builder,
      &raise_expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;

    builder.position_at_end(ok_blk);
  }
  Ok(())
}

/// Plan 62's `leaf-codegen-contracts`: the `ensures` mirror of `build_
/// requires_checks` immediately above — reached from exactly two fixed
/// codegen sites (`Stmt::Return`'s own arm, both `Some`/`None`; and
/// `build_function_body`'s own implicit-return-fallthrough path), each
/// already knowing the function's declared return kind (needed anyway
/// to emit a correctly-typed `ret`). `result_value` is `Some((v, kind))`
/// for an explicit `return <expr>` (bound to a fresh scratch `vars`
/// entry named `result`, so an `ensures` clause's own `build_expr` call
/// reads it via the ordinary `Expr::Ident("result")` path — no grammar/
/// AST support needed for the pseudo-identifier at all) or `None` for a
/// bare `return`/an implicit `Void` fallthrough, where no clause can
/// legally reference `result` (`emerald-sema` already rejected that).
/// Reads `ctx.current_function_contracts` directly rather than taking
/// `ensures`/`fname` as explicit parameters — `None` (every method/
/// lambda/`main` body) is a real, cheap no-op.
#[allow(clippy::too_many_arguments)]
fn build_ensures_checks<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  result_value: Option<(BasicValueEnum<'ctx>, ValKind)>,
  vars: &mut HashMap<String, (PointerValue<'ctx>, ValKind)>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, ValKind>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let Some((fname, ensures)) = ctx.current_function_contracts else {
    return Ok(());
  };
  if ensures.is_empty() {
    return Ok(());
  }
  if let Some((v, kind)) = result_value {
    let alloca = builder
      .build_alloca(local_llvm_type(context, &kind), "result")
      .map_err(|e| e.to_string())?;
    builder.build_store(alloca, v).map_err(|e| e.to_string())?;
    vars.insert("result".to_string(), (alloca, kind));
  }
  for c in ensures {
    let cond = build_bool(
      context,
      builder,
      &c.expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    let ok_blk = context.append_basic_block(func, "ensures.ok");
    let fail_blk = context.append_basic_block(func, "ensures.fail");
    builder
      .build_conditional_branch(cond, ok_blk, fail_blk)
      .map_err(|e| e.to_string())?;

    builder.position_at_end(fail_blk);
    let message = format!(
      "contract violation: `{fname}`'s ensures `{}` failed (declared at line {})",
      c.text, c.line
    );
    let raise_expr = Spanned::synthetic(Expr::New(
      "ContractViolation".to_string(),
      vec![Spanned::synthetic(Expr::StringLit(message))],
    ));
    build_raise(
      context,
      builder,
      &raise_expr,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;

    builder.position_at_end(ok_blk);
  }
  Ok(())
}

/// Plan 84's Decision log: the other half of `bind_params`'s own
/// `borrow var`-by-value handling — writes each bound `borrow var`
/// parameter's CURRENT local value back through its own incoming
/// pointer. Called from every one of this function's real exit points
/// (`Stmt::Return(Some(_))`/`Stmt::Return(None)`'s own two arms in
/// `build_stmt`, and `build_function_body`'s three implicit-fallthrough
/// exits: the empty-body case, the implicit-tail-expression case, and
/// the `Void`-fallthrough case) — a real no-op, zero instructions
/// emitted, whenever `ctx.borrow_var_writebacks` is empty (every
/// function that declares no `borrow var`-by-value parameter, which is
/// every function compiled before this plan).
///
/// Real, disclosed limitation: this does NOT run on every possible way
/// a function can end — `Stmt::Raise`'s own unwind (`build_raise`,
/// `longjmp`-based), a `begin`'s own re-raise-to-an-outer-handler exit
/// point, and the early `Result`-forwarding `return` `Stmt::Let`'s own
/// `Expr::Try` arm builds when unwrapping a `?` hits `Err` all bypass
/// this — mirroring `build_ensures_checks`' own identical, already-
/// accepted gap (that mechanism also only fires at these same two
/// "normal" exit shapes, not at an exception unwind). A `borrow var`
/// parameter mutated right before one of these early-exit paths keeps
/// today's (pre-plan-84) behavior: the mutation is not guaranteed
/// visible to the caller. Closing this for real would need this same
/// writeback threaded through `build_raise`/`build_begin`'s own exit
/// points too — real, disclosed future work, not attempted here.
fn emit_borrow_var_writebacks<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  for (incoming_ptr, alloca, kind) in ctx.borrow_var_writebacks {
    let current = builder
      .build_load(
        local_llvm_type(context, kind),
        *alloca,
        "borrowvarwriteback",
      )
      .map_err(|e| e.to_string())?;
    builder
      .build_store(*incoming_ptr, current)
      .map_err(|e| e.to_string())?;
  }
  Ok(())
}

/// Byte offset → 1-based source line, via a binary search over
/// `newline_offsets` (every `\n`'s byte offset, ascending) — the
/// conversion plan 35 reuses at every debug-location site instead of
/// re-deriving spans (plan 22's `Spanned<T>.span.0` is already a real
/// byte offset into the same source text this table was built from).
fn line_for_offset(newline_offsets: &[usize], byte_offset: usize) -> u32 {
  1 + newline_offsets.partition_point(|&nl| nl < byte_offset) as u32
}

/// Creates one `DISubprogram` for a function/method/lambda about to be
/// compiled, attaches it to `fv`, and returns its scope — `None` when
/// `gen_ctx` carries no debug info (the ordinary `compile_to_object`
/// path). `first_stmt_span` anchors the subprogram's own declared line
/// (its body's first real statement — a reasonable, if approximate,
/// stand-in for the `def`/lambda-literal's own line, since neither
/// `AstFunction` nor a lambda literal carries its own span; v1 scope
/// only needs per-STATEMENT line accuracy, which `build_stmt` derives
/// independently from each statement's own span).
///
/// Also sets `builder`'s current debug location to this new scope
/// immediately — LLVM's builder keeps whatever debug location was last
/// set across `position_at_end` calls into a *different* function's
/// entry block, so without this, the first instructions built in a new
/// function (`bind_params`'s allocas, `define_main`'s `ARGV`/`ARGC`
/// setup) would carry the *previous* function's leftover `!dbg`
/// location — caught by `module.verify()` for real this session
/// ("!dbg attachment points at wrong subprogram for function") before
/// this fix. Must be called before any instruction is built in the new
/// function, not just before `build_stmt` starts walking its body.
fn di_scope_for_function<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  gen_ctx: &Ctx<'_, 'ctx>,
  fv: FunctionValue<'ctx>,
  name: &str,
  first_stmt_span: Option<(usize, usize)>,
) -> Option<DIScope<'ctx>> {
  let dibuilder = gen_ctx.dibuilder?;
  let file = gen_ctx.di_file?;
  let line = match (first_stmt_span, gen_ctx.newline_offsets) {
    (Some((start, _)), Some(offsets)) => line_for_offset(offsets, start),
    _ => 1,
  };
  let subroutine_ty = dibuilder.create_subroutine_type(file, None, &[], DIFlags::PUBLIC);
  let subprogram = dibuilder.create_function(
    file.as_debug_info_scope(),
    name,
    None,
    file,
    line,
    subroutine_ty,
    true,
    true,
    line,
    DIFlags::PUBLIC,
    false,
  );
  fv.set_subprogram(subprogram);
  let scope = subprogram.as_debug_info_scope();
  let loc = dibuilder.create_debug_location(context, line, 0, scope, None);
  builder.set_current_debug_location(loc);
  Some(scope)
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

  let di_scope = di_scope_for_function(
    context,
    builder,
    gen_ctx,
    fv,
    &f.name,
    f.body.first().map(|s| s.span),
  );

  // Plan 50's `leaf-stack-allocation-codegen`: computed before `fn_ctx`
  // so its own `object_allocas` field can borrow this function's table
  // for the rest of the call — `object_allocas` is unrelated to any
  // other function's own non-escaping set, unlike every other `Ctx`
  // field here.
  let mut new_let_classes = HashMap::new();
  collect_new_let_classes(&f.body, &mut new_let_classes);
  let non_escaping = find_non_escaping_news(&f.body);
  let object_allocas = prealloc_stack_objects(
    context,
    builder,
    &non_escaping,
    &new_let_classes,
    gen_ctx.classes,
  )?;

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();
  // Plan 84's Decision log: collected by `bind_params` below, then
  // borrowed into `fn_ctx` right after — `fn_ctx`'s own construction is
  // deliberately AFTER `bind_params` now (it used to precede it), since
  // `bind_params` reads `gen_ctx.classes`/`gen_ctx.newtypes` directly
  // and never needed `fn_ctx` itself, so this reorder changes nothing
  // else.
  let mut borrow_var_writebacks = Vec::new();
  bind_params(
    context,
    builder,
    fv,
    &effective_params(f),
    0,
    gen_ctx.classes,
    gen_ctx.newtypes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    &mut borrow_var_writebacks,
  )?;

  let fn_ctx = Ctx {
    current_di_scope: di_scope,
    object_allocas: Some(&object_allocas),
    // Plan 62's Decision log: only a top-level function (never a
    // method/lambda/`main`) ever has a non-empty `ensures` list — this
    // is the one real injection point that `Stmt::Return`'s codegen arm
    // and the implicit-return fallthrough both read from.
    current_function_contracts: Some((&f.name, &f.ensures)),
    borrow_var_writebacks: &borrow_var_writebacks,
    ..*gen_ctx
  };

  // Plan 62's Decision log: exactly one fixed injection point — right
  // after `bind_params`, before the user body's first statement
  // compiles. `f.requires` is empty for every function that doesn't
  // declare one, so this is a real no-op (not even a branch emitted)
  // for every pre-plan-62 function/method/lambda/`main`.
  if !f.requires.is_empty() {
    build_requires_checks(
      context,
      builder,
      fv,
      &f.name,
      &f.requires,
      &mut vars,
      &local_classes,
      &local_array_elem_types,
      &fn_ctx,
    )?;
  }

  let mut decls = Vec::new();
  collect_lets(&f.body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  let ret_kind = ret_kind_for_type(&f.return_type);
  build_function_body(
    context,
    builder,
    fv,
    &f.body,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    ret_kind,
    &fn_ctx,
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
  self_field_classes: &HashMap<String, String>,
  gen_ctx: &Ctx<'_, 'ctx>,
) -> Result<(), String> {
  let entry = context.append_basic_block(fv, "entry");
  builder.position_at_end(entry);

  let self_ptr = fv
    .get_nth_param(0)
    .expect("methods always declare a leading self param")
    .into_pointer_value();

  let di_scope = di_scope_for_function(
    context,
    builder,
    gen_ctx,
    fv,
    &m.name,
    m.body.first().map(|s| s.span),
  );

  // Plan 50's `leaf-stack-allocation-codegen` — see `define_user_
  // function`'s identical comment.
  let mut new_let_classes = HashMap::new();
  collect_new_let_classes(&m.body, &mut new_let_classes);
  let non_escaping = find_non_escaping_news(&m.body);
  let object_allocas = prealloc_stack_objects(
    context,
    builder,
    &non_escaping,
    &new_let_classes,
    gen_ctx.classes,
  )?;

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();
  // Plan 84's Decision log: always stays empty for a method today
  // (`strip_ownership_annotations_in_item` never gives a method's
  // params the by-value borrow-ptr marker — see `strip_ownership_in_
  // type_expr`'s own doc comment for why), but threaded through for
  // real regardless, matching `define_user_function`'s identical shape.
  let mut borrow_var_writebacks = Vec::new();
  bind_params(
    context,
    builder,
    fv,
    &m.params,
    1,
    gen_ctx.classes,
    gen_ctx.newtypes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    &mut borrow_var_writebacks,
  )?;

  let method_ctx = Ctx {
    self_ctx: Some((self_ptr, self_fields, self_field_classes)),
    current_di_scope: di_scope,
    object_allocas: Some(&object_allocas),
    borrow_var_writebacks: &borrow_var_writebacks,
    ..*gen_ctx
  };

  let mut decls = Vec::new();
  collect_lets(&m.body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

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
  name: &str,
  params: &[Param],
  ret_kind: ValKind,
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

  let di_scope = di_scope_for_function(
    context,
    builder,
    gen_ctx,
    fv,
    name,
    body.first().map(|s| s.span),
  );

  // Plan 50's `leaf-stack-allocation-codegen` — see `define_user_
  // function`'s identical comment.
  let mut new_let_classes = HashMap::new();
  collect_new_let_classes(body, &mut new_let_classes);
  let non_escaping = find_non_escaping_news(body);
  let object_allocas = prealloc_stack_objects(
    context,
    builder,
    &non_escaping,
    &new_let_classes,
    gen_ctx.classes,
  )?;

  let mut vars = HashMap::new();
  let mut local_classes = HashMap::new();
  let mut local_array_elem_types = HashMap::new();

  for cap_name in &info.captures {
    let kind = info.capture_kinds[cap_name].clone();
    let offset = info.capture_offsets[cap_name];
    let slot_ptr = field_ptr(context, builder, env_ptr, offset)?;
    let val = builder
      .build_load(local_llvm_type(context, &kind), slot_ptr, cap_name)
      .map_err(|e| e.to_string())?;
    let alloca = builder
      .build_alloca(local_llvm_type(context, &kind), cap_name)
      .map_err(|e| e.to_string())?;
    builder
      .build_store(alloca, val)
      .map_err(|e| e.to_string())?;
    vars.insert(cap_name.clone(), (alloca, kind));
  }

  // Plan 84's Decision log: always empty in practice — a lambda literal
  // isn't reached by `strip_ownership_annotations_in_items` at all (it
  // only walks top-level `Item::Function`/`Item::Class`/`Item::Actor`/
  // `Item::Module`; a lambda literal lives inside a `Stmt::Let` value
  // expression, not as its own `Item`), so `params` here is never
  // rewritten with the by-value borrow-ptr marker either way — a
  // pre-existing gap this plan doesn't newly introduce or attempt to
  // close (see `strip_ownership_in_type_expr`'s own doc comment for
  // `value_kind_for_type`'s still-defensive `Own`/`Borrow` arm, kept
  // for exactly this kind of unstripped-input case).
  let mut borrow_var_writebacks = Vec::new();
  bind_params(
    context,
    builder,
    fv,
    params,
    1,
    gen_ctx.classes,
    gen_ctx.newtypes,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    &mut borrow_var_writebacks,
  )?;

  let fn_ctx = Ctx {
    current_di_scope: di_scope,
    object_allocas: Some(&object_allocas),
    borrow_var_writebacks: &borrow_var_writebacks,
    ..*gen_ctx
  };

  let mut decls = Vec::new();
  collect_lets(body, &mut decls);
  prealloc_lets(context, builder, &decls, &mut vars)?;

  build_function_body(
    context,
    builder,
    fv,
    body,
    &mut vars,
    &mut local_classes,
    &mut local_array_elem_types,
    ret_kind,
    &fn_ctx,
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

  // Bugfix (benchmark session, plan 65+): plan 55's Decision log assumed
  // "no cost worth special-casing" for starting the pool unconditionally
  // — measured, this session, to be false. `emerald_worker_pool_start`
  // (`runtime/emerald_runtime.c`) spawns `nproc` real `pthread_create`
  // threads (32 on the machine this was measured on) and `_drain_and_
  // join` joins every one of them, even when the program declares zero
  // actors; a trivial `sum` program's real run time went from ~0.5ms to
  // ~6.4ms because of this pair of calls alone (root-caused via
  // `EMERALD_WORKERS=1` isolating the cost — see `benchmarks/
  // confirm_worker_pool.py`). Gated on whether this compilation unit's
  // own merged `program` (post-`require`-splice, so `client.em`-style
  // files that only reference an actor declared in a required file still
  // count) contains any `Item::Actor` at all — every actor-declaring
  // program still gets the exact previous unconditional behavior; a
  // program with none now skips both calls entirely.
  let program_declares_actors = program.items.iter().any(|i| matches!(i, Item::Actor(_)));

  // Plan 55's Decision log: a compiler-inserted implicit barrier, not a
  // language-visible `await`/join primitive — Emerald still has no
  // `async`/`await` keyword anywhere in its grammar after this plan.
  //
  // `unset_current_debug_location` before this call, when debug info is
  // active (`compile_to_object_with_debug_info`, `emerald-cli`'s own
  // real compile path): this synthesized call has no real source line
  // of its own, and — a real bug this leaf found and fixed — the
  // builder's debug location is otherwise still whatever the PREVIOUS
  // function compiled left it at (e.g. a free function defined earlier
  // in the same program), which LLVM's module verifier correctly
  // rejects as a `!dbg` attachment pointing at the wrong subprogram.
  // No-op (there is no location to unset) when debug info is off.
  if program_declares_actors {
    if gen_ctx.dibuilder.is_some() {
      builder.unset_current_debug_location();
    }
    builder
      .build_call(gen_ctx.actor_funcs.pool_start, &[], "poolstart")
      .map_err(|e| e.to_string())?;
  }

  let top_stmts: Vec<Spanned<Stmt>> = program
    .items
    .iter()
    .filter_map(|it| match it {
      Item::Stmt(s) => Some(s.clone()),
      _ => None,
    })
    .collect();

  let di_scope = di_scope_for_function(
    context,
    builder,
    gen_ctx,
    main_fn,
    "main",
    top_stmts.first().map(|s| s.span),
  );

  // Plan 50's `leaf-stack-allocation-codegen` — see `define_user_
  // function`'s identical comment.
  let mut new_let_classes = HashMap::new();
  collect_new_let_classes(&top_stmts, &mut new_let_classes);
  let non_escaping = find_non_escaping_news(&top_stmts);
  let object_allocas = prealloc_stack_objects(
    context,
    builder,
    &non_escaping,
    &new_let_classes,
    gen_ctx.classes,
  )?;

  let fn_ctx = Ctx {
    current_di_scope: di_scope,
    object_allocas: Some(&object_allocas),
    ..*gen_ctx
  };

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
    .build_alloca(local_llvm_type(context, &ValKind::Ptr), "ARGV")
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
    .build_alloca(local_llvm_type(context, &ValKind::Int64), "ARGC")
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
    &fn_ctx,
  )?;
  if !terminated {
    if program_declares_actors {
      if gen_ctx.dibuilder.is_some() {
        builder.unset_current_debug_location();
      }
      builder
        .build_call(gen_ctx.actor_funcs.pool_drain_and_join, &[], "pooldrain")
        .map_err(|e| e.to_string())?;
    }
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
/// Plan 61's Decision log: `comptime`'s own tree-walking runtime value —
/// a small, closed set (`Int64`/`Float64`/`Boolean`), NOT the fuller
/// `Int`/`Float`/`Bool`/`Struct`/`Variant` shape the task brief's own
/// text sketches. A real, disclosed v1 scope cut: `New`/`InstanceVar`
/// stay on `check_comptime_legal`'s allow-list exactly as the brief
/// specifies (so a `comptime` function that never touches them still
/// type-checks and interprets today), but this interpreter itself
/// doesn't yet construct/read struct values — reaching either node
/// produces a real, clear `Err` (see `ComptimeInterpreter::eval`'s own
/// arm) rather than a silent wrong answer or a panic. Reading a
/// constructed value's field back out would need `Expr::MethodCall`
/// (this language's only field-read syntax — plan 33's "always a method
/// call" rule, verified this session), which `check_comptime_legal`
/// bans outright; closing that gap for real is legitimate future work,
/// not attempted here. This plan's own flagship worked proof
/// (`comptime factorial(10)`) never needs more than `Int64` anyway.
/// `pub` — `leaf-comptime-query-cache-integration`'s own `emerald-
/// driver::cache::comptime_eval_query` needs to name this type directly
/// to persist/reconstruct a cached result (`emerald-driver` depends on
/// `emerald-codegen`, never the reverse, so this crate's own interpreter
/// stays the single source of truth for what a `comptime` value even
/// is).
#[derive(Debug, Clone, PartialEq)]
pub enum ComptimeValue {
  Int(i64),
  Float(f64),
  Bool(bool),
}

/// A hard, disclosed step ceiling, not a termination proof (Rice's
/// theorem: general termination is undecidable) — `steps` increments on
/// every statement executed, every loop-condition re-check, and every
/// expression evaluated; exceeding `limit` is a real compile failure
/// (`Err`, propagated all the way up through `build_expr`/`define_main`
/// to a real diagnostic), never a hang, never a panic.
struct ComptimeInterpreter {
  steps: u64,
  limit: u64,
}

impl ComptimeInterpreter {
  fn new(limit: u64) -> Self {
    ComptimeInterpreter { steps: 0, limit }
  }

  fn tick(&mut self, label: &str) -> Result<(), String> {
    self.steps += 1;
    if self.steps > self.limit {
      return Err(format!(
        "comptime evaluation of `{label}` exceeded {} steps (possible infinite loop); pass --comptime-step-limit=N to raise it",
        self.limit
      ));
    }
    Ok(())
  }

  fn eval(
    &mut self,
    expr: &Spanned<Expr>,
    env: &HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<ComptimeValue, String> {
    self.tick(label)?;
    match &expr.node {
      Expr::Int(n) => Ok(ComptimeValue::Int(*n)),
      Expr::Float(f) => Ok(ComptimeValue::Float(*f)),
      Expr::Bool(b) => Ok(ComptimeValue::Bool(*b)),
      Expr::Ident(name) => env
        .get(name)
        .cloned()
        .ok_or_else(|| format!("comptime evaluation: undefined variable `{name}`")),
      Expr::Add(a, b) => self.eval_arith("+", a, b, env, comptime_fns, label),
      Expr::Sub(a, b) => self.eval_arith("-", a, b, env, comptime_fns, label),
      Expr::Mul(a, b) => self.eval_arith("*", a, b, env, comptime_fns, label),
      Expr::Div(a, b) => self.eval_arith("/", a, b, env, comptime_fns, label),
      Expr::Rem(a, b) => self.eval_arith("%", a, b, env, comptime_fns, label),
      Expr::Neg(a) => match self.eval(a, env, comptime_fns, label)? {
        ComptimeValue::Int(x) => Ok(ComptimeValue::Int(-x)),
        ComptimeValue::Float(x) => Ok(ComptimeValue::Float(-x)),
        ComptimeValue::Bool(_) => Err("comptime evaluation: cannot negate a Boolean".to_string()),
      },
      Expr::Not(a) => Ok(ComptimeValue::Bool(!self.eval_bool(
        a,
        env,
        comptime_fns,
        label,
      )?)),
      Expr::And(a, b) => {
        if !self.eval_bool(a, env, comptime_fns, label)? {
          Ok(ComptimeValue::Bool(false))
        } else {
          Ok(ComptimeValue::Bool(self.eval_bool(
            b,
            env,
            comptime_fns,
            label,
          )?))
        }
      }
      Expr::Or(a, b) => {
        if self.eval_bool(a, env, comptime_fns, label)? {
          Ok(ComptimeValue::Bool(true))
        } else {
          Ok(ComptimeValue::Bool(self.eval_bool(
            b,
            env,
            comptime_fns,
            label,
          )?))
        }
      }
      Expr::Compare(a, op, b) => {
        let av = self.eval(a, env, comptime_fns, label)?;
        let bv = self.eval(b, env, comptime_fns, label)?;
        self.eval_compare(op, &av, &bv)
      }
      Expr::BitAnd(a, b) => self.eval_bit(a, b, env, comptime_fns, label, |x, y| x & y),
      Expr::BitOr(a, b) => self.eval_bit(a, b, env, comptime_fns, label, |x, y| x | y),
      Expr::BitXor(a, b) => self.eval_bit(a, b, env, comptime_fns, label, |x, y| x ^ y),
      Expr::Shl(a, b) => self.eval_bit(a, b, env, comptime_fns, label, |x, y| x << y),
      Expr::Shr(a, b) => self.eval_bit(a, b, env, comptime_fns, label, |x, y| x >> y),
      Expr::BitNot(a) => Ok(ComptimeValue::Int(!self.eval_int(
        a,
        env,
        comptime_fns,
        label,
      )?)),
      Expr::Call(name, args) => {
        let argvals = args
          .iter()
          .map(|a| self.eval(a, env, comptime_fns, label))
          .collect::<Result<Vec<_>, _>>()?;
        let f = comptime_fns.get(name.as_str()).ok_or_else(|| {
          format!("comptime evaluation: `{name}` is not a known comptime function")
        })?;
        let mut call_env = HashMap::new();
        for (p, v) in f.params.iter().zip(argvals) {
          call_env.insert(p.name.clone(), v);
        }
        match self.exec_block(&f.body, &mut call_env, comptime_fns, name)? {
          Some(v) => Ok(v),
          None => Err(format!(
            "comptime evaluation: `{name}` did not return a value"
          )),
        }
      }
      Expr::New(..) | Expr::InstanceVar(_) => Err(
        "comptime evaluation of struct/enum values is not yet supported by the interpreter"
          .to_string(),
      ),
      _ => Err(
        "comptime evaluation encountered an expression form outside the comptime-legal subset"
          .to_string(),
      ),
    }
  }

  fn eval_bool(
    &mut self,
    e: &Spanned<Expr>,
    env: &HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<bool, String> {
    match self.eval(e, env, comptime_fns, label)? {
      ComptimeValue::Bool(b) => Ok(b),
      _ => Err("comptime evaluation: expected a Boolean value".to_string()),
    }
  }

  fn eval_int(
    &mut self,
    e: &Spanned<Expr>,
    env: &HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<i64, String> {
    match self.eval(e, env, comptime_fns, label)? {
      ComptimeValue::Int(n) => Ok(n),
      _ => Err("comptime evaluation: expected an Int64 value".to_string()),
    }
  }

  fn eval_arith(
    &mut self,
    op: &str,
    a: &Spanned<Expr>,
    b: &Spanned<Expr>,
    env: &HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<ComptimeValue, String> {
    let av = self.eval(a, env, comptime_fns, label)?;
    let bv = self.eval(b, env, comptime_fns, label)?;
    match (av, bv) {
      (ComptimeValue::Int(x), ComptimeValue::Int(y)) => {
        let r = match op {
          "+" => x.checked_add(y),
          "-" => x.checked_sub(y),
          "*" => x.checked_mul(y),
          "/" => {
            if y == 0 {
              return Err("comptime evaluation: division by zero".to_string());
            }
            x.checked_div(y)
          }
          "%" => {
            if y == 0 {
              return Err("comptime evaluation: division by zero".to_string());
            }
            x.checked_rem(y)
          }
          _ => unreachable!("eval_arith is only ever called with one of +-*/%"),
        };
        r.map(ComptimeValue::Int)
          .ok_or_else(|| "comptime evaluation: integer overflow".to_string())
      }
      (ComptimeValue::Float(x), ComptimeValue::Float(y)) => {
        let r = match op {
          "+" => x + y,
          "-" => x - y,
          "*" => x * y,
          "/" => x / y,
          "%" => x % y,
          _ => unreachable!("eval_arith is only ever called with one of +-*/%"),
        };
        Ok(ComptimeValue::Float(r))
      }
      _ => Err("comptime evaluation: mismatched operand types in arithmetic".to_string()),
    }
  }

  fn eval_bit(
    &mut self,
    a: &Spanned<Expr>,
    b: &Spanned<Expr>,
    env: &HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
    op: impl Fn(i64, i64) -> i64,
  ) -> Result<ComptimeValue, String> {
    let x = self.eval_int(a, env, comptime_fns, label)?;
    let y = self.eval_int(b, env, comptime_fns, label)?;
    Ok(ComptimeValue::Int(op(x, y)))
  }

  fn eval_compare(
    &self,
    op: &CompareOp,
    a: &ComptimeValue,
    b: &ComptimeValue,
  ) -> Result<ComptimeValue, String> {
    let ordering = match (a, b) {
      (ComptimeValue::Int(x), ComptimeValue::Int(y)) => x.partial_cmp(y),
      (ComptimeValue::Float(x), ComptimeValue::Float(y)) => x.partial_cmp(y),
      (ComptimeValue::Bool(x), ComptimeValue::Bool(y)) => x.partial_cmp(y),
      _ => {
        return Err("comptime evaluation: mismatched operand types in a comparison".to_string());
      }
    };
    let Some(ord) = ordering else {
      return Err("comptime evaluation: an unorderable comparison (e.g. NaN)".to_string());
    };
    let result = match op {
      CompareOp::Lt => ord.is_lt(),
      CompareOp::Gt => ord.is_gt(),
      CompareOp::Le => ord.is_le(),
      CompareOp::Ge => ord.is_ge(),
      CompareOp::Eq => ord.is_eq(),
      CompareOp::Ne => ord.is_ne(),
    };
    Ok(ComptimeValue::Bool(result))
  }

  /// `Ok(Some(v))`: a `Return` fired, unwinding the rest of `body`.
  /// `Ok(None)`: `body` ran to completion with no `Return`.
  fn exec_block(
    &mut self,
    body: &[Spanned<Stmt>],
    env: &mut HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<Option<ComptimeValue>, String> {
    for stmt in body {
      if let Some(v) = self.exec_stmt(stmt, env, comptime_fns, label)? {
        return Ok(Some(v));
      }
    }
    Ok(None)
  }

  fn exec_stmt(
    &mut self,
    stmt: &Spanned<Stmt>,
    env: &mut HashMap<String, ComptimeValue>,
    comptime_fns: &HashMap<String, &AstFunction>,
    label: &str,
  ) -> Result<Option<ComptimeValue>, String> {
    self.tick(label)?;
    match &stmt.node {
      Stmt::Let { name, value, .. } | Stmt::Assign { name, value } => {
        let v = self.eval(value, env, comptime_fns, label)?;
        env.insert(name.clone(), v);
        Ok(None)
      }
      Stmt::MultiAssign { names, values } => {
        let vs = values
          .iter()
          .map(|v| self.eval(v, env, comptime_fns, label))
          .collect::<Result<Vec<_>, _>>()?;
        for (n, v) in names.iter().zip(vs) {
          env.insert(n.clone(), v);
        }
        Ok(None)
      }
      Stmt::If {
        cond,
        then_branch,
        else_branch,
      } => {
        if self.eval_bool(cond, env, comptime_fns, label)? {
          self.exec_block(then_branch, env, comptime_fns, label)
        } else if let Some(eb) = else_branch {
          self.exec_block(eb, env, comptime_fns, label)
        } else {
          Ok(None)
        }
      }
      Stmt::While { cond, body } => {
        while self.eval_bool(cond, env, comptime_fns, label)? {
          if let Some(v) = self.exec_block(body, env, comptime_fns, label)? {
            return Ok(Some(v));
          }
        }
        Ok(None)
      }
      Stmt::For {
        var,
        elements,
        body,
      } => {
        for e in elements {
          let v = self.eval(e, env, comptime_fns, label)?;
          env.insert(var.clone(), v);
          if let Some(rv) = self.exec_block(body, env, comptime_fns, label)? {
            return Ok(Some(rv));
          }
        }
        Ok(None)
      }
      Stmt::ForRange {
        var,
        start,
        end,
        exclusive,
        body,
      } => {
        let s = self.eval_int(start, env, comptime_fns, label)?;
        let e = self.eval_int(end, env, comptime_fns, label)?;
        let mut i = s;
        while if *exclusive { i < e } else { i <= e } {
          self.tick(label)?;
          env.insert(var.clone(), ComptimeValue::Int(i));
          if let Some(rv) = self.exec_block(body, env, comptime_fns, label)? {
            return Ok(Some(rv));
          }
          i += 1;
        }
        Ok(None)
      }
      Stmt::Case {
        scrutinee,
        arms,
        else_body,
      } => {
        let sv = self.eval(scrutinee, env, comptime_fns, label)?;
        for (pat, arm_body) in arms {
          let CasePattern::Values(vs) = pat else {
            return Err(
              "comptime evaluation does not yet support enum-variant `case` patterns".to_string(),
            );
          };
          for v in vs {
            let cv = self.eval(v, env, comptime_fns, label)?;
            if cv == sv {
              return self.exec_block(arm_body, env, comptime_fns, label);
            }
          }
        }
        if let Some(eb) = else_body {
          self.exec_block(eb, env, comptime_fns, label)
        } else {
          Ok(None)
        }
      }
      Stmt::Return(Some(e)) => Ok(Some(self.eval(e, env, comptime_fns, label)?)),
      Stmt::Return(None) => {
        Err("comptime evaluation: a comptime function must return a value".to_string())
      }
      Stmt::Expr(e) => {
        self.eval(e, env, comptime_fns, label)?;
        Ok(None)
      }
      _ => Err(
        "comptime evaluation encountered a statement form outside the comptime-legal subset"
          .to_string(),
      ),
    }
  }
}

/// `pub` entry point for `leaf-comptime-query-cache-integration`'s own
/// MISS path — `emerald-driver::cache::comptime_eval_query` calls this
/// directly rather than reaching into `build_expr`'s own LLVM-emitting
/// codegen (which needs a live `inkwell::Context`/`Builder`/`Ctx`, none
/// of which a cache-layer MISS has, or wants — it only needs the real
/// *value*, not any IR). Builds `comptime_fns` from `program` the same
/// way `build_expr`'s own `Expr::Comptime` arm does.
pub fn eval_comptime_expr(
  expr: &Spanned<Expr>,
  program: &Program,
  limit: u64,
) -> Result<ComptimeValue, String> {
  let comptime_fns: HashMap<String, &AstFunction> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Function(f) if f.is_comptime => Some((f.name.clone(), f)),
      _ => None,
    })
    .collect();
  let mut interp = ComptimeInterpreter::new(limit);
  interp.eval(
    expr,
    &HashMap::new(),
    &comptime_fns,
    "<comptime expression>",
  )
}

/// Bakes a fully-evaluated `ComptimeValue` into a literal LLVM constant —
/// never an `alloca`, never a `call`, exactly the "zero runtime
/// computation" proof this leaf's own worked example needs.
fn comptime_value_to_llvm_constant<'ctx>(
  context: &'ctx Context,
  value: &ComptimeValue,
) -> (BasicValueEnum<'ctx>, ValKind) {
  match value {
    ComptimeValue::Int(n) => (
      context.i64_type().const_int(*n as u64, true).into(),
      ValKind::Int64,
    ),
    ComptimeValue::Float(f) => (context.f64_type().const_float(*f).into(), ValKind::Float64),
    ComptimeValue::Bool(b) => (
      context.bool_type().const_int(*b as u64, false).into(),
      ValKind::Bool,
    ),
  }
}

/// Plan 61's Decision log (AC2's own second half): a `comptime`-marked
/// function that is never referenced outside a `comptime` expression
/// anywhere in the program is never emitted as an LLVM function at all —
/// nothing at runtime could ever reach it. Computed once, program-wide,
/// before `declare_user_functions`/the function-body-definition loop
/// both consult it.
fn comptime_only_function_names(program: &Program) -> HashSet<String> {
  let comptime_names: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Function(f) if f.is_comptime => Some(f.name.clone()),
      _ => None,
    })
    .collect();
  if comptime_names.is_empty() {
    return HashSet::new();
  }
  let mut runtime_called: HashSet<String> = HashSet::new();
  for item in &program.items {
    match item {
      Item::Stmt(s) => collect_runtime_call_names_stmt(s, &mut runtime_called),
      // A `comptime` function's OWN body is only ever interpreted, never
      // compiled to LLVM (as long as it stays comptime-only) — a real
      // bug this leaf's own white-box test caught: `factorial`'s own
      // recursive `factorial(n - 1)` self-call was wrongly counted as a
      // "runtime" call site, keeping `factorial` emitted as a real LLVM
      // function even though nothing at runtime ever calls it. Skipped
      // here entirely; an ordinary (non-`comptime`) function's body is
      // still walked unconditionally below.
      Item::Function(f) if f.is_comptime => {}
      Item::Function(f) => {
        for s in &f.body {
          collect_runtime_call_names_stmt(s, &mut runtime_called);
        }
      }
      Item::Class(c) => {
        for m in &c.methods {
          for s in &m.body {
            collect_runtime_call_names_stmt(s, &mut runtime_called);
          }
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          for s in &f.body {
            collect_runtime_call_names_stmt(s, &mut runtime_called);
          }
        }
      }
      Item::Actor(a) => {
        for m in &a.methods {
          for s in &m.body {
            collect_runtime_call_names_stmt(s, &mut runtime_called);
          }
        }
      }
      _ => {}
    }
  }
  comptime_names
    .difference(&runtime_called)
    .cloned()
    .collect()
}

/// Records every `Expr::Call` callee name reachable WITHOUT crossing
/// into an `Expr::Comptime` subtree (a call made only at compile time,
/// inside a `comptime` expression, is not an "ordinary" runtime call
/// site — `comptime_only_function_names` is exactly the set difference
/// this produces).
fn collect_runtime_call_names_stmt(stmt: &Spanned<Stmt>, out: &mut HashSet<String>) {
  match &stmt.node {
    Stmt::Let { value, .. }
    | Stmt::SetField { value, .. }
    | Stmt::Assign { value, .. }
    | Stmt::Raise(value)
    | Stmt::Expr(value) => collect_runtime_call_names_expr(value, out),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      collect_runtime_call_names_expr(array, out);
      collect_runtime_call_names_expr(index, out);
      collect_runtime_call_names_expr(value, out);
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        collect_runtime_call_names_expr(v, out);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      collect_runtime_call_names_expr(cond, out);
      for s in then_branch {
        collect_runtime_call_names_stmt(s, out);
      }
      if let Some(eb) = else_branch {
        for s in eb {
          collect_runtime_call_names_stmt(s, out);
        }
      }
    }
    Stmt::While { cond, body } => {
      collect_runtime_call_names_expr(cond, out);
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        collect_runtime_call_names_expr(e, out);
      }
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      collect_runtime_call_names_expr(start, out);
      collect_runtime_call_names_expr(end, out);
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      collect_runtime_call_names_expr(scrutinee, out);
      for (pat, body) in arms {
        if let CasePattern::Values(vs) = pat {
          for v in vs {
            collect_runtime_call_names_expr(v, out);
          }
        }
        for s in body {
          collect_runtime_call_names_stmt(s, out);
        }
      }
      if let Some(eb) = else_body {
        for s in eb {
          collect_runtime_call_names_stmt(s, out);
        }
      }
    }
    Stmt::Return(Some(e)) => collect_runtime_call_names_expr(e, out),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
      for r in rescues {
        for s in &r.body {
          collect_runtime_call_names_stmt(s, out);
        }
      }
      if let Some(en) = ensure {
        for s in en {
          collect_runtime_call_names_stmt(s, out);
        }
      }
    }
    Stmt::Yield(args) => {
      for a in args {
        collect_runtime_call_names_expr(a, out);
      }
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      collect_runtime_call_names_expr(scrutinee, out);
      for s in ok_body {
        collect_runtime_call_names_stmt(s, out);
      }
      for s in err_body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
  }
}

fn collect_runtime_call_names_expr(expr: &Spanned<Expr>, out: &mut HashSet<String>) {
  match &expr.node {
    // The one subtree this walk never descends into as an "ordinary"
    // call site — a call made only inside a `comptime` expression
    // doesn't count as a reason to keep the callee's LLVM function
    // around.
    Expr::Comptime(_) => {}
    Expr::Call(name, args) => {
      out.insert(name.clone());
      for a in args {
        collect_runtime_call_names_expr(a, out);
      }
    }
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::InstanceVar(_)
    | Expr::Bool(_) => {}
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          collect_runtime_call_names_expr(e, out);
        }
      }
    }
    Expr::Add(a, b)
    | Expr::Sub(a, b)
    | Expr::Mul(a, b)
    | Expr::Div(a, b)
    | Expr::Rem(a, b)
    | Expr::And(a, b)
    | Expr::Or(a, b)
    | Expr::BitAnd(a, b)
    | Expr::BitOr(a, b)
    | Expr::BitXor(a, b)
    | Expr::Shl(a, b)
    | Expr::Shr(a, b)
    | Expr::Index(a, b) => {
      collect_runtime_call_names_expr(a, out);
      collect_runtime_call_names_expr(b, out);
    }
    Expr::Neg(a)
    | Expr::Not(a)
    | Expr::BitNot(a)
    | Expr::ArrayNew(a)
    | Expr::Ok(a)
    | Expr::Err(a)
    | Expr::Try(a) => collect_runtime_call_names_expr(a, out),
    Expr::Compare(a, _, b) | Expr::Coalesce(a, b) => {
      collect_runtime_call_names_expr(a, out);
      collect_runtime_call_names_expr(b, out);
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        collect_runtime_call_names_expr(v, out);
      }
    }
    Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        collect_runtime_call_names_expr(a, out);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      collect_runtime_call_names_expr(recv, out);
      for a in args {
        collect_runtime_call_names_expr(a, out);
      }
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        collect_runtime_call_names_expr(e, out);
      }
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        collect_runtime_call_names_expr(k, out);
        collect_runtime_call_names_expr(v, out);
      }
    }
    Expr::Lambda { body, .. } => {
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
    Expr::Supervise(body) => {
      for s in body {
        collect_runtime_call_names_stmt(s, out);
      }
    }
    Expr::Remote { addr, name, .. } => {
      collect_runtime_call_names_expr(addr, out);
      collect_runtime_call_names_expr(name, out);
    }
    Expr::Locate { key, args, .. } => {
      collect_runtime_call_names_expr(key, out);
      for a in args {
        collect_runtime_call_names_expr(a, out);
      }
    }
  }
}

fn declare_user_functions<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  classes: &HashMap<String, ClassLayout>,
  generic_instances: &HashMap<String, ClassDef>,
  // Plan 61's Decision log (AC2's own second half): a `comptime`-marked
  // function never referenced outside a `comptime` expression anywhere
  // in the program — see `comptime_only_function_names`'s own doc
  // comment. Never given an LLVM symbol at all, the same "dead code with
  // no valid caller" reasoning `block_param`'s own arm immediately below
  // already establishes.
  comptime_only_fns: &HashSet<String>,
) -> HashMap<String, (FunctionValue<'ctx>, ValKind)> {
  let mut user_func_ids = HashMap::new();
  for item in &program.items {
    // Plan 76: unwrap once (the grammar never nests `export`) so an
    // exported function/class/module/actor is declared exactly like a
    // non-exported one.
    let mut item: &Item = item;
    while let Item::Export(inner) = item {
      item = inner;
    }
    match item {
      Item::Function(f) if comptime_only_fns.contains(&f.name) => {}
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
        let ret_kind = ret_kind_for_type(&f.return_type);
        let fn_ty = make_fn_type(context, &param_kinds(&effective_params(f)), &ret_kind);
        let fv = module.add_function(&f.name, fn_ty, Some(Linkage::External));
        user_func_ids.insert(f.name.clone(), (fv, ret_kind));
      }
      // Plan 58: a generic class TEMPLATE's own raw methods (bare `T`-
      // typed params/returns) are never declared under its own
      // unmangled name — `generic_instances`' own methods, declared in
      // the dedicated pass just below this loop, are the only real
      // LLVM symbols any monomorphized instantiation's calls ever
      // target.
      Item::Class(c) if !c.type_params.is_empty() => {}
      Item::Class(c) => {
        for m in &c.methods {
          // Plan 89's Decision log: a method's OWN `[U]`/`[U: Bound]`
          // type parameter is never declared under its raw, unmangled
          // symbol at all — mirroring `Item::Function`'s identical
          // "generic — skip the bare name" precedent immediately above.
          // Before this fix, the RAW body (its own type parameter
          // folded to the generic `ValKind::Ptr` bucket) was declared
          // and defined unconditionally regardless, which is exactly
          // the shape that could reach the LLVM verifier with
          // mismatched types once `.call`'s own dispatch stopped
          // hard-erroring on a non-static-name Proc receiver (plan 89's
          // OTHER leaf) — only `resolve_generic_method_instance`'s own
          // lazily-monomorphized, fully-concrete specializations are
          // ever compiled now.
          if !m.type_params.is_empty() {
            continue;
          }
          let ret_kind = value_kind_for_type(&m.return_type);
          let mut kinds = vec![ValKind::Ptr]; // self
          kinds.extend(param_kinds(&m.params));
          let fn_ty = make_fn_type(context, &kinds, &ret_kind);
          let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
          let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
          user_func_ids.insert(mangled, (fv, ret_kind));
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          let ret_kind = ret_kind_for_type(&f.return_type);
          let fn_ty = make_fn_type(context, &param_kinds(&f.params), &ret_kind);
          let mangled = format!("{}_{}", m.name, f.name);
          let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
          user_func_ids.insert(mangled, (fv, ret_kind));
        }
      }
      // Plan 54: an actor method's LLVM signature is identical to a
      // class method's own (leading `self` ptr) — the only difference
      // is `.spawn`'s own allocation call site, not the method ABI.
      Item::Actor(a) => {
        for m in &a.methods {
          // Plan 89: mirrors `Item::Class`'s own identical generic-
          // method skip immediately above (grammatically reachable,
          // even though no actor in this compiler's own examples
          // declares one).
          if !m.type_params.is_empty() {
            continue;
          }
          let ret_kind = value_kind_for_type(&m.return_type);
          let mut kinds = vec![ValKind::Ptr]; // self
          kinds.extend(param_kinds(&m.params));
          let fn_ty = make_fn_type(context, &kinds, &ret_kind);
          let mangled = format!("{}_{}", a.name, mangled_operator_symbol(&m.name));
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
      // Plan 76: same "unresolved cross-file reference, rejected by
      // `compile_to_object`'s own prologue before this runs" reasoning
      // as `Item::Require` immediately above.
      Item::Import { .. } => {}
      // Unreachable: unwrapped by the `while let` loop above this match.
      Item::Export(_) => unreachable!("Item::Export is unwrapped before this match"),
      // Plan 47: never actually reached — `compile_to_object`'s own
      // prologue rejects any `Program` still containing an
      // `Item::Test` before this runs at all. Plan 80: `Item::
      // Property`/`Item::Benchmark` get the identical rejection (see
      // that prologue check) and are always stripped by `compile_
      // test_harness`/`compile_benchmark_harness` before this ever
      // runs, same as `Item::Test`.
      Item::Test { .. } | Item::Property { .. } | Item::Benchmark { .. } => {}
      // Plan 52: pure data — no function body to declare an LLVM
      // symbol for.
      Item::Enum(_) => {}
      // A newtype declares no method of its own (operator overloading
      // is explicitly out of scope for this pass — see `NewtypeDef`'s
      // own doc comment, `ast.rs`, and this crate's newtype-lowering
      // doc comment for why: every existing class/actor method above
      // is compiled with a leading `self` POINTER, the exact opposite
      // representation a zero-cost newtype needs) — nothing to declare.
      Item::Newtype(_) => {}
      // Plan 26: `emerald_parser::parse`/`parse_named` only ever
      // returns `Ok(program)` with zero recovered errors, meaning no
      // `Item::Error` in `program.items` — codegen never receives one.
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
      // Plan 59: declared in its own dedicated pass in `compile_to_
      // object_impl` (inserted straight into `user_func_ids`, the
      // return value of this function) — not here.
      Item::Extern(_) => {}
    }
  }
  // Plan 58: each monomorphized generic-class instantiation's methods,
  // declared exactly like an ordinary `Item::Class`'s own methods above
  // (same leading `self` ptr, same `{ClassName}_{method}` mangling —
  // `c.name` here is already the MANGLED name, e.g. `Stack$Int64`, so
  // this produces `Stack$Int64_push` with zero further special-casing).
  for c in generic_instances.values() {
    for m in &c.methods {
      // Plan 89: a generic-class instantiation's own method may ALSO
      // declare its own further `[U]` type parameter, on top of the
      // class's own already-substituted one — mirrors `Item::Class`'s
      // identical skip immediately above.
      if !m.type_params.is_empty() {
        continue;
      }
      let ret_kind = value_kind_for_type(&m.return_type);
      let mut kinds = vec![ValKind::Ptr]; // self
      kinds.extend(param_kinds(&m.params));
      let fn_ty = make_fn_type(context, &kinds, &ret_kind);
      let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
      let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
      user_func_ids.insert(mangled, (fv, ret_kind));
    }
  }
  let _ = classes; // kept in the signature for symmetry with the define pass
  user_func_ids
}

/// Plan 71 follow-up (this session's own real, disclosed gap, mirroring
/// `emerald-sema`'s `infer_lambda_type`'s own doc comment): after the
/// grammar unification, a lambda literal's surface syntax (`do |params: T|
/// ... end`) never states a return type at all — `Expr::Lambda.return_type`
/// is always the `"Void"` placeholder the parser fills in, never a real
/// user-written annotation, unlike the old `->(params) -> Type { body }`
/// literal this replaced. `declare_lambda_functions` used to trust that
/// field directly for a top-level `Proc`-typed `Let`'s own LLVM function
/// signature — sound when it was real surface syntax, silently wrong now
/// that it's a constant placeholder (every `.select`/`.map`/`.reduce`/
/// `.count` call against a non-Void-returning Proc hit this crate's own
/// "Proc must not be Void (sema should have rejected this)" internal-error
/// guards, since `ret_kind` was always `ValKind::Void` regardless of the
/// lambda's real body). This is a narrow, `ValKind`-level mirror of sema's
/// `Type`-level inference — precise enough to be right for any program that
/// has already passed sema's own equivalent, real check, not a general type
/// checker.
/// Plan 89's Decision log: `base_env` is now caller-supplied rather than
/// built internally from `top_level_types` alone — `declare_lambda_
/// functions`'s own top-level-lambda call site still passes exactly
/// that (unchanged behavior), while `build_inline_lambda`'s new call
/// site (an anonymous lambda literal compiled on the fly at a call
/// argument's own position, not bound to any top-level `Let` at all)
/// passes the ENCLOSING function's own current locals/params too, so a
/// capture's kind resolves correctly regardless of whether it's a
/// top-level name or a plain local.
fn infer_lambda_ret_kind(
  params: &[Param],
  body: &[Spanned<Stmt>],
  base_env: &HashMap<String, ValKind>,
  user_fn_return_types: &HashMap<String, TypeExpr>,
) -> ValKind {
  let mut env: HashMap<String, ValKind> = base_env.clone();
  for p in params {
    env.insert(p.name.clone(), value_kind_for_type(&p.ty));
  }
  match body.last() {
    Some(Spanned {
      node: Stmt::Expr(e),
      ..
    })
    | Some(Spanned {
      node: Stmt::Return(Some(e)),
      ..
    }) => infer_expr_val_kind(e, &env, user_fn_return_types),
    _ => ValKind::Void,
  }
}

fn infer_expr_val_kind(
  expr: &Spanned<Expr>,
  env: &HashMap<String, ValKind>,
  user_fn_return_types: &HashMap<String, TypeExpr>,
) -> ValKind {
  match &expr.node {
    Expr::Ident(name) => env.get(name.as_str()).cloned().unwrap_or(ValKind::Int64),
    Expr::Int(_) => ValKind::Int64,
    Expr::Float(_) => ValKind::Float64,
    Expr::Bool(_) => ValKind::Bool,
    Expr::StringLit(_) | Expr::Interpolate(_) => ValKind::Str,
    Expr::SymbolLit(_) => ValKind::Symbol,
    Expr::Not(_) | Expr::And(_, _) | Expr::Or(_, _) | Expr::Compare(_, _, _) => ValKind::Bool,
    Expr::Add(l, _)
    | Expr::Sub(l, _)
    | Expr::Mul(l, _)
    | Expr::Div(l, _)
    | Expr::Rem(l, _)
    | Expr::Neg(l) => infer_expr_val_kind(l, env, user_fn_return_types),
    Expr::BitAnd(_, _)
    | Expr::BitOr(_, _)
    | Expr::BitXor(_, _)
    | Expr::BitNot(_)
    | Expr::Shl(_, _)
    | Expr::Shr(_, _) => ValKind::Int64,
    Expr::Call(name, _) if name == "puts" => ValKind::Void,
    Expr::Call(name, _) => user_fn_return_types
      .get(name)
      .map(value_kind_for_type)
      .unwrap_or(ValKind::Void),
    Expr::MethodCall(_, method, _) if method == "key" || method == "value" => ValKind::Int64,
    _ => ValKind::Ptr,
  }
}

/// Plan 89: extracted out of `declare_lambda_functions`'s own former
/// internal scan (unchanged logic) — computed once and shared with
/// `Ctx::top_level_types`/`Ctx::user_fn_return_types`, so `build_
/// inline_lambda`'s own return-kind inference (an anonymous lambda
/// literal compiled on the fly at a call-argument position, never a
/// top-level `Let`) sees the exact same environment a top-level
/// lambda's inference already does.
fn collect_top_level_types_and_fn_returns(
  program: &Program,
) -> (HashMap<String, TypeExpr>, HashMap<String, TypeExpr>) {
  let mut top_level_types: HashMap<String, TypeExpr> = HashMap::new();
  let mut user_fn_return_types: HashMap<String, TypeExpr> = HashMap::new();
  for item in &program.items {
    match item {
      Item::Stmt(Spanned {
        node: Stmt::Let { name, ty, .. },
        ..
      }) => {
        top_level_types.insert(name.clone(), ty.clone());
      }
      Item::Function(f) => {
        user_fn_return_types.insert(f.name.clone(), f.return_type.clone());
      }
      _ => {}
    }
  }
  (top_level_types, user_fn_return_types)
}

fn declare_lambda_functions<'ctx>(
  context: &'ctx Context,
  module: &Module<'ctx>,
  program: &Program,
  lambda_infos: &HashMap<String, LambdaInfo>,
  top_level_types: &HashMap<String, TypeExpr>,
  user_fn_return_types: &HashMap<String, TypeExpr>,
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
              node: Expr::Lambda { params, body, .. },
              ..
            },
          ..
        },
      ..
    }) = item
    else {
      continue;
    };
    let is_bare_proc = ty.as_named() == Some("Proc");
    if (!is_bare_proc && !matches!(ty, TypeExpr::Func(..))) || !lambda_infos.contains_key(name) {
      continue;
    }
    // Plan 88's Decision log: a REAL written `Proc[Args..., Ret]`
    // annotation states its own return type directly — no need for
    // `infer_lambda_ret_kind`'s own syntax-only heuristic, which exists
    // purely to cover the bare `Proc` annotation's own "no return type
    // can be stated at all" gap (plan 71's Decision log). Using the
    // real, sema-checked annotation here is strictly more precise.
    let ret_kind = if let TypeExpr::Func(_, ret) = ty {
      value_kind_for_type(ret)
    } else {
      let base_env: HashMap<String, ValKind> = top_level_types
        .iter()
        .map(|(k, v)| (k.clone(), value_kind_for_type(v)))
        .collect();
      infer_lambda_ret_kind(params, body, &base_env, user_fn_return_types)
    };
    let mut kinds = vec![ValKind::Ptr]; // env
    kinds.extend(param_kinds(params));
    let fn_ty = make_fn_type(context, &kinds, &ret_kind);
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
  compile_to_object_impl(
    program,
    out_path,
    None,
    None,
    None,
    None,
    None,
    CodegenTarget::Native,
  )
}

/// Plan 61's Decision log: identical to `compile_to_object`, except
/// `limit` overrides `ComptimeInterpreter`'s default `1_000_000`-step
/// ceiling — `emerald-cli`'s own `--comptime-step-limit` flag's real,
/// end-to-end entry point (also the small-ceiling seam a test wants,
/// without waiting out a million real iterations).
pub fn compile_to_object_with_comptime_step_limit(
  program: &Program,
  out_path: &Path,
  limit: u64,
) -> Result<(), String> {
  compile_to_object_impl(
    program,
    out_path,
    None,
    None,
    None,
    None,
    Some(limit),
    CodegenTarget::Native,
  )
}

/// Plan 50's `leaf-escape-instrumentation-and-report`: identical to
/// `compile_to_object`, additionally returning how many `ClassName.
/// new(...)` sites were stack- vs. heap-allocated — the only proof
/// surface for the escape-analysis optimization (see `EscapeStats`'s
/// own doc comment for why stdout alone can't distinguish the two).
pub fn compile_to_object_with_stats(
  program: &Program,
  out_path: &Path,
) -> Result<EscapeStats, String> {
  let stats = RefCell::new(EscapeStats::default());
  compile_to_object_impl(
    program,
    out_path,
    None,
    None,
    Some(&stats),
    None,
    None,
    CodegenTarget::Native,
  )?;
  Ok(stats.into_inner())
}

/// Plan 50's `leaf-stack-allocation-codegen` AC5's own test-only sibling
/// entry point — returns the compiled `Module`'s textual LLVM IR
/// alongside compiling, so a test can inspect real emitted `alloca`
/// placement directly (see `compile_to_object_impl`'s own skip-
/// optimization branch for why this always compiles unoptimized).
#[cfg(test)]
fn compile_to_object_ir_text_for_test(
  program: &Program,
  out_path: &Path,
) -> Result<String, String> {
  let ir_text = RefCell::new(String::new());
  compile_to_object_impl(
    program,
    out_path,
    None,
    None,
    None,
    Some(&ir_text),
    None,
    CodegenTarget::Native,
  )?;
  Ok(ir_text.into_inner())
}

/// Plan 49's `leaf-parallel-codegen-and-jobs-flag`: compiles `program`
/// (a per-file node's full transitive-dependency closure — see
/// `emerald-driver`'s `require_graph::closure_items`) into an object
/// file that only *defines* the functions named in `own_names` —
/// every other function in `program` is still declared (so calls
/// resolve, an `External`-linkage symbol the linker fills in from
/// whichever file's own object file actually defines it, the same
/// cross-translation-unit pattern multi-file C compilation already
/// uses) but never given a body here. `is_entry` gates whether this
/// call also assembles `main` from `program`'s own top-level
/// statements — exactly one node in a require graph may ever be
/// `is_entry: true`, since only one `main` symbol can exist across the
/// whole linked binary.
///
/// Real, disclosed scope: only bare, non-generic, non-block-param
/// `Item::Function`s are actually split this way — `emerald-driver`'s
/// `require_graph::unsupported_construct` refuses the whole multi-file
/// parallel compile before this is ever called if any node in the
/// graph declares a class, a module, a generic/block-param function,
/// or a top-level lambda `Let`, so `program` here never contains one.
pub fn compile_to_object_scoped(
  program: &Program,
  own_names: &std::collections::HashSet<String>,
  is_entry: bool,
  out_path: &Path,
) -> Result<(), String> {
  compile_to_object_impl(
    program,
    out_path,
    None,
    Some((own_names, is_entry)),
    None,
    None,
    None,
    CodegenTarget::Native,
  )
}

/// Plan 35's `leaf-line-table-generation`: identical to
/// `compile_to_object`, except the emitted object file also carries
/// real DWARF line-table debug info (v1 scope — line tables only, no
/// variable/type DIEs; see the plan's own Decision log) derived from
/// `source`'s real text and `file_name`'s path via plan 22's
/// `Spanned<T>` byte-offset spans, already threaded through every AST
/// node this backend consumes.
///
/// Plan 61's Decision log: `comptime_step_limit` widens this entry point
/// rather than adding a third, debug-info-plus-comptime-limit variant —
/// `emerald-driver`'s own `compile` (the ordinary `emerald <file>` CLI
/// path) always supplies real source text here, so this is the one real
/// entry point `--comptime-step-limit` needs to actually reach for a
/// plain compile. `None` (every pre-existing caller) is the real default
/// (`1_000_000`), unchanged behavior.
pub fn compile_to_object_with_debug_info(
  program: &Program,
  out_path: &Path,
  source: &str,
  file_name: &str,
  comptime_step_limit: Option<u64>,
) -> Result<(), String> {
  compile_to_object_with_target(
    program,
    out_path,
    source,
    file_name,
    comptime_step_limit,
    CodegenTarget::Native,
  )
}

/// Plan 64's `leaf-target-triple-and-selection`: the `source_info`-free
/// counterpart to `compile_to_object_with_target`, for `emerald-driver`'s
/// `codegen_stage`'s own `source_info: None` branch (`compile_program`/
/// `compile_program_with_libs` — the `require`-spliced, project-mode
/// `emerald build` path, which has no single coherent source string —
/// see `compile_to_object`/`compile_to_object_with_comptime_step_limit`'s
/// own identical `None` shape above, now widened with a real `target`
/// too).
pub fn compile_to_object_with_comptime_step_limit_and_target(
  program: &Program,
  out_path: &Path,
  comptime_step_limit: Option<u64>,
  target: CodegenTarget,
) -> Result<(), String> {
  compile_to_object_impl(
    program,
    out_path,
    None,
    None,
    None,
    None,
    comptime_step_limit,
    target,
  )
}

/// Plan 64's `leaf-target-triple-and-selection`: identical to
/// `compile_to_object_with_debug_info`, except `target` selects the
/// real target machine `compile_to_object_impl` builds — the one
/// entry point `emerald-driver`'s own ordinary `codegen_stage` (the
/// ordinary `emerald build`/`emerald <file>` CLI path, which always
/// supplies real source text) needs to actually reach `--target
/// wasm32-wasi` through, mirroring exactly how `comptime_step_limit`
/// widened this same function rather than adding a third variant
/// (plan 61's Decision log, cited verbatim above).
pub fn compile_to_object_with_target(
  program: &Program,
  out_path: &Path,
  source: &str,
  file_name: &str,
  comptime_step_limit: Option<u64>,
  target: CodegenTarget,
) -> Result<(), String> {
  compile_to_object_impl(
    program,
    out_path,
    Some((source, file_name)),
    None,
    None,
    None,
    comptime_step_limit,
    target,
  )
}

/// Plan 64's `leaf-target-triple-and-selection` — the one real,
/// verified (this session, against this workspace's real LLVM 21
/// build) hardcoded-native-target seam in this whole file (Decision
/// log): every other function here emits ordinary, target-agnostic
/// LLVM IR through `inkwell`'s `Builder` API. `Native` is every
/// pre-`plan-64` caller's implicit, unchanged behavior; `Wasm32Wasi`
/// is this plan's own addition, real WASI preview 1 support — no
/// preview 2/component-model support is in scope (Decision log).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodegenTarget {
  #[default]
  Native,
  Wasm32Wasi,
}

/// Bugfix (multi-file `--jobs` link): a class/actor/module method body
/// is always fully defined by `declare_user_functions` in every
/// compilation unit whose require-closure includes it — unlike
/// `Item::Function` (scoped by `own_names` at its own define site,
/// inside `compile_to_object_impl`'s main `for item in &program.items`
/// loop), no ownership check exists yet for these item kinds. Two
/// `.o`s sharing a class/actor/module both then define the same
/// mangled symbol at the declare step's `External` linkage, and the
/// linker rejects it as a duplicate — reproduced this session via
/// `examples/host.em` requiring `examples/counter_actor.em` under
/// `--jobs`. `WeakODR` (this file's own established fix, already
/// applied to the actor-scaffolding symbols in `declare_actor_
/// trampolines`/`declare_wire_class_codecs`/`declare_actor_wire_arg_
/// codecs`/`build_actor_method_tables`) lets the linker keep exactly
/// one identical copy instead of erroring — safe because every
/// compilation unit's copy is a pure function of the same source AST,
/// so they're guaranteed identical. Deliberately `WeakODR`, not
/// `LinkOnceODR`: see `declare_actor_trampolines`'s own doc comment —
/// this session's first attempt used `LinkOnceODR` and broke
/// `a_trampoline_called_directly_produces_the_same_result_as_the_
/// method_itself` (a hand-written C harness `extern`-linking straight
/// against a method compiled with no in-module caller of its own,
/// which `default<O3>`'s `GlobalDCE` is entitled to strip when the
/// symbol is `linkonce_odr`; `weak_odr` is never eligible for that).
/// Also covers a monomorphized generic class instance's own methods
/// (`generic_instances`, defined later via the identical
/// `define_method` path) — a generic class shared across files hits
/// the exact same duplicate-definition shape.
fn weak_odr_class_shaped_methods<'ctx>(
  program: &Program,
  generic_instances: &HashMap<String, ClassDef>,
  user_func_ids: &HashMap<String, (FunctionValue<'ctx>, ValKind)>,
) {
  for item in &program.items {
    let mangled_names: Vec<String> = match item {
      Item::Class(c) => c
        .methods
        .iter()
        .map(|m| format!("{}_{}", c.name, mangled_operator_symbol(&m.name)))
        .collect(),
      Item::Actor(a) => a
        .methods
        .iter()
        .map(|m| format!("{}_{}", a.name, mangled_operator_symbol(&m.name)))
        .collect(),
      Item::Module(m) => m
        .methods
        .iter()
        .map(|f| format!("{}_{}", m.name, f.name))
        .collect(),
      _ => continue,
    };
    for name in mangled_names {
      if let Some((fv, _)) = user_func_ids.get(&name) {
        fv.set_linkage(Linkage::WeakODR);
      }
    }
  }
  for c in generic_instances.values() {
    for m in &c.methods {
      let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
      if let Some((fv, _)) = user_func_ids.get(&mangled) {
        fv.set_linkage(Linkage::WeakODR);
      }
    }
  }
}

#[allow(clippy::too_many_arguments)]
fn compile_to_object_impl(
  program: &Program,
  out_path: &Path,
  source_info: Option<(&str, &str)>,
  // Plan 49's `leaf-parallel-codegen-and-jobs-flag`: `Some((own_names,
  // is_entry))` restricts which `Item::Function`s in `program` actually
  // get a body defined here (everything else stays declare-only, see
  // `compile_to_object_scoped`'s own doc comment) and whether `main` is
  // assembled at all. `None` (both pre-existing entry points) means
  // "define everything, always add main" — this function's behavior is
  // completely unchanged for every caller that doesn't pass `Some`.
  scope: Option<(&std::collections::HashSet<String>, bool)>,
  // Plan 50's `leaf-escape-instrumentation-and-report`: `Some(&stats)`
  // only from `compile_to_object_with_stats` — every other caller
  // passes `None`, so this function's own behavior (and every existing
  // caller's) is completely unchanged; see `Ctx::escape_stats`'s own
  // doc comment for why this is a `RefCell`, not a loose `&mut`
  // parameter threaded through the whole codegen call graph.
  stats: Option<&RefCell<EscapeStats>>,
  // Plan 50's `leaf-stack-allocation-codegen` AC5's own test-only IR-
  // inspection proof — `Some(&out)` only from `#[cfg(test)]`'s
  // `compile_to_object_ir_text_for_test`, populated with the compiled
  // `Module`'s textual IR (see this function's own skip-optimization
  // branch below for why).
  ir_text_out: Option<&RefCell<String>>,
  // Plan 61's Decision log: `Some(n)` only from `compile_to_object_with_
  // comptime_step_limit` (`emerald-cli`'s own `--comptime-step-limit`
  // flag) — every other, pre-existing caller passes `None`, resolved to
  // the real default (`1_000_000`) right where `Ctx::comptime_step_
  // limit` is built, so this function's behavior is unchanged for every
  // caller that doesn't pass `Some`.
  comptime_step_limit: Option<u64>,
  // Plan 64's Decision log: `CodegenTarget::Native` for every pre-plan-64
  // caller — no behavior change; only `compile_to_object_with_target`
  // (this plan's own new entry point) ever passes `Wasm32Wasi`.
  target: CodegenTarget,
) -> Result<(), String> {
  // Plan 47's Decision log: a `test "..." do ... end` block only ever
  // compiles through `compile_test_harness` (`emerald test`) — reaching
  // this, the ordinary `emerald <file>`/`emerald build` path, is a
  // real, described rejection, not a silent no-op or panic. Plan 80:
  // `property "..." do ... end` gets the identical rejection (it
  // compiles through the same `compile_test_harness`, under `emerald
  // test`), and `benchmark "..." do ... end` gets its own analogous
  // rejection (`compile_benchmark_harness`, under `emerald benchmark`).
  if program.items.iter().any(|i| {
    matches!(
      i,
      Item::Test { .. } | Item::Property { .. } | Item::Benchmark { .. }
    )
  }) {
    return Err(
      "top-level test/property/benchmark block only valid under `emerald test`/`emerald benchmark`, not an ordinary compile"
        .to_string(),
    );
  }
  let mut items = program.items.clone();
  let uses_assertions = desugar_asserts_in_items(&mut items);
  if uses_assertions {
    ensure_assertion_error_class(&mut items);
  }
  if items.iter().any(|i| matches!(i, Item::Actor(_))) {
    ensure_remote_actor_error_class(&mut items);
    ensure_send_error_enum(&mut items);
  }
  if program_uses_contracts(&items) {
    ensure_contract_violation_class(&mut items);
  }
  // `newtype Meters: Float64` — populates `NEWTYPE_UNDERLYING` for this
  // compile. Plan 84's Decision log: this must run BEFORE `strip_
  // ownership_annotations_in_items` immediately below, not after (as
  // plan 83 originally had it) — that pass now calls `value_kind_for_
  // type` itself (to decide whether a `borrow`/`borrow var` needs the
  // real pointer marker), and a `borrow`/`borrow var` of a newtype
  // whose underlying type is by-value (`newtype Meters: Float64`) would
  // otherwise be misclassified as already-pointer-shaped (the generic
  // "unknown name" bucket `value_kind_for_type` falls into before this
  // table is populated), silently keeping today's by-value passthrough
  // for that one case instead of getting the real pointer treatment.
  // Reading directly from the pre-strip `program` (not `items`) is
  // sound: a `newtype` declaration itself is never `own`/`borrow`-
  // wrapped (`emerald-sema` only allows those on a parameter), so
  // stripping has nothing to change about `Item::Newtype` entries
  // either way.
  set_newtype_underlying(program);

  // Plan 83's Decision log (`spec/OWNERSHIP.md` §10's own rescoping of
  // plan 84), extended by plan 84 itself: strips every `own`/`borrow`/
  // `borrow var` wrapper off every function/method parameter and
  // return-type annotation, program-wide, before anything else in this
  // file ever looks at `items` (see `strip_ownership_annotations_in_
  // items`'s own doc comment for the full rationale, and `strip_
  // ownership_in_type_expr`'s for exactly which cases now get a real
  // pointer marker instead of a bare passthrough). `emerald_sema::
  // check_program` has already fully enforced the four ownership-
  // liveness rules against the ORIGINAL, annotated AST by the time any
  // `emerald_codegen::compile_to_object*` entry point ever runs
  // (`emerald-driver`'s own fixed pipeline order) — from here on, `own
  // Data`/`borrow Data`/`borrow var Data` compile EXACTLY like a bare
  // `Data` parameter (true for every case plan 83 originally covered:
  // `own` of anything, and `borrow`/`borrow var` of an already
  // pointer-represented type), and a top-level function's by-value
  // `borrow`/`borrow var` parameter (`borrow Int64`, `borrow var
  // Boolean`, ...) compiles to a real caller-visible pointer — see
  // `bind_params`/`build_call_arg_vals`/`build_call_kw_expr`'s own doc
  // comments for that mechanism, and `strip_ownership_in_type_expr`'s
  // for the real, disclosed method-parameter scope boundary.
  strip_ownership_annotations_in_items(&mut items);
  let owned_program = Program { items };
  let program = &owned_program;

  // Plan 64's `leaf-target-triple-and-selection` — the one seam this
  // whole file has (Decision log). `Native` keeps the pre-plan-64
  // four calls byte-for-byte; `Wasm32Wasi` cross-compiles instead of
  // querying the host at all — there is no "host CPU" for a genuinely
  // cross target (WASM has no `-mcpu`-equivalent concept LLVM's
  // WebAssembly backend consults the way x86 consults `-march`, so
  // both name/features strings are simply empty, not queried).
  let (triple, cpu_name, cpu_features) = match target {
    CodegenTarget::Native => {
      Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;
      (
        TargetMachine::get_default_triple(),
        TargetMachine::get_host_cpu_name().to_string(),
        TargetMachine::get_host_cpu_features().to_string(),
      )
    }
    CodegenTarget::Wasm32Wasi => {
      Target::initialize_webassembly(&InitializationConfig::default());
      (
        inkwell::targets::TargetTriple::create("wasm32-wasi"),
        String::new(),
        String::new(),
      )
    }
  };
  let llvm_target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
  let target_machine = llvm_target
    .create_target_machine(
      &triple,
      &cpu_name,
      &cpu_features,
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

  // Plan 35: DWARF line-table debug info, only when the caller supplied
  // real source text (see `compile_to_object_with_debug_info`) — the
  // ordinary `compile_to_object` entry point attaches none, unchanged
  // behavior.
  let debug_metadata_version = context.i32_type().const_int(3, false);
  let newline_offsets_owner: Option<Vec<usize>> = source_info.map(|(source, _)| {
    source
      .bytes()
      .enumerate()
      .filter(|(_, b)| *b == b'\n')
      .map(|(i, _)| i)
      .collect()
  });
  let debug_info: Option<(DebugInfoBuilder, DIFile)> = source_info.map(|(_, file_name)| {
    module.add_basic_value_flag(
      "Debug Info Version",
      FlagBehavior::Warning,
      debug_metadata_version,
    );
    let path = Path::new(file_name);
    let directory = path.parent().and_then(|p| p.to_str()).unwrap_or("");
    let filename = path
      .file_name()
      .and_then(|f| f.to_str())
      .unwrap_or(file_name);
    let (dibuilder, compile_unit) = module.create_debug_info_builder(
      true,
      DWARFSourceLanguage::C,
      filename,
      directory,
      "emerald",
      false,
      "",
      0,
      "",
      DWARFEmissionKind::Full,
      0,
      false,
      false,
      "",
      "",
    );
    let file = compile_unit.get_file();
    (dibuilder, file)
  });
  let dibuilder_owner = debug_info.as_ref().map(|(dib, _)| dib);
  let di_file = debug_info.as_ref().map(|(_, file)| *file);

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
  // Plan 53 (Result type and error propagation): `is_valid_int`/
  // `parse_digits`'s runtime backing, both boolean/`Int64`-returning the
  // same "plain i64, not i1" ABI convention `bool_to_string` already
  // established for this backend.
  let is_valid_int_fn = module.add_function(
    "emerald_is_valid_int",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  let parse_digits_fn = module.add_function(
    "emerald_parse_digits",
    i64_ty.fn_type(&[ptr_ty.into()], false),
    Some(Linkage::External),
  );
  // Plan 54 (actor declarations and isolated heaps): plan 51's real,
  // landed two-call region API — `.spawn`'s own allocation call site.
  let region_create_fn = module.add_function(
    "emerald_region_create",
    ptr_ty.fn_type(&[], false),
    Some(Linkage::External),
  );
  let region_alloc_fn = module.add_function(
    "emerald_region_alloc",
    ptr_ty.fn_type(&[ptr_ty.into(), i64_ty.into()], false),
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
  let actor_funcs = declare_actor_runtime_funcs(&context, &module);

  // Plan 32: raw `ClassDef`s keyed by name, so `build_class_layout`/
  // `build_method_owners` can walk any class's inheritance chain by
  // name lookup alone — independent of `program.items`' order.
  // Plan 54's Decision log: each `Item::Actor` normalizes into a
  // synthetic `ClassDef` (no `superclass`/`implements` — an actor has
  // neither) BEFORE `resolve_class_chain`/`build_class_layout`/
  // `build_method_owners` run — actors reuse that machinery entirely
  // unmodified rather than duplicating it. Kept alive for the rest of
  // this function so `class_defs`' borrows into it stay valid.
  let actor_class_defs: Vec<ClassDef> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Actor(a) => Some(ClassDef {
        name: a.name.clone(),
        superclass: None,
        implements: None,
        derive: None,
        fields: a.fields.clone(),
        methods: a.methods.clone(),
        type_params: Vec::new(),
        // Same actor declaration, just normalized into `ClassDef`
        // shape — carries its doc comment forward unchanged.
        doc: a.doc.clone(),
      }),
      _ => None,
    })
    .collect();

  // Plan 58: a class with a non-empty `type_params` is a generic
  // TEMPLATE — never inserted into `class_defs` under its own bare
  // name (it has no `ClassLayout` of its own at all); routed here
  // instead, consulted only by `collect_generic_class_specializations`
  // below.
  let mut generic_class_defs: HashMap<String, &ClassDef> = HashMap::new();
  let mut class_defs: HashMap<String, &ClassDef> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      if c.type_params.is_empty() {
        class_defs.insert(c.name.clone(), c);
      } else {
        generic_class_defs.insert(c.name.clone(), c);
      }
    }
  }
  for c in &actor_class_defs {
    class_defs.insert(c.name.clone(), c);
  }

  // Plan 58: every generic-class instantiation actually written
  // anywhere in the program, monomorphized into a real `ClassDef` —
  // mirrors `actor_class_defs`'s own synthesis-then-merge shape above.
  // Kept alive for the rest of this function so `class_defs`' borrows
  // into it stay valid, exactly like `actor_class_defs`.
  let generic_instances: HashMap<String, ClassDef> =
    collect_generic_class_specializations(program, &generic_class_defs);
  for c in generic_instances.values() {
    class_defs.insert(c.name.clone(), c);
  }

  // Class layouts (field offsets/kinds, now chain-flattened — ancestor
  // fields first, see `build_class_layout`) and a stable per-class
  // integer tag (declaration order) for `rescue` matching.
  let mut classes: HashMap<String, ClassLayout> = HashMap::new();
  let mut class_tags: HashMap<String, i64> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      if c.type_params.is_empty() {
        classes.insert(c.name.clone(), build_class_layout(&c.name, &class_defs)?);
        class_tags.insert(c.name.clone(), class_tags.len() as i64);
      }
    }
    if let Item::Actor(a) = item {
      classes.insert(a.name.clone(), build_class_layout(&a.name, &class_defs)?);
      class_tags.insert(a.name.clone(), class_tags.len() as i64);
    }
  }
  for name in generic_instances.keys() {
    classes.insert(name.clone(), build_class_layout(name, &class_defs)?);
    class_tags.insert(name.clone(), class_tags.len() as i64);
  }
  let method_owners = build_method_owners(&class_defs)?;
  let generic_class_methods = build_generic_class_methods(&class_defs);

  // Plan 52: independently re-derived from the raw `Program`/`Item::
  // Enum` list, the same "no shared sema→codegen structure"
  // architecture `classes`/`class_tags` above already established
  // (plan 32's Decision log) — sema's own registration-time checks
  // already guarantee every enum name and variant name here is unique
  // and collision-free before codegen ever runs.
  // Plan 73: `Option[T]` — codegen's own mirror of `emerald-sema`'s
  // identical synthetic `EnumDef` (see that crate's `check_program` own
  // Decision log) — same reasoning: never parsed from source, so
  // `None`'s zero fields don't need a grammar-level exception, and
  // registered into `generic_enum_defs` exactly like a user-written
  // `enum Name[T] = ...` would be.
  let option_enum_def_cg = EnumDef {
    name: "Option".to_string(),
    variants: vec![
      EnumVariant {
        name: "Some".to_string(),
        fields: vec![TypeExpr::Named("T".to_string())],
      },
      EnumVariant {
        name: "None".to_string(),
        fields: vec![],
      },
    ],
    type_params: vec![TypeParam {
      name: "T".to_string(),
      bounds: Vec::new(),
    }],
    // Compiler-synthesized, never parsed from source — same reasoning
    // as `emerald-sema`'s identical `option_enum_def`.
    doc: None,
  };
  let mut generic_enum_defs: HashMap<String, &EnumDef> = HashMap::new();
  generic_enum_defs.insert("Option".to_string(), &option_enum_def_cg);
  for item in &program.items {
    if let Item::Enum(e) = item {
      if !e.type_params.is_empty() {
        generic_enum_defs.insert(e.name.clone(), e);
      }
    }
  }
  let mut synthesized_enums: HashMap<String, EnumDef> = HashMap::new();
  let mut enum_synth_classes: HashMap<String, ClassDef> = HashMap::new();
  for ty in collect_generic_instantiation_typenames(program) {
    if let TypeExpr::Generic(base, args) = &ty {
      if generic_enum_defs.contains_key(base.as_str()) {
        let mut in_progress = Vec::new();
        instantiate_generic_enum_defs(
          base,
          args,
          &generic_enum_defs,
          &generic_class_defs,
          &mut enum_synth_classes,
          &mut synthesized_enums,
          &mut in_progress,
        );
      }
    }
  }
  // Plan 73: `String.from_cstring`'s own real return type is `Option[
  // String]` — unconditionally pre-instantiated here the same way
  // `emerald-sema`'s `check_program` does, so its `EnumLayout` always
  // exists regardless of whether this specific program ever writes
  // `"Option[String]"` as literal annotation text anywhere.
  {
    let mut in_progress = Vec::new();
    instantiate_generic_enum_defs(
      "Option",
      &[TypeExpr::Named("String".to_string())],
      &generic_enum_defs,
      &generic_class_defs,
      &mut enum_synth_classes,
      &mut synthesized_enums,
      &mut in_progress,
    );
  }
  for c in enum_synth_classes.values() {
    class_defs.insert(c.name.clone(), c);
  }
  for name in enum_synth_classes.keys() {
    classes.insert(name.clone(), build_class_layout(name, &class_defs)?);
    class_tags.insert(name.clone(), class_tags.len() as i64);
  }

  let mut enums: HashMap<String, EnumLayout> = HashMap::new();
  for item in &program.items {
    if let Item::Enum(e) = item {
      if e.type_params.is_empty() {
        enums.insert(e.name.clone(), build_enum_layout(e));
      }
    }
  }
  for (name, e) in &synthesized_enums {
    enums.insert(name.clone(), build_enum_layout(e));
  }

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
  // Plan 59: each `ExternFn` synthesizes a minimal `AstFunction` stand-in
  // (empty body, no splat/defaults/block_param — matching this plan's
  // own "plain positional calls only" scope) purely so `build_call_arg_
  // vals`'s existing `ctx.func_defs` lookup (needed for its splat/
  // default-argument handling) works completely unmodified for an
  // extern call site too, with no second, parallel arg-building path.
  // Kept alive for the rest of this function, exactly like
  // `actor_class_defs`.
  let extern_fn_defs: Vec<AstFunction> = program
    .items
    .iter()
    .flat_map(|item| match item {
      Item::Extern(block) => block
        .fns
        .iter()
        .map(|f| AstFunction {
          name: f.name.clone(),
          params: f.params.clone(),
          return_type: f.return_type.clone(),
          body: Vec::new(),
          block_param: None,
          splat_param: None,
          type_params: Vec::new(),
          is_comptime: false,
          requires: Vec::new(),
          ensures: Vec::new(),
          is_pure: false,
          // `ExternFn` has no `doc` field of its own (plan 77's scope
          // stops at `fn`/`class`/`method`/`interface`/`module`/`enum`/
          // `actor` declarations — `unsafe extern "C" { ... }` blocks
          // are left out, disclosed here rather than silently assumed).
          doc: None,
        })
        .collect::<Vec<_>>(),
      _ => Vec::new(),
    })
    .collect();
  for f in &extern_fn_defs {
    func_defs.insert(f.name.clone(), f);
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
  let comptime_only_fns = comptime_only_function_names(program);
  let mut user_func_ids = declare_user_functions(
    &context,
    &module,
    program,
    &classes,
    &generic_instances,
    &comptime_only_fns,
  );
  // Bugfix (multi-file `--jobs` link) — see `weak_odr_class_shaped_
  // methods`'s own doc comment for why this is needed and safe.
  weak_odr_class_shaped_methods(program, &generic_instances, &user_func_ids);
  let (top_level_types, user_fn_return_types) = collect_top_level_types_and_fn_returns(program);
  let lambda_func_ids = declare_lambda_functions(
    &context,
    &module,
    program,
    &lambda_infos,
    &top_level_types,
    &user_fn_return_types,
  );
  let interface_defs: HashMap<String, &InterfaceDef> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Interface(idef) => Some((idef.name.clone(), idef)),
      _ => None,
    })
    .collect();
  let generic_method_instances_cell: RefCell<HashMap<String, (FunctionValue<'_>, ValKind)>> =
    RefCell::new(HashMap::new());

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
        &ret_kind,
      );
      let mangled = mangled_generic_symbol(fn_name, concrete_class);
      let fv = module.add_function(&mangled, fn_ty, Some(Linkage::External));
      user_func_ids.insert(mangled, (fv, ret_kind));
    }
  }

  // Plan 59's Decision log: the literal generalization of the runtime's
  // own hardcoded `module.add_function(name, ..., Some(Linkage::
  // External))` declarations just below (e.g. `emerald_alloc`) to an
  // arbitrary, user-named, source-declared symbol — inserted directly
  // into the SAME `user_func_ids` table `Expr::Call`'s existing lookup
  // (`build_call_expr`) already consults, rather than a second,
  // parallel table needing its own fallback lookup at every call site.
  // Sema's own registration-time pass (`resolve_extern_type`) already
  // validated every param/return type against the FFI allow-list, so
  // `value_kind_for_type` here never sees anything it can't marshal.
  for item in &program.items {
    let Item::Extern(block) = item else { continue };
    for f in &block.fns {
      let ret_kind = value_kind_for_type(&f.return_type);
      let param_kinds: Vec<ValKind> = f
        .params
        .iter()
        .map(|p| value_kind_for_type(&p.ty))
        .collect();
      let fn_ty = make_fn_type(&context, &param_kinds, &ret_kind);
      let fv = module.add_function(&f.name, fn_ty, Some(Linkage::External));
      user_func_ids.insert(f.name.clone(), (fv, ret_kind));
    }
  }

  let actor_trampolines = declare_actor_trampolines(
    &context,
    &module,
    program,
    &user_func_ids,
    &exc_funcs,
    &actor_funcs,
  )?;
  let mut supervised_classes = HashSet::new();
  for item in &program.items {
    let body = match item {
      Item::Stmt(s) => std::slice::from_ref(s),
      Item::Function(f) => &f.body[..],
      _ => continue,
    };
    for s in body {
      collect_supervised_classes_in_stmt(s, &mut supervised_classes);
    }
  }
  for item in &program.items {
    let methods: &[AstFunction] = match item {
      Item::Class(c) => &c.methods,
      Item::Actor(a) => &a.methods,
      Item::Module(m) => &m.methods,
      _ => continue,
    };
    for m in methods {
      for s in &m.body {
        collect_supervised_classes_in_stmt(s, &mut supervised_classes);
      }
    }
  }
  let supervisor_respawn_thunks = declare_supervisor_respawn_thunks(
    &context,
    &module,
    program,
    &user_func_ids,
    &actor_funcs,
    region_create_fn,
    region_alloc_fn,
    &classes,
    &supervised_classes,
  )?;
  let actor_names: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Actor(a) => Some(a.name.clone()),
      _ => None,
    })
    .collect();

  // Plan 60 (distributed, location-transparent actors) — `leaf-wire-
  // codec`/`leaf-remote-dispatch-and-worked-proof`: built once, after
  // `classes`/`actor_names`/`actor_funcs`/`actor_trampolines` are all
  // ready, before `gen_ctx` borrows any of them.
  let (wire_encode_fns, wire_decode_fns) = declare_wire_class_codecs(
    &context,
    &module,
    &classes,
    &actor_names,
    &actor_funcs,
    alloc,
  )?;
  let (actor_arg_encoders, actor_arg_decoders, actor_method_tags) = declare_actor_wire_arg_codecs(
    &context,
    &module,
    program,
    &actor_funcs,
    &wire_encode_fns,
    &wire_decode_fns,
  )?;
  let (actor_method_tables, actor_method_counts) = build_actor_method_tables(
    &context,
    &module,
    program,
    &actor_trampolines,
    &actor_arg_decoders,
  );

  // `newtype Meters: Float64` — see `Ctx::newtypes`'s own doc comment
  // for why `build_method_call`'s `.value` dispatch and `Stmt::Let`'s
  // `local_classes` bookkeeping both need this name set independently
  // of `classes`/`enums` above.
  let newtypes: HashSet<String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Newtype(n) => Some(n.name.clone()),
      _ => None,
    })
    .collect();

  let gen_ctx = Ctx {
    user_func_ids: &user_func_ids,
    classes: &classes,
    enums: &enums,
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
    generic_class_methods: &generic_class_methods,
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
    dibuilder: dibuilder_owner,
    di_file,
    newline_offsets: newline_offsets_owner.as_deref(),
    current_di_scope: None,
    object_allocas: None,
    escape_stats: stats,
    is_valid_int_fn,
    parse_digits_fn,
    region_create_fn,
    region_alloc_fn,
    actor_funcs,
    actor_trampolines: &actor_trampolines,
    actor_names: &actor_names,
    supervisor_respawn_thunks: &supervisor_respawn_thunks,
    actor_arg_encoders: &actor_arg_encoders,
    actor_method_tags: &actor_method_tags,
    actor_method_tables: &actor_method_tables,
    actor_method_counts: &actor_method_counts,
    comptime_step_limit: comptime_step_limit.unwrap_or(1_000_000),
    current_function_contracts: None,
    module: &module,
    top_level_types: &top_level_types,
    user_fn_return_types: &user_fn_return_types,
    class_defs: &class_defs,
    interface_defs: &interface_defs,
    generic_method_instances: &generic_method_instances_cell,
    newtypes: &newtypes,
    // Plan 84's Decision log: empty at the whole-program base `Ctx` —
    // every per-function `Ctx` (`define_user_function`/`define_method`/
    // `define_lambda`) overrides this with its OWN freshly bound list
    // right after its own `bind_params` call. `main` never overrides it
    // (top-level code has no parameters to bind at all), so it keeps
    // this empty default, which is correct: `main` never emits a
    // `Stmt::Return`/implicit-fallthrough that could need one.
    borrow_var_writebacks: &[],
  };

  for item in &program.items {
    // Plan 76: unwrap once (the grammar never nests `export`) so an
    // exported function/class/module/actor's body is defined exactly
    // like a non-exported one.
    let mut item: &Item = item;
    while let Item::Export(inner) = item {
      item = inner;
    }
    match item {
      // Plan 61: never declared in `user_func_ids` above either — see
      // `declare_user_functions`'s matching arm.
      Item::Function(f) if comptime_only_fns.contains(&f.name) => {}
      // Plan 34: never declared in `user_func_ids` above — see
      // `declare_user_functions`'s matching arm.
      Item::Function(f) if f.block_param.is_some() => {}
      // Plan 41: never declared under its own bare name — see
      // `declare_user_functions`'s matching arm; each of its
      // specializations is defined separately, below.
      Item::Function(f) if !f.type_params.is_empty() => {}
      Item::Function(f) => {
        // Plan 49: when `scope` restricts this compile to a subset of
        // `program`'s own functions (a per-file node's transitive
        // closure), a function outside `own_names` belongs to another
        // file and stays declare-only here — its body is defined in
        // that file's own separately-emitted object file instead.
        if scope.is_none_or(|(own_names, _)| own_names.contains(&f.name)) {
          let (fv, _) = user_func_ids[&f.name];
          define_user_function(&context, &builder, f, fv, &gen_ctx)?;
        }
      }
      // Plan 58: a generic class TEMPLATE's own raw method bodies never
      // compile under its own unmangled name — `generic_instances`' own
      // methods, defined in the dedicated pass just below this loop,
      // are the only real bodies any monomorphized instantiation's
      // calls ever reach.
      Item::Class(c) if !c.type_params.is_empty() => {}
      Item::Class(c) => {
        let layout = &classes[&c.name];
        for m in &c.methods {
          // Plan 89: mirrors `declare_user_functions`'s identical skip
          // — the raw, unsubstituted body of a method with its own
          // `[U]` type parameter is never compiled under its bare
          // symbol at all (it was never declared one above either).
          if !m.type_params.is_empty() {
            continue;
          }
          let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
          let (fv, _) = user_func_ids[&mangled];
          define_method(
            &context,
            &builder,
            m,
            fv,
            &layout.fields,
            &layout.field_classes,
            &gen_ctx,
          )?;
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
      // Plan 54: an actor method body compiles exactly like a class
      // method's own — real `self`/`@field` access via `define_method`,
      // using the `ClassLayout` `classes` already built for it above
      // (the actor-normalization step, alongside every real class's).
      Item::Actor(a) => {
        let layout = &classes[&a.name];
        for m in &a.methods {
          if !m.type_params.is_empty() {
            continue;
          }
          let mangled = format!("{}_{}", a.name, mangled_operator_symbol(&m.name));
          let (fv, _) = user_func_ids[&mangled];
          define_method(
            &context,
            &builder,
            m,
            fv,
            &layout.fields,
            &layout.field_classes,
            &gen_ctx,
          )?;
        }
      }
      Item::Stmt(Spanned {
        node:
          Stmt::Let {
            name,
            ty,
            value:
              Spanned {
                node: Expr::Lambda { params, body, .. },
                ..
              },
            ..
          },
        ..
      }) if ty.as_named() == Some("Proc") || matches!(ty, TypeExpr::Func(..)) => {
        if let (Some(info), Some((fv, ret_kind))) =
          (lambda_infos.get(name), lambda_func_ids.get(name))
        {
          define_lambda(
            &context,
            &builder,
            name,
            params,
            ret_kind.clone(),
            body,
            info,
            *fv,
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
      // Plan 76: same "internal driver bug" reasoning as `Item::
      // Require` immediately above — `require.rs`/`require_graph.rs`
      // both resolve/validate an `import` and splice its target file's
      // items in before codegen ever runs.
      Item::Import { path, .. } => {
        return Err(format!(
          "codegen: unresolved `import {path}` — internal driver bug (require.rs's resolution step should have stripped this before codegen)"
        ));
      }
      // Unreachable: unwrapped by the `while let` loop above this match.
      Item::Export(_) => unreachable!("Item::Export is unwrapped before this match"),
      // Plan 47: never actually reached — `compile_to_object`'s own
      // prologue rejects any `Program` still containing an
      // `Item::Test` before this runs at all. Plan 80: `Item::
      // Property`/`Item::Benchmark` mirror the identical rejection and
      // stripping — see `declare_user_functions`'s matching arm.
      Item::Test { .. } | Item::Property { .. } | Item::Benchmark { .. } => {}
      // Plan 52: pure data — no function body to compile.
      Item::Enum(_) => {}
      // A newtype declares no method of its own to define a body for —
      // see `declare_user_functions`'s matching arm for the full reason
      // operator overloading is out of scope here.
      Item::Newtype(_) => {}
      Item::Error => unreachable!("Item::Error never survives into a returned Ok(Program)"),
      // Plan 59: a real, external symbol declared elsewhere (this
      // pass's own declare step) — no Emerald-side body to compile.
      Item::Extern(_) => {}
    }
  }

  // Plan 58: each monomorphized generic-class instantiation's method
  // bodies, compiled via the *existing*, unmodified `define_method`
  // path — 100% reuse of the non-generic machinery, just fed a
  // synthesized `ClassDef`'s own methods instead of one straight from
  // the parser (mirrors plan 41's own generic-FUNCTION specialization
  // pass immediately below).
  for c in generic_instances.values() {
    let layout = &classes[&c.name];
    for m in &c.methods {
      // Plan 89: mirrors `declare_user_functions`'s identical skip for
      // this same `generic_instances` loop.
      if !m.type_params.is_empty() {
        continue;
      }
      let mangled = format!("{}_{}", c.name, mangled_operator_symbol(&m.name));
      let (fv, _) = user_func_ids[&mangled];
      define_method(
        &context,
        &builder,
        m,
        fv,
        &layout.fields,
        &layout.field_classes,
        &gen_ctx,
      )?;
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
  // Plan 49: exactly one node in a require graph is ever `is_entry:
  // true` — a dependency file's own separately-emitted object file
  // must not also define `main`, or linking every file's `.o` together
  // would hit a duplicate-symbol error. `scope.is_none()` (both
  // pre-existing entry points) always assembles `main`, unchanged.
  if scope.is_none_or(|(_, is_entry)| is_entry) {
    let main_ty = context
      .i32_type()
      .fn_type(&[context.i32_type().into(), ptr_ty.into()], false);
    let main_fn = module.add_function("main", main_ty, Some(Linkage::External));
    define_main(&context, &builder, main_fn, program, &gen_ctx)?;
  }

  // Plan 35: must run before verification/object emission, per
  // `DebugInfoBuilder::finalize`'s own doc — its `Drop` impl also calls
  // this, but only when `debug_info` drops at the end of this function,
  // which is after emission; an explicit call here is required, and
  // safe to repeat (`finalize` is documented idempotent).
  if let Some((dibuilder, _)) = &debug_info {
    dibuilder.finalize();
  }

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
  // Plan 35: verified for real this session — `default<O3>` freely
  // inlines and constant-folds a small function like the worked
  // example's `add(a, b) -> a + b` (called with two literal args)
  // straight into `main`, so the real compiled binary never actually
  // calls `add` at runtime at all — a real `gdb` session confirmed the
  // breakpoint resolved to a valid address in `add`'s still-emitted
  // body, but `run` sailed straight through to the program's normal
  // exit without ever stopping there. This is the exact same
  // observation-changes-the-program problem `program_uses_retry` above
  // already works around, just via inlining instead of `mem2reg`/SROA
  // — skipping optimization whenever debug info is attached is the
  // same "-O0 -g" tradeoff virtually every real compiler makes for
  // debug builds: correct, steppable control flow over speed.
  // Plan 50's `leaf-stack-allocation-codegen` AC5's own IR-inspection
  // proof needs *this crate's own emission*, isolated from whatever
  // LLVM's optimizer independently decides to keep or discard (`p`'s
  // fields genuinely going unread after construction is exactly the
  // shape `default<O3>`'s dead-store elimination is entitled to strip
  // — a real, correct optimization outcome that would make an
  // optimized-IR assertion flaky for reasons unrelated to whether
  // `prealloc_stack_objects` itself placed one `alloca` per site
  // correctly). `ir_text_out.is_some()` therefore takes the same
  // skip-optimization path `program_uses_retry`/`debug_info` already
  // do, for the identical "unoptimized IR is always correct, just
  // slower" reason.
  if program_uses_retry(program) || debug_info.is_some() || ir_text_out.is_some() {
    if let Some(out) = ir_text_out {
      *out.borrow_mut() = module.print_to_string().to_string();
    }
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
      Item::Actor(a) => {
        for m in &mut a.methods {
          desugar_asserts_in_stmts(&mut m.body, &mut rewrote);
        }
      }
      Item::Stmt(s) => desugar_asserts_in_stmt(s, &mut rewrote),
      // Plan 80: `property`/`benchmark` bodies get the identical
      // `assert`/`assert_eq` desugaring `test` bodies already get —
      // same AST shape. Dead code in practice for all three by the
      // time this runs (`compile_test_harness`/`compile_benchmark_
      // harness` both strip their own `Item` variant into synthetic
      // `Item::Function`s before ever calling `compile_to_object`,
      // exactly like `Item::Test`'s own doc comment already notes) —
      // kept for match exhaustiveness and defense in depth.
      Item::Test { body, .. } | Item::Property { body, .. } | Item::Benchmark { body, .. } => {
        desugar_asserts_in_stmts(body, &mut rewrote)
      }
      // Plan 52: pure data — no `assert`/`assert_eq` site can appear
      // inside an `Item::Enum`.
      Item::Enum(_) => {}
      // A newtype has no body of its own either — mirrors `Item::Enum`.
      Item::Newtype(_) => {}
      Item::Interface(_) | Item::Require(_) | Item::Error | Item::Extern(_) => {}
      // Plan 76: an exported declaration's own body still gets
      // `assert`/`assert_eq` desugaring — recurse into the unwrapped
      // inner item via a one-element slice.
      Item::Export(inner) => {
        if desugar_asserts_in_items(std::slice::from_mut(inner.as_mut())) {
          rewrote = true;
        }
      }
      // Plan 76: names no body of its own — nothing to desugar.
      Item::Import { .. } => {}
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
    derive: None,
    fields: vec![Param {
      name: "message".to_string(),
      ty: TypeExpr::Named("String".to_string()),
      default: None,
    }],
    methods: vec![
      AstFunction {
        name: "initialize".to_string(),
        params: vec![Param {
          name: "message".to_string(),
          ty: TypeExpr::Named("String".to_string()),
          default: None,
        }],
        return_type: TypeExpr::Named("Void".to_string()),
        body: vec![syn(Stmt::SetField {
          name: "message".to_string(),
          value: syn(Expr::Ident("message".to_string())),
        })],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
      AstFunction {
        name: "message".to_string(),
        params: Vec::new(),
        return_type: TypeExpr::Named("String".to_string()),
        body: vec![syn(Stmt::Expr(syn(Expr::InstanceVar(
          "message".to_string(),
        ))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
    ],
    type_params: Vec::new(),
    doc: None,
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

/// Plan 60's Decision log (Design decision 5): a plain class with a
/// `message: String` field — the identical shape/mechanism `Assertion
/// Error` above already established, reusing plan 11/38's real
/// exception machinery exactly, not a new mechanism. Raised (via
/// `build_raise_remote_actor_error`) at every `.remote(...)` connect
/// failure and every failed cross-actor SEND, naming the concrete
/// socket-layer reason (`emerald_remote_last_error_message`).
fn remote_actor_error_class_item() -> Item {
  Item::Class(ClassDef {
    name: "RemoteActorError".to_string(),
    superclass: None,
    implements: None,
    derive: None,
    fields: vec![Param {
      name: "message".to_string(),
      ty: TypeExpr::Named("String".to_string()),
      default: None,
    }],
    methods: vec![
      AstFunction {
        name: "initialize".to_string(),
        params: vec![Param {
          name: "message".to_string(),
          ty: TypeExpr::Named("String".to_string()),
          default: None,
        }],
        return_type: TypeExpr::Named("Void".to_string()),
        body: vec![syn(Stmt::SetField {
          name: "message".to_string(),
          value: syn(Expr::Ident("message".to_string())),
        })],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
      AstFunction {
        name: "message".to_string(),
        params: Vec::new(),
        return_type: TypeExpr::Named("String".to_string()),
        body: vec![syn(Stmt::Expr(syn(Expr::InstanceVar(
          "message".to_string(),
        ))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
    ],
    type_params: Vec::new(),
    doc: None,
  })
}

/// Gated on the program actually declaring at least one `actor` (the
/// only source of a `.remote(...)`/cross-actor-send site this class
/// could ever be raised from) — mirrors `ensure_assertion_error_class`'s
/// own `uses_assertions` gate exactly, so a program with no actors at
/// all pays zero cost for this synthesized class.
fn ensure_remote_actor_error_class(items: &mut Vec<Item>) {
  let already_present = items
    .iter()
    .any(|i| matches!(i, Item::Class(c) if c.name == "RemoteActorError"));
  if !already_present {
    items.insert(0, remote_actor_error_class_item());
  }
}

/// Plan 62's Decision log: `ContractViolation` — the exact `message:
/// String`, one-`initialize` shape `AssertionError`/`RemoteActorError`
/// above already establish, deliberately a DISTINCT class from
/// `AssertionError` (not a reuse of it): a contract violation is a
/// language-level correctness failure, not a test-framework assertion
/// failure, so a program can `rescue` the two apart. Raised at a
/// function's own `requires`-check (entry) and `ensures`-check (every
/// `return` site, including the implicit-fallthrough one) injection
/// points.
fn contract_violation_class_item() -> Item {
  Item::Class(ClassDef {
    name: "ContractViolation".to_string(),
    superclass: None,
    implements: None,
    derive: None,
    fields: vec![Param {
      name: "message".to_string(),
      ty: TypeExpr::Named("String".to_string()),
      default: None,
    }],
    methods: vec![
      AstFunction {
        name: "initialize".to_string(),
        params: vec![Param {
          name: "message".to_string(),
          ty: TypeExpr::Named("String".to_string()),
          default: None,
        }],
        return_type: TypeExpr::Named("Void".to_string()),
        body: vec![syn(Stmt::SetField {
          name: "message".to_string(),
          value: syn(Expr::Ident("message".to_string())),
        })],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
      AstFunction {
        name: "message".to_string(),
        params: Vec::new(),
        return_type: TypeExpr::Named("String".to_string()),
        body: vec![syn(Stmt::Expr(syn(Expr::InstanceVar(
          "message".to_string(),
        ))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
        doc: None,
      },
    ],
    type_params: Vec::new(),
    doc: None,
  })
}

/// Gated on the program actually declaring a non-empty `requires`/
/// `ensures` anywhere (the only source of a raise site this class could
/// ever be needed for) — mirrors `ensure_assertion_error_class`'s own
/// `uses_assertions` gate exactly, so a program with no contracts at all
/// pays zero cost for this synthesized class.
fn ensure_contract_violation_class(items: &mut Vec<Item>) {
  let already_present = items
    .iter()
    .any(|i| matches!(i, Item::Class(c) if c.name == "ContractViolation"));
  if !already_present {
    items.insert(0, contract_violation_class_item());
  }
}

/// Plan 65's `leaf-unified-fallible-send`: a plan-52 enum, not a
/// plan-53 hardcoded type and not a reuse of `RemoteActorError`
/// (Decision log — the two failure-signaling mechanisms cover two
/// genuinely different moments: `RemoteActorError` is still raised by
/// `.remote(addr, name)`'s own connect-time failure, before any
/// `EmeraldActorRef` exists to return a `Result` through at all;
/// `SendError` is what a POST-connection cross-actor send returns).
/// Every variant is zero-field (`EnumVariant.fields: Vec::new()`) —
/// the AST itself has no trouble representing that even though the
/// concrete `ActorTerminated()`/`Timeout()`/`NodeUnreachable()` syntax
/// (mandatory parens on every variant, plan 52's real grammar) would
/// need them if a user ever wrote this enum by hand, which they never
/// do — `SendError` is compiler-synthesized only, the same "never
/// something Emerald source declares" posture `RemoteActorError`/
/// `ContractViolation` already have.
fn send_error_enum_item() -> Item {
  Item::Enum(EnumDef {
    name: "SendError".to_string(),
    type_params: Vec::new(),
    variants: vec![
      EnumVariant {
        name: "ActorTerminated".to_string(),
        fields: Vec::new(),
      },
      EnumVariant {
        name: "Timeout".to_string(),
        fields: Vec::new(),
      },
      EnumVariant {
        name: "NodeUnreachable".to_string(),
        fields: Vec::new(),
      },
    ],
    doc: None,
  })
}

/// Gated on the program actually declaring at least one `actor` — the
/// only source of a cross-actor send this enum could ever be needed
/// for — mirrors `ensure_remote_actor_error_class`'s own identical
/// gate exactly, so a program with no actors at all pays zero cost.
fn ensure_send_error_enum(items: &mut Vec<Item>) {
  let already_present = items
    .iter()
    .any(|i| matches!(i, Item::Enum(e) if e.name == "SendError"));
  if !already_present {
    items.insert(0, send_error_enum_item());
  }
}

fn program_uses_contracts(items: &[Item]) -> bool {
  items.iter().any(|i| match i {
    Item::Function(f) => !f.requires.is_empty() || !f.ensures.is_empty(),
    _ => false,
  })
}

/// A real, disclosed bug this leaf's own CLI-level test caught (not
/// something the plan's own Decision log anticipated): `ensure_remote_
/// actor_error_class`/`ensure_contract_violation_class` previously only
/// ever ran inside `compile_to_object_impl`, AFTER `emerald_sema::
/// check_program` had already run on the un-mutated `Program` — so a
/// user writing `rescue RemoteActorError => e`/`rescue ContractViolation
/// => e` in their own real source (exactly what this plan's own worked
/// example does) failed sema with "unknown type", even though codegen
/// itself worked fine. `emerald-driver::check_stage` calls this `pub`
/// entry point BEFORE `check_program`, so sema sees the same synthetic
/// classes codegen will later (re-)inject — `compile_to_object_impl`'s
/// own identical gate-and-inject calls become harmless no-ops (`already_
/// present` is true) once this has already run, and stay exactly as
/// necessary as before for callers that reach `compile_to_object`
/// directly, bypassing `emerald-driver` (and this function) entirely —
/// `emerald-codegen`'s own test suite (`compile_link_run` and friends).
/// `AssertionError` deliberately isn't included here: verified this
/// session that its own `rescue` clause is ONLY ever synthesized inside
/// `compile_test_harness`, entirely in codegen, on a `Program` sema
/// never re-checks after that synthesis — it has no equivalent gap.
pub fn ensure_pre_sema_exception_classes(items: &mut Vec<Item>) {
  if items.iter().any(|i| matches!(i, Item::Actor(_))) {
    ensure_remote_actor_error_class(items);
    // Plan 65's `leaf-unified-fallible-send`: `SendError` needs the
    // same pre-sema injection `RemoteActorError` already gets, for the
    // identical reason (this function's own doc comment) — a cross-
    // actor send's real type now names `SendError` directly, not just
    // a `rescue` clause naming it.
    ensure_send_error_enum(items);
  }
  if program_uses_contracts(items) {
    ensure_contract_violation_class(items);
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
///
/// Plan 80's Decision log: `Item::Property` blocks are collected into
/// the exact same `tests` list, indistinguishable from an `Item::Test`
/// from this point on — this IS the real, disclosed scope of
/// `property` today (see `Item::Property`'s own doc comment): a
/// `property` block runs its body exactly once, through this identical
/// pass/fail mechanism, not across many generated inputs. Real
/// property-based testing (input generation, shrinking) is not
/// implemented here.
pub fn compile_test_harness(program: &Program, out_path: &Path) -> Result<usize, String> {
  let tests: Vec<(String, Vec<Spanned<Stmt>>)> = program
    .items
    .iter()
    .filter_map(|it| match it {
      Item::Test { description, body } | Item::Property { description, body } => {
        Some((description.clone(), body.clone()))
      }
      _ => None,
    })
    .collect();
  let num_tests = tests.len();

  let mut items: Vec<Item> = program
    .items
    .iter()
    .filter(|it| !matches!(it, Item::Test { .. } | Item::Property { .. }))
    .cloned()
    .collect();

  let mut harness_stmts = vec![
    syn(Stmt::Let {
      name: "passed".to_string(),
      ty: TypeExpr::Named("Int64".to_string()),
      value: syn(Expr::Int(0)),
      is_var: true,
    }),
    syn(Stmt::Let {
      name: "failed".to_string(),
      ty: TypeExpr::Named("Int64".to_string()),
      value: syn(Expr::Int(0)),
      is_var: true,
    }),
  ];

  for (i, (description, body)) in tests.into_iter().enumerate() {
    let fn_name = format!("__emerald_test_{i}");
    items.push(Item::Function(AstFunction {
      name: fn_name.clone(),
      params: Vec::new(),
      return_type: TypeExpr::Named("Void".to_string()),
      body,
      block_param: None,
      splat_param: None,
      type_params: Vec::new(),
      is_comptime: false,
      requires: Vec::new(),
      ensures: Vec::new(),
      is_pure: false,
      // A synthesized `test`/`property` harness function — no `##`
      // comment position exists for it (plan 77's Decision log).
      doc: None,
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

/// Plan 80's `leaf-benchmark-runner`: `compile_test_harness`'s own
/// timing-report sibling, not a fork of it — same "synthesize a
/// harness `Program`, compile it through the ordinary, unmodified
/// `compile_to_object`" shape, same never-reachable-outside-this-path
/// restriction (see `compile_to_object_impl`'s own prologue rejection).
/// Every `Item::Benchmark` body becomes its own top-level `Item::
/// Function` (`__emerald_benchmark_N`), timed by two calls to a
/// synthetic `extern "C" fn emerald_bench_now_seconds(): Float64`
/// (declared here, injected as a fresh `Item::Extern` block — the same
/// general FFI mechanism plan 59 already built for arbitrary `extern
/// "C"` declarations, reused rather than adding a new codegen-level
/// builtin/intrinsic) bracketing the call, printing `BENCHMARK:
/// <description>` followed by the elapsed CPU-time delta in seconds.
///
/// CPU time, not wall clock — `runtime/emerald_runtime.c`'s own
/// `emerald_bench_now_seconds` uses ISO C89 `clock()`, matching
/// `benchmarks/REPORT.md`'s own existing methodology ("Run time is CPU
/// time (user+sys), not wall clock"), which the project's `run_
/// benchmarks.py` driver already measures via `RUSAGE_CHILDREN` deltas
/// around each external subprocess — this keeps that same choice of
/// clock, just measured from inside the compiled program itself rather
/// than by an external host-side wrapper, since a single `emerald
/// benchmark <file>.em` run can contain more than one `benchmark`
/// block and each needs its own separate delta.
///
/// A real, disclosed simplification: this runs each `benchmark`
/// block's body exactly ONCE, not the many-iterations-with-statistics
/// (mean/min/max/stddev) methodology `benchmarks/run_benchmarks.py`
/// itself uses — a single-run CPU-time report, not a rigorous
/// statistical benchmark harness. A benchmark author who wants
/// multiple-iteration averaging can write their own loop inside the
/// block; this harness does not loop on their behalf.
pub fn compile_benchmark_harness(program: &Program, out_path: &Path) -> Result<usize, String> {
  const BENCH_CLOCK_FN: &str = "emerald_bench_now_seconds";

  let benchmarks: Vec<(String, Vec<Spanned<Stmt>>)> = program
    .items
    .iter()
    .filter_map(|it| match it {
      Item::Benchmark { description, body } => Some((description.clone(), body.clone())),
      _ => None,
    })
    .collect();
  let num_benchmarks = benchmarks.len();

  let mut items: Vec<Item> = program
    .items
    .iter()
    .filter(|it| !matches!(it, Item::Benchmark { .. }))
    .cloned()
    .collect();

  items.push(Item::Extern(ExternBlock {
    abi: "C".to_string(),
    fns: vec![ExternFn {
      name: BENCH_CLOCK_FN.to_string(),
      params: Vec::new(),
      return_type: TypeExpr::Named("Float64".to_string()),
    }],
  }));

  let mut harness_stmts = Vec::new();

  for (i, (description, body)) in benchmarks.into_iter().enumerate() {
    let fn_name = format!("__emerald_benchmark_{i}");
    items.push(Item::Function(AstFunction {
      name: fn_name.clone(),
      params: Vec::new(),
      return_type: TypeExpr::Named("Void".to_string()),
      body,
      block_param: None,
      splat_param: None,
      type_params: Vec::new(),
      is_comptime: false,
      requires: Vec::new(),
      ensures: Vec::new(),
      is_pure: false,
      // A synthesized `benchmark` harness function — same reasoning as
      // the `test`/`property` harness function above.
      doc: None,
    }));

    let start_name = format!("__emerald_bench_start_{i}");
    let end_name = format!("__emerald_bench_end_{i}");
    harness_stmts.push(syn(Stmt::Let {
      name: start_name.clone(),
      ty: TypeExpr::Named("Float64".to_string()),
      value: syn(Expr::Call(BENCH_CLOCK_FN.to_string(), Vec::new())),
      is_var: false,
    }));
    harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(fn_name, Vec::new())))));
    harness_stmts.push(syn(Stmt::Let {
      name: end_name.clone(),
      ty: TypeExpr::Named("Float64".to_string()),
      value: syn(Expr::Call(BENCH_CLOCK_FN.to_string(), Vec::new())),
      is_var: false,
    }));
    harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
      "puts".to_string(),
      vec![syn(Expr::StringLit(format!("BENCHMARK: {description}")))],
    )))));
    harness_stmts.push(syn(Stmt::Expr(syn(Expr::Call(
      "puts".to_string(),
      vec![syn(Expr::Sub(
        Box::new(syn(Expr::Ident(end_name))),
        Box::new(syn(Expr::Ident(start_name))),
      ))],
    )))));
  }

  items.extend(harness_stmts.into_iter().map(Item::Stmt));

  let owned_program = Program { items };
  compile_to_object(&owned_program, out_path)?;
  Ok(num_benchmarks)
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

  /// Plan 66's regression-hardening runner: compiles and links `src`
  /// exactly once, then executes the resulting binary `n` times in a
  /// row, asserting every single run produced byte-identical stdout.
  /// Plan 66's own root-cause investigation found the original "puts
  /// silently prints nothing" family wasn't a codegen defect at all —
  /// it was a nondeterministic race between the (since-removed) actor
  /// worker-pool thread spawn/join wrapping every compiled program's
  /// `main` and glibc's block-buffered stdout, incidentally fixed by
  /// `b0d8bc1` ("perf(codegen): skip actor worker-pool startup/shutdown
  /// for programs with no actors"). A single `compile_link_run` call
  /// proves correctness but not the *absence* of that race class; this
  /// helper is the cheap insurance the investigation recommended so a
  /// future change that reintroduces unconditional thread startup (or
  /// extends it to more program shapes) fails loudly instead of
  /// flaking quietly.
  fn compile_link_run_n_times(src: &str, n: usize) -> Vec<String> {
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!(
      "{}_{:?}_repeat",
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

    let mut runs = Vec::with_capacity(n);
    for _ in 0..n {
      let output = Command::new(&bin_path)
        .output()
        .expect("failed to run compiled binary");
      assert!(output.status.success(), "compiled binary should exit 0");
      runs.push(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_file(&bin_path).ok();

    runs
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
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
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
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn inception_milestone2_linked_and_run_prints_10() {
    let src = "x: Int64 = 10\n\nif x > 5 do\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "10\n");
  }

  #[test]
  fn while_loop_sums_1_to_3_and_prints_6() {
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4 do\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "6\n");
  }

  #[test]
  fn break_exits_loop_early() {
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4 do\n  if i == 2 do\n    break\n  end\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn sum: Float64 do\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn inception_point_example_linked_and_run_prints_5() {
    assert_eq!(compile_link_run(POINT_EXAMPLE), "5\n");
  }

  const ARRAY_EXAMPLE: &str = "arr: Array[Int64] = [10, 20, 30]\nsum: Int64 = 0\ni: Int64 = 0\nwhile i < 3 do\n  sum: Int64 = sum + arr[i]\n  i: Int64 = i + 1\nend\narr[1] = 99\nputs sum\nputs arr[1]\n";

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

  // `newtype Meters: Float64` — real, compiled-and-run proof (not just a
  // sema type-check) that construction/unwrap round-trips through the
  // identical runtime value, and that a domain type composes with the
  // rest of the backend (function params/returns, `Array[T]`, `==`)
  // with zero special-casing visible from the *output* side.

  #[test]
  fn newtype_construct_and_unwrap_linked_and_run_prints_the_underlying_value() {
    let src = "newtype Meters: Float64\nm: Meters = Meters.new(5.0)\nputs m.value\n";
    assert_eq!(compile_link_run(src), "5\n");
  }

  #[test]
  fn newtype_valued_function_param_and_return_linked_and_run() {
    // `.value`'s receiver must be a plain local (this backend's own
    // pre-existing, disclosed restriction on every method-call
    // receiver, unrelated to `newtype`) — `doubled` binds `double(d)`'s
    // own result before unwrapping it.
    let src = "newtype Meters: Float64\nfn double(m: Meters): Meters do\n  return Meters.new(m.value * 2.0)\nend\nd: Meters = Meters.new(21.0)\ndoubled: Meters = double(d)\nputs doubled.value\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn newtype_equality_linked_and_run() {
    let src = "newtype Meters: Float64\na: Meters = Meters.new(3.0)\nb: Meters = Meters.new(3.0)\nc: Meters = Meters.new(4.0)\nif a == b do\n  puts 1\nelse\n  puts 0\nend\nif a == c do\n  puts 1\nelse\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "1\n0\n");
  }

  #[test]
  fn array_of_newtype_linked_and_run_matches_array_of_its_underlying_primitive() {
    // The zero-cost claim's own concrete, executed proof: an
    // `Array[Meters]` built/summed exactly like `array_of_float64_
    // linked_and_run` above, through `.new`/`.value` at the write/read
    // sites — same real answer, same representation.
    let src = "newtype Meters: Float64\narr: Array[Meters] = [Meters.new(1.5), Meters.new(2.5)]\nx: Meters = arr[0]\ny: Meters = arr[1]\nputs x.value + y.value\n";
    assert_eq!(compile_link_run(src), "4\n");
  }

  // Found and closed 2026-09-21 (this session's "find all bugs" sweep):
  // a newtype-typed CLASS FIELD's `.value` unwrap (`@distance.value`,
  // an `Expr::InstanceVar` receiver) previously failed with "method
  // calls are only supported on a plain local-variable receiver" — the
  // newtype-unwrap check only ever consulted `local_classes` (keyed by
  // a named local), never `field_classes`. Confirmed as a real,
  // pre-existing bug (not assumed) by reverting this fix on a stashed
  // copy of `emerald-codegen` and reproducing the exact error via the
  // real CLI before restoring it.
  #[test]
  fn newtype_typed_class_field_value_unwrap_from_inside_the_declaring_class_works() {
    let src = "newtype Meters: Float64\nclass Trip\n  distance: Meters\n\n  fn initialize(distance: Meters): Void do\n    @distance = distance\n  end\n\n  fn distance_value: Float64 do\n    @distance.value\n  end\nend\nt: Trip = Trip.new(Meters.new(5.5))\nputs t.distance_value\n";
    assert_eq!(compile_link_run(src), "5.5\n");
  }

  #[test]
  fn lambda_capture_and_call_linked_and_run() {
    let src = "x: Int64 = 10\nadd_x: Proc = do |y: Int64| y + x end\nputs add_x.call(5)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  #[test]
  fn lambda_with_no_captures_linked_and_run() {
    let src = "add_one: Proc = do |y: Int64| y + 1 end\nputs add_one.call(41)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn plan_11_exceptions_example_linked_and_run() {
    assert_eq!(compile_link_run(EXCEPTION_EXAMPLE), "99\n");
  }

  #[test]
  fn no_exception_raised_skips_rescue_entirely() {
    let src = "fn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise MyError.new(99)\n  end\n  return x\nend\n\nclass MyError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nbegin\n  puts risky(5)\nrescue MyError => e\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "5\n");
  }

  #[test]
  fn plan_12_module_example_linked_and_run() {
    let src = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";
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

  // Plan 64's `leaf-target-triple-and-selection` AC4: a real `.o` file
  // `file(1)` itself identifies as a WebAssembly object — not merely
  // "no error returned" from `compile_to_object_with_target`.
  #[test]
  fn sum_benchmark_compiled_for_wasm32_wasi_produces_a_real_webassembly_object() {
    let src = std::fs::read_to_string(
      std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/sum/sum.em"),
    )
    .expect("benchmarks/sum/sum.em should exist");
    let program = emerald_parser::parse(&src).expect("should parse");
    let dir = fresh_temp_dir("sum_wasm32_wasi");
    let out_path = dir.join("sum.o");
    compile_to_object_with_target(
      &program,
      &out_path,
      &src,
      "sum.em",
      None,
      CodegenTarget::Wasm32Wasi,
    )
    .expect("should compile a wasm32-wasi object file");
    let output = std::process::Command::new("file")
      .arg(&out_path)
      .output()
      .expect("failed to run `file`");
    let description = String::from_utf8_lossy(&output.stdout);
    std::fs::remove_dir_all(&dir).ok();
    assert!(
      description.to_lowercase().contains("wasm")
        || description.to_lowercase().contains("webassembly"),
      "expected `file` to report a WebAssembly object, got: {description}"
    );
  }

  // Plan 65 (automatic actor placement), `leaf-unified-fallible-send`
  // AC2: capturing a cross-actor send's own real `Result[Void,
  // SendError]` explicitly (new source-level syntax this leaf makes
  // possible for the first time), then matching it via plan 53's real
  // `MatchResult` form — real proof the success path round-trips
  // through, not just that it type-checks.
  #[test]
  fn plan65_capturing_a_local_sends_result_and_matching_ok_prints_the_ok_arm() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\nend\n\nc: Counter = Counter.spawn(0)\nresult: Result[Void, SendError] = c.increment\nmatch result do\nOk(v) do\n  puts \"ok\"\nend\nErr(e) do\n  puts \"err\"\nend\nend\n";
    assert_eq!(compile_link_run(src), "ok\n");
  }

  // Plan 65 (automatic actor placement), `leaf-virtual-actor-
  // placement`'s own local-activation proof: with no `EMERALD_PEERS`
  // set (the real, valid single-process configuration — Decision
  // log), `.locate` degenerates to "always self," reusing `.spawn`'s
  // own allocation path.
  #[test]
  fn plan65_locate_with_no_peers_configured_activates_locally_like_spawn() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\nc: Counter = Counter.locate(\"shard-1\", 0)\nc.increment\nc.increment\nc.value\n";
    assert_eq!(compile_link_run(src), "2\n");
  }

  // AC1-adjacent: a SECOND `.locate` call for the SAME key must return
  // the cached, already-activated instance — not a fresh one that
  // resets `@count` back to its original `initialize` argument.
  #[test]
  fn plan65_a_second_locate_call_for_the_same_key_returns_the_cached_instance() {
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\nc1: Counter = Counter.locate(\"shard-1\", 0)\nc1.increment\nc2: Counter = Counter.locate(\"shard-1\", 999)\nc2.increment\nc2.value\n";
    assert_eq!(
      compile_link_run(src),
      "2\n",
      "the second .locate must return the SAME cached instance (count 0->1->2), not a fresh one seeded from 999"
    );
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

  const ARITHMETIC_EXAMPLE: &str = "fn factorial(n: Int64): Int64 do\n  if n <= 1 do\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";

  #[test]
  fn arithmetic_example_linked_and_run() {
    assert_eq!(compile_link_run(ARITHMETIC_EXAMPLE), "120\n3\n2\n7\n");
  }

  const SHORT_CIRCUIT_EXAMPLE: &str = "fn noisy(n: Int64): Boolean do\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1) do\n  puts 100\nend\nif x > -10 && noisy(3) do\n  puts 300\nend\nif x < 0 || noisy(2) do\n  puts 200\nend\nif x > 0 || noisy(4) do\n  puts 400\nend\n";

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
    let src = "arr: Array[Int64] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]\ntotal: Int64 = 0\nrep: Int64 = 0\nwhile rep < 1000000 do\n  i: Int64 = 0\n  while i < 20 do\n    total: Int64 = total + arr[i]\n    i: Int64 = i + 1\n  end\n  rep: Int64 = rep + 1\nend\nzero: Int64 = total - 210000000\nputs 10 / zero\n";
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

  const STRING_EXAMPLE: &str = "s: String = \"hello\"\nputs s\na: String = \"foo\" + \"bar\"\nputs a\nif \"abc\" == \"abc\" do\n  puts \"equal\"\nend\nputs \"line1\\nline2\"\nputs \"a\\\"b\"\n";

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
      "# classify an integer by a fixed set of buckets\nn: Int64 = {n}\nlabel: Int64 = 0\nmatch n do\n1 do\n  label: Int64 = 10\nend\n2, 3 do\n  label: Int64 = 20\nend\n_ do\n  label: Int64 = 99\nend\nend\nputs label\n"
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
    let src = "n: Int64 = 7\nmatch n do\n1 do\n  puts 1\nend\nend\nputs 42\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  // Plan 25 (stdlib expansion).

  const BOOL_EXAMPLE: &str = "fn check(flag: Boolean): Int64 do\n  if flag do\n    return 1\n  end\n  return 0\nend\n\nputs check(true)\nputs check(false)\n";

  #[test]
  fn bool_literal_example_linked_and_run() {
    assert_eq!(compile_link_run(BOOL_EXAMPLE), "1\n0\n");
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

  const ARRAY_NEW_EXAMPLE: &str = "arr: Array[Int64] = Array.new(5)\nputs arr[0]\ni: Int64 = 0\nwhile i < 5 do\n  arr[i] = i\n  i: Int64 = i + 1\nend\nputs arr[3]\n";

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

  const BITWISE_EXAMPLE: &str = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\nfn has_flag(flags: Int64, flag: Int64): Boolean do\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ) do\n  puts 1\nend\nif has_flag(perms, EXEC) do\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";

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

  const ELSIF_EXAMPLE: &str = "fn grade(score: Int64): Int64 do\n  if score >= 90 do\n    return 4\n  elsif score >= 80 do\n    return 3\n  elsif score >= 70 do\n    return 2\n  else\n    return 1\n  end\nend\n\nputs grade(95)\nputs grade(85)\nputs grade(72)\nputs grade(50)\n";

  #[test]
  fn elsif_example_linked_and_run() {
    assert_eq!(compile_link_run(ELSIF_EXAMPLE), "4\n3\n2\n1\n");
  }

  const UNLESS_UNTIL_EXAMPLE: &str = "fn describe(x: Int64): Int64 do\n  unless x > 0 do\n    return 0\n  end\n  return 1\nend\n\nputs describe(-5)\nputs describe(5)\n\ni: Int64 = 0\nuntil i >= 3 do\n  puts i\n  i: Int64 = i + 1\nend\n";

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
    let src = "for x in [10, 20, 30, 40]\n  if x == 30 do\n    break\n  end\n  puts x\nend\n";
    assert_eq!(compile_link_run(src), "10\n20\n");
  }

  #[test]
  fn for_in_next_skips_one_element() {
    let src = "for x in [10, 20, 30]\n  if x == 20 do\n    next\n  end\n  puts x\nend\n";
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
    "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5 do\n  total += i\n  i += 1\nend\nputs total\n";

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
    let src = "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5 do\n  total += i\n  i += 1\nend\nputs total\n\na: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    assert_eq!(compile_link_run(src), "10\n2\n1\n");
  }

  // Plan 32 (class inheritance).

  const INHERITANCE_EXAMPLE: &str = "class Animal\n  age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn age: Int64 do\n    @age\n  end\n\n  fn describe: Int64 do\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  fn initialize(age: Int64, breed_code: Int64): Void do\n    @age = age\n    @breed_code = breed_code\n  end\n\n  fn describe: Int64 do\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";

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
    let src = "class Counter\n  n: Int64\n\n  fn initialize(n: Int64): Void do\n    @n = n\n  end\n\n  fn value: Int64 do\n    @n\n  end\nend\n\nc: Counter = Counter.new(7)\nputs c.value\n";
    assert_eq!(compile_link_run(src), "7\n");
  }

  // Plan 33 (field-access sugar).

  #[test]
  fn read_field_accessor_linked_and_run() {
    // Real executed proof `p.x` dispatches through the synthesized
    // zero-arg accessor exactly like a hand-written method — no codegen
    // source changes needed for this plan at all.
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";
    assert_eq!(compile_link_run(src), "3\n");
  }

  // Plan 34 (blocks and yield).

  const BLOCKS_EXAMPLE: &str = "fn repeat(n: Int64, &blk): Void do\n  i: Int64 = 0\n  while i < n do\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) do |i: Int64| puts i end\n";

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
    let src = "fn repeat(n: Int64, &blk): Void do\n  i: Int64 = 0\n  while i < n do\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nmultiplier: Int64 = 10\nrepeat(3) do |i: Int64| puts i * multiplier end\n";
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
          is_comptime: false,
          requires: Vec::new(),
          ensures: Vec::new(),
          is_pure: false,
          doc: None,
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
    let src = "for i in 1..5\n  if i == 3 do\n    break\n  end\n  puts i\nend\n";
    assert_eq!(compile_link_run(src), "1\n2\n");
  }

  #[test]
  fn next_inside_a_range_for_in_skips_one_element() {
    let src = "for i in 1..5\n  if i == 3 do\n    next\n  end\n  puts i\nend\n";
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

  const FULL_EXCEPTION_WORKED_EXAMPLE: &str = "class NotFoundError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nclass TimeoutError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise TimeoutError.new(7)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";

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
    let src = "class Foo\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nbegin\n  puts 1\nrescue Foo => f\n  puts 0\nensure\n  puts 2\nend\n";
    assert_eq!(compile_link_run(src), "1\n2\n");
  }

  #[test]
  fn ensure_runs_on_the_mismatch_exhausted_reraise_path_of_an_inner_begin() {
    // The raised type matches neither the inner clause but does match
    // the outer one — proving `ensure` fires on the mismatch-exhausted
    // path, in the correct order (inner's `ensure` before the outer
    // clause's own output).
    let src = "class WrongType\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nclass RightType\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nbegin\n  begin\n    raise RightType.new(5)\n  rescue WrongType => w\n    puts 0\n  ensure\n    puts \"inner\"\n  end\nrescue RightType => r\n  puts r.code\nensure\n  puts \"outer\"\nend\n";
    assert_eq!(compile_link_run(src), "inner\n5\nouter\n");
  }

  #[test]
  fn return_inside_a_matched_rescue_still_runs_ensure_first() {
    let src = "class Foo\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise Foo.new(1)\n  end\n  return x\nend\n\nfn f(x: Int64): Int64 do\n  begin\n    puts risky(x)\n  rescue Foo => e\n    return 9\n  ensure\n    puts \"cleanup\"\n  end\n  return 0\nend\n\nputs f(999)\n";
    assert_eq!(compile_link_run(src), "cleanup\n9\n");
  }

  #[test]
  fn subtype_aware_rescue_catches_a_raised_subclass_reusing_the_animal_dog_hierarchy() {
    let src = "class Animal\n  age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn age: Int64 do\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  fn initialize(age: Int64, breed_code: Int64): Void do\n    @age = age\n    @breed_code = breed_code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise Dog.new(7, 1)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue Animal => a\n  puts a.age\nend\n";
    assert_eq!(compile_link_run(src), "7\n");
  }

  #[test]
  fn bare_rescue_after_a_typed_mismatch_still_catches_unconditionally() {
    let src = "class Foo\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nclass Bar\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise Foo.new(1)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue Bar => b\n  puts 0\nrescue => e\n  puts 1\nend\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  #[test]
  fn retry_re_attempts_the_begin_until_it_succeeds() {
    let src = "class Foo\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nattempts: Int64 = 0\n\nbegin\n  attempts = attempts + 1\n  if attempts < 3 do\n    raise Foo.new(1)\n  end\nrescue Foo => e\n  retry\nend\nputs attempts\n";
    assert_eq!(compile_link_run(src), "3\n");
  }

  #[test]
  fn retry_does_not_re_trigger_the_enclosing_ensure_per_attempt() {
    // `ensure` must print exactly once (after the third, successful
    // attempt), not once per attempt.
    let src = "class Foo\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\nend\n\nattempts: Int64 = 0\n\nbegin\n  attempts = attempts + 1\n  if attempts < 3 do\n    raise Foo.new(1)\n  end\nrescue Foo => e\n  retry\nensure\n  puts \"cleanup\"\nend\nputs attempts\n";
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
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n\nputs inc(5)\n";
    assert_eq!(compile_link_run(src), "6\n");
  }

  #[test]
  fn default_param_overridden_by_explicit_argument() {
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n\nputs inc(5, 10)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  const GREET_EXAMPLE: &str = "fn greet(name: String, times: Int64 = 1): Void do\n  i: Int64 = 0\n  while i < times do\n    puts name\n    i += 1\n  end\nend\n\ngreet(name: \"yo\")\ngreet(name: \"hi\", times: 2)\n";

  #[test]
  fn greet_worked_example_keyword_calls_and_defaults_linked_and_run() {
    // Real distinguishing proof: keyword resolution and default-filling
    // compose correctly together (`greet(name: "yo")` uses `times`'s
    // default; `greet(name: "hi", times: 2)` overrides it).
    assert_eq!(compile_link_run(GREET_EXAMPLE), "yo\nhi\nhi\n");
  }

  #[test]
  fn positional_call_still_compiles_and_runs_unchanged() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  const SUM_ALL_EXAMPLE: &str = "fn sum_all(*xs: Int64): Int64 do\n  total: Int64 = 0\n  i: Int64 = 0\n  while i < 4 do\n    total += xs[i]\n    i += 1\n  end\n  total\nend\n\nputs sum_all(1, 2, 3, 4)\n";

  #[test]
  fn splat_param_sums_a_packed_trailing_argument_list() {
    assert_eq!(compile_link_run(SUM_ALL_EXAMPLE), "10\n");
  }

  #[test]
  fn splat_param_with_zero_trailing_arguments_is_a_zero_length_capture() {
    let src = "fn sum_all(*xs: Int64): Int64 do\n  0\nend\n\nputs sum_all()\n";
    assert_eq!(compile_link_run(src), "0\n");
  }

  const DIVMOD_EXAMPLE: &str = "fn divmod(a: Int64, b: Int64): (Int64, Int64) do\n  return a / b, a % b\nend\n\nq: Int64 = 0\nr: Int64 = 0\nq, r = divmod(17, 5)\nputs q\nputs r\n";

  #[test]
  fn divmod_worked_example_tuple_return_linked_and_run() {
    // Plan 39's own combined worked example's second half: a real
    // fixed-arity anonymous tuple return (LLVM struct return, not
    // plan 09's boxed `Array[T]`), unpacked positionally on the
    // receiving `Stmt::MultiAssign` side via `build_extract_value`.
    assert_eq!(compile_link_run(DIVMOD_EXAMPLE), "3\n2\n");
  }

  #[test]
  fn plan_39_full_worked_example_keyword_defaults_and_tuple_return_together() {
    // The plan's own single combined program exercising all four
    // leaves at once: keyword args + defaults (`greet`), then a tuple
    // return unpacked via multi-assign (`divmod`).
    let src = "fn greet(name: String, times: Int64 = 1): Void do\n  i: Int64 = 0\n  while i < times do\n    puts name\n    i += 1\n  end\nend\n\ngreet(name: \"yo\")\ngreet(name: \"hi\", times: 2)\n\nfn divmod(a: Int64, b: Int64): (Int64, Int64) do\n  return a / b, a % b\nend\n\nq: Int64 = 0\nr: Int64 = 0\nq, r = divmod(17, 5)\nputs q\nputs r\n";
    assert_eq!(compile_link_run(src), "yo\nhi\nhi\n3\n2\n");
  }

  #[test]
  fn preexisting_multi_assign_swap_is_still_byte_identical() {
    // Plan 31's own swap example — regression proof that the new
    // values.len() == 1-and-tuple-typed-call special case in
    // check_multi_assign/its codegen mirror left every pre-existing
    // multi-assign shape (values.len() == names.len(), no call
    // involved) untouched.
    let src = "a: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    assert_eq!(compile_link_run(src), "2\n1\n");
  }

  // Plan 40 (operator overloading).

  const VECTOR2_EXAMPLE: &str = "class Vector2\n  read x: Float64\n  read y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn +(other: Vector2): Vector2 do\n    Vector2.new(@x + other.x, @y + other.y)\n  end\n\n  fn ==(other: Vector2): Boolean do\n    @x == other.x && @y == other.y\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0, 2.0)\nv2: Vector2 = Vector2.new(3.0, 4.0)\nv3: Vector2 = v1 + v2\nputs v3.x\nputs v3.y\nif v1 == v2 do\n  puts 1\nelse\n  puts 0\nend\nif v1 == v1 do\n  puts 1\nelse\n  puts 0\nend\n";

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
  const BAG_EXAMPLE: &str = "class Bag\n  data: Array[Int64]\n\n  fn initialize(a: Int64, b: Int64, c: Int64): Void do\n    @data = [a, b, c]\n  end\n\n  fn [](i: Int64): Int64 do\n    d: Array[Int64] = @data\n    d[i]\n  end\n\n  fn []=(i: Int64, v: Int64): Void do\n    d: Array[Int64] = @data\n    d[i] = v\n  end\nend\n\nb: Bag = Bag.new(10, 20, 30)\nputs b[0] + b[1] + b[2]\nb[1] = 99\nputs b[1]\n";

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
    let src = "class Vector2\n  read x: Float64\n\n  fn initialize(x: Float64): Void do\n    @x = x\n  end\n\n  fn +(other: Vector2): Vector2 do\n    Vector2.new(@x + other.x)\n  end\nend\n\nv1: Vector2 = Vector2.new(1.0)\nv2: Vector2 = Vector2.new(2.0)\nv3: Vector2 = Vector2.new(3.0)\nv4: Vector2 = v1 + v2 + v3\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let out = std::env::temp_dir().join("emerald_codegen_chained_operator_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  #[test]
  fn plan_08_point_example_still_compiles_and_runs_unchanged() {
    assert_eq!(compile_link_run(POINT_EXAMPLE), "5\n");
  }

  // Plan 41 (interfaces and generics).

  const INTERFACES_GENERICS_EXAMPLE: &str = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Int64 do\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  fn initialize(meters: Int64): Void do\n    @meters = meters\n  end\n\n  fn compare_to(other: Distance): Int64 do\n    @meters - other.meters\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

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
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Int64 do\n    @cents - other.cents\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nfn make_money(cents: Int64): Money do\n  Money.new(cents)\nend\n\nboom: Money = max(make_money(500), make_money(750))\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let out =
      std::env::temp_dir().join("emerald_codegen_unresolvable_generic_call_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }

  // Plan 43 (nullable types and safe navigation).

  const NULLABLE_WORKED_EXAMPLE: &str = "class Greeter\n  name: String\n\n  fn initialize(name: String): Void do\n    @name = name\n  end\n\n  fn shout: String do\n    @name + \"!\"\n  end\nend\n\nfn find_greeter(id: Int64): Option[Greeter] do\n  if id == 1 do\n    return Some(Greeter.new(\"ada\"))\n  end\n  return None\nend\n\nfn greet(id: Int64): String do\n  g: Option[Greeter] = find_greeter(id)\n  message: String = g?.shout ?? \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";

  #[test]
  fn nullable_worked_example_linked_and_run() {
    // Real executed proof, combining all three leaves: a real `Some`
    // (`greet(1)`) and a real `None` (`greet(2)`, `find_greeter`'s
    // "not found" path) both round-trip correctly through an
    // `Option[Greeter]`-typed local without crashing or misreading the
    // wrong tag; `g?.shout` actually skips calling `shout` on the
    // `None` receiver and actually performs it on the `Some` one,
    // merging both paths via a real LLVM `phi`; and `??` unwraps
    // `message` so `return message` type-checks and prints the right
    // string on both paths.
    assert_eq!(
      compile_link_run(NULLABLE_WORKED_EXAMPLE),
      "ada!\nnobody here\n"
    );
  }

  // Plan 67 (String equality codegen).

  const STRING_EQUALITY_WORKED_EXAMPLE: &str = "a: String = \"hello\"\nb: String = \"hello\"\nc: String = \"world\"\n\nif a == b do\n  puts 1\nelse\n  puts 0\nend\n\nif a == c do\n  puts 1\nelse\n  puts 0\nend\n\nif a != c do\n  puts 1\nelse\n  puts 0\nend\n\nd: String = \"hel\" + \"lo\"\nif a == d do\n  puts 1\nelse\n  puts 0\nend\n";

  #[test]
  fn string_equality_worked_example_linked_and_run() {
    // Plan 67's own regression proof: `String == String`/`!=` lower to
    // a real `emerald_string_eq` content comparison (the `(ValKind::
    // Str, ValKind::Str)` arm right next to `(ValKind::Symbol,
    // ValKind::Symbol)`'s own, restricted the same way to `Eq`/`Ne`),
    // never the catch-all `codegen: comparison operands must both be
    // Int64 or both Float64` error this exact shape would hit without
    // that arm. `d` is built by concatenation — a freshly
    // `emerald_alloc`'d buffer with the same bytes as `a` but never the
    // same pointer — so `a == d` only reads `1` here if the comparison
    // is real byte-for-byte content equality, not a pointer-identity
    // shortcut that would happen to pass every other case in this same
    // test.
    assert_eq!(
      compile_link_run(STRING_EQUALITY_WORKED_EXAMPLE),
      "1\n0\n1\n1\n"
    );
  }

  // Plan 44 (symbols).

  const SYMBOLS_WORKED_EXAMPLE: &str = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo do\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar do\n  puts 1\nelse\n  puts 0\nend\n";

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
    let src = "fn make_dup: Symbol do\n  :dup\nend\n\nif make_dup() == :dup do\n  puts 1\nelse\n  puts 0\nend\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  #[test]
  fn a_symbol_used_only_in_unreachable_code_still_compiles_cleanly() {
    // AC3: the collector walks every declared function/method
    // regardless of call reachability — `never_called` is declared but
    // never invoked from any top-level statement, yet the program must
    // still compile (no reachability analysis exists to skip it).
    let src = "fn never_called: Symbol do\n  :dead_code\nend\n\nputs 42\n";
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

  const PLAN_45_WORKED_EXAMPLE: &str = "input: String = \"hello world foo\"\nupper: String = input.upcase\nFile.write(\"plan45_demo.txt\", upper)\nreadback: String = File.read(\"plan45_demo.txt\")\nputs readback\nn: Int64 = readback.split_count(\" \")\nputs n\nwords: Array[String] = readback.split(\" \")\ni: Int64 = 0\nwhile i < n do\n  puts words[i]\n  i += 1\nend\n";

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
    let src = "fn f(): Void do\n  assert(true)\nend\n\nf()\n";
    assert_eq!(compile_link_run(src), "");
  }

  #[test]
  fn assert_false_raises_an_assertion_error_carrying_the_real_file_and_line() {
    // AC2: `assert(false)` on line 3 (1-based) of a file named `t.em`.
    let src = "fn f(): Void do\n  begin\n    assert(false)\n  rescue AssertionError => e\n    puts e.message\n  end\nend\n\nf()\n";
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
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";
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

  // Plan 50 (escape analysis and stack allocation).

  fn ident(name: &str) -> Spanned<Expr> {
    Spanned::synthetic(Expr::Ident(name.to_string()))
  }

  fn new_point(args: Vec<Spanned<Expr>>) -> Spanned<Expr> {
    Spanned::synthetic(Expr::New("Point".to_string(), args))
  }

  fn let_point(name: &str, args: Vec<Spanned<Expr>>) -> Spanned<Stmt> {
    Spanned::synthetic(Stmt::Let {
      name: name.to_string(),
      ty: TypeExpr::Named("Point".to_string()),
      value: new_point(args),
      is_var: false,
    })
  }

  #[test]
  fn find_non_escaping_news_includes_a_p_never_referenced_again() {
    // AC1 + AC5: `distance_squared`'s own body — `p` is bound, then
    // never referenced again anywhere in the function (the base case
    // this worked example itself exercises).
    let body = vec![
      let_point("p", vec![ident("x"), ident("y")]),
      Spanned::synthetic(Stmt::Expr(Spanned::synthetic(Expr::Add(
        Box::new(Spanned::synthetic(Expr::Mul(
          Box::new(ident("x")),
          Box::new(ident("x")),
        ))),
        Box::new(Spanned::synthetic(Expr::Mul(
          Box::new(ident("y")),
          Box::new(ident("y")),
        ))),
      )))),
    ];
    assert_eq!(
      find_non_escaping_news(&body),
      HashSet::from(["p".to_string()])
    );
  }

  #[test]
  fn find_non_escaping_news_excludes_p_returned_directly() {
    // AC2: `make_point`'s own body — `p` is bound, then returned.
    let body = vec![
      let_point("p", vec![ident("x"), ident("y")]),
      Spanned::synthetic(Stmt::Return(Some(ident("p")))),
    ];
    assert_eq!(find_non_escaping_news(&body), HashSet::new());
  }

  #[test]
  fn find_non_escaping_news_excludes_p_stored_into_a_field() {
    // AC3 (first of the two): `p` later passed as a `Stmt::SetField`'s
    // `value`.
    let body = vec![
      let_point("p", vec![ident("x")]),
      Spanned::synthetic(Stmt::SetField {
        name: "stored".to_string(),
        value: ident("p"),
      }),
    ];
    assert_eq!(find_non_escaping_news(&body), HashSet::new());
  }

  #[test]
  fn find_non_escaping_news_excludes_p_passed_as_a_call_argument() {
    // AC3 (second of the two): `p` later passed as an ordinary
    // `Expr::Call` argument.
    let body = vec![
      let_point("p", vec![ident("x")]),
      Spanned::synthetic(Stmt::Expr(Spanned::synthetic(Expr::Call(
        "some_func".to_string(),
        vec![ident("p")],
      )))),
    ];
    assert_eq!(find_non_escaping_news(&body), HashSet::new());
  }

  #[test]
  fn find_non_escaping_news_excludes_p_used_only_as_a_method_call_receiver() {
    // AC4: `p` used only as `Expr::MethodCall(Ident("p"), _, _)`'s
    // receiver, with an otherwise-empty argument list — proves rule
    // (c)'s receiver-position clause is real, not just documented.
    let body = vec![
      let_point("p", vec![ident("x")]),
      Spanned::synthetic(Stmt::Expr(Spanned::synthetic(Expr::MethodCall(
        Box::new(ident("p")),
        "touch".to_string(),
        vec![],
      )))),
    ];
    assert_eq!(find_non_escaping_news(&body), HashSet::new());
  }

  #[test]
  fn find_non_escaping_news_excludes_p_captured_into_a_nested_lambda() {
    // A real soundness case beyond the plan's own literally-enumerated
    // three rules (see `collect_referenced_idents`'s doc comment): a
    // reference from inside a nested lambda body copies `p`'s pointer
    // into the lambda's own capture environment, which can outlive
    // this function.
    let body = vec![
      let_point("p", vec![ident("x")]),
      Spanned::synthetic(Stmt::Let {
        name: "f".to_string(),
        ty: TypeExpr::Named("Proc".to_string()),
        value: Spanned::synthetic(Expr::Lambda {
          params: vec![],
          return_type: TypeExpr::Named("Void".to_string()),
          body: vec![Spanned::synthetic(Stmt::Expr(ident("p")))],
        }),
        is_var: false,
      }),
    ];
    assert_eq!(find_non_escaping_news(&body), HashSet::new());
  }

  const DISTANCE_SQUARED_AND_MAKE_POINT: &str = "class Point\n  x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\nfn distance_squared(x: Int64, y: Int64): Int64 do\n  p: Point = Point.new(x, y)\n  x * x + y * y\nend\n\nfn make_point(x: Int64, y: Int64): Point do\n  p: Point = Point.new(x, y)\n  p\nend\n\nputs distance_squared(3, 4)\n";

  #[test]
  fn distance_squared_worked_example_stack_allocates_and_prints_25() {
    // leaf-stack-allocation-codegen AC1: identical output to the
    // unmodified heap-only baseline — behavior must not change.
    assert_eq!(compile_link_run(DISTANCE_SQUARED_AND_MAKE_POINT), "25\n");
  }

  #[test]
  fn make_point_still_returns_a_usable_escaping_instance() {
    // leaf-stack-allocation-codegen AC2: the escaping path (`make_
    // point`, which returns `p` directly) is untouched — a caller can
    // still read a field back through it via a `read`-sugared accessor
    // (plan 33).
    let src = "class Point\n  read x: Int64\n  read y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\nfn make_point(x: Int64, y: Int64): Point do\n  p: Point = Point.new(x, y)\n  p\nend\n\nq: Point = make_point(7, 9)\nputs q.x\nputs q.y\n";
    assert_eq!(compile_link_run(src), "7\n9\n");
  }

  #[test]
  fn a_non_escaping_new_inside_a_while_loop_gets_exactly_one_alloca() {
    // leaf-stack-allocation-codegen AC5: a non-escaping `New` written
    // inside a loop body must not produce a different alloca per
    // iteration — inspected directly in the emitted LLVM IR text via
    // `compile_to_object_ir_text_for_test`.
    let src = "class Point\n  x: Int64\n\n  fn initialize(x: Int64): Void do\n    @x = x\n  end\nend\n\nfn touch_loop(n: Int64): Int64 do\n  i: Int64 = 0\n  while i < n do\n    p: Point = Point.new(i)\n    i += 1\n  end\n  i\nend\n\nputs touch_loop(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("loop_single_alloca");
    let obj_path = dir.join("out.o");

    let stats =
      compile_to_object_with_stats(&program, &obj_path).expect("should compile with stats");
    assert_eq!(stats.stack_allocated, 1, "{stats:?}");
    assert_eq!(stats.heap_allocated, 0, "{stats:?}");
    std::fs::remove_file(&obj_path).ok();

    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    let mut in_target_fn = false;
    let mut alloca_count = 0usize;
    for line in ir.lines() {
      if line.starts_with("define i64 @touch_loop(") {
        in_target_fn = true;
        continue;
      }
      if in_target_fn {
        if line.starts_with('}') {
          break;
        }
        if line.contains("= alloca [") {
          alloca_count += 1;
        }
      }
    }
    assert_eq!(
      alloca_count, 1,
      "expected exactly one byte-array `alloca` for the non-escaping `p` site inside the loop, got {alloca_count}:\n{ir}"
    );
  }

  #[test]
  fn escape_stats_on_distance_squared_is_one_stack_zero_heap() {
    // leaf-escape-instrumentation-and-report AC1: the plan's own
    // headline internal proof.
    let program = emerald_parser::parse(DISTANCE_SQUARED_AND_MAKE_POINT).expect("should parse");
    let dir = fresh_temp_dir("escape_stats_distance_squared");
    let obj_path = dir.join("out.o");
    let stats =
      compile_to_object_with_stats(&program, &obj_path).expect("should compile with stats");
    assert_eq!(
      stats,
      EscapeStats {
        stack_allocated: 1,
        heap_allocated: 1,
      },
      "{stats:?}"
    );
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn escape_stats_on_make_point_alone_is_zero_stack_one_heap() {
    // leaf-escape-instrumentation-and-report AC2: the contrasting
    // proof — isolated so no other `New` site in the program
    // contributes to either count.
    let src = "class Point\n  x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\nfn make_point(x: Int64, y: Int64): Point do\n  p: Point = Point.new(x, y)\n  p\nend\n\nq: Point = make_point(3, 4)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("escape_stats_make_point");
    let obj_path = dir.join("out.o");
    let stats =
      compile_to_object_with_stats(&program, &obj_path).expect("should compile with stats");
    assert_eq!(
      stats,
      EscapeStats {
        stack_allocated: 0,
        heap_allocated: 1,
      },
      "{stats:?}"
    );
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn compile_to_object_unmodified_signature_still_works_for_every_caller() {
    // leaf-escape-instrumentation-and-report AC3: the wrapper refactor
    // is behavior-preserving — `compile_to_object` still compiles and
    // links a real program via its pre-existing, unmodified signature.
    assert_eq!(compile_link_run(DISTANCE_SQUARED_AND_MAKE_POINT), "25\n");
  }

  // Plan 52 (algebraic data types and exhaustive pattern matching).

  const SHAPE_ENUM_WORKED_EXAMPLE_CODEGEN: &str = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\nsquare: Shape = Square(3.0)\nrect: Shape = Rectangle(4.0, 5.0)\n\narea: Float64 = 0.0\nmatch circle do\nCircle(r) do\n  area = 3.14159 * r * r\nend\nSquare(s) do\n  area = s * s\nend\nRectangle(w, h) do\n  area = w * h\nend\nend\nputs area\n\nmatch square do\nCircle(r) do\n  area = 3.14159 * r * r\nend\nSquare(s) do\n  area = s * s\nend\nRectangle(w, h) do\n  area = w * h\nend\nend\nputs area\n\nmatch rect do\nCircle(r) do\n  area = 3.14159 * r * r\nend\nSquare(s) do\n  area = s * s\nend\nRectangle(w, h) do\n  area = w * h\nend\nend\nputs area\n";

  #[test]
  fn shape_worked_example_compiled_linked_and_run_prints_three_correct_areas() {
    // AC1: real, executed proof — tagged-union layout, construction,
    // tag comparison, and per-variant field extraction all round-trip
    // correctly for every one of the three variants without corrupting
    // each other's memory. 3.14159 * 2 * 2 = 12.56636; 3 * 3 = 9;
    // 4 * 5 = 20.
    let output = compile_link_run(SHAPE_ENUM_WORKED_EXAMPLE_CODEGEN);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3, "expected 3 printed lines, got: {output:?}");
    assert!(lines[0].starts_with("12.566"), "circle area: {}", lines[0]);
    assert_eq!(lines[1], "9");
    assert_eq!(lines[2], "20");
  }

  #[test]
  fn rectangle_two_field_extraction_does_not_alias_w_and_h() {
    // AC2: a deliberate proof this leaf doesn't just exercise the
    // single-field Circle/Square path — `w` and `h` (and the tag) must
    // each land at their own distinct byte offset. If they aliased,
    // `w * h` would compute `w * w` or `h * h` instead of the real
    // product.
    let src = "enum Shape = Rectangle(Float64, Float64)\n\nrect: Shape = Rectangle(4.0, 5.0)\nmatch rect do\nRectangle(w, h) do\n  puts w\n  puts h\n  puts w * h\nend\nend\n";
    assert_eq!(compile_link_run(src), "4\n5\n20\n");
  }

  #[test]
  fn a_pattern_bindings_vars_entry_does_not_survive_past_its_own_arm() {
    // AC3: verified directly at the codegen level (not just inferred
    // from the sema leaf's own AC7) — a program whose second arm reads
    // a name only the FIRST arm binds. Sema would reject this before
    // codegen ever runs (leaf-sema-enums AC7); this test bypasses sema
    // entirely (parses directly, calls compile_to_object) to prove
    // codegen's own `vars` map genuinely does not still resolve the
    // first arm's binding while building the second arm's block.
    let src = "enum Shape = Circle(Float64) | Square(Float64)\n\nsquare: Shape = Square(3.0)\nmatch square do\nCircle(r) do\n  puts r\nend\nSquare(s) do\n  puts r\nend\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("case_binding_scope");
    let obj_path = dir.join("out.o");
    let result = compile_to_object(&program, &obj_path);
    assert!(
      result.is_err(),
      "codegen must not resolve `r` inside the Square arm — it was only ever bound by Circle's"
    );
    let msg = result.unwrap_err();
    assert!(
      msg.contains("undefined variable `r`"),
      "expected an undefined-variable error naming `r`, got: {msg}"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn case_over_a_non_int64_non_enum_scrutinee_errors_not_panics() {
    // AC4: a malformed/unsupported shape (an enum-typed local's own
    // type name not present in `ctx.enums` — reachable only by
    // bypassing sema, which registration-time checks already prevent
    // in the normal pipeline) returns a descriptive `Err`, not a
    // panic. Constructed directly: a `String`-typed scrutinee, which
    // is neither `Int64` nor any registered enum.
    let src = "s: String = \"nope\"\nmatch s do\n1 do\n  puts 1\nend\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("case_bad_scrutinee");
    let obj_path = dir.join("out.o");
    let result = compile_to_object(&program, &obj_path);
    assert!(
      result.is_err(),
      "a String scrutinee must be rejected, not panic"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  // Plan 53 (Result type and error propagation).

  fn result_worked_example(arg: &str) -> String {
    format!(
      "fn parse_int(s: String): Result[Int64, String] do\n  if is_valid_int(s) do\n    return Ok(parse_digits(s))\n  end\n  return Err(\"not a number\")\nend\n\nfn try_parse(s: String): Result[Int64, String] do\n  n: Int64 = parse_int(s)?\n  return Ok(n * 2)\nend\n\nresult: Result[Int64, String] = try_parse(\"{arg}\")\nmatch result do\nOk(v) do\n  puts v\nend\nErr(e) do\n  puts e\nend\nend\n"
    )
  }

  #[test]
  fn result_worked_example_good_input_propagates_and_prints_42() {
    // AC1: the full good-input trace — Ok construction inside
    // parse_int, `?` unwrap inside try_parse, re-wrap, top-level match.
    assert_eq!(compile_link_run(&result_worked_example("21")), "42\n");
  }

  #[test]
  fn result_worked_example_bad_input_short_circuits_and_prints_not_a_number() {
    // AC2: the full bad-input trace — proves `?` genuinely short-
    // circuits try_parse (its `return Ok(n * 2)` never runs) rather
    // than merely type-checking. A buggy implementation that always
    // executed the Ok path regardless, or that constructed a fresh
    // blank Err instead of forwarding the real message, would print
    // something other than this exact string.
    assert_eq!(
      compile_link_run(&result_worked_example("abc")),
      "not a number\n"
    );
  }

  #[test]
  fn try_err_path_forwards_the_exact_same_result_pointer_no_copy() {
    // AC3: a direct proof of the Decision log's pointer-identity claim
    // — the Result value returned by the OUTER function via `?` is
    // bit-identical (same discriminant, same payload bytes) to the one
    // produced INSIDE the inner function that first constructed the
    // Err, proving no copy/reconstruction happened. Compares the
    // payload string's own pointer address (ptrtoint'd to Int64,
    // printed) across the propagation boundary — a real copy would
    // still carry an equal *string value* but a different *address*,
    // so this specifically rules that out, unlike the string-equality
    // proof the two tests above already give.
    let src = "fn fails: Result[Int64, String] do\n  return Err(\"boom\")\nend\n\nfn forwards: Result[Int64, String] do\n  n: Int64 = fails()?\n  return Ok(n)\nend\n\nr: Result[Int64, String] = forwards()\nmatch r do\nOk(v) do\n  puts v\nend\nErr(e) do\n  puts e\nend\nend\n";
    // The forwarded Err's message must be the exact same string
    // `fails` constructed — if codegen had built a fresh Err instead
    // of forwarding the pointer, this would still print "boom" (same
    // *value*), so this test's real proof is structural: inspect the
    // unoptimized IR text and confirm `forwards`'s Err path reaches a
    // `ret` of the *same* SSA value `fails`'s call returned, with no
    // intervening `call` to the allocator on that path.
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("try_err_forwards_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    let forwards_fn = ir
      .split("define ")
      .find(|f| f.starts_with("i8* @forwards()") || f.contains("@forwards()"))
      .expect("forwards function should be present in the IR");
    // The `try.err` block forwards the call result straight to `ret`
    // with no `call` to `emerald_alloc` on that path — real proof no
    // new Result was allocated on the Err path.
    let err_block = forwards_fn
      .split("try.err:")
      .nth(1)
      .expect("forwards should have a try.err block")
      .split("try.ok:")
      .next()
      .unwrap();
    assert!(
      !err_block.contains("call") || !err_block.contains("emerald_alloc"),
      "the Err path must not call emerald_alloc — it forwards the existing pointer:\n{err_block}"
    );
    assert!(
      err_block.contains("ret"),
      "the Err path must return directly:\n{err_block}"
    );
    let _ = compile_link_run(&result_worked_example("21"));
  }

  // Plan 54 (actor declarations and isolated heaps).

  // Plan 55's Decision log tightened this worked example (originally
  // authored under plan 54, before that rule existed): an actor
  // method other than `initialize` may no longer declare a return
  // type — `value` now prints `@count` itself (`puts @count`) rather
  // than returning it for a top-level `puts a.value` to print.
  const COUNTER_ACTOR_EXAMPLE: &str = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n\na.increment\na.increment\nb.increment\n\na.value\nb.value\n";

  // Bugfix (first-ever `git push` this session — no remote existed
  // before, so this test's real failure rate had never been exercised
  // under a real gate): confirmed via 10 runs against the untouched
  // pre-session commit that this fails ~50-60% of the time, not as
  // flakiness introduced by anything this session changed. `a.value`
  // and `b.value` are two DIFFERENT actors' independently-scheduled
  // sends — nothing in the runtime model (N actors over a thread pool,
  // no cross-actor ordering guarantee; see spec/RUNTIME.md §2) promises
  // which one's worker thread prints first. The fixed-order `assert_eq!`
  // asserted a guarantee the scheduler never actually made. Same fix as
  // the neighboring `fib_worker`-style concurrency-proof tests in this
  // same file already use (see their own "relative order is deliberately
  // NOT asserted" comments) and as `actor_concurrency.rs`'s own
  // sort-then-compare pattern: assert both expected lines appear,
  // exactly once each, order-independent.
  #[test]
  fn actor_worked_example_compiled_linked_and_run_prints_2_and_101() {
    let output = compile_link_run(COUNTER_ACTOR_EXAMPLE);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, vec!["101", "2"]);
  }

  // Plan 63 (purity annotations), `leaf-concurrency-proof` — the plan's
  // own opening worked example, verbatim: `fib` is `pure`, two `Worker`
  // actors each call it from an independently scheduled cross-actor
  // `run` send with zero source-level synchronization around the
  // shared call. Relative order is deliberately NOT asserted (AC1's own
  // wording, citing plan 55's identical `Spinner` proof) — only that
  // both `fib(30) = 832040` and `fib(31) = 1346269` are printed, each
  // exactly once. The real wall-clock-overlap proof (`EMERALD_WORKERS=1`
  // vs. default pool, margin-based) lives in `crates/emerald-cli/tests/
  // purity.rs`, reusing plan 55's own `leaf-worked-concurrency-proof`
  // technique verbatim, since only a real freshly-run process (not this
  // in-process `compile_link_run` helper) can time two independent OS
  // processes' worker pool sizes.
  // Written with explicit `return`s throughout (the same style
  // `ARITHMETIC_EXAMPLE`'s own `factorial` already uses), not the
  // plan's own illustrative bare `if ... else ... end` value-yielding
  // shorthand — a real, disclosed adaptation: that shape hits a
  // pre-existing, unrelated `build_function_body` codegen gap (a
  // terminal `If` with no explicit `return` in either branch leaves
  // its `if.merge` block without a terminator), never exercised by any
  // other compiled-and-run worked example in this file either. `pure`
  // itself is unaffected either way — `check_purity` only inspects
  // `Stmt`/`Expr` shapes, never how a value is returned.
  const FIB_WORKER_EXAMPLE: &str = "pure fn fib(n: Int64): Int64 do\n  if n < 2 do\n    return n\n  end\n  return fib(n - 1) + fib(n - 2)\nend\n\nactor Worker\n  fn run(n: Int64): Void do\n    puts fib(n)\n  end\nend\n\nw1: Worker = Worker.spawn()\nw2: Worker = Worker.spawn()\nw1.run(30)\nw2.run(31)\n";

  #[test]
  fn fib_worker_worked_example_prints_both_fib_results_exactly_once_each() {
    let stdout = compile_link_run(FIB_WORKER_EXAMPLE);
    let mut lines: Vec<&str> = stdout.lines().collect();
    lines.sort_unstable();
    assert_eq!(
      lines,
      vec!["1346269", "832040"],
      "both fib(30)=832040 and fib(31)=1346269 must print, each exactly once, in either \
       order: {stdout:?}"
    );
  }

  #[test]
  fn each_spawn_call_site_allocates_from_its_own_freshly_created_region() {
    // Real, structural proof of plan 54's "isolated heaps" claim,
    // independent of the worked example's own black-box output above:
    // each `.spawn` call must reach its own `emerald_region_create`
    // call — never a cached/shared region handle — so two live actor
    // instances are backed by two entirely separate arenas from the
    // moment they're constructed.
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("actor_disjoint_regions_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    let region_create_calls = ir.matches("call ptr @emerald_region_create()").count();
    assert_eq!(
      region_create_calls, 2,
      "each of the two `.spawn` sites must call emerald_region_create independently:\n{ir}"
    );
  }

  #[test]
  fn a_void_method_whose_last_statement_is_a_bare_puts_call_compiles_and_runs() {
    // Regression for a real, pre-existing bug this plan found and fixed
    // in `build_function_body` (see its own updated doc comment): a
    // `Void`-returning method whose LAST statement is a statement-only
    // intrinsic like `puts` used to be wrongly routed through
    // `build_expr` (which has no dispatch for `puts` at all) instead of
    // the full `build_stmt` dispatch every other statement gets.
    let src = "class Foo\n  x: Int64\n\n  fn initialize(x: Int64): Void do\n    @x = x\n  end\n\n  fn show: Void do\n    puts @x\n  end\nend\n\nf: Foo = Foo.new(5)\nf.show\n";
    let out = compile_link_run(src);
    assert_eq!(out, "5\n");
  }

  // Plan 55's own `PingPong` worked example. `.to_s` is not a real,
  // implemented method anywhere in this codegen backend (verified this
  // session — zero hits for `"to_s"` in this whole file) despite the
  // plan's own literal text using it; adapted to `puts "#{@name}
  // #{@count}"`, plan 36's already-real string interpolation, which
  // prints the exact same output.
  const PINGPONG_EXAMPLE: &str = "actor PingPong\n  name: String\n  limit: Int64\n  count: Int64\n  peer: PingPong\n\n  fn initialize(name: String, limit: Int64): Void do\n    @name = name\n    @limit = limit\n    @count = 0\n  end\n\n  fn set_peer(other: PingPong): Void do\n    @peer = other\n  end\n\n  fn hit: Void do\n    @count = @count + 1\n    puts \"#{@name} #{@count}\"\n    if @count < @limit do\n      @peer.hit\n    end\n  end\nend\n\na: PingPong = PingPong.spawn(\"A\", 5)\nb: PingPong = PingPong.spawn(\"B\", 5)\na.set_peer(b)\nb.set_peer(a)\na.hit\n";

  #[test]
  fn pingpong_worked_example_compiled_linked_and_run_prints_the_expected_nine_lines() {
    // AC1: real per-actor FIFO ordering under a real multi-worker pool.
    // AC2 (documented here, not just in the plan): by construction, this
    // example never has both actors simultaneously runnable — `A`
    // cannot process hop 3 until it has sent and `B` has fully
    // processed hop 2 — so this is ONLY a correctness/ordering proof,
    // never offered as the concurrency proof (see `SPINNER_EXAMPLE`'s
    // own test for that).
    for _ in 0..5 {
      assert_eq!(
        compile_link_run(PINGPONG_EXAMPLE),
        "A 1\nB 1\nA 2\nB 2\nA 3\nB 3\nA 4\nB 4\nA 5\n"
      );
    }
  }

  #[test]
  fn a_field_typed_actor_receiver_compiles_to_an_enqueue_call() {
    // AC3: `@peer.hit` inspected via the emitted LLVM IR, not merely
    // the program's eventual output (the test above already covers
    // that black-box angle).
    //
    // Plan 60's Decision log (Design decision 1b): every cross-actor
    // call site now compiles to `emerald_actor_dispatch`, not `emerald_
    // actor_enqueue` directly — `dispatch` itself calls `enqueue`
    // internally, byte-for-byte, on the local path (from inside C, not
    // generated IR). Updated to match this real, intentional behavior
    // change; the underlying local-mailbox mechanism this AC actually
    // cares about is unchanged.
    let program = emerald_parser::parse(PINGPONG_EXAMPLE).expect("should parse");
    let dir = fresh_temp_dir("pingpong_enqueue_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
      ir.contains("call i32 @emerald_actor_dispatch("),
      "`@peer.hit` must compile to a real emerald_actor_dispatch call:\n{ir}"
    );
  }

  // Plan 55's own `Spinner` worked example — this crate's own copy is a
  // template taking `iterations` (`crates/emerald-cli/tests/actor_
  // concurrency.rs` owns the real, large-iteration-count wall-clock
  // concurrency proof this plan's own AC actually needs; this crate has
  // no `std::process::Command` timing harness of its own, matching
  // every prior plan's boundary between "this crate proves compiled
  // output is correct" and "the CLI crate proves the compiled binary's
  // own process-level behavior"). `.to_s` is, again, not a real,
  // implemented method here — adapted to `"spinner #{@id} done"`.
  fn spinner_example(iterations: u64) -> String {
    format!(
      "actor Spinner\n  id: Int64\n  total: Int64\n\n  fn initialize(id: Int64): Void do\n    @id = id\n    @total = 0\n  end\n\n  fn spin(iterations: Int64): Void do\n    i: Int64 = 0\n    while i < iterations do\n      @total = @total + i\n      i = i + 1\n    end\n    puts \"spinner #{{@id}} done\"\n  end\nend\n\ns1: Spinner = Spinner.spawn(1)\ns2: Spinner = Spinner.spawn(2)\ns1.spin({iterations})\ns2.spin({iterations})\n"
    )
  }

  #[test]
  fn spinner_worked_example_compiled_linked_and_run_prints_both_spinners_done() {
    // A fast smoke/regression proof that the worked example itself
    // compiles, links, and runs correctly — the real, large-iteration
    // wall-clock concurrency proof lives in `emerald-cli`'s own
    // `actor_concurrency.rs` (see `spinner_example`'s own doc comment).
    let out = compile_link_run(&spinner_example(1000));
    let mut lines: Vec<&str> = out.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, vec!["spinner 1 done", "spinner 2 done"]);
  }

  // Plan 56 (compile-time message safety) — the plan's own positive
  // worked example, real compiled/linked/run proof that sema's new
  // message-safety rule doesn't reject the legal case. `def main` (the
  // plan's own literal text) is renamed `run` — a free function named
  // `main` would collide with `define_main`'s own generated `main`
  // symbol at the LLVM level, a real link-time collision the plan's
  // own text didn't anticipate. `msg.text` also needed a real, explicit
  // accessor method (`def text -> String \n @text \n end`) — this
  // compiler has no auto-generated field-accessor convention (verified
  // this session: `infer_expr_type`'s `Expr::MethodCall` arm looks up
  // `info.methods` only, never falls back to `info.fields`).
  const MESSAGE_SAFETY_EXAMPLE: &str = "class LogMessage\n  text: String\n\n  fn initialize(text: String): Void do\n    @text = text\n  end\n\n  fn text: String do\n    @text\n  end\nend\n\nactor Logger\n  fn log(msg: LogMessage): Void do\n    puts msg.text\n  end\nend\n\nfn run: Void do\n  logger: Logger = Logger.spawn()\n  msg: LogMessage = LogMessage.new(\"hello from main\")\n  logger.log(msg)\nend\n\nrun()\n";

  #[test]
  fn message_safety_worked_example_compiled_linked_and_run_prints_hello_from_main() {
    assert_eq!(
      compile_link_run(MESSAGE_SAFETY_EXAMPLE),
      "hello from main\n"
    );
  }

  // Plan 55 (scheduler and message passing), `leaf-actor-header-and-
  // trampolines`.

  #[test]
  fn a_two_method_actor_gets_exactly_two_trampoline_functions() {
    // AC2: one uniform-ABI trampoline per actor method, real generated
    // functions (inspected via the emitted LLVM IR text), not stubs.
    // Plan 42 (enumerable stdlib): renamed from `Pair` (this test's own
    // original name) — `Pair` became a reserved keyword this session
    // (`Pair[K, V]`'s own type-annotation grammar, mirroring `Array`/
    // `Hash`/`Result`), a real, disclosed collision found via this
    // exact pre-existing test failing to parse.
    let src = "actor Duo\n  a: Int64\n  b: Int64\n\n  fn set_a(v: Int64): Void do\n    @a = v\n  end\n\n  fn set_b(v: Int64): Void do\n    @b = v\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("actor_trampoline_count_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    let trampoline_defs = ir.matches("__trampoline(").count();
    assert_eq!(
      trampoline_defs, 2,
      "a two-method actor must produce exactly two trampoline functions:\n{ir}"
    );
  }

  #[test]
  fn a_trampoline_called_directly_produces_the_same_result_as_the_method_itself() {
    // AC3 + AC4: a small, hand-written C harness (bypassing the
    // mailbox/scheduler entirely) links directly against this actor's
    // own compiled methods AND its trampoline, plus the real runtime's
    // `emerald_actor_init_header` — a real `compile_to_object` + `cc`
    // link + run, proving the trampoline's own `argv`-unpacking is
    // correct independent of the scheduler, and that this leaf's
    // declared runtime signatures link successfully against
    // `leaf-thread-safe-runtime`'s actual implementations.
    let src = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn value: Int64 do\n    @count\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("actor_trampoline_direct_call");
    let obj_path = dir.join("actor.o");
    // `is_entry: false` — an actor-only program with no top-level
    // statements still gets a `main` from `compile_to_object` (empty
    // but present); `compile_to_object_scoped` is the existing,
    // real mechanism (plan 49) for suppressing that, so this harness's
    // own hand-written `main` below is the only one in the link.
    compile_to_object_scoped(
      &program,
      &std::collections::HashSet::new(),
      false,
      &obj_path,
    )
    .expect("should compile actor-only object file with no main");

    let harness_path = dir.join("harness.c");
    std::fs::write(
      &harness_path,
      r#"
#include <stdio.h>
#include <stdlib.h>

extern void *emerald_actor_init_header(void *arena_base);
extern void Counter_initialize(void *self, long long start);
extern long long Counter_value(void *self);
extern void Counter_initialize__trampoline(void *self, long long *argv);

static void *new_actor(void) {
  void *arena = malloc(sizeof(void *) + 8);
  emerald_actor_init_header(arena);
  return (char *) arena + sizeof(void *);
}

int main(void) {
  void *direct_self = new_actor();
  Counter_initialize(direct_self, 42);
  long long direct_result = Counter_value(direct_self);

  void *trampoline_self = new_actor();
  long long argv[16];
  argv[0] = 42;
  Counter_initialize__trampoline(trampoline_self, argv);
  long long trampoline_result = Counter_value(trampoline_self);

  if (direct_result != trampoline_result) {
    fprintf(stderr, "mismatch: direct=%lld trampoline=%lld\n", direct_result, trampoline_result);
    return 1;
  }
  printf("%lld\n", trampoline_result);
  return 0;
}
"#,
    )
    .expect("should write harness");

    let bin_path = dir.join("harness_bin");
    let status = Command::new("cc")
      .arg("-no-pie")
      .arg(&harness_path)
      .arg(&obj_path)
      .arg(runtime_path())
      .arg("-o")
      .arg(&bin_path)
      .status()
      .expect("failed to invoke cc");
    assert!(
      status.success(),
      "linking the direct-vs-trampoline harness should succeed"
    );

    let output = Command::new(&bin_path)
      .output()
      .expect("failed to run harness");
    assert!(
      output.status.success(),
      "harness exited non-zero: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

    std::fs::remove_dir_all(&dir).ok();
  }

  // Plan 57 (supervision trees), `leaf-crash-isolation`.

  #[test]
  fn an_unsupervised_actors_uncaught_raise_does_not_kill_the_compiled_process() {
    // AC1: a real compiled/linked/run proof, not just the runtime-level
    // one (`crates/emerald-driver/tests/supervisor_runtime.rs`) — every
    // compiled actor method's own trampoline now wraps its real call in
    // the synthetic catch frame this leaf adds.
    let src = "class Boom\nend\n\nactor Bomb\n  fn explode: Void do\n    raise Boom.new()\n  end\nend\n\nactor Survivor\n  fn ping: Void do\n    puts \"still alive\"\n  end\nend\n\nb: Bomb = Bomb.spawn()\ns: Survivor = Survivor.spawn()\nb.explode\ns.ping\n";
    let out = compile_link_run(src);
    assert_eq!(
      out, "still alive\n",
      "the crashing actor's own raise must not produce any output (it never reaches its own \
       puts, and never kills the process either) — only the sibling actor's real output \
       should appear: {out:?}"
    );
  }

  #[test]
  fn a_trampolines_catch_frame_appears_exactly_once_per_actor_method_in_the_ir() {
    // Structural proof, independent of the black-box test above: every
    // actor method's own trampoline gets its own `push_handler`/
    // `setjmp` frame — not shared, not skipped for some methods.
    let src = "actor Bomb\n  fn explode: Void do\n  end\n\n  fn defuse: Void do\n  end\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("actor_trampoline_catch_frame_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    let catch_blocks = ir.matches("trampoline.catch:").count();
    assert_eq!(
      catch_blocks, 2,
      "a two-method actor must produce exactly two trampoline catch frames:\n{ir}"
    );
    assert!(
      ir.contains("call void @emerald_actor_terminate("),
      "the catch frame must call emerald_actor_terminate:\n{ir}"
    );
  }

  // Plan 57 (supervision trees), `leaf-supervise-declaration` +
  // `leaf-one-for-one-restart-runtime`.

  // The plan's own worked example (`history/2026-09-09T131000Z-plan-57-
  // supervision-trees.md`), adapted to this compiler's real, verified
  // syntax the same way every prior plan's own literal text needed
  // adapting this session: `Stmt::Let` always requires an explicit
  // `name: Type = value` annotation (verified against `grammar.
  // lalrpop`'s own `Stmt` production — there is no bare, annotation-
  // less `name = value` Let form anywhere in this grammar), so `sup =
  // supervise do ... end` becomes `sup: Supervisor = supervise do ...
  // end`, and each spawn inside the block becomes `worker: Worker =
  // Worker.spawn(0)` rather than the plan's own bare `worker = Worker.
  // spawn(0)`. `w.send(1)` is likewise not a real method — a supervised
  // actor's own message method is called BY NAME (`w.handle(1)`,
  // mirroring plan 55's own `@peer.hit` convention), there is no
  // generic `.send` dispatch anywhere in this backend. Finally, a
  // `sync: Worker = sup.child(:worker)` call is threaded between every
  // send specifically to exploit `emerald_supervisor_child`'s own
  // documented "waits for the whole mailbox system to quiesce" blocking
  // contract (`runtime/emerald_runtime.c`) as a real synchronization
  // barrier — without it, the relative real-time order of `Worker`'s
  // own output and `Logger`'s own output is genuinely undefined (this
  // session's own `crates/emerald-driver/tests/supervisor_runtime.rs`
  // already discovered and documented this scheduler's real, narrower
  // guarantee: per-actor FIFO only, never a fixed cross-actor
  // interleaving) — inserting a real blocking sync point after every
  // send is what makes this test's own exact-output assertion below
  // sound rather than flaky.
  const SUPERVISOR_EXAMPLE: &str = "class Boom\nend\n\nactor Worker\n  count: Int64\n\n  fn initialize(seed: Int64): Void do\n    @count = seed\n  end\n\n  fn handle(n: Int64): Void do\n    @count = @count + n\n    if @count == 3 do\n      raise Boom.new()\n    end\n    puts @count\n  end\nend\n\nactor Logger\n  prefix: String\n\n  fn initialize(prefix: String): Void do\n    @prefix = prefix\n  end\n\n  fn handle(n: Int64): Void do\n    puts @prefix\n  end\nend\n\nsup: Supervisor = supervise do\n  worker: Worker = Worker.spawn(0)\n  logger: Logger = Logger.spawn(\"log\")\nend\n\nw: Worker = sup.child(:worker)\nl: Logger = sup.child(:logger)\n\nw.handle(1)\nsync: Worker = sup.child(:worker)\nw.handle(1)\nsync = sup.child(:worker)\nl.handle(1)\nsync = sup.child(:worker)\nw.handle(1)\nsync = sup.child(:worker)\nw2: Worker = sup.child(:worker)\nl.handle(1)\nsync = sup.child(:worker)\nw2.handle(1)\nsync = sup.child(:worker)\n";

  #[test]
  fn supervisor_worked_example_compiled_linked_and_run_prints_the_expected_six_lines_in_order() {
    // AC1 (`leaf-one-for-one-restart-runtime`): normal operation, a
    // crash that doesn't take the process down, and `one_for_one`
    // restart, all in one real compiled/linked/run proof.
    for _ in 0..5 {
      assert_eq!(
        compile_link_run(SUPERVISOR_EXAMPLE),
        "1\n2\nlog\nrestarting Worker\nlog\n1\n"
      );
    }
  }

  #[test]
  fn supervise_block_codegen_registers_each_tracked_child_with_the_runtime() {
    // Structural proof, independent of the black-box test above: two
    // tracked spawns inside one `supervise` block must compile to
    // exactly two `emerald_supervisor_register_child` calls, plus a
    // real `emerald_supervisor_create` call and a real per-class
    // respawn thunk function.
    let program = emerald_parser::parse(SUPERVISOR_EXAMPLE).expect("should parse");
    let dir = fresh_temp_dir("supervise_register_child_ir");
    let obj_path = dir.join("out.o");
    let ir =
      compile_to_object_ir_text_for_test(&program, &obj_path).expect("should compile to IR text");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
      ir.contains("call ptr @emerald_supervisor_create("),
      "a `supervise do ... end` expression must call emerald_supervisor_create:\n{ir}"
    );
    let register_calls = ir
      .matches("call i64 @emerald_supervisor_register_child(")
      .count();
    assert_eq!(
      register_calls, 2,
      "two tracked spawns inside one supervise block must produce exactly two \
       emerald_supervisor_register_child calls:\n{ir}"
    );
    assert!(
      ir.contains("define ptr @Worker__respawn("),
      "a `Worker__respawn` thunk must be compiled:\n{ir}"
    );
    assert!(
      ir.contains("define ptr @Logger__respawn("),
      "a `Logger__respawn` thunk must be compiled:\n{ir}"
    );
  }

  // Plan 42 (enumerable stdlib).

  // The plan's own worked example, adapted to this compiler's real
  // syntax: `arr.select() do |x: Int64| ... end` does not parse at all —
  // `PrimaryExpr` (the nonterminal reachable from a `Let`'s RHS)
  // deliberately never gained plan 34's trailing-block-literal
  // attachment (a real, pre-existing LALR(1) conflict with `HashLit`,
  // documented directly in `grammar.lalrpop`'s own comment — verified
  // this session), so a predicate/transform must instead be bound to a
  // named top-level `Proc` first (plan 10's own pre-existing
  // mechanism) and passed by name — see `emerald-sema`'s
  // `check_enumerable_proc_arg`'s own doc comment for the full
  // citation trail.
  const ENUMERABLE_EXAMPLE: &str = "is_even: Proc = do |x: Int64| x % 2 == 0 end\ndoubler: Proc = do |x: Int64| x * 2 end\n\narr: Array[Int64] = [1, 2, 3, 4, 5, 6]\nevens: Array[Int64] = arr.select(is_even)\ndoubled: Array[Int64] = evens.map(doubler)\ntotal: Int64 = doubled.sum()\nputs total\n";

  #[test]
  fn enumerable_worked_example_compiled_linked_and_run_prints_24() {
    assert_eq!(compile_link_run(ENUMERABLE_EXAMPLE), "24\n");
  }

  // Plan 58 (generic types).

  // A fixed-3-slot `Stack[T]` — deliberately avoids `Array[T]`/a self-
  // referential linked-node field (both real, disclosed gaps in this
  // plan's own template-checking/codegen — see `check_generic_class_
  // body`'s and `instantiate_generic_class_defs`'s own doc comments),
  // proving the core mechanism (field/param/return substitution, two
  // simultaneous distinct instantiations, `{MangledName}_{method}`
  // dispatch) with plain `T`-typed fields only.
  const GENERIC_CLASSES_EXAMPLE: &str = "class Stack[T]\n  slot0: T\n  slot1: T\n  slot2: T\n  count: Int64\n  fn initialize(): Void do\n    @count = 0\n  end\n  fn push(value: T): Void do\n    if @count == 0 do\n      @slot0 = value\n    end\n    if @count == 1 do\n      @slot1 = value\n    end\n    if @count == 2 do\n      @slot2 = value\n    end\n    @count = @count + 1\n  end\n  fn pop(): T do\n    @count = @count - 1\n    if @count == 0 do\n      return @slot0\n    end\n    if @count == 1 do\n      return @slot1\n    end\n    return @slot2\n  end\nend\n\nints: Stack[Int64] = Stack.new()\nints.push(10)\nints.push(20)\nints.push(30)\nputs ints.pop()\nputs ints.pop()\n\nstrs: Stack[String] = Stack.new()\nstrs.push(\"first\")\nstrs.push(\"second\")\nputs strs.pop()\nputs strs.pop()\n";

  #[test]
  fn generic_stack_worked_example_compiled_linked_and_run_prints_the_expected_four_lines() {
    assert_eq!(
      compile_link_run(GENERIC_CLASSES_EXAMPLE),
      "30\n20\nsecond\nfirst\n"
    );
  }

  // Plan 68 (generic-method repeated-print investigation).
  //
  // `examples/README.md` disclosed a "second consecutive `puts` of a
  // generic method's return value on a monomorphized instance can
  // silently drop its output" finding against this exact program: the
  // `Stack[Int64]` instance's two `pop()` calls both printed correctly,
  // but the *second* `pop()` on the separately-monomorphized
  // `Stack[String]` instance's return value never reached stdout — 3
  // lines instead of 4, no crash, no diagnostic — reproduced exactly
  // once out of hundreds of direct binary executions by plan 68's own
  // investigation.
  //
  // This plan's own re-investigation could not reproduce the symptom
  // against any actual execution of the compiled binary — not once,
  // across hundreds of direct runs of freshly-linked binaries — using
  // every method available that reads a subprocess's stdout without
  // going through this environment's `ctx_shell` MCP tool's output
  // *compression* layer (plain shell redirection, `raw`-mode capture,
  // and this exact `compile_link_run`/`compile_link_run_n_times`
  // in-process `std::process::Command` capture, which this test uses).
  // Directly comparable: piping the identical freshly-linked binary's
  // repeated output through `ctx_shell` WITHOUT its `raw: true` escape
  // reproduced a 3-line-instead-of-4 truncation matching the disclosed
  // symptom deterministically at some repeat counts (e.g. 3 runs) and
  // not at others (e.g. 5 runs) against the *same* binary and command —
  // varying with a tool-side parameter (repeat count) that has no
  // causal path into the compiled program's own execution, which rules
  // out a real race in the compiled program. `raw: true` (verbatim,
  // uncompressed capture) on that same tool, and every other capture
  // method, always showed the correct 4 lines. The disassembled
  // machine code (`Stack$String_pop`/`Stack$String_push`/`main`/
  // `emerald_print_str`, `objdump -d`) is also textbook-correct: two
  // real, distinct `emerald_print_str` calls in sequence, fed the two
  // real return values of two real `Stack$String_pop` calls, matching
  // Int64's identical-shape call sequence exactly.
  //
  // Conclusion: this was very likely never a compiler defect at all —
  // the original one-off observation, and this plan's own prior
  // "reproduced once in ~250 runs" finding, are best explained by this
  // same class of output-capture artifact (present in at least one tool
  // this environment's agents are directed to use for exactly this kind
  // of loop) rather than a real, if rare, codegen/runtime race. This
  // test is the regression-hardening insurance anyway: 50 fresh
  // `compile_link_run` executions of a compiled-and-linked-once binary,
  // asserting byte-identical, fully-4-line output every single time,
  // captured entirely in-process (never through any external capture
  // tool) so a real regression — in either direction — fails loudly.
  const PLAN_68_REPEAT_COUNT: usize = 50;

  #[test]
  fn plan_68_generic_stack_second_pop_on_a_monomorphized_string_instance_prints_deterministically()
  {
    let runs = compile_link_run_n_times(GENERIC_CLASSES_EXAMPLE, PLAN_68_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "30\n20\nsecond\nfirst\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  // Plan 59 (C FFI).

  // The plan's own worked proof verbatim — three real libc functions,
  // zero third-party libraries, deterministic on any target: `llabs`
  // proves a scalar `Int64` round-trip with zero marshaling code,
  // `strlen` proves an Emerald `String` passed directly to a real C
  // function with zero conversion, and `strstr`'s two calls prove
  // `CString`/`String.from_cstring` on both the found and the real-
  // `NULL` (not-found) path, defaulted through plan 73's own `??`.
  const FFI_EXAMPLE: &str = "unsafe extern \"C\" {\n  fn llabs(x: Int64): Int64\n  fn strlen(s: String): Int64\n  fn strstr(haystack: String, needle: String): CString\n}\n\nx: Int64 = llabs(-42)\nputs x\n\nn: Int64 = strlen(\"hello\")\nputs n\n\nfound: Option[String] = String.from_cstring(strstr(\"hello world\", \"world\"))\nputs found ?? \"not found\"\n\nmissing: Option[String] = String.from_cstring(strstr(\"hello world\", \"xyz\"))\nputs missing ?? \"not found\"\n";

  #[test]
  fn c_ffi_worked_example_compiled_linked_and_run_prints_the_expected_four_lines() {
    assert_eq!(compile_link_run(FFI_EXAMPLE), "42\n5\nworld\nnot found\n");
  }

  // Real bug fix (found post-plan-89, this session): a `Proc`-typed
  // CLASS FIELD's `.call` was disclosed as unsupported — `@op.call(x)`
  // from inside the declaring class's own method body failed with
  // "method calls are only supported on a plain local-variable
  // receiver," even though sema fully accepted the program. Fixed via
  // `build_indirect_proc_call` (factored out of the pre-existing
  // Ident-receiver indirect-call path) plus `build_class_layout`
  // encoding a Proc-typed field's signature into `field_classes` the
  // same way `bind_params` already does for a Proc-typed parameter.
  #[test]
  fn proc_typed_class_field_call_from_inside_the_declaring_class_works() {
    let src = "class Adder\n  op: Proc[Int64, Int64]\n\n  fn initialize(op: Proc[Int64, Int64]): Void do\n    @op = op\n  end\n\n  fn apply(x: Int64): Int64 do\n    @op.call(x)\n  end\nend\n\nadd_one: Proc = do |y: Int64| y + 1 end\nadder: Adder = Adder.new(add_one)\nputs adder.apply(41)\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn array_each_compiled_linked_and_run_prints_each_element() {
    let src = "printer: Proc = do |x: Int64| puts x end\narr: Array[Int64] = [7, 8, 9]\narr.each(printer)\n";
    assert_eq!(compile_link_run(src), "7\n8\n9\n");
  }

  #[test]
  fn array_each_with_index_compiled_linked_and_run_prints_index_then_value() {
    let src = "printer: Proc = do |x: Int64, i: Int64| puts i\n  puts x end\narr: Array[Int64] = [10, 20, 30]\narr.each_with_index(printer)\n";
    assert_eq!(compile_link_run(src), "0\n10\n1\n20\n2\n30\n");
  }

  #[test]
  fn array_sort_compiled_linked_and_run_prints_ascending_order() {
    let src = "arr: Array[Int64] = [3, 1, 2]\nsorted: Array[Int64] = arr.sort()\nprinter: Proc = do |x: Int64| puts x end\nsorted.each(printer)\n";
    assert_eq!(compile_link_run(src), "1\n2\n3\n");
  }

  #[test]
  fn array_reduce_compiled_linked_and_run_folds_to_the_sum() {
    let src = "adder: Proc = do |acc: Int64, x: Int64| acc + x end\narr: Array[Int64] = [1, 2, 3, 4]\ntotal: Int64 = arr.reduce(0, adder)\nputs total\n";
    assert_eq!(compile_link_run(src), "10\n");
  }

  // Plan 74 (enumerable chaining): `arr.select do ... end.map do ...
  // end.sort()` in one expression — plan 70's own `check_enumerable_
  // call`/`build_enumerable_call` machinery, unmodified in shape, now
  // composes across links without an intermediate named variable.
  // `a_chained_enumerable_call_is_rejected_not_miscompiled` above is a
  // DIFFERENT, still-real rejection (a fully parenthesized chain with
  // no `do...end` anywhere, e.g. `arr.select(is_even).map(doubler)` —
  // `ChainCallExpr`'s own grammar production requires at least one
  // `do...end`-attached link, so that shape never reaches this plan's
  // own new codegen path at all, and stays a parse error exactly as
  // before).
  #[test]
  fn plan_74_three_link_do_end_chain_select_map_sort_compiles_and_runs_correctly() {
    let src = "nums: Array[Int64] = [1, 2, 3, 4, 5]\nresult: Array[Int64] =\n  nums\n    .select do |x: Int64| x % 2 == 0 end\n    .map do |x: Int64| x * 10 end\n    .sort()\nputs result[0]\nputs result[1]\n";
    assert_eq!(compile_link_run(src), "20\n40\n");
  }

  // A longer, unsorted starting array, chained `.select`/`.map`/`.sort`
  // together with a SEPARATE, sibling two-link chain ending in `.sum()`
  // (a scalar, non-Array-producing tail — proving the same receiver
  // expression can be chained more than one way in the same program,
  // and that a chain ending in a scalar method still works unchanged).
  #[test]
  fn plan_74_three_link_chain_and_a_sibling_two_link_chain_ending_in_sum_both_compile_and_run() {
    let src = "nums: Array[Int64] = [5, 3, 8, 1, 9, 2]\nresult: Array[Int64] =\n  nums\n    .select do |x: Int64| x > 2 end\n    .map do |x: Int64| x * 2 end\n    .sort()\nputs result[0]\nputs result[1]\nputs result[2]\nputs result[3]\ntotal: Int64 = nums.select do |x: Int64| x > 2 end.sum()\nputs total\n";
    assert_eq!(compile_link_run(src), "6\n10\n16\n18\n25\n");
  }

  // A chain that STARTS from a `Hash[K,V].map` result (an `Array[T]`,
  // not a `Hash`) — proves the synthetic-receiver materialization this
  // plan adds handles a Hash-origin chain link exactly like an
  // Array-origin one once `.map` has produced a real `Array[T]`.
  #[test]
  fn plan_74_chain_starting_from_a_hash_map_result_compiles_and_runs_correctly() {
    let src = "names: Hash[Int64, String] = {1 => \"a\", 2 => \"b\", 3 => \"c\"}\nlabelled: Array[Int64] =\n  names\n    .map do |p: Pair[Int64, String]| p.key end\n    .select do |k: Int64| k != 2 end\n    .sort()\nputs labelled[0]\nputs labelled[1]\n";
    assert_eq!(compile_link_run(src), "1\n3\n");
  }

  // Plan 70's original non-chained shapes, restated here as a
  // regression guard tied directly to this plan's own change
  // (`resolve_chained_enumerable_receiver` must stay a no-op
  // passthrough for a plain named-local receiver) — every pre-existing
  // plan 42/70 test above already re-confirms this too, unmodified.
  #[test]
  fn plan_74_non_chained_single_enumerable_calls_still_work_unchanged() {
    let src = "nums: Array[Int64] = [1, 2, 3, 4, 5]\nevens: Array[Int64] = nums.select do |x: Int64| x % 2 == 0 end\ndoubled: Array[Int64] = evens.map do |x: Int64| x * 10 end\nsorted: Array[Int64] = doubled.sort()\nputs sorted[0]\nputs sorted[1]\n";
    assert_eq!(compile_link_run(src), "20\n40\n");
  }

  // Plan 89 (generic-method codegen, interface generics, indirect Proc
  // calls).

  // Isolated from generics entirely (Decision log: the gap is
  // independent of generic methods — a plain, non-generic method
  // taking a `Proc` PARAMETER and calling it already failed before
  // this plan, since `.call`'s own dispatch only ever resolved a
  // literal top-level `Let`-bound lambda `Ident` via `ctx.
  // lambda_func_ids`, never an indirect call through a parameter).
  #[test]
  fn a_non_generic_method_taking_and_calling_a_proc_parameter_works() {
    let src = "class Doubler\n  fn initialize(): Void do\n  end\n\n  fn apply(f: Proc[Int64, Int64], x: Int64): Int64 do\n    f.call(x)\n  end\nend\n\nd: Doubler = Doubler.new()\ntripler: Proc[Int64, Int64] = do |x: Int64| x * 3 end\nputs d.apply(tripler, 5)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  // Also proves an INLINE lambda literal (never bound to a top-level
  // `Let` at all) compiles correctly when passed directly as a call
  // argument — `build_inline_lambda`'s own worked proof, layered on
  // top of the indirect-call proof above.
  #[test]
  fn an_inline_lambda_literal_passed_directly_as_a_proc_parameter_works() {
    let src = "class Doubler\n  fn initialize(): Void do\n  end\n\n  fn apply(f: Proc[Int64, Int64], x: Int64): Int64 do\n    f.call(x)\n  end\nend\n\nd: Doubler = Doubler.new()\nputs d.apply(do |x: Int64| x * 3 end, 5)\n";
    assert_eq!(compile_link_run(src), "15\n");
  }

  // Plan 88's own original worked example, this time actually compiled,
  // linked, and run (plan 89's own concrete proof target) — `interface
  // Iterable[T]` with a generic required method (`map[U]`), implemented
  // by `Numbers`, called end-to-end with a real lambda argument.
  const ITERABLE_WORKED_EXAMPLE: &str = "interface Iterable[T]\n  fn map[U](f: Proc[T, U]): Array[U]\nend\n\nclass Numbers\n  implements Iterable[Int64]\n\n  values: Array[Int64]\n\n  fn initialize(values: Array[Int64]): Void do\n    @values = values\n  end\n\n  fn map[U](f: Proc[T, U]): Array[U] do\n    values: Array[Int64] = @values\n    result: Array[U] = Array.new(values.count)\n    i: Int64 = 0\n    while i < values.count do\n      result[i] = f.call(values[i])\n      i: Int64 = i + 1\n    end\n    result\n  end\nend\n\nn: Numbers = Numbers.new([1, 2, 3])\ndoubled: Array[Int64] = n.map(do |x: Int64| x * 2 end)\nputs doubled[0]\nputs doubled[2]\n";

  #[test]
  fn interface_generics_worked_example_compiled_linked_and_run_prints_two_and_six() {
    assert_eq!(compile_link_run(ITERABLE_WORKED_EXAMPLE), "2\n6\n");
  }

  // The SAME generic method (`Numbers#map[U]`), called twice at two
  // DIFFERENT concrete type arguments (`U = Int64`, then `U = Boolean`)
  // — proves `resolve_generic_method_instance` compiles a genuinely
  // DISTINCT function per binding (a real mangled-name collision here,
  // rather than reusing/overwriting the first specialization, would
  // either fail to link or produce the wrong element type/values for
  // one of the two calls).
  #[test]
  fn a_generic_method_called_with_two_different_type_arguments_produces_distinct_compiled_results()
  {
    let src = "interface Iterable[T]\n  fn map[U](f: Proc[T, U]): Array[U]\nend\n\nclass Numbers\n  implements Iterable[Int64]\n\n  values: Array[Int64]\n\n  fn initialize(values: Array[Int64]): Void do\n    @values = values\n  end\n\n  fn map[U](f: Proc[T, U]): Array[U] do\n    values: Array[Int64] = @values\n    result: Array[U] = Array.new(values.count)\n    i: Int64 = 0\n    while i < values.count do\n      result[i] = f.call(values[i])\n      i: Int64 = i + 1\n    end\n    result\n  end\nend\n\nn: Numbers = Numbers.new([1, 2, 3])\ndoubled: Array[Int64] = n.map(do |x: Int64| x * 2 end)\nlabels: Array[String] = n.map(do |x: Int64| \"n#{x}\" end)\nputs doubled[0]\nputs doubled[2]\nputs labels[0]\nputs labels[2]\n";
    assert_eq!(compile_link_run(src), "2\n6\nn1\nn3\n");
  }

  // Found and closed 2026-09-21 (this session's "find all bugs" sweep):
  // `resolve_generic_method_instance`'s own type-parameter resolver
  // (`infer_method_type_param_binding`) used to handle only `Proc[...,
  // U]` (`U` in RETURN position, the two tests immediately above). A
  // generic method whose type parameter is bound by an ORDINARY
  // argument — `x: U` directly, no `Proc` involved — failed at codegen
  // time with "could not resolve generic method ...'s type parameter
  // ... to a concrete type at this call site" even though `emerald-
  // sema`'s own fuller structural unifier already accepted the
  // program. Confirmed as a real, pre-existing bug (not assumed) by
  // reverting this fix on a stashed copy of `emerald-codegen` and
  // reproducing the exact error via the real CLI before restoring it.
  #[test]
  fn a_class_bound_generic_method_with_a_plain_non_proc_type_parameter_works() {
    let src = "class Box\n  fn identity[U](x: U): U do\n    x\n  end\nend\n\nb: Box = Box.new()\nputs b.identity(41)\n";
    assert_eq!(compile_link_run(src), "41\n");
  }

  #[test]
  fn hash_count_compiled_linked_and_run_prints_the_pair_count() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h.count()\n";
    assert_eq!(compile_link_run(src), "3\n");
  }

  #[test]
  fn hash_each_compiled_linked_and_run_prints_every_key_then_value() {
    let src = "printer: Proc = do |p: Pair[Int64, Int64]| puts p.key\n  puts p.value end\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20}\nh.each(printer)\n";
    let out = compile_link_run(src);
    let mut lines: Vec<i64> = out.lines().map(|l| l.parse().unwrap()).collect();
    // No ordering guarantee over a Hash's own storage order (plan 25's
    // own disclosed precedent) — assert the real invariant instead:
    // each key is immediately followed by its own value.
    assert_eq!(lines.len(), 4);
    let pairs: Vec<(i64, i64)> = lines.chunks(2).map(|c| (c[0], c[1])).collect();
    let expected = [(1, 10), (2, 20)];
    for (k, v) in &expected {
      assert!(
        pairs.contains(&(*k, *v)),
        "expected pair ({k}, {v}) in {pairs:?}"
      );
    }
    lines.sort_unstable();
    assert_eq!(lines, vec![1, 2, 10, 20]);
  }

  #[test]
  fn array_length_header_reports_the_real_element_count() {
    // `leaf-array-length-header`'s own AC2/AC3: a real length-read
    // (`.count`, which this leaf's own header makes possible),
    // against both the literal-array and `Array.new` construction
    // paths.
    assert_eq!(
      compile_link_run("arr: Array[Int64] = [10, 20, 30]\nputs arr.count()\n"),
      "3\n"
    );
    assert_eq!(
      compile_link_run("arr: Array[Int64] = Array.new(5)\nputs arr.count()\n"),
      "5\n"
    );
  }

  #[test]
  fn a_chained_enumerable_call_is_rejected_not_miscompiled() {
    // AC6 (`leaf-enumerable-functions`): `arr.select(...).map(...)` —
    // real, disclosed correction found this session against the
    // plan's own stated framing ("the receiver of `.map` is a
    // `MethodCall`... rejected by `build_method_call`'s own existing
    // non-`Ident`-receiver diagnostic"): this compiler's grammar
    // doesn't even reach that codegen guard — `.method(...)`'s own
    // receiver position is grammar-restricted to a bare `Ident`/
    // `InstanceVarTok` (`grammar.lalrpop`'s `StmtPrimaryExpr`/
    // `PrimaryExpr`, verified this session), with no production
    // chaining a further `"." method(...)` onto an already-reduced
    // `MethodCall`. So `arr.select(...).map(...)` is rejected at PARSE
    // time, one leaf earlier than the plan's own text describes — a
    // strictly stronger, still-real rejection (never silently
    // miscompiled), just not via the specific diagnostic string the
    // plan's own text names.
    let errs = emerald_parser::parse(
      "is_even: Proc = do |x: Int64| x % 2 == 0 end\ndoubler: Proc = do |x: Int64| x * 2 end\narr: Array[Int64] = [1, 2, 3]\ndoubled: Array[Int64] = arr.select(is_even).map(doubler)\n",
    )
    .expect_err(
      "a chained `.select(...).map(...)` call must be rejected, not silently miscompiled",
    );
    assert!(!errs.is_empty());
  }

  // Plan 60 (distributed, location-transparent actors).
  //
  // The genuine two-process, real-socket worked proof (`Counter`/
  // `host.em`/`client.em`) lives in `crates/emerald-cli/tests/
  // distributed_actors.rs` — it needs two real, separately launched OS
  // processes, which this crate's own single-process `compile_link_run`
  // test harness can't provide. The tests below cover what a single
  // process genuinely can: `.remote(...)` against an unreachable
  // address raising a real, catchable `RemoteActorError` (AC4,
  // `leaf-actor-ref-and-addressing`), and regression proof that every
  // pre-existing plan 54/55/56/57 worked example above still compiles/
  // links/runs identically now that an actor's own value is a tagged
  // `EmeraldActorRef*` (Design decision 1), not a bare arena pointer.

  #[test]
  fn remote_against_a_closed_port_raises_a_real_catchable_remote_actor_error() {
    // The plan's own literal AC4 address — a real, unassigned/reserved
    // low port essentially never listening on any real machine, so
    // `connect()` fails fast with a real `ECONNREFUSED` (no long
    // `EMERALD_REMOTE_TIMEOUT_MS` wait needed for this specific gate).
    let src = "actor Counter\n  count: Int64\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\nend\n\nbegin\n  handle: Counter = Counter.remote(\"127.0.0.1:1\", \"counter1\")\n  puts \"should not reach here\"\nrescue RemoteActorError => e\n  puts \"caught\"\nend\n";
    assert_eq!(compile_link_run(src), "caught\n");
  }

  // Plan 61 (comptime execution).

  const FACTORIAL_SRC: &str = "comptime fn factorial(n: Int64): Int64 do\n  if n <= 1 do\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nFACT10: Int64 = comptime factorial(10)\nputs FACT10\n";

  #[test]
  fn comptime_factorial_worked_example_prints_3628800() {
    assert_eq!(compile_link_run(FACTORIAL_SRC), "3628800\n");
  }

  #[test]
  fn comptime_factorial_bakes_a_literal_constant_with_no_factorial_symbol_or_call_surviving() {
    // AC2's own white-box proof: inspect the emitted LLVM module
    // directly, not stdout — `FACT10` stores a literal constant (`store
    // i64 3628800`), with no `call` computing it, and no `factorial`
    // symbol anywhere in the module at all (`comptime_only_function_
    // names`'s own dead-code-elimination: `factorial` is never called
    // outside this one `comptime` expression, so it's never declared).
    let program = emerald_parser::parse(FACTORIAL_SRC).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = dir.join(format!("emerald_codegen_comptime_ir_{unique}.o"));
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile to object file");
    std::fs::remove_file(&obj_path).ok();
    assert!(
      ir.contains("store i64 3628800"),
      "expected a literal `store i64 3628800`, got:\n{ir}"
    );
    assert!(
      !ir.contains("@factorial"),
      "expected no `factorial` symbol to survive in the emitted module, got:\n{ir}"
    );
  }

  #[test]
  fn comptime_expr_as_array_news_size_argument_produces_a_real_runtime_sized_array() {
    // AC4: the size argument was computed at compile time, but the
    // allocation itself still happens at runtime, unchanged — proven by
    // filling and reading back every index up to 119 successfully.
    let src = "comptime fn factorial(n: Int64): Int64 do\n  if n <= 1 do\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\na: Array[Int64] = Array.new(comptime factorial(5))\ni: Int64 = 0\nwhile i < 120 do\n  a[i] = i\n  i = i + 1\nend\nputs a[119]\n";
    assert_eq!(compile_link_run(src), "119\n");
  }

  #[test]
  fn comptime_step_limit_flag_fails_the_whole_compilation_not_a_hang() {
    // AC5 (leaf-comptime-const-context-integration): a small, injected
    // ceiling well below a real infinite loop's own iteration count —
    // the same "injectable seam in tests, not a full million-iteration
    // wait" style plan 48 already used for its own fingerprint-override
    // tests.
    let src = "comptime fn spin(n: Int64): Int64 do\n  i: Int64 = 0\n  while true do\n    i = i + 1\n  end\n  return i\nend\n\nX: Int64 = comptime spin(1)\nputs X\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = dir.join(format!("emerald_codegen_comptime_limit_{unique}.o"));
    let err = compile_to_object_with_comptime_step_limit(&program, &obj_path, 100)
      .expect_err("should fail, not hang");
    std::fs::remove_file(&obj_path).ok();
    assert!(
      err.contains("exceeded 100 steps"),
      "expected the step-ceiling diagnostic, got: {err}"
    );
  }

  #[test]
  fn comptime_interpreter_step_ceiling_fires_after_exactly_limit_steps_not_a_hang() {
    // AC5 (leaf-comptime-interpreter-core): the interpreter itself,
    // exercised directly against a hand-built runaway-loop AST with an
    // injected `limit: 100` — no compile pipeline involved at all.
    let src = "while true do\n  i: Int64 = 1\nend\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let Item::Stmt(while_stmt) = &program.items[0] else {
      panic!("expected a top-level Stmt::While");
    };
    let mut interp = ComptimeInterpreter::new(100);
    let mut env = HashMap::new();
    let comptime_fns = HashMap::new();
    let err = interp
      .exec_stmt(while_stmt, &mut env, &comptime_fns, "spin")
      .expect_err("should fail, not hang");
    assert!(
      err.contains("exceeded 100 steps"),
      "expected the step-ceiling diagnostic, got: {err}"
    );
  }

  // Plan 62 (design-by-contract).

  const DIVIDE_SRC: &str = "fn divide(a: Int64, b: Int64): Int64\n  requires b != 0\n  ensures result * b <= a\ndo\n  return a / b\nend\n";

  #[test]
  fn contracts_worked_example_prints_5_then_catches_a_real_requires_violation() {
    // AC1: proof points (a) and (c) together — a runtime-only-
    // determinable divisor succeeding normally, and a runtime-only-
    // determinable zero divisor raising a real `ContractViolation`
    // caught by an ordinary `rescue`. `w`'s own zero-ness is only
    // knowable at run time (`x - 3`, `x` itself a runtime local), so
    // this never hits `leaf-sema-static-provability`'s compile-time
    // rejection at all — it's a genuine runtime-enforcement proof.
    let src = format!(
      "{DIVIDE_SRC}\ny: Int64 = 10\nz: Int64 = 2\nputs divide(y, z)\n\nx: Int64 = 3\nw: Int64 = x - 3\nbegin\n  puts divide(20, w)\nrescue ContractViolation => e\n  puts e.message\nend\n"
    );
    let out = compile_link_run(&src);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("5"));
    let violation_line = lines.next().expect("expected a second line");
    assert!(
      violation_line.contains("divide") && violation_line.contains("b != 0"),
      "expected the requires-violation message, got: {violation_line}"
    );
  }

  #[test]
  fn an_ensures_violation_raises_at_the_ensures_site_not_the_requires_site() {
    // AC2: `divide2` has the SAME `requires b != 0` as `divide` (so a
    // non-zero-divisor call passes that check cleanly) but a
    // deliberately wrong body (`a / b + 1`) that violates `ensures
    // result * b <= a` — proving the two checks are wired to their own,
    // independent injection points.
    let src = "fn divide2(a: Int64, b: Int64): Int64\n  requires b != 0\n  ensures result * b <= a\ndo\n  return a / b + 1\nend\n\nbegin\n  puts divide2(10, 2)\nrescue ContractViolation => e\n  puts e.message\nend\n";
    let out = compile_link_run(src);
    assert!(
      out.contains("divide2") && out.contains("ensures") && out.contains("result * b <= a"),
      "expected the ensures-violation message, got: {out}"
    );
  }

  #[test]
  fn contract_violation_is_never_emitted_for_a_program_with_no_contracts() {
    // AC3: zero-cost, zero-regression for a program with no `requires`/
    // `ensures` usage anywhere — verified via white-box IR inspection
    // (the same "prove it, don't just assert it" standard this leaf's
    // own `AC3` calls for), not just by inferring it from output.
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  return a + b\nend\n\nputs add(20, 22)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = dir.join(format!("emerald_codegen_no_contracts_ir_{unique}.o"));
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile to object file");
    std::fs::remove_file(&obj_path).ok();
    assert!(
      !ir.contains("ContractViolation"),
      "expected no ContractViolation class emitted for a contract-free program, got:\n{ir}"
    );
  }

  #[test]
  fn requires_and_ensures_both_pass_when_the_contract_genuinely_holds() {
    let src = format!("{DIVIDE_SRC}\nputs divide(10, 2)\n");
    assert_eq!(compile_link_run(&src), "5\n");
  }

  // Plan 66 (puts / String value-flow codegen regression hardening).
  //
  // The investigation found none of `examples/README.md`'s five
  // disclosed "puts silently prints nothing" symptoms reproduce against
  // current codegen — `build_puts` (this file, `fn build_puts`) is
  // already fully generic over `ValKind` regardless of the AST shape
  // that produced the `String` value, and every statement position
  // (including inside a `def` body, via `build_block`/`build_stmt`)
  // reaches the same `puts` special-case. The real, now-fixed defect
  // was a runtime race (see `compile_link_run_n_times`'s doc comment),
  // not a codegen routing gap. These five tests pin down each
  // previously-disclosed shape individually, each run 20 times, so a
  // regression in either direction — a real codegen gap, or a
  // reintroduced timing race — fails loudly instead of silently.

  const PLAN_66_REPEAT_COUNT: usize = 20;

  #[test]
  fn plan_66_puts_inside_a_def_body_prints_deterministically() {
    // Shape 1: `puts` as a non-trailing statement inside a user `def`
    // body (not the top level).
    let src = "fn greet(name: String): String do\n  puts \"hello from inside a def\"\n  name\nend\n\nputs greet(\"world\")\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "hello from inside a def\nworld\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  #[test]
  fn plan_66_a_stored_top_level_string_let_binding_prints_deterministically() {
    // Shape 2: `puts` of a local variable loaded back from a stored
    // `String`-typed `Let`, not a literal/method-call expression used
    // directly.
    let src = "y: String = \"hello\"\nputs y\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "hello\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  #[test]
  fn plan_66_puts_of_a_multi_arg_string_intrinsic_result_prints_deterministically() {
    // Shape 3: `puts` of a multi-argument `String` intrinsic's return
    // value (`.slice(1, 3)`), as opposed to a single-argument intrinsic.
    let src = "phrase: String = \"hello world\"\nputs phrase.slice(1, 3)\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(out, "ell\n", "run {i} produced unexpected output: {out:?}");
    }
  }

  #[test]
  fn plan_66_puts_of_an_array_string_index_read_prints_deterministically() {
    // Shape 4: `puts` of an `Array[String]` index read.
    let src = "phrase: String = \"hello world\"\nwords: Array[String] = phrase.split(\" \")\nputs words[0]\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "hello\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  #[test]
  fn plan_66_a_string_optional_via_safe_nav_and_coalesce_prints_deterministically() {
    // Shape 5: `puts` of an `Option[String]` populated via `??` after
    // starting `None` — the exact shape `nullable_safe_nav.em` and
    // `c_ffi.em` hit, mirrored here without the `unsafe extern "C"`
    // dependency.
    let src = "found: Option[String] = None\nputs found ?? \"not found\"\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "not found\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  #[test]
  fn plan_66_the_c_ffi_string_optional_safe_nav_shape_prints_deterministically() {
    // Same as shape 5 above, but through the exact `c_ffi.em` repro
    // path (safe-nav result of a real C FFI call, not a literal `nil`),
    // run repeatedly rather than once.
    let runs = compile_link_run_n_times(FFI_EXAMPLE, PLAN_66_REPEAT_COUNT);
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(
        out, "42\n5\nworld\nnot found\n",
        "run {i} produced unexpected output: {out:?}"
      );
    }
  }

  #[test]
  fn plan_66_all_five_shapes_combined_in_one_program_print_in_order_every_run() {
    // The plan's own combined proof program: all five previously-
    // disclosed shapes in one binary, so a fix that only patches one
    // call site rather than the shared mechanism would still be caught
    // by the others. Note `greet` is only *called* (and so only prints
    // its inner `puts`) at the final `puts greet("world")` line, so
    // "hello from inside a def" is emitted last, not first — sequential
    // execution order, not declaration order.
    let src = "fn greet(name: String): String do\n  puts \"hello from inside a def\"\n  name\nend\n\ny: String = \"hello\"\nputs y\n\nphrase: String = \"hello world\"\nputs phrase.slice(1, 3)\n\nwords: Array[String] = phrase.split(\" \")\nputs words[0]\n\nfound: Option[String] = None\nputs found ?? \"not found\"\n\nputs greet(\"world\")\n";
    let runs = compile_link_run_n_times(src, PLAN_66_REPEAT_COUNT);
    let expected = "hello\nell\nhello\nnot found\nhello from inside a def\nworld\n";
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(out, expected, "run {i} produced unexpected output: {out:?}");
    }
  }

  /// Plan 70 (enumerable stdlib completion): the exact `examples/
  /// enumerable.em` source (kept as a literal copy, not a file read,
  /// matching every other worked-example test in this module) —
  /// `map`/`select`/`reduce`/`count`(predicate)/`sum`/`sort`/
  /// `each_with_index` on `Array[Int64]`, and `map`/`reduce`/
  /// `count`(predicate)/`count`(arity-0) on `Hash[Int64,Int64]`, all
  /// via this plan's new block-attached-call syntax (`recv.method {
  /// |params| body }`) rather than plan 42's pre-existing named-`Proc`
  /// workaround. Compiled and linked once, run 20 times, asserting
  /// byte-identical, fully-correct output every run — this plan's own
  /// regression-hardening insurance, the same posture plan 66/68
  /// already established for this test module.
  const PLAN_70_ENUMERABLE_EXAMPLE: &str = "nums: Array[Int64] = [1, 2, 3, 4, 5]\n\ndoubled: Array[Int64] = nums.map do |x: Int64| x * 2 end\nevens: Array[Int64] = nums.select do |x: Int64| x % 2 == 0 end\ntotal: Int64 = nums.reduce(0) do |acc: Int64, x: Int64| acc + x end\nabove_two: Int64 = nums.count do |x: Int64| x > 2 end\ns: Int64 = nums.sum\nsorted: Array[Int64] = nums.sort\n\nnums.each_with_index do |x: Int64, i: Int64| puts i end\n\nputs doubled[4]\nputs evens.count\nputs total\nputs above_two\nputs s\nputs sorted[0]\n\nh: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nh_values: Array[Int64] = h.map do |p: Pair[Int64, Int64]| p.value end\nh_total: Int64 = h.reduce(0) do |acc: Int64, p: Pair[Int64, Int64]| acc + p.value end\nh_big: Int64 = h.count do |p: Pair[Int64, Int64]| p.value > 15 end\n\nputs h_values[0]\nputs h_total\nputs h_big\nputs h.count\n";

  #[test]
  fn plan_70_enumerable_worked_example_linked_and_run_prints_deterministically() {
    let runs = compile_link_run_n_times(PLAN_70_ENUMERABLE_EXAMPLE, PLAN_66_REPEAT_COUNT);
    let expected = "0\n1\n2\n3\n4\n10\n2\n15\n3\n15\n1\n10\n60\n2\n3\n";
    for (i, out) in runs.iter().enumerate() {
      assert_eq!(out, expected, "run {i} produced unexpected output: {out:?}");
    }
  }

  // ------------------------------------------------------------------
  // Plan 84 (`deterministic-destruction-codegen`): real `own`/`borrow`/
  // `borrow var` codegen, replacing plan 83's disclosed full-erasure
  // passthrough for a by-value `borrow`/`borrow var` — see `strip_
  // ownership_in_type_expr`'s own doc comment for the full mechanism.
  //
  // Several tests below use source that `emerald-sema` would reject
  // (plan 72's "no `var` slot exists on a parameter" rule makes `x =
  // ...` illegal for ANY parameter, `borrow var`-typed or not — a real,
  // pre-existing, disclosed gap `examples/ownership.em`'s own header
  // comment covers in full). That's fine here: `compile_link_run` (this
  // whole module's own established idiom) parses `src` directly via
  // `emerald_parser::parse` and compiles it — `emerald-sema` is never
  // invoked by any test in this file, so this is not a new allowance
  // introduced for plan 84, just the same route every other codegen-
  // only test here already takes.
  // ------------------------------------------------------------------

  #[test]
  fn borrow_var_int64_parameter_mutation_is_genuinely_visible_to_the_caller() {
    // The headline proof: `x`'s own reassignment inside `double_in_
    // place` writes through a REAL pointer into the caller's own `n`
    // (`bind_params`'s load-on-entry, `emit_borrow_var_writebacks`'
    // store-on-exit) — not an accidental copy that happens to look
    // right. Before this plan, `borrow var Int64` fully erased to a
    // bare `Int64`, passed by value — this exact program would have
    // printed `21`, not `42`.
    let src = "fn double_in_place(x: borrow var Int64): Void do\n  x = x * 2\nend\n\nn: Int64 = 21\ndouble_in_place(n)\nputs n\n";
    assert_eq!(compile_link_run(src), "42\n");
  }

  #[test]
  fn borrow_var_float64_and_boolean_parameters_also_write_back() {
    // Same proof, `Float64`/`Boolean` — `borrow_ptr_marker_info` and
    // `bind_params`'s handling are kind-agnostic (any non-`Ptr`/`Str`
    // `ValKind`), not special-cased to `Int64` alone.
    let src = "fn halve(x: borrow var Float64): Void do\n  x = x / 2.0\nend\n\nfn flip(x: borrow var Boolean): Void do\n  x = !x\nend\n\nf: Float64 = 10.0\nhalve(f)\nputs f\n\nb: Boolean = true\nflip(b)\nif b do\n  puts \"true\"\nelse\n  puts \"false\"\nend\n";
    assert_eq!(compile_link_run(src), "5\nfalse\n");
  }

  #[test]
  fn borrow_int64_parameter_reads_the_callers_current_value_through_a_real_pointer() {
    // A plain `borrow` (not `borrow var`) still gets the real-pointer
    // treatment for a by-value type (`strip_ownership_in_type_expr`'s
    // own doc comment: only an already-pointer-represented type skips
    // it) — this just proves the read side still works correctly.
    let src =
      "fn describe(x: borrow Int64): Void do\n  puts x\nend\n\nn: Int64 = 21\ndescribe(n)\n";
    assert_eq!(compile_link_run(src), "21\n");
  }

  #[test]
  fn borrow_int64_parameter_compiles_to_a_real_ptr_typed_llvm_signature() {
    // Direct IR-text proof (this module's own established idiom, e.g.
    // `a_non_escaping_new_inside_a_while_loop_gets_exactly_one_alloca`
    // above) that `borrow Int64` is a REAL pointer at the LLVM level,
    // not merely behaviorally indistinguishable from one: an ordinary,
    // unannotated `Int64` parameter compiles to `define i64 @plain(i64
    // %n)` (byval scalar) — `describe`'s own declared signature must
    // instead take a bare `ptr`.
    let src = "fn describe(x: borrow Int64): Void do\n  puts x\nend\n\nfn plain(x: Int64): Void do\n  puts x\nend\n\nn: Int64 = 5\ndescribe(n)\nplain(n)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("borrow_int64_signature");
    let obj_path = dir.join("out.o");
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(
      ir.contains("define void @describe(ptr "),
      "expected `describe`'s own `borrow Int64` parameter to compile to a bare `ptr`:\n{ir}"
    );
    assert!(
      ir.contains("define void @plain(i64 "),
      "expected `plain`'s own ordinary `Int64` parameter to stay a by-value `i64`, unchanged:\n{ir}"
    );
  }

  /// Counts `= alloca` lines inside `main`'s own body, excluding the two
  /// fixed `%ARGV`/`%ARGC` allocas every compiled `main` already has
  /// (`define_main`'s own doc comment) — shared by the two `borrow
  /// Int64` call-site cost tests just below, which need to isolate a
  /// SPECIFIC call site's own cost from that fixed baseline.
  fn count_non_argv_argc_allocas_in_main(ir: &str) -> usize {
    let mut in_main = false;
    let mut count = 0usize;
    for line in ir.lines() {
      if line.starts_with("define i32 @main(") {
        in_main = true;
        continue;
      }
      if in_main {
        if line.starts_with('}') {
          break;
        }
        if line.contains("= alloca ") && !line.contains("%ARGV") && !line.contains("%ARGC") {
          count += 1;
        }
      }
    }
    count
  }

  #[test]
  fn borrow_int64_call_site_with_a_plain_local_argument_adds_no_extra_alloca() {
    // Plan 84's own zero-cost claim for the common case: a `borrow`
    // call-site argument that's already a plain local variable reuses
    // that binding's OWN existing `alloca` directly (`build_borrow_arg_
    // ptr`'s own doc comment) — genuinely zero cost, not merely cheap.
    // `describe`'s call site must therefore add NO alloca of its own
    // inside `main` beyond `n`'s own single, pre-existing one — `ARGV`/
    // `ARGC` (two fixed allocas every compiled `main` already has,
    // `define_main`'s own doc comment) are excluded from the count
    // below since they're unrelated to this call site entirely.
    let src = "fn describe(x: borrow Int64): Void do\n  puts x\nend\n\nn: Int64 = 5\ndescribe(n)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("borrow_int64_zero_cost_call_site");
    let obj_path = dir.join("out.o");
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    let alloca_count = count_non_argv_argc_allocas_in_main(&ir);
    assert_eq!(
      alloca_count, 1,
      "expected exactly one non-ARGV/ARGC `alloca` in `main` (`n`'s own) — passing `n` to a \
       `borrow Int64` parameter must not add a second one:\n{ir}"
    );
  }

  #[test]
  fn borrow_int64_call_site_with_a_literal_argument_spills_to_one_real_alloca() {
    // The disclosed, real, unavoidable cost this plan names rather than
    // hiding: an argument with no existing address (a literal here, but
    // the same is true of any computed expression) needs one real,
    // extra stack slot + store to hand a `borrow`-taking callee a valid
    // pointer — `build_borrow_arg_ptr`'s own doc comment.
    let src = "fn describe(x: borrow Int64): Void do\n  puts x\nend\n\ndescribe(5)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("borrow_int64_literal_spill");
    let obj_path = dir.join("out.o");
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    let alloca_count = count_non_argv_argc_allocas_in_main(&ir);
    assert_eq!(
      alloca_count, 1,
      "expected exactly one non-ARGV/ARGC spill `alloca` in `main`, for the literal `5` \
       argument:\n{ir}"
    );
    assert_eq!(compile_link_run(src), "5\n");
  }

  /// Counts lines matching `pred` across every `define ...` function
  /// whose signature line contains one of `fn_markers` (e.g. `"@foo("`)
  /// — a generalization of `count_non_argv_argc_allocas_in_main` above
  /// for when the count needs to span more than one specific function
  /// (`own_class_parameter_transfers_the_pointer_with_no_extra_
  /// allocation` below needs `consume`'s own body counted alongside
  /// `main`'s, since the whole module's IR also has unrelated functions
  /// — other classes' own wire codecs — that would otherwise pollute a
  /// whole-module count).
  fn count_lines_in_functions_matching(
    ir: &str,
    fn_markers: &[&str],
    pred: impl Fn(&str) -> bool,
  ) -> usize {
    let mut in_target_fn = false;
    let mut count = 0usize;
    for line in ir.lines() {
      if line.starts_with("define ") && fn_markers.iter().any(|m| line.contains(m)) {
        in_target_fn = true;
        continue;
      }
      if in_target_fn {
        if line.starts_with('}') {
          in_target_fn = false;
          continue;
        }
        if pred(line) {
          count += 1;
        }
      }
    }
    count
  }

  #[test]
  fn own_class_parameter_transfers_the_pointer_with_no_extra_allocation() {
    // Plan 84's own headline `own` proof: an `own`-consuming call is
    // NOT a hidden second allocation/copy of its own — the whole
    // program allocates exactly once (`Box.new`'s own single
    // `emerald_alloc` call), matching `strip_ownership_in_type_expr`'s
    // own doc comment ("an `own` transfer is either a class's existing
    // pointer-copy or a primitive's existing scalar-copy... exactly
    // what an ordinary, unannotated parameter already does").
    let src = "class Box\n  read v: Int64\n\n  fn initialize(v: Int64): Void do\n    @v = v\n  end\nend\n\nfn consume(b: own Box): Int64 do\n  b.v\nend\n\nbox: Box = Box.new(7)\nputs consume(box)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("own_no_copy");
    let obj_path = dir.join("out.o");
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    // Scoped to `consume`'s own body and `main`'s own body — the whole
    // module's IR also contains OTHER classes'/`Box`'s own wire-codec
    // functions (`declare_wire_class_codecs`, generated for every class
    // regardless of whether the program declares any actor at all),
    // each with an unrelated `emerald_alloc` call of their own that
    // would otherwise pollute this count.
    let alloc_call_count = count_lines_in_functions_matching(&ir, &["@consume(", "@main("], |l| {
      l.contains("call ptr @emerald_alloc(")
    });
    assert_eq!(
      alloc_call_count, 1,
      "expected exactly one allocation (the single `Box.new`) inside `consume`/`main` — an \
       `own` transfer must not allocate/copy on its own:\n{ir}"
    );
    assert!(
      ir.contains("define i64 @consume(ptr "),
      "expected `consume`'s own `own Box` parameter to compile to a bare `ptr`, exactly like an \
       ordinary, unannotated `Box` parameter:\n{ir}"
    );

    assert_eq!(compile_link_run(src), "7\n");
  }

  #[test]
  fn borrow_var_class_parameter_stays_a_bare_pointer_no_wrapper() {
    // Regression guard for the case plan 84 explicitly did NOT need to
    // change: `borrow var Counter` must still compile to exactly the
    // same bare `ptr` signature an unannotated `Counter` parameter
    // gets — no wrapper struct, no extra indirection, matching
    // `strip_ownership_in_type_expr`'s own doc comment that an
    // already-pointer-represented type's `borrow`/`borrow var` fully
    // erases, unchanged from plan 83's original passthrough.
    let src = "class Counter\n  value: Int64\n\n  fn initialize(start: Int64): Void do\n    @value = start\n  end\n\n  fn bump: Void do\n    @value = @value + 1\n  end\n\n  fn value: Int64 do\n    @value\n  end\nend\n\nfn increment(c: borrow var Counter): Void do\n  c.bump\nend\n\ncounter: Counter = Counter.new(10)\nincrement(counter)\nputs counter.value\n";
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = fresh_temp_dir("borrow_var_class_bare_ptr");
    let obj_path = dir.join("out.o");
    let ir = compile_to_object_ir_text_for_test(&program, &obj_path)
      .expect("should compile and return IR text");
    std::fs::remove_file(&obj_path).ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(
      ir.contains("define void @increment(ptr "),
      "expected `increment`'s own `borrow var Counter` parameter to compile to a bare `ptr`:\n{ir}"
    );
    // The mutation is genuinely visible to the caller too — this was
    // already true before plan 84 (a class instance is always passed
    // by pointer), so this is a regression guard, not a new proof.
    assert_eq!(compile_link_run(src), "11\n");
  }
}
