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

use cranelift::codegen::ir::condcodes::IntCC;
use cranelift_module::FuncId;
use cranelift_object::{ObjectBuilder, ObjectModule};
use emerald_parser::{CompareOp, Expr, Function as AstFunction, Item, Program, Stmt};
use std::collections::HashMap;
use std::path::Path;

/// The header/exit blocks of the innermost enclosing loop, for `break`
/// (jump to `exit`) / `next` (jump back to `header`) to target.
struct LoopTargets {
  header: Block,
  exit: Block,
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

fn build_expr(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  expr: &Expr,
  vars: &HashMap<String, Variable>,
  user_func_ids: &HashMap<String, FuncId>,
) -> Result<Value, String> {
  match expr {
    Expr::Ident(name) => {
      let var = vars
        .get(name)
        .ok_or_else(|| format!("codegen: undefined variable `{name}`"))?;
      Ok(builder.use_var(*var))
    }
    Expr::Int(n) => Ok(builder.ins().iconst(types::I64, *n)),
    Expr::Add(lhs, rhs) => {
      let l = build_expr(builder, module, lhs, vars, user_func_ids)?;
      let r = build_expr(builder, module, rhs, vars, user_func_ids)?;
      Ok(builder.ins().iadd(l, r))
    }
    Expr::Compare(lhs, op, rhs) => {
      let l = build_expr(builder, module, lhs, vars, user_func_ids)?;
      let r = build_expr(builder, module, rhs, vars, user_func_ids)?;
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
      let func_id = *user_func_ids.get(name).ok_or_else(|| {
        format!("codegen: unsupported call to `{name}` (not a compiled user function)")
      })?;
      let func_ref = module.declare_func_in_func(func_id, builder.func);
      let mut arg_vals = Vec::with_capacity(args.len());
      for a in args {
        arg_vals.push(build_expr(builder, module, a, vars, user_func_ids)?);
      }
      let call = builder.ins().call(func_ref, &arg_vals);
      Ok(builder.inst_results(call)[0])
    }
  }
}

/// Emits `puts <inner>` as a call to the imported `emerald_print_i64`
/// runtime symbol (see `runtime/emerald_runtime.c`).
fn build_puts(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  arg: &Expr,
  vars: &HashMap<String, Variable>,
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: Option<FuncId>,
) -> Result<(), String> {
  let print_func_id = print_func_id.ok_or(
    "codegen: `puts` is only supported at the program's top level, not inside a function body",
  )?;
  let val = build_expr(builder, module, arg, vars, user_func_ids)?;
  let print_ref = module.declare_func_in_func(print_func_id, builder.func);
  builder.ins().call(print_ref, &[val]);
  Ok(())
}

/// Emits one statement. `vars` is threaded flat (no block scoping —
/// matches `emerald-sema`'s equally flat environment, see plan
/// 07's Implementation Notes); `loop_stack`'s top is `break`/`next`'s
/// target. Returns `true` if the statement emitted a block terminator
/// (`return`/the loop-jump for `break`/`next`) — callers must not emit
/// further instructions into the current block afterward.
#[allow(clippy::too_many_arguments)]
fn build_stmt(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  stmt: &Stmt,
  vars: &mut HashMap<String, Variable>,
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: Option<FuncId>,
  loop_stack: &mut Vec<LoopTargets>,
) -> Result<bool, String> {
  match stmt {
    Stmt::Let { name, value, .. } => {
      let v = build_expr(builder, module, value, vars, user_func_ids)?;
      if let Some(existing) = vars.get(name) {
        builder.def_var(*existing, v);
      } else {
        let var = builder.declare_var(types::I64);
        builder.def_var(var, v);
        vars.insert(name.clone(), var);
      }
      Ok(false)
    }
    Stmt::Expr(Expr::Call(name, args)) if name == "puts" && args.len() == 1 => {
      build_puts(
        builder,
        module,
        &args[0],
        vars,
        user_func_ids,
        print_func_id,
      )?;
      Ok(false)
    }
    Stmt::Expr(e) => {
      build_expr(builder, module, e, vars, user_func_ids)?;
      Ok(false)
    }
    Stmt::Return(Some(e)) => {
      let v = build_expr(builder, module, e, vars, user_func_ids)?;
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
      let cond_val = build_expr(builder, module, cond, vars, user_func_ids)?;
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
        user_func_ids,
        print_func_id,
        loop_stack,
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
          user_func_ids,
          print_func_id,
          loop_stack,
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
      let cond_val = build_expr(builder, module, cond, vars, user_func_ids)?;
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
        user_func_ids,
        print_func_id,
        loop_stack,
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
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: Option<FuncId>,
  loop_stack: &mut Vec<LoopTargets>,
) -> Result<bool, String> {
  for stmt in stmts {
    let terminated = build_stmt(
      builder,
      module,
      stmt,
      vars,
      user_func_ids,
      print_func_id,
      loop_stack,
    )?;
    if terminated {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Builds a function's `Vec<Stmt>` body, honoring Ruby-style implicit
/// return: if the body doesn't already end in an explicit terminator
/// (`return`/`break`/`next`), the last statement — if a bare `Stmt::Expr`
/// — has its value returned, matching `emerald-sema`'s implicit-return
/// check in `check_function_body`.
#[allow(clippy::too_many_arguments)]
fn build_function_body(
  builder: &mut FunctionBuilder,
  module: &mut ObjectModule,
  body: &[Stmt],
  vars: &mut HashMap<String, Variable>,
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: Option<FuncId>,
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
    user_func_ids,
    print_func_id,
    &mut loop_stack,
  )?;
  if terminated {
    return Ok(());
  }

  match last {
    Stmt::Expr(e) => {
      let v = build_expr(builder, module, e, vars, user_func_ids)?;
      builder.ins().return_(&[v]);
    }
    other => {
      build_stmt(
        builder,
        module,
        other,
        vars,
        user_func_ids,
        print_func_id,
        &mut loop_stack,
      )?;
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

    let mut vars: HashMap<String, Variable> = HashMap::new();
    for (i, p) in f.params.iter().enumerate() {
      let param_val = builder.block_params(block)[i];
      let var = builder.declare_var(types::I64);
      builder.def_var(var, param_val);
      vars.insert(p.name.clone(), var);
    }

    build_function_body(
      &mut builder,
      module,
      &f.body,
      &mut vars,
      user_func_ids,
      // `puts` is only wired up for the top-level `main` body (matching
      // examples/hello.em's shape); a `puts` call inside a user function
      // returns a descriptive `Err` via `build_puts`, not a panic.
      None,
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
  user_func_ids: &HashMap<String, FuncId>,
  print_func_id: Option<FuncId>,
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
      user_func_ids,
      print_func_id,
      &mut loop_stack,
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
    Some(print_func_id),
  )?;

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
