//! Emerald's codegen backend, built on Cranelift — chosen in
//! `02 toolchain-prototype` over Inkwell/LLVM for v1 (see
//! `spec/COMPILER.md` for the decision record and the revisit trigger).
//!
//! This prototype JIT-compiles a single hand-built function equivalent to
//! `fn add(a: i64, b: i64) -> i64 { a + b }` — proving the codegen path
//! inception §17's first milestone needs, ahead of a real typed-IR input
//! (which does not exist until `emerald-ir` lands).

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// Builds and JIT-compiles `fn add(i64, i64) -> i64 { a + b }`, then calls
/// it with `(a, b)` and returns the result.
pub fn build_and_run_add(a: i64, b: i64) -> i64 {
  let mut flag_builder = settings::builder();
  flag_builder.set("is_pic", "false").unwrap();
  let isa_builder =
    cranelift_native::builder().expect("host machine is not supported by Cranelift");
  let isa = isa_builder
    .finish(settings::Flags::new(flag_builder))
    .unwrap();

  let jit_builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
  let mut module = JITModule::new(jit_builder);

  let mut ctx = module.make_context();
  let mut func_ctx = FunctionBuilderContext::new();

  ctx.func.signature.params.push(AbiParam::new(types::I64));
  ctx.func.signature.params.push(AbiParam::new(types::I64));
  ctx.func.signature.returns.push(AbiParam::new(types::I64));

  let frontend_config = module.target_config();
  {
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);

    let a_val = builder.block_params(block)[0];
    let b_val = builder.block_params(block)[1];
    let sum = builder.ins().iadd(a_val, b_val);
    builder.ins().return_(&[sum]);
    builder.finalize(frontend_config);
  }

  let func_id = module
    .declare_function("add", Linkage::Export, &ctx.func.signature)
    .expect("declare add");
  module
    .define_function(func_id, &mut ctx)
    .expect("define add");
  module.clear_context(&mut ctx);
  module.finalize_definitions().expect("finalize add");

  let code_ptr = module.get_finalized_function(func_id);
  // SAFETY: `code_ptr` points at freshly JIT-compiled code whose signature
  // (two i64 params, one i64 return) exactly matches `fn(i64, i64) -> i64`
  // as declared above.
  let add_fn = unsafe { std::mem::transmute::<*const u8, fn(i64, i64) -> i64>(code_ptr) };
  add_fn(a, b)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn add_20_and_22_is_42() {
    assert_eq!(build_and_run_add(20, 22), 42);
  }

  #[test]
  fn add_is_commutative_for_negatives() {
    assert_eq!(build_and_run_add(-5, 3), -2);
  }
}

// --- Ahead-of-time path: real Program -> object file (plan 06) --------

use cranelift_module::FuncId;
use cranelift_object::{ObjectBuilder, ObjectModule};
use emerald_parser::{Expr, Function as AstFunction, Item, Program};
use std::collections::HashMap;
use std::path::Path;

fn host_isa() -> Result<std::sync::Arc<dyn cranelift::codegen::isa::TargetIsa>, String> {
  let mut flag_builder = settings::builder();
  // Non-PIC: emerald-cli always links a plain executable, never a shared
  // library, so position-dependent code avoids the DT_TEXTREL relocation
  // ld otherwise warns about for calls into the runtime shim.
  flag_builder
    .set("is_pic", "false")
    .map_err(|e| e.to_string())?;
  let isa_builder = cranelift_native::builder().map_err(|e| e.to_string())?;
  isa_builder
    .finish(settings::Flags::new(flag_builder))
    .map_err(|e| e.to_string())
}

fn build_expr(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  expr: &Expr,
  env: &HashMap<String, Value>,
  user_func_ids: &HashMap<String, FuncId>,
) -> Result<Value, String> {
  match expr {
    Expr::Ident(name) => env
      .get(name)
      .copied()
      .ok_or_else(|| format!("codegen: undefined variable `{name}`")),
    Expr::Int(n) => Ok(builder.ins().iconst(types::I64, *n)),
    Expr::Add(lhs, rhs) => {
      let l = build_expr(builder, module, lhs, env, user_func_ids)?;
      let r = build_expr(builder, module, rhs, env, user_func_ids)?;
      Ok(builder.ins().iadd(l, r))
    }
    Expr::Call(name, args) => {
      let func_id = *user_func_ids.get(name).ok_or_else(|| {
        format!("codegen: unsupported call to `{name}` (not a compiled user function)")
      })?;
      let func_ref = module.declare_func_in_func(func_id, builder.func);
      let mut arg_vals = Vec::with_capacity(args.len());
      for a in args {
        arg_vals.push(build_expr(builder, module, a, env, user_func_ids)?);
      }
      let call = builder.ins().call(func_ref, &arg_vals);
      Ok(builder.inst_results(call)[0])
    }
  }
}

fn define_user_function(
  module: &mut ObjectModule,
  ctx: &mut cranelift::codegen::Context,
  func_ctx: &mut FunctionBuilderContext,
  f: &AstFunction,
  id: FuncId,
  user_func_ids: &HashMap<String, FuncId>,
) -> Result<(), String> {
  ctx.func.signature.params.clear();
  ctx.func.signature.returns.clear();
  for _ in &f.params {
    ctx.func.signature.params.push(AbiParam::new(types::I64));
  }
  ctx.func.signature.returns.push(AbiParam::new(types::I64));

  let frontend_config = module.target_config();
  {
    let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);

    let mut env: HashMap<String, Value> = HashMap::new();
    for (i, p) in f.params.iter().enumerate() {
      env.insert(p.name.clone(), builder.block_params(block)[i]);
    }

    let result = build_expr(&mut builder, module, &f.body, &env, user_func_ids)?;
    builder.ins().return_(&[result]);
    builder.finalize(frontend_config);
  }

  module.define_function(id, ctx).map_err(|e| e.to_string())?;
  module.clear_context(ctx);
  Ok(())
}

