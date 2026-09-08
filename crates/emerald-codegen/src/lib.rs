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

// --- Ahead-of-time path: real Program -> object file (plans 06, 07, 08) -

use cranelift::codegen::ir::MemFlagsData;
use cranelift::codegen::ir::condcodes::IntCC;
use cranelift_module::FuncId;
use cranelift_object::{ObjectBuilder, ObjectModule};
use emerald_parser::{ClassDef, CompareOp, Expr, Function as AstFunction, Item, Program, Stmt};
use std::collections::HashMap;
use std::path::Path;

/// The header/exit blocks of the innermost enclosing loop, for `break`
/// (jump to `exit`) / `next` (jump back to `header`) to target.
struct LoopTargets {
  header: Block,
  exit: Block,
}

/// A field's byte offset and Cranelift storage type within its class's
/// instance layout (plan 08: every field is naively 8 bytes — see the
/// plan's Decision log).
#[derive(Clone, Copy)]
struct FieldInfo {
  offset: i32,
  cl_type: types::Type,
}

struct ClassLayout {
  fields: HashMap<String, FieldInfo>,
  size: i64,
}

/// `Float64` fields/params/locals are Cranelift `F64`; everything else
/// (`Int64`, and class-instance pointers, which are just addresses) is
/// `I64`. No other primitive width is in the type universe this compiler
/// supports yet.
fn cranelift_type(ty_name: &str) -> types::Type {
  if ty_name == "Float64" {
    types::F64
  } else {
    types::I64
  }
}

/// Pushes a return `AbiParam` for `ty_name` — unless it's `"Void"`, which
/// gets a genuinely empty Cranelift return list rather than a phantom
/// `I64` slot nothing in the body ever produces (a Void-returning method
/// like `initialize`, whose body ends in `@field = value` rather than an
/// expression, must not be forced to fabricate a return value).
fn push_return_type(returns: &mut Vec<AbiParam>, ty_name: &str) {
  if ty_name != "Void" {
    returns.push(AbiParam::new(cranelift_type(ty_name)));
  }
}

fn build_class_layout(c: &ClassDef) -> ClassLayout {
  let mut fields = HashMap::new();
  let mut offset = 0i32;
  for f in &c.fields {
    fields.insert(
      f.name.clone(),
      FieldInfo {
        offset,
        cl_type: cranelift_type(&f.ty),
      },
    );
    offset += 8;
  }
  ClassLayout {
    fields,
    size: offset as i64,
  }
}

/// Context that's fixed for the duration of compiling one function/method
/// body — bundled to keep `build_expr`/`build_stmt`'s own parameter lists
/// from growing without bound as plans 07/08 added control flow and
/// classes. `self_ctx` is `Some((self_var, &class.fields))` only while
/// compiling a method body (see `define_method`).
#[derive(Clone, Copy)]
struct Ctx<'a> {
  user_func_ids: &'a HashMap<String, FuncId>,
  classes: &'a HashMap<String, ClassLayout>,
  print_i64_func_id: Option<FuncId>,
  print_f64_func_id: Option<FuncId>,
  alloc_func_id: FuncId,
  self_ctx: Option<(Variable, &'a HashMap<String, FieldInfo>)>,
}

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

/// Every array element is 8 bytes — `Int64`/`Float64`/a class-instance
/// pointer all are (plan 08's `FieldInfo` makes the same simplification
/// for fields; plan 09's Decision log carries it forward for `Array[T]`
/// storage, per `spec/TYPE_SYSTEM.md` §8's packed/contiguous requirement).
const ARRAY_ELEM_SIZE: i64 = 8;

