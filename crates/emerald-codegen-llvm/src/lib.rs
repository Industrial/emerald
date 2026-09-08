//! Emerald's second codegen backend, built on LLVM via `inkwell` (plan 16
//! — the Cranelift-vs-LLVM bake-off `spec/COMPILER.md`'s plan-02 decision
//! record flagged as its own revisit trigger).
//!
//! Deliberately scoped to the AST subset `benchmarks/sum/sum.em` and
//! `benchmarks/array_traversal/array_traversal.em` use: top-level
//! statements only (`Let`/`While`/`If`/`Expr`), `Int64`/`Float64` scalars,
//! `Array[Int64]`/`Array[Float64]`. NOT full language parity with
//! `emerald-codegen` (Cranelift) — see plan 16's Decision log for why
//! that's the right amount of scope for a bake-off, not a shortcut.
//! Anything outside this subset returns `Err`, never panics.

use emerald_parser::{CompareOp, Expr, Item, Program, Stmt};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::targets::{
  CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, ValueKind};
use inkwell::{AddressSpace, IntPredicate, OptimizationLevel};
use std::collections::HashMap;
use std::path::Path;

/// The two scalar types this backend's scoped AST subset supports —
/// mirrors `emerald-codegen`'s own `Int64`/`Float64`-only simplification
/// (see its `cranelift_type`), just spelled as an LLVM-side enum instead
/// of going straight to a Cranelift `types::Type`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
  Int64,
  Float64,
}

fn parse_scalar_ty(name: &str) -> Result<Ty, String> {
  match name {
    "Int64" => Ok(Ty::Int64),
    "Float64" => Ok(Ty::Float64),
    other => Err(format!(
      "emerald-codegen-llvm: unsupported scalar type `{other}` (this backend only supports Int64/Float64 — see plan 16's Decision log)"
    )),
  }
}

/// `Array[Int64]` / `Array[Float64]` -> its element `Ty`. Any other shape
/// (including nested/other element types) is out of this backend's scope.
fn parse_array_elem_ty(name: &str) -> Result<Ty, String> {
  match name {
    "Array[Int64]" => Ok(Ty::Int64),
    "Array[Float64]" => Ok(Ty::Float64),
    other => Err(format!(
      "emerald-codegen-llvm: unsupported array type `{other}` (only Array[Int64]/Array[Float64] — see plan 16's Decision log)"
    )),
  }
}

/// Per-function codegen state — the LLVM-side counterpart to
/// `emerald-codegen`'s `Ctx` plus its `vars`/`local_array_elem_types`
/// locals, bundled here since this backend only ever compiles one
/// function (`main`).
struct FnCtx<'ctx> {
  scalar_vars: HashMap<String, (PointerValue<'ctx>, Ty)>,
  array_vars: HashMap<String, (PointerValue<'ctx>, Ty)>,
  print_i64: FunctionValue<'ctx>,
  print_f64: FunctionValue<'ctx>,
  alloc: FunctionValue<'ctx>,
}

/// Recursively collects every top-level `Let`'s `(name, Ty)` — scalars
/// only, arrays are handled separately since they're never reassigned in
/// this backend's scope (no `SetIndex`, no re-`Let`ting an array name) —
/// so every scalar variable gets exactly one `alloca`, created up front in
/// `main`'s entry block. This is what makes LLVM's `mem2reg` pass able to
/// promote every scalar local straight to an SSA register: an `alloca`
/// declared once, at function entry, dominating every loop iteration that
/// stores to it — not one created fresh inside a loop body. Recurses into
/// `While`/`If` bodies (the only two nesting constructs this scope
/// supports) so a `Let` re-declared partway down a loop (this backend's
/// `total: Int64 = total + i` re-`Let` pattern) still gets a single,
/// entry-block-hoisted slot.
fn collect_scalar_lets(stmts: &[Stmt], out: &mut Vec<(String, Ty)>) -> Result<(), String> {
  for stmt in stmts {
    match stmt {
      Stmt::Let { name, ty, .. } if !ty.starts_with("Array[") => {
        out.push((name.clone(), parse_scalar_ty(ty)?));
      }
      Stmt::Let { .. } => {} // array Let — handled by build_stmt directly, not pre-allocated
      Stmt::While { body, .. } => collect_scalar_lets(body, out)?,
      Stmt::If {
        then_branch,
        else_branch,
        ..
      } => {
        collect_scalar_lets(then_branch, out)?;
        if let Some(else_b) = else_branch {
          collect_scalar_lets(else_b, out)?;
        }
      }
      _ => {}
    }
  }
  Ok(())
}

fn llvm_scalar_type<'ctx>(context: &'ctx Context, ty: Ty) -> BasicTypeEnum<'ctx> {
  match ty {
    Ty::Int64 => context.i64_type().into(),
    Ty::Float64 => context.f64_type().into(),
  }
}

