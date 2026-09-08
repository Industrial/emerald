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