fn define_main(
  module: &mut ObjectModule,
  ctx: &mut cranelift::codegen::Context,
  func_ctx: &mut FunctionBuilderContext,
  program: &Program,
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: FuncId,
) -> Result<(), String> {
  ctx.func.signature.params.clear();
  ctx.func.signature.returns.clear();
  ctx.func.signature.returns.push(AbiParam::new(types::I32));

  let main_id = module
    .declare_function("main", Linkage::Export, &ctx.func.signature)
    .map_err(|e| e.to_string())?;

  let frontend_config = module.target_config();
  {
    let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
    let block = builder.create_block();
    builder.switch_to_block(block);
    builder.seal_block(block);

    let env = HashMap::new();
    for item in &program.items {
      if let Item::Expr(Expr::Call(name, args)) = item {
        if name == "puts" && args.len() == 1 {
          let val = build_expr(&mut builder, module, &args[0], &env, user_func_ids)?;
          let print_ref = module.declare_func_in_func(print_func_id, builder.func);
          builder.ins().call(print_ref, &[val]);
        } else {
          return Err(format!("codegen: unsupported top-level call `{name}`"));
        }
      } else if let Item::Expr(other) = item {
        return Err(format!(
          "codegen: unsupported top-level expression shape: {other:?}"
        ));
      }
    }

    let zero = builder.ins().iconst(types::I32, 0);
    builder.ins().return_(&[zero]);
    builder.finalize(frontend_config);
  }

  module
    .define_function(main_id, ctx)
    .map_err(|e| e.to_string())?;
  module.clear_context(ctx);
  Ok(())
}

/// Compiles a type-checked `Program` to a native object file at `out_path`.
/// One exported function per `Item::Function`, plus a `main` (`extern "C"
/// fn() -> i32`) that evaluates the top-level `puts` call statement via
/// the imported `emerald_print_i64` runtime symbol (see
/// `runtime/emerald_runtime.c`) and returns 0.
pub fn compile_to_object(program: &Program, out_path: &Path) -> Result<(), String> {
  let isa = host_isa()?;
  let obj_builder = ObjectBuilder::new(
    isa,
    "emerald_module",
    cranelift_module::default_libcall_names(),
  )
  .map_err(|e| e.to_string())?;
  let mut module = ObjectModule::new(obj_builder);

  let mut print_sig = module.make_signature();
  print_sig.params.push(AbiParam::new(types::I64));
  let print_func_id = module
    .declare_function("emerald_print_i64", Linkage::Import, &print_sig)
    .map_err(|e| e.to_string())?;

  let mut user_func_ids: HashMap<String, FuncId> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      let mut sig = module.make_signature();
      for _ in &f.params {
        sig.params.push(AbiParam::new(types::I64));
      }
      sig.returns.push(AbiParam::new(types::I64));
      let id = module
        .declare_function(&f.name, Linkage::Export, &sig)
        .map_err(|e| e.to_string())?;
      user_func_ids.insert(f.name.clone(), id);
    }
  }

  let mut ctx = module.make_context();
  let mut func_ctx = FunctionBuilderContext::new();

  for item in &program.items {
    if let Item::Function(f) = item {
      let id = *user_func_ids.get(&f.name).unwrap();
      define_user_function(&mut module, &mut ctx, &mut func_ctx, f, id, &user_func_ids)?;
    }
  }

  define_main(
    &mut module,
    &mut ctx,
    &mut func_ctx,
    program,
    &user_func_ids,
    print_func_id,
  )?;

  let object = module.finish();
  let bytes = object.emit().map_err(|e| e.to_string())?;
  std::fs::write(out_path, bytes).map_err(|e| e.to_string())?;
  Ok(())
}

#[cfg(test)]
mod aot_tests {
  use super::*;

  #[test]
  fn compiles_hello_em_to_an_object_file() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";
    let program = emerald_parser::parse(src).expect("should parse");
    emerald_sema_stub_check(&program);

    let dir = std::env::temp_dir();
    let out_path = dir.join(format!("emerald_codegen_test_{}.o", std::process::id()));
    compile_to_object(&program, &out_path).expect("should compile to object file");

    let bytes = std::fs::read(&out_path).expect("object file should exist");
    assert!(!bytes.is_empty(), "object file should not be empty");
    // ELF magic number, since this environment targets Linux.
    assert_eq!(
      &bytes[0..4],
      b"\x7fELF",
      "should be a valid ELF object file"
    );

    std::fs::remove_file(&out_path).ok();
  }

  #[test]
  fn unsupported_top_level_shape_errors_not_panics() {
    // A bare Ident item (no such surface syntax exists yet, but codegen
    // must not panic if it ever receives one) — constructed directly
    // since the grammar can't produce it.
    let program = Program {
      items: vec![Item::Expr(Expr::Ident("x".into()))],
    };
    let dir = std::env::temp_dir();
    let out_path = dir.join(format!("emerald_codegen_test_bad_{}.o", std::process::id()));
    let result = compile_to_object(&program, &out_path);
    assert!(result.is_err());
    std::fs::remove_file(&out_path).ok();
  }

  /// This crate's tests intentionally don't depend on `emerald-sema` (no
  /// cyclic dev-dependency); this is a no-op placeholder documenting that
  /// codegen assumes its input already passed `emerald_sema::check_program`
  /// (enforced end-to-end by `emerald-cli`'s pipeline, not by this crate).
  fn emerald_sema_stub_check(_program: &Program) {}
}