/// Evaluates a general-purpose expression, returning its LLVM value and
/// inferred `Ty`. Handles every `Expr` variant this backend's scope
/// covers: `Ident`, `Int`, `Float`, `Add`, `Index`. `Compare` is
/// deliberately NOT handled here — it only ever appears as a `While`/`If`
/// condition, which `build_cond` compiles directly to an `i1`, since
/// there's no boolean `Ty` in this backend's scalar type universe to
/// return it as. Anything else (`Call`, `New`, `MethodCall`,
/// `InstanceVar`, `ArrayLit` outside a `Let`, `Lambda`) is out of scope
/// and returns `Err`, not a panic.
fn build_expr<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  expr: &Expr,
  fctx: &FnCtx<'ctx>,
) -> Result<(BasicValueEnum<'ctx>, Ty), String> {
  match expr {
    Expr::Ident(name) => {
      let (ptr, ty) = fctx.scalar_vars.get(name).ok_or_else(|| {
        format!(
          "emerald-codegen-llvm: `{name}` is not a known scalar local (unsupported shape for this backend's scope)"
        )
      })?;
      let loaded = builder
        .build_load(llvm_scalar_type(context, *ty), *ptr, name)
        .map_err(|e| e.to_string())?;
      Ok((loaded, *ty))
    }
    Expr::Int(n) => Ok((
      context.i64_type().const_int(*n as u64, true).into(),
      Ty::Int64,
    )),
    Expr::Float(f) => Ok((context.f64_type().const_float(*f).into(), Ty::Float64)),
    Expr::Add(l, r) => {
      let (lv, lty) = build_expr(context, builder, l, fctx)?;
      let (rv, rty) = build_expr(context, builder, r, fctx)?;
      match (lty, rty) {
        (Ty::Int64, Ty::Int64) => {
          let sum = builder
            .build_int_add(lv.into_int_value(), rv.into_int_value(), "addtmp")
            .map_err(|e| e.to_string())?;
          Ok((sum.into(), Ty::Int64))
        }
        (Ty::Float64, Ty::Float64) => {
          let sum = builder
            .build_float_add(lv.into_float_value(), rv.into_float_value(), "faddtmp")
            .map_err(|e| e.to_string())?;
          Ok((sum.into(), Ty::Float64))
        }
        _ => Err("emerald-codegen-llvm: `+` operands must both be Int64 or both Float64 (mixed types should have been rejected by sema already)".to_string()),
      }
    }
    Expr::Index(array, index) => {
      let Expr::Ident(arr_name) = array.as_ref() else {
        return Err(
          "emerald-codegen-llvm: array indexing is only supported on a plain local-variable array"
            .to_string(),
        );
      };
      let (base_ptr, elem_ty) = *fctx
        .array_vars
        .get(arr_name)
        .ok_or_else(|| format!("emerald-codegen-llvm: `{arr_name}` is not a known array local"))?;
      let (idx_val, idx_ty) = build_expr(context, builder, index, fctx)?;
      if idx_ty != Ty::Int64 {
        return Err("emerald-codegen-llvm: array index must be Int64".to_string());
      }
      let elem_llvm_ty = llvm_scalar_type(context, elem_ty);
      let elem_ptr = unsafe {
        builder
          .build_in_bounds_gep(
            elem_llvm_ty,
            base_ptr,
            &[idx_val.into_int_value()],
            "elemptr",
          )
          .map_err(|e| e.to_string())?
      };
      let loaded = builder
        .build_load(elem_llvm_ty, elem_ptr, "elem")
        .map_err(|e| e.to_string())?;
      Ok((loaded, elem_ty))
    }
    _ => Err(format!(
      "emerald-codegen-llvm: unsupported expression `{expr:?}` (outside plan 16's scoped LLVM backend)"
    )),
  }
}