/// `local_classes` maps a local variable name to its declared class name
/// (from `Stmt::Let`'s explicit type annotation) — codegen has no typed
/// IR to consult (see `spec/COMPILER.md`'s deferred-`emerald-ir` note), so
/// a `.method` call on a local resolves its receiver's class this way
/// rather than by inferring it from the (type-erased, both-just-`i64`)
/// runtime pointer value. `local_array_elem_types` is the same idea for
/// `Array[Elem]` locals, mapping to `Elem`'s type name (used to pick the
/// Cranelift load type when indexing — an array's own runtime pointer
/// value carries no element-type information).
fn build_expr(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  expr: &Expr,
  vars: &HashMap<String, Variable>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, String>,
  ctx: &Ctx,
) -> Result<Value, String> {
  match expr {
    Expr::Ident(name) => {
      let var = vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      Ok(builder.use_var(*var))
    }
    Expr::Int(n) => Ok(builder.ins().iconst(types::I64, *n)),
    Expr::Float(f) => Ok(builder.ins().f64const(*f)),
    Expr::Add(lhs, rhs) => {
      let l = build_expr(
        builder,
        module,
        lhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let r = build_expr(
        builder,
        module,
        rhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      if builder.func.dfg.value_type(l) == types::F64 {
        Ok(builder.ins().fadd(l, r))
      } else {
        Ok(builder.ins().iadd(l, r))
      }
    }
    Expr::Compare(lhs, op, rhs) => {
      let l = build_expr(
        builder,
        module,
        lhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let r = build_expr(
        builder,
        module,
        rhs,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let cc = match op {
        CompareOp::Lt => IntCC::SignedLessThan,
        CompareOp::Gt => IntCC::SignedGreaterThan,
        CompareOp::Le => IntCC::SignedLessThanOrEqual,
        CompareOp::Ge => IntCC::SignedGreaterThanOrEqual,
        CompareOp::Eq => IntCC::Equal,
        CompareOp::Ne => IntCC::NotEqual,
      };
      Ok(builder.ins().icmp(cc, l, r))
    }
    Expr::Call(name, args) => {
      let func_id = *ctx.user_func_ids.get(name).ok_or_else(|| {
        format!("codegen: unsupported call to `{name}` (not a compiled user function)")
      })?;
      let func_ref = module.declare_func_in_func(func_id, builder.func);
      let mut arg_vals = Vec::with_capacity(args.len());
      for a in args {
        arg_vals.push(build_expr(
          builder,
          module,
          a,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?);
      }
      let call = builder.ins().call(func_ref, &arg_vals);
      Ok(builder.inst_results(call)[0])
    }
    Expr::New(class_name, args) => {
      let layout = ctx
        .classes
        .get(class_name)
        .ok_or_else(|| format!("codegen: unknown class `{class_name}`"))?;
      let size_val = builder.ins().iconst(types::I64, layout.size);
      let alloc_ref = module.declare_func_in_func(ctx.alloc_func_id, builder.func);
      let call = builder.ins().call(alloc_ref, &[size_val]);
      let ptr = builder.inst_results(call)[0];

      let init_key = format!("{class_name}_initialize");
      if let Some(&init_id) = ctx.user_func_ids.get(&init_key) {
        let init_ref = module.declare_func_in_func(init_id, builder.func);
        let mut call_args = vec![ptr];
        for a in args {
          call_args.push(build_expr(
            builder,
            module,
            a,
            vars,
            local_classes,
            local_array_elem_types,
            ctx,
          )?);
        }
        builder.ins().call(init_ref, &call_args);
      }
      Ok(ptr)
    }
    Expr::MethodCall(recv, method, args) => {
      let Expr::Ident(recv_name) = recv.as_ref() else {
        return Err(
          "codegen: method calls are only supported on a plain local-variable receiver".to_string(),
        );
      };
      let class_name = local_classes.get(recv_name).ok_or_else(|| {
        format!("codegen: cannot determine the class of `{recv_name}` for `.{method}`")
      })?;
      let recv_val = build_expr(
        builder,
        module,
        recv,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let key = format!("{class_name}_{method}");
      let func_id = *ctx
        .user_func_ids
        .get(&key)
        .ok_or_else(|| format!("codegen: unsupported method call `{class_name}.{method}`"))?;
      let func_ref = module.declare_func_in_func(func_id, builder.func);
      let mut call_args = vec![recv_val];
      for a in args {
        call_args.push(build_expr(
          builder,
          module,
          a,
          vars,
          local_classes,
          local_array_elem_types,
          ctx,
        )?);
      }
      let call = builder.ins().call(func_ref, &call_args);
      Ok(builder.inst_results(call)[0])
    }
    Expr::InstanceVar(name) => {
      let (self_var, fields) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name}` used outside of a method body"))?;
      let field = fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?;
      let self_ptr = builder.use_var(self_var);
      Ok(
        builder
          .ins()
          .load(field.cl_type, MemFlagsData::new(), self_ptr, field.offset),
      )
    }
    Expr::ArrayLit(elements) => build_array_lit(
      builder,
      module,
      elements,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
    Expr::Index(array, index) => build_index(
      builder,
      module,
      array,
      index,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    ),
  }
}

/// `emerald_alloc`s a flat `elements.len() * 8`-byte buffer, then stores
/// each element at its `i * 8` offset — no length prefix, no bounds
/// checking (plan 09's Decision log). The returned `Value` is just the
/// base pointer; nothing about it carries the element type, which is why
/// `build_index` needs `local_array_elem_types`.
#[allow(clippy::too_many_arguments)]
fn build_array_lit(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  elements: &[Expr],
  vars: &HashMap<String, Variable>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, String>,
  ctx: &Ctx,
) -> Result<Value, String> {
  let size_val = builder
    .ins()
    .iconst(types::I64, elements.len() as i64 * ARRAY_ELEM_SIZE);
  let alloc_ref = module.declare_func_in_func(ctx.alloc_func_id, builder.func);
  let call = builder.ins().call(alloc_ref, &[size_val]);
  let ptr = builder.inst_results(call)[0];
  for (i, e) in elements.iter().enumerate() {
    let v = build_expr(
      builder,
      module,
      e,
      vars,
      local_classes,
      local_array_elem_types,
      ctx,
    )?;
    builder.ins().store(
      MemFlagsData::new(),
      v,
      ptr,
      i as i32 * ARRAY_ELEM_SIZE as i32,
    );
  }
  Ok(ptr)
}

/// `arr[i]`. Same "plain local-variable" restriction as `MethodCall`'s
/// receiver (plan 09's Decision log) — the element type comes from
/// `local_array_elem_types`, keyed by the array's own local name.
#[allow(clippy::too_many_arguments)]
fn build_index(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  array: &Expr,
  index: &Expr,
  vars: &HashMap<String, Variable>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, String>,
  ctx: &Ctx,
) -> Result<Value, String> {
  let Expr::Ident(arr_name) = array else {
    return Err(
      "codegen: array indexing is only supported on a plain local-variable array".to_string(),
    );
  };
  let elem_ty_name = local_array_elem_types.get(arr_name).ok_or_else(|| {
    format!("codegen: cannot determine the element type of `{arr_name}` for indexing")
  })?;
  let cl_elem_ty = cranelift_type(elem_ty_name);
  let base_ptr = build_expr(
    builder,
    module,
    array,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let idx_val = build_expr(
    builder,
    module,
    index,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let byte_offset = builder.ins().imul_imm_s(idx_val, ARRAY_ELEM_SIZE);
  let addr = builder.ins().iadd(base_ptr, byte_offset);
  Ok(builder.ins().load(cl_elem_ty, MemFlagsData::new(), addr, 0))
}

/// Emits `puts <inner>` as a call to whichever of `emerald_print_i64` /
/// `emerald_print_f64` matches the built argument's actual Cranelift
/// value type (see `runtime/emerald_runtime.c` and plan 08's Decision
/// log — `puts` is a call-site-polymorphic intrinsic, not an overloaded
/// user function).
#[allow(clippy::too_many_arguments)]
fn build_puts(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  arg: &Expr,
  vars: &HashMap<String, Variable>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, String>,
  ctx: &Ctx,
) -> Result<(), String> {
  let val = build_expr(
    builder,
    module,
    arg,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let target_id = if builder.func.dfg.value_type(val) == types::F64 {
    ctx.print_f64_func_id.ok_or(
      "codegen: `puts` is only supported at the program's top level, not inside a function body",
    )?
  } else {
    ctx.print_i64_func_id.ok_or(
      "codegen: `puts` is only supported at the program's top level, not inside a function body",
    )?
  };
  let print_ref = module.declare_func_in_func(target_id, builder.func);
  builder.ins().call(print_ref, &[val]);
  Ok(())
}

/// `arr[i] = value`. Same "plain local-variable array" restriction as
/// `build_index`'s read side (plan 09's Decision log) — the element size
/// is always `ARRAY_ELEM_SIZE`, so unlike a read this needs no element
/// type lookup, just the address arithmetic.
#[allow(clippy::too_many_arguments)]
fn build_set_index(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  array: &Expr,
  index: &Expr,
  value: &Expr,
  vars: &HashMap<String, Variable>,
  local_classes: &HashMap<String, String>,
  local_array_elem_types: &HashMap<String, String>,
  ctx: &Ctx,
) -> Result<bool, String> {
  let Expr::Ident(_) = array else {
    return Err(
      "codegen: array assignment is only supported on a plain local-variable array".to_string(),
    );
  };
  let base_ptr = build_expr(
    builder,
    module,
    array,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let idx_val = build_expr(
    builder,
    module,
    index,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let v = build_expr(
    builder,
    module,
    value,
    vars,
    local_classes,
    local_array_elem_types,
    ctx,
  )?;
  let byte_offset = builder.ins().imul_imm_s(idx_val, ARRAY_ELEM_SIZE);
  let addr = builder.ins().iadd(base_ptr, byte_offset);
  builder.ins().store(MemFlagsData::new(), v, addr, 0);
  Ok(false)
}

/// Emits one statement. `vars`/`local_classes` are threaded flat (no
/// block scoping — matches `emerald-sema`'s equally flat environment, see
/// plan 07's Implementation Notes); `loop_stack`'s top is `break`/`next`'s
/// target. Returns `true` if the statement emitted a block terminator
/// (`return`/the loop-jump for `break`/`next`) — callers must not emit
/// further instructions into the current block afterward.
#[allow(clippy::too_many_arguments)]
fn build_stmt(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  stmt: &Stmt,
  vars: &mut HashMap<String, Variable>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, String>,
  loop_stack: &mut Vec<LoopTargets>,
  ctx: &Ctx,
) -> Result<bool, String> {
  match stmt {
    Stmt::Let { name, ty, value } => {
      let v = build_expr(
        builder,
        module,
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
        local_array_elem_types.insert(name.clone(), elem_name.to_string());
      }
      if let Some(existing) = vars.get(name) {
        builder.def_var(*existing, v);
      } else {
        let var = builder.declare_var(cranelift_type(ty));
        builder.def_var(var, v);
        vars.insert(name.clone(), var);
      }
      Ok(false)
    }
    Stmt::SetField { name, value } => {
      let (self_var, fields) = ctx
        .self_ctx
        .ok_or_else(|| format!("codegen: `@{name} = ...` used outside of a method body"))?;
      let field = *fields
        .get(name)
        .ok_or_else(|| format!("codegen: undefined field `@{name}`"))?;
      let v = build_expr(
        builder,
        module,
        value,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let self_ptr = builder.use_var(self_var);
      builder
        .ins()
        .store(MemFlagsData::new(), v, self_ptr, field.offset);
      Ok(false)
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => build_set_index(
      builder,
      module,
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
        builder,
        module,
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
        builder,
        module,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      Ok(false)
    }
    Stmt::Return(Some(e)) => {
      let v = build_expr(
        builder,
        module,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder.ins().return_(&[v]);
      Ok(true)
    }
    Stmt::Return(None) => {
      builder.ins().return_(&[]);
      Ok(true)
    }
    Stmt::Break => {
      let target = loop_stack
        .last()
        .ok_or("codegen: `break` outside of a loop")?;
      builder.ins().jump(target.exit, &[]);
      Ok(true)
    }
    Stmt::Next => {
      let target = loop_stack
        .last()
        .ok_or("codegen: `next` outside of a loop")?;
      builder.ins().jump(target.header, &[]);
      Ok(true)
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      let cond_val = build_expr(
        builder,
        module,
        cond,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      let then_blk = builder.create_block();
      let merge_blk = builder.create_block();
      let else_target_blk = if else_branch.is_some() {
        builder.create_block()
      } else {
        merge_blk
      };

      builder
        .ins()
        .brif(cond_val, then_blk, &[], else_target_blk, &[]);

      builder.switch_to_block(then_blk);
      builder.seal_block(then_blk);
      let then_terminated = build_block(
        builder,
        module,
        then_branch,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ctx,
      )?;
      if !then_terminated {
        builder.ins().jump(merge_blk, &[]);
      }

      if let Some(else_branch) = else_branch {
        builder.switch_to_block(else_target_blk);
        builder.seal_block(else_target_blk);
        let else_terminated = build_block(
          builder,
          module,
          else_branch,
          vars,
          local_classes,
          local_array_elem_types,
          loop_stack,
          ctx,
        )?;
        if !else_terminated {
          builder.ins().jump(merge_blk, &[]);
        }
      }

      builder.switch_to_block(merge_blk);
      builder.seal_block(merge_blk);
      Ok(false)
    }
    Stmt::While { cond, body } => {
      let header_blk = builder.create_block();
      let body_blk = builder.create_block();
      let exit_blk = builder.create_block();

      builder.ins().jump(header_blk, &[]);

      builder.switch_to_block(header_blk);
      let cond_val = build_expr(
        builder,
        module,
        cond,
        vars,
        local_classes,
        local_array_elem_types,
        ctx,
      )?;
      builder.ins().brif(cond_val, body_blk, &[], exit_blk, &[]);
      // header_blk isn't sealed yet — the loop body's back-edge (below) is
      // a predecessor that doesn't exist until after the body is built.

      builder.switch_to_block(body_blk);
      builder.seal_block(body_blk);
      loop_stack.push(LoopTargets {
        header: header_blk,
        exit: exit_blk,
      });
      let body_terminated = build_block(
        builder,
        module,
        body,
        vars,
        local_classes,
        local_array_elem_types,
        loop_stack,
        ctx,
      )?;
      loop_stack.pop();
      if !body_terminated {
        builder.ins().jump(header_blk, &[]);
      }
      builder.seal_block(header_blk);

      builder.switch_to_block(exit_blk);
      builder.seal_block(exit_blk);
      Ok(false)
    }
  }
}

/// Emits a straight-line sequence of statements. Stops early (without
/// erroring) after any statement that terminates the current block —
/// anything syntactically after `return`/`break`/`next` in the same list
/// is unreachable and must not be emitted into an already-terminated
/// Cranelift block.
#[allow(clippy::too_many_arguments)]
fn build_block(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  stmts: &[Stmt],
  vars: &mut HashMap<String, Variable>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, String>,
  loop_stack: &mut Vec<LoopTargets>,
  ctx: &Ctx,
) -> Result<bool, String> {
  for stmt in stmts {
    let terminated = build_stmt(
      builder,
      module,
      stmt,
      vars,
      local_classes,
      local_array_elem_types,
      loop_stack,
      ctx,
    )?;
    if terminated {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Builds a function/method's `Vec<Stmt>` body, honoring Ruby-style
/// implicit return: if the body doesn't already end in an explicit
/// terminator (`return`/`break`/`next`), the last statement — if a bare
/// `Stmt::Expr` — has its value returned, matching `emerald-sema`'s
/// implicit-return check.
#[allow(clippy::too_many_arguments)]
fn build_function_body(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  body: &[Stmt],
  vars: &mut HashMap<String, Variable>,
  local_classes: &mut HashMap<String, String>,
  local_array_elem_types: &mut HashMap<String, String>,
  gen_ctx: &Ctx,
) -> Result<(), String> {
  let mut loop_stack = Vec::new();
  let Some((last, init)) = body.split_last() else {
    builder.ins().return_(&[]);
    return Ok(());
  };

  let terminated = build_block(
    builder,
    module,
    init,
    vars,
    local_classes,
    local_array_elem_types,
    &mut loop_stack,
    gen_ctx,
  )?;
  if terminated {
    return Ok(());
  }

  match last {
    Stmt::Expr(e) => {
      let v = build_expr(
        builder,
        module,
        e,
        vars,
        local_classes,
        local_array_elem_types,
        gen_ctx,
      )?;
      builder.ins().return_(&[v]);
    }
    other => {
      let terminated = build_stmt(
        builder,
        module,
        other,
        vars,
        local_classes,
        local_array_elem_types,
        &mut loop_stack,
        gen_ctx,
      )?;
      // A Void-returning body whose last statement isn't a value-
      // producing Stmt::Expr (e.g. `initialize`'s trailing `@y = y`)
      // needs an explicit empty `return` — nothing else would ever
      // terminate the block, and Cranelift requires every block to end
      // in a terminator.
      if !terminated && builder.func.signature.returns.is_empty() {
        builder.ins().return_(&[]);
      }
    }
  }
  Ok(())
}

fn define_user_function(
  module: &mut ObjectModule,
  ctx: &mut cranelift::codegen::Context,
  func_ctx: &mut FunctionBuilderContext,
  f: &AstFunction,
  id: FuncId,
  gen_ctx: &Ctx,
) -> Result<(), String> {
  ctx.func.signature.params.clear();
  ctx.func.signature.returns.clear();
  for p in &f.params {
    ctx
      .func
      .signature
      .params
      .push(AbiParam::new(cranelift_type(&p.ty)));
  }
  push_return_type(&mut ctx.func.signature.returns, &f.return_type);

  let frontend_config = module.target_config();
  {
    let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);

    let mut vars: HashMap<String, Variable> = HashMap::new();
    let mut local_classes: HashMap<String, String> = HashMap::new();
    let mut local_array_elem_types: HashMap<String, String> = HashMap::new();
    for (i, p) in f.params.iter().enumerate() {
      let param_val = builder.block_params(block)[i];
      let var = builder.declare_var(cranelift_type(&p.ty));
      builder.def_var(var, param_val);
      vars.insert(p.name.clone(), var);
      if gen_ctx.classes.contains_key(p.ty.as_str()) {
        local_classes.insert(p.name.clone(), p.ty.clone());
      }
      if let Some(elem_name) = p
        .ty
        .strip_prefix("Array[")
        .and_then(|s| s.strip_suffix(']'))
      {
        local_array_elem_types.insert(p.name.clone(), elem_name.to_string());
      }
    }

    build_function_body(
      &mut builder,
      module,
      &f.body,
      &mut vars,
      &mut local_classes,
      &mut local_array_elem_types,
      gen_ctx,
    )?;
    builder.finalize(frontend_config);
  }

  module.define_function(id, ctx).map_err(|e| e.to_string())?;
  module.clear_context(ctx);
  Ok(())
}

/// Compiles one class method as `{ClassName}_{methodName}`, taking an
/// implicit leading `self: i64` pointer parameter ahead of the method's
/// own declared parameters. `self_fields` (the class's field layout) is
/// threaded through `gen_ctx.self_ctx` for the method body's `@field`
/// reads/writes.
fn define_method(
  module: &mut ObjectModule,
  ctx: &mut cranelift::codegen::Context,
  func_ctx: &mut FunctionBuilderContext,
  m: &AstFunction,
  id: FuncId,
  self_fields: &HashMap<String, FieldInfo>,
  gen_ctx: &Ctx,
) -> Result<(), String> {
  ctx.func.signature.params.clear();
  ctx.func.signature.returns.clear();
  ctx.func.signature.params.push(AbiParam::new(types::I64)); // self
  for p in &m.params {
    ctx
      .func
      .signature
      .params
      .push(AbiParam::new(cranelift_type(&p.ty)));
  }
  push_return_type(&mut ctx.func.signature.returns, &m.return_type);

  let frontend_config = module.target_config();
  {
    let mut builder = FunctionBuilder::new(&mut ctx.func, func_ctx);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);

    let self_var = builder.declare_var(types::I64);
    builder.def_var(self_var, builder.block_params(block)[0]);

    let mut vars: HashMap<String, Variable> = HashMap::new();
    let mut local_classes: HashMap<String, String> = HashMap::new();
    let mut local_array_elem_types: HashMap<String, String> = HashMap::new();
    for (i, p) in m.params.iter().enumerate() {
      let param_val = builder.block_params(block)[i + 1];
      let var = builder.declare_var(cranelift_type(&p.ty));
      builder.def_var(var, param_val);
      vars.insert(p.name.clone(), var);
      if gen_ctx.classes.contains_key(p.ty.as_str()) {
        local_classes.insert(p.name.clone(), p.ty.clone());
      }
      if let Some(elem_name) = p
        .ty
        .strip_prefix("Array[")
        .and_then(|s| s.strip_suffix(']'))
      {
        local_array_elem_types.insert(p.name.clone(), elem_name.to_string());
      }
    }

    let method_ctx = Ctx {
      self_ctx: Some((self_var, self_fields)),
      ..*gen_ctx
    };
    build_function_body(
      &mut builder,
      module,
      &m.body,
      &mut vars,
      &mut local_classes,
      &mut local_array_elem_types,
      &method_ctx,
    )?;
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
  gen_ctx: &Ctx,
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

    let mut vars: HashMap<String, Variable> = HashMap::new();
    let mut local_classes: HashMap<String, String> = HashMap::new();
    let mut local_array_elem_types: HashMap<String, String> = HashMap::new();
    let mut loop_stack = Vec::new();

    let top_stmts: Vec<Stmt> = program
      .items
      .iter()
      .filter_map(|it| {
        if let Item::Stmt(s) = it {
          Some(s.clone())
        } else {
          None
        }
      })
      .collect();

    let terminated = build_block(
      &mut builder,
      module,
      &top_stmts,
      &mut vars,
      &mut local_classes,
      &mut local_array_elem_types,
      &mut loop_stack,
      gen_ctx,
    )?;

    if !terminated {
      let zero = builder.ins().iconst(types::I32, 0);
      builder.ins().return_(&[zero]);
    }
    builder.finalize(frontend_config);
  }

  module
    .define_function(main_id, ctx)
    .map_err(|e| e.to_string())?;
  module.clear_context(ctx);
  Ok(())
}

/// Compiles a type-checked `Program` to a native object file at `out_path`.
/// One exported function per `Item::Function`, `{Class}_{method}` per
/// class method (plan 08), plus a `main` (`extern "C" fn() -> i32`) that
/// evaluates the top-level statements (including the `puts` call
/// statement) via the imported `emerald_print_i64`/`emerald_print_f64`/
/// `emerald_alloc` runtime symbols (see `runtime/emerald_runtime.c`) and
/// returns 0.
pub fn compile_to_object(program: &Program, out_path: &Path) -> Result<(), String> {
  let isa = host_isa()?;
  let obj_builder = ObjectBuilder::new(
    isa,
    "emerald_module",
    cranelift_module::default_libcall_names(),
  )
  .map_err(|e| e.to_string())?;
  let mut module = ObjectModule::new(obj_builder);

  let mut print_i64_sig = module.make_signature();
  print_i64_sig.params.push(AbiParam::new(types::I64));
  let print_i64_func_id = module
    .declare_function("emerald_print_i64", Linkage::Import, &print_i64_sig)
    .map_err(|e| e.to_string())?;

  let mut print_f64_sig = module.make_signature();
  print_f64_sig.params.push(AbiParam::new(types::F64));
  let print_f64_func_id = module
    .declare_function("emerald_print_f64", Linkage::Import, &print_f64_sig)
    .map_err(|e| e.to_string())?;

  let mut alloc_sig = module.make_signature();
  alloc_sig.params.push(AbiParam::new(types::I64));
  alloc_sig.returns.push(AbiParam::new(types::I64));
  let alloc_func_id = module
    .declare_function("emerald_alloc", Linkage::Import, &alloc_sig)
    .map_err(|e| e.to_string())?;

  // Class layouts (field offsets/types) — computed once, independent of
  // declaration order between classes (plan 08 doesn't support classes
  // referencing each other's fields yet, so no ordering dependency here).
  let mut classes: HashMap<String, ClassLayout> = HashMap::new();
  for item in &program.items {
    if let Item::Class(c) = item {
      classes.insert(c.name.clone(), build_class_layout(c));
    }
  }

  let mut user_func_ids: HashMap<String, FuncId> = HashMap::new();
  for item in &program.items {
    if let Item::Function(f) = item {
      let mut sig = module.make_signature();
      for p in &f.params {
        sig.params.push(AbiParam::new(cranelift_type(&p.ty)));
      }
      push_return_type(&mut sig.returns, &f.return_type);
      let id = module
        .declare_function(&f.name, Linkage::Export, &sig)
        .map_err(|e| e.to_string())?;
      user_func_ids.insert(f.name.clone(), id);
    }
    if let Item::Class(c) = item {
      for m in &c.methods {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // self
        for p in &m.params {
          sig.params.push(AbiParam::new(cranelift_type(&p.ty)));
        }
        push_return_type(&mut sig.returns, &m.return_type);
        let mangled = format!("{}_{}", c.name, m.name);
        let id = module
          .declare_function(&mangled, Linkage::Export, &sig)
          .map_err(|e| e.to_string())?;
        user_func_ids.insert(mangled, id);
      }
    }
  }

  let gen_ctx = Ctx {
    user_func_ids: &user_func_ids,
    classes: &classes,
    print_i64_func_id: Some(print_i64_func_id),
    print_f64_func_id: Some(print_f64_func_id),
    alloc_func_id,
    self_ctx: None,
  };

  let mut ctx = module.make_context();
  let mut func_ctx = FunctionBuilderContext::new();

  for item in &program.items {
    if let Item::Function(f) = item {
      let id = *user_func_ids.get(&f.name).unwrap();
      define_user_function(&mut module, &mut ctx, &mut func_ctx, f, id, &gen_ctx)?;
    }
    if let Item::Class(c) = item {
      let layout = classes.get(&c.name).unwrap();
      for m in &c.methods {
        let mangled = format!("{}_{}", c.name, m.name);
        let id = *user_func_ids.get(&mangled).unwrap();
        define_method(
          &mut module,
          &mut ctx,
          &mut func_ctx,
          m,
          id,
          &layout.fields,
          &gen_ctx,
        )?;
      }
    }
  }

  define_main(&mut module, &mut ctx, &mut func_ctx, program, &gen_ctx)?;

  let object = module.finish();
  let bytes = object.emit().map_err(|e| e.to_string())?;
  std::fs::write(out_path, bytes).map_err(|e| e.to_string())?;
  Ok(())
}

#[cfg(test)]
mod aot_tests {
  use super::*;
  use std::process::Command;

  fn runtime_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime/emerald_runtime.c")
  }

  /// Compiles `src` to an object file, links it with the runtime shim via
  /// `cc`, runs the resulting binary, and returns its captured stdout.
  /// Real, executed proof — not a simulation (matches plan 06's practice).
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
    // ELF magic number, since this environment targets Linux.
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
    // Without `break` this prints 6 (1+2+3); breaking when i==2 means only
    // the first iteration's addition happens, so it prints 1 — proving
    // `break` actually skips the remaining iterations, not just that the
    // loop runs at all.
    let src = "total: Int64 = 0\ni: Int64 = 1\nwhile i < 4\n  if i == 2\n    break\n  end\n  total: Int64 = total + i\n  i: Int64 = i + 1\nend\nputs total\n";
    assert_eq!(compile_link_run(src), "1\n");
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def sum -> Float64\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn inception_point_example_linked_and_run_prints_5() {
    // 2.0 + 3.0 = 5.0; emerald_print_f64 uses "%g", which renders a whole
    // number without a trailing ".0" — pinned from the real observed
    // output, not guessed (plan 08's leaf-codegen-class AC1).
    assert_eq!(compile_link_run(POINT_EXAMPLE), "5\n");
  }

  const ARRAY_EXAMPLE: &str = "arr: Array[Int64] = [10, 20, 30]\nsum: Int64 = 0\ni: Int64 = 0\nwhile i < 3\n  sum: Int64 = sum + arr[i]\n  i: Int64 = i + 1\nend\narr[1] = 99\nputs sum\nputs arr[1]\n";

  #[test]
  fn plan_09_collections_example_linked_and_run() {
    // Plan 09's own worked example (inception has no literal one for
    // collections): allocate, sum via a `while`-loop indexed read, mutate
    // via `arr[1] = 99`, read back — real executed proof, not simulated
    // (plan 09 AC1).
    assert_eq!(compile_link_run(ARRAY_EXAMPLE), "60\n99\n");
  }

  #[test]
  fn array_literal_index_read_and_write_linked_and_run() {
    // Sums the literal (10+20+30=60), then overwrites index 1 and reads
    // it back (99) — proves allocation, read-indexing, and write-indexing
    // all touch the same underlying buffer, not simulated (plan 09 AC3).
    let src =
      "arr: Array[Int64] = [10, 20, 30]\nputs arr[0] + arr[1] + arr[2]\narr[1] = 99\nputs arr[1]\n";
    assert_eq!(compile_link_run(src), "60\n99\n");
  }

  #[test]
  fn array_of_float64_linked_and_run() {
    // A second element type proves indexing picks its load width from
    // `local_array_elem_types`, not a single hard-coded Cranelift type.
    let src = "arr: Array[Float64] = [1.5, 2.5]\nputs arr[0] + arr[1]\n";
    assert_eq!(compile_link_run(src), "4\n");
  }

  #[test]
  fn unsupported_top_level_shape_errors_not_panics() {
    // A two-argument `puts` call — no such surface syntax exists yet (the
    // grammar's `puts` production takes exactly one Expr), but codegen
    // must not panic if it ever receives this AST shape. Constructed
    // directly since the grammar can't produce it.
    let program = Program {
      items: vec![Item::Stmt(Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Int(1), Expr::Int(2)],
      )))],
    };
    let dir = std::env::temp_dir();
    let out_path = dir.join(format!("emerald_codegen_test_bad_{}.o", std::process::id()));
    let result = compile_to_object(&program, &out_path);
    assert!(result.is_err());
    std::fs::remove_file(&out_path).ok();
  }
}