/// Compiles a `While`/`If` condition — always an `Expr::Compare` in this
/// backend's scope (the only boolean-producing expression the AST has) —
/// straight to an `i1`.
fn build_cond<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  expr: &Expr,
  fctx: &FnCtx<'ctx>,
) -> Result<IntValue<'ctx>, String> {
  let Expr::Compare(l, op, r) = expr else {
    return Err(format!(
      "emerald-codegen-llvm: condition must be a comparison (got `{expr:?}`) — unsupported in this backend's scope"
    ));
  };
  let (lv, lty) = build_expr(context, builder, l, fctx)?;
  let (rv, rty) = build_expr(context, builder, r, fctx)?;
  if lty != rty {
    return Err("emerald-codegen-llvm: comparison operands must be the same type".to_string());
  }
  match lty {
    Ty::Int64 => {
      let pred = match op {
        CompareOp::Lt => IntPredicate::SLT,
        CompareOp::Gt => IntPredicate::SGT,
        CompareOp::Le => IntPredicate::SLE,
        CompareOp::Ge => IntPredicate::SGE,
        CompareOp::Eq => IntPredicate::EQ,
        CompareOp::Ne => IntPredicate::NE,
      };
      builder
        .build_int_compare(pred, lv.into_int_value(), rv.into_int_value(), "cmptmp")
        .map_err(|e| e.to_string())
    }
    Ty::Float64 => {
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
        .build_float_compare(
          pred,
          lv.into_float_value(),
          rv.into_float_value(),
          "fcmptmp",
        )
        .map_err(|e| e.to_string())
    }
  }
}

/// Builds one statement block. Returns `Ok(())` — every construct in this
/// backend's scope (`Let`/`While`/`If`/`Expr(puts ...)`) always falls
/// through to the next statement; there's no `Return`/`Break`/`Next` in
/// scope to make early termination a concern here (unlike
/// `emerald-codegen`'s `build_block`, which does need to track that).
fn build_stmts<'ctx>(
  context: &'ctx Context,
  builder: &Builder<'ctx>,
  func: FunctionValue<'ctx>,
  stmts: &[Stmt],
  fctx: &mut FnCtx<'ctx>,
) -> Result<(), String> {
  for stmt in stmts {
    match stmt {
      Stmt::Let { name, ty, value } if ty.starts_with("Array[") => {
        let elem_ty = parse_array_elem_ty(ty)?;
        let Expr::ArrayLit(elements) = value else {
          return Err(format!(
            "emerald-codegen-llvm: `{name}: {ty}` must be initialized with an array literal (this backend's scope doesn't support re-binding an existing array)"
          ));
        };
        let elem_llvm_ty = llvm_scalar_type(context, elem_ty);
        // `emerald_alloc` takes a byte size, not an element count (every
        // element is 8 bytes — mirrors `emerald-codegen`'s own
        // `ARRAY_ELEM_SIZE`). Passing the raw element count here under-
        // allocates the backing buffer by 8x and corrupts the heap the
        // first time the array is written past its true (undersized)
        // extent — caught by `array_traversal_benchmark_program_matches_
        // expected_output` crashing with a glibc malloc consistency
        // assertion once enough heap activity happened afterward to
        // notice the corruption.
        let byte_size = elements.len() as u64 * 8;
        let len = context.i64_type().const_int(byte_size, false);
        let call = builder
          .build_call(fctx.alloc, &[len.into()], "arralloc")
          .map_err(|e| e.to_string())?;
        let base_ptr = match call.try_as_basic_value() {
          ValueKind::Basic(v) => v.into_pointer_value(),
          ValueKind::Instruction(_) => {
            return Err("emerald-codegen-llvm: emerald_alloc call produced no value".to_string());
          }
        };
        for (i, e) in elements.iter().enumerate() {
          let (v, vty) = build_expr(context, builder, e, fctx)?;
          if vty != elem_ty {
            return Err(format!(
              "emerald-codegen-llvm: array element {i} of `{name}` has the wrong type"
            ));
          }
          let idx = context.i64_type().const_int(i as u64, false);
          let elem_ptr = unsafe {
            builder
              .build_in_bounds_gep(elem_llvm_ty, base_ptr, &[idx], "initelem")
              .map_err(|e| e.to_string())?
          };
          builder
            .build_store(elem_ptr, v)
            .map_err(|e| e.to_string())?;
        }
        fctx.array_vars.insert(name.clone(), (base_ptr, elem_ty));
      }
      Stmt::Let { name, ty, value } => {
        let decl_ty = parse_scalar_ty(ty)?;
        let (v, vty) = build_expr(context, builder, value, fctx)?;
        if vty != decl_ty {
          return Err(format!(
            "emerald-codegen-llvm: `{name}: {ty}` assigned a value of a different type"
          ));
        }
        let (ptr, _) = *fctx.scalar_vars.get(name).ok_or_else(|| {
          format!("emerald-codegen-llvm: internal error — `{name}` wasn't pre-allocated")
        })?;
        builder.build_store(ptr, v).map_err(|e| e.to_string())?;
      }
      Stmt::While { cond, body } => {
        let cond_bb = context.append_basic_block(func, "while.cond");
        let body_bb = context.append_basic_block(func, "while.body");
        let after_bb = context.append_basic_block(func, "while.after");

        builder
          .build_unconditional_branch(cond_bb)
          .map_err(|e| e.to_string())?;
        builder.position_at_end(cond_bb);
        let cond_val = build_cond(context, builder, cond, fctx)?;
        builder
          .build_conditional_branch(cond_val, body_bb, after_bb)
          .map_err(|e| e.to_string())?;

        builder.position_at_end(body_bb);
        build_stmts(context, builder, func, body, fctx)?;
        builder
          .build_unconditional_branch(cond_bb)
          .map_err(|e| e.to_string())?;

        builder.position_at_end(after_bb);
      }
      Stmt::If {
        cond,
        then_branch,
        else_branch,
      } => {
        let then_bb = context.append_basic_block(func, "if.then");
        let else_bb = context.append_basic_block(func, "if.else");
        let merge_bb = context.append_basic_block(func, "if.merge");

        let cond_val = build_cond(context, builder, cond, fctx)?;
        builder
          .build_conditional_branch(cond_val, then_bb, else_bb)
          .map_err(|e| e.to_string())?;

        builder.position_at_end(then_bb);
        build_stmts(context, builder, func, then_branch, fctx)?;
        builder
          .build_unconditional_branch(merge_bb)
          .map_err(|e| e.to_string())?;

        builder.position_at_end(else_bb);
        if let Some(else_b) = else_branch {
          build_stmts(context, builder, func, else_b, fctx)?;
        }
        builder
          .build_unconditional_branch(merge_bb)
          .map_err(|e| e.to_string())?;

        builder.position_at_end(merge_bb);
      }
      Stmt::Expr(Expr::Call(name, args)) if name == "puts" && args.len() == 1 => {
        let (v, ty) = build_expr(context, builder, &args[0], fctx)?;
        match ty {
          Ty::Int64 => {
            builder
              .build_call(fctx.print_i64, &[v.into()], "puts_i64")
              .map_err(|e| e.to_string())?;
          }
          Ty::Float64 => {
            builder
              .build_call(fctx.print_f64, &[v.into()], "puts_f64")
              .map_err(|e| e.to_string())?;
          }
        }
      }
      other => {
        return Err(format!(
          "emerald-codegen-llvm: unsupported statement `{other:?}` (outside plan 16's scoped LLVM backend)"
        ));
      }
    }
  }
  Ok(())
}

/// Compiles a type-checked `Program` to a native object file at
/// `out_path` via LLVM, at `OptimizationLevel::Aggressive` — the LLVM
/// counterpart to `emerald-codegen::compile_to_object`, sharing its exact
/// signature so `emerald-cli` can pick either backend behind one `match`.
///
/// Scoped to top-level-statement-only programs (see this crate's module
/// doc); a `Program` containing any `Item::Function`/`Class`/`Module`
/// returns `Err` immediately, before any LLVM work starts.
pub fn compile_to_object(program: &Program, out_path: &Path) -> Result<(), String> {
  if program.items.iter().any(|it| !matches!(it, Item::Stmt(_))) {
    return Err(
      "emerald-codegen-llvm: this backend only supports top-level-statement programs (no functions/classes/modules) — see plan 16's Decision log".to_string(),
    );
  }

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
    .ok_or_else(|| "emerald-codegen-llvm: failed to create a target machine".to_string())?;

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

  // `extern "C" fn() -> i32` — matches the C ABI's `int main(void)` (and
  // `emerald-codegen`'s own `define_main`), not the `Int64` scalar type
  // used inside the program.
  let i32_ty = context.i32_type();
  let main_fn = module.add_function("main", i32_ty.fn_type(&[], false), Some(Linkage::External));
  let entry = context.append_basic_block(main_fn, "entry");
  builder.position_at_end(entry);

  let top_stmts: Vec<Stmt> = program
    .items
    .iter()
    .map(|it| {
      let Item::Stmt(s) = it else {
        unreachable!("checked above")
      };
      s.clone()
    })
    .collect();

  let mut scalar_decls = Vec::new();
  collect_scalar_lets(&top_stmts, &mut scalar_decls)?;
  let mut scalar_vars = HashMap::new();
  for (name, ty) in scalar_decls {
    // A `Let` inside a loop body re-declares the same name every
    // iteration — only the first occurrence needs an `alloca`.
    scalar_vars.entry(name.clone()).or_insert_with(|| {
      let alloca = builder
        .build_alloca(llvm_scalar_type(&context, ty), &name)
        .expect("alloca in a fresh entry block cannot fail");
      (alloca, ty)
    });
  }

  let mut fctx = FnCtx {
    scalar_vars,
    array_vars: HashMap::new(),
    print_i64,
    print_f64,
    alloc,
  };

  build_stmts(&context, &builder, main_fn, &top_stmts, &mut fctx)?;

  let zero = i32_ty.const_int(0, false);
  builder
    .build_return(Some(&zero))
    .map_err(|e| e.to_string())?;

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

  fn compile_link_run(src: &str) -> String {
    let program = emerald_parser::parse(src).expect("should parse");
    let dir = std::env::temp_dir();
    let unique = format!("{}_{:?}", std::process::id(), std::thread::current().id());
    let obj_path = dir.join(format!("emerald_codegen_llvm_aot_{unique}.o"));
    let bin_path = dir.join(format!("emerald_codegen_llvm_aot_bin_{unique}"));

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
  fn while_loop_sums_1_to_3_and_prints_6() {
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "6\n");
  }

  #[test]
  fn array_literal_index_and_sum() {
    let src = "arr: Array[Int64] = [10, 20, 30]\nputs arr[0] + arr[1] + arr[2]\n";
    assert_eq!(compile_link_run(src), "60\n");
  }

  #[test]
  fn float64_arithmetic_and_print() {
    let src = "x: Float64 = 1.5\ny: Float64 = 2.5\nputs x + y\n";
    assert_eq!(compile_link_run(src), "4\n");
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

  #[test]
  fn function_definition_is_rejected_not_panicked() {
    let program = Program {
      items: vec![Item::Function(emerald_parser::Function {
        name: "f".into(),
        params: vec![],
        return_type: "Void".into(),
        body: vec![],
      })],
    };
    let out = std::env::temp_dir().join("emerald_codegen_llvm_should_not_exist.o");
    assert!(compile_to_object(&program, &out).is_err());
  }
}
