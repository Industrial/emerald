//! Emerald's parser, built on LALRPOP — chosen in `02 toolchain-prototype`
//! over Chumsky (see `spec/COMPILER.md` for the decision record). Parses a
//! full `Program` of `Item`s, each a function definition or a top-level
//! `Stmt` (plan `07` extended the milestone-1-only single-expr shape to a
//! real statement/block language); grows as later milestones extend
//! `grammar.lalrpop`.

pub mod ast;

// Bugfix (first-ever `git push` this session — pre-push's `check-docs`
// gate had never actually run before, no remote existed): `clippy::all`
// doesn't cover `clippy::missing_docs_in_private_items` — it's a
// `clippy::restriction` lint, deliberately excluded from `all` since
// restriction lints are opt-in-only — so `moon.yml`'s `check-docs` task
// (`-W clippy::missing_docs_in_private_items` promoted to deny by its
// own `-D warnings`) flagged every one of LALRPOP's thousands of
// generated, unavoidably-undocumented items in `grammar.rs`. Generated
// code, not something to hand-document.
#[allow(clippy::all, clippy::missing_docs_in_private_items, missing_docs)]
mod grammar {
  lalrpop_util::lalrpop_mod!(pub grammar, "/grammar.rs");
}

// Plan 36: string interpolation's `#{...}`-splitting logic — needs to
// call back into `grammar`'s second, independent `pub Expr` entry
// point, a real dependency `ast.rs` (a pure-data module, no dependency
// on the generated parser) deliberately doesn't have.
mod interpolate;

pub use ast::{
  expand_derives, ActorDef, CaseArm, CasePattern, ClassDef, CompareOp, Contract, EnumDef,
  EnumVariant, Expr, ExternBlock, ExternFn, Function, InterfaceDef, Item, ModuleDef, Param,
  Program, RescueClause, Spanned, Stmt, StringPart, TypeParam,
};

/// A parse failure, carrying enough of `lalrpop_util::ParseError`'s own
/// byte-offset location info (plan `13`) to render a real source
/// snippet + caret via `miette` — not just a bare message string. This
/// is the parser's ONLY diagnostic with a real span; sema/codegen
/// diagnostics stay text-only (see plan `13`'s Decision log — spans
/// there would mean threading a span through every `Expr`/`Stmt`
/// variant, out of this plan's scope).
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{message}")]
pub struct ParseError {
  /// Human-readable description of what went wrong.
  message: String,
  /// The full source text, for `miette`'s snippet rendering.
  #[source_code]
  src: miette::NamedSource<String>,
  /// Byte-offset span of the failure within `src`.
  #[label("here")]
  span: miette::SourceSpan,
}

/// Extracts a byte-offset span from whichever `lalrpop_util::ParseError`
/// variant matched. `User` (this grammar's one fallible `=>?` action,
/// the assignment-target check) carries no location at all — see plan
/// 13's Decision log for that one disclosed gap; every other variant
/// already tracks a real offset LALRPOP computed during parsing.
fn to_parse_error<T: std::fmt::Display>(
  e: lalrpop_util::ParseError<usize, T, String>,
  name: &str,
  src: &str,
) -> ParseError {
  let message = e.to_string();
  let (start, end) = match &e {
    lalrpop_util::ParseError::InvalidToken { location } => (*location, *location),
    lalrpop_util::ParseError::UnrecognizedEof { location, .. } => (*location, *location),
    lalrpop_util::ParseError::UnrecognizedToken { token, .. } => (token.0, token.2),
    lalrpop_util::ParseError::ExtraToken { token } => (token.0, token.2),
    lalrpop_util::ParseError::User { .. } => (0, 0),
  };
  let len = end.saturating_sub(start).max(1);
  ParseError {
    message,
    src: miette::NamedSource::new(name, src.to_string()),
    span: (start, len).into(),
  }
}

/// Plan 26's Decision log: a fixed, disclosed robustness bound —
/// panic-mode recovery on a badly malformed file can cascade into a
/// flood of low-quality secondary errors, so this cap exists purely to
/// bound worst-case output, not because any real fixture needs it.
const MAX_RECOVERED_ERRORS: usize = 50;

/// Same as [`parse`], but names the source (shown in the rendered
/// diagnostic's snippet header) as `name` instead of the generic
/// `"<source>"` placeholder — `emerald-cli` uses this with the real file
/// path it read `src` from.
///
/// Returns every top-level `Item` boundary's syntax error in one pass
/// (plan 26), not just the first — LALRPOP's own `!` error-recovery
/// marker on the `Item` production (`grammar.lalrpop`) resynchronizes
/// at the next top-level construct instead of aborting the whole parse.
/// `Ok(program)` is returned only when *zero* errors were recovered
/// (`program.items` then contains no `Item::Error` either, by
/// construction) — the moment one or more exist, this returns
/// `Err(Vec<ParseError>)` instead, exactly the same all-or-nothing
/// shape `parse_named` had before this plan, just now potentially
/// carrying more than one error.
/// Plan 47's Decision log: `assert(cond)`/`assert_eq(expected, actual)`
/// capture a raw `@L` byte offset at parse time (as a placeholder
/// `Expr::Int`, the grammar's own action) — this targeted post-parse
/// pass is the only place that offset is ever converted into the real
/// `Expr::StringLit("{name}:{line}")` a test failure's `.message`
/// reads. Deliberately not plan 22's general `Spanned<T>` overhaul
/// (out of scope, would touch every `Expr`/`Stmt` variant); this walks
/// only to find `Expr::Call("assert"|"assert_eq", _)` shapes.
fn line_at(source: &str, offset: usize) -> usize {
  1 + source.as_bytes()[..offset.min(source.len())]
    .iter()
    .filter(|&&b| b == b'\n')
    .count()
}

/// Plan 62's Decision log: mirrors `rewrite_assert_locations`'s own
/// technique exactly — a small, targeted post-parse pass over the
/// freshly-built `Program`, not a generic `Spanned` walk. Slices
/// `source[expr.span.0..expr.span.1]` into `Contract.text` and computes
/// `Contract.line` the same way plan 47's own `loc` does (`line_at`,
/// counting `\n` bytes). Scoped to top-level `Item::Function` only — the
/// one production `requires`/`ensures` are grammatically reachable from
/// (see `grammar.lalrpop`'s own `FuncDef`-only `ContractClause*`
/// placement) — so no other `Item`/`Stmt` variant needs visiting here at
/// all, unlike `rewrite_assert_locations`'s own whole-program walk.
fn fill_contract_text(items: &mut [Item], source: &str) {
  for item in items {
    if let Item::Function(f) = item {
      for c in f.requires.iter_mut().chain(f.ensures.iter_mut()) {
        let (start, end) = c.expr.span;
        c.text = source[start..end].to_string();
        c.line = line_at(source, start);
      }
    }
  }
}

/// Whole-program walk rewriting every `assert`/`assert_eq` call's
/// synthesized location arguments (file `name`, line) in place, by
/// byte-offset lookup into `source` — the counterpart to
/// `fill_contract_text`'s narrower, `FuncDef`-only walk above.
fn rewrite_assert_locations(items: &mut [Item], name: &str, source: &str) {
  for item in items {
    match item {
      Item::Function(f) => rewrite_stmts(&mut f.body, name, source),
      Item::Class(c) => {
        for m in &mut c.methods {
          rewrite_stmts(&mut m.body, name, source);
        }
      }
      Item::Module(m) => {
        for f in &mut m.methods {
          rewrite_stmts(&mut f.body, name, source);
        }
      }
      Item::Actor(a) => {
        for m in &mut a.methods {
          rewrite_stmts(&mut m.body, name, source);
        }
      }
      Item::Interface(_) | Item::Require(_) | Item::Error | Item::Enum(_) | Item::Extern(_) => {}
      Item::Stmt(s) => rewrite_stmt(s, name, source),
      Item::Test { body, .. } => rewrite_stmts(body, name, source),
    }
  }
}

/// `rewrite_assert_locations`'s per-statement-list recursion step.
fn rewrite_stmts(stmts: &mut [Spanned<Stmt>], name: &str, source: &str) {
  for s in stmts {
    rewrite_stmt(s, name, source);
  }
}

/// `rewrite_assert_locations`'s per-statement recursion step — walks
/// into every `Stmt` variant's own sub-expressions/sub-blocks.
fn rewrite_stmt(stmt: &mut Spanned<Stmt>, name: &str, source: &str) {
  match &mut stmt.node {
    Stmt::Let { value, .. }
    | Stmt::SetField { value, .. }
    | Stmt::Assign { value, .. }
    | Stmt::Raise(value)
    | Stmt::OrAssign { default: value, .. }
    | Stmt::AndAssign { value, .. }
    | Stmt::Expr(value) => rewrite_expr(value, name, source),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      rewrite_expr(array, name, source);
      rewrite_expr(index, name, source);
      rewrite_expr(value, name, source);
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        rewrite_expr(v, name, source);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      rewrite_expr(cond, name, source);
      rewrite_stmts(then_branch, name, source);
      if let Some(b) = else_branch {
        rewrite_stmts(b, name, source);
      }
    }
    Stmt::While { cond, body } => {
      rewrite_expr(cond, name, source);
      rewrite_stmts(body, name, source);
    }
    Stmt::Return(Some(e)) => rewrite_expr(e, name, source),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      rewrite_stmts(body, name, source);
      for r in rescues {
        rewrite_stmts(&mut r.body, name, source);
      }
      if let Some(e) = ensure {
        rewrite_stmts(e, name, source);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      rewrite_expr(scrutinee, name, source);
      for (pattern, body) in arms {
        if let CasePattern::Values(values) = pattern {
          for v in values {
            rewrite_expr(v, name, source);
          }
        }
        rewrite_stmts(body, name, source);
      }
      if let Some(b) = else_body {
        rewrite_stmts(b, name, source);
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        rewrite_expr(e, name, source);
      }
      rewrite_stmts(body, name, source);
    }
    Stmt::Yield(args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      rewrite_expr(start, name, source);
      rewrite_expr(end, name, source);
      rewrite_stmts(body, name, source);
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      rewrite_expr(scrutinee, name, source);
      rewrite_stmts(ok_body, name, source);
      rewrite_stmts(err_body, name, source);
    }
  }
}

/// `rewrite_assert_locations`'s leaf recursion step — walks into every
/// `Expr` variant's own sub-expressions, and is the one that actually
/// rewrites an `assert`/`assert_eq` call's synthesized offset argument
/// into a `"name:line"` string literal (see the `Expr::Call` arm).
fn rewrite_expr(expr: &mut Spanned<Expr>, name: &str, source: &str) {
  match &mut expr.node {
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::Nil
    | Expr::InstanceVar(_) => {}
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          rewrite_expr(e, name, source);
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
      rewrite_expr(a, name, source);
      rewrite_expr(b, name, source);
    }
    Expr::Neg(a) | Expr::Not(a) | Expr::BitNot(a) | Expr::ArrayNew(a) | Expr::Comptime(a) => {
      rewrite_expr(a, name, source)
    }
    Expr::Compare(a, _, b) => {
      rewrite_expr(a, name, source);
      rewrite_expr(b, name, source);
    }
    Expr::Call(fn_name, args) => {
      for a in args.iter_mut() {
        rewrite_expr(a, name, source);
      }
      let expected_arity = match fn_name.as_str() {
        "assert" => Some(2),
        "assert_eq" => Some(3),
        _ => None,
      };
      if expected_arity == Some(args.len()) {
        let offset = match args.last().map(|s| &s.node) {
          Some(Expr::Int(offset)) => Some(*offset),
          _ => None,
        };
        if let Some(offset) = offset {
          let line = line_at(source, offset as usize);
          args.last_mut().expect("checked non-empty above").node =
            Expr::StringLit(format!("{name}:{line}"));
        }
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, e) in kwargs {
        rewrite_expr(e, name, source);
      }
    }
    Expr::New(_, args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      rewrite_expr(recv, name, source);
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Expr::ArrayLit(elems) => {
      for e in elems {
        rewrite_expr(e, name, source);
      }
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        rewrite_expr(k, name, source);
        rewrite_expr(v, name, source);
      }
    }
    Expr::Lambda { body, .. } => rewrite_stmts(body, name, source),
    Expr::TupleLit(elems) => {
      for e in elems {
        rewrite_expr(e, name, source);
      }
    }
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => rewrite_expr(e, name, source),
    Expr::Spawn(_, args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Expr::Supervise(body) => rewrite_stmts(body, name, source),
    Expr::Remote { addr, name: n, .. } => {
      rewrite_expr(addr, name, source);
      rewrite_expr(n, name, source);
    }
    Expr::Locate { key, args, .. } => {
      rewrite_expr(key, name, source);
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
  }
}

/// Plan 70 (enumerable stdlib completion): the fixed set of `Array[T]`/
/// `Hash[K,V]` intrinsic methods that ever take a trailing block-literal
/// argument (`grammar.lalrpop`'s new block-attached-call productions,
/// added by this same plan). Kept as its own list here rather than
/// reused from `emerald-sema` (which this crate can't depend on without
/// inverting the workspace's dependency graph) purely to scope
/// `hoist_enumerable_blocks`'s rewrite to call sites that can actually
/// use it — a block attached to any OTHER method still parses (any
/// `.method { ... }` shape does, after this plan's grammar change) but
/// is left completely alone here, falling through to `emerald-sema`'s
/// own pre-existing "expected a Proc" diagnostic unchanged.
const ENUMERABLE_BLOCK_METHODS: [&str; 8] = [
  "each",
  "map",
  "select",
  "filter",
  "reduce",
  "inject",
  "each_with_index",
  "count",
];

/// Plan 70's Decision log: bridges `grammar.lalrpop`'s new block-
/// attached-call syntax onto the pre-existing (plan 42), already fully
/// working named-`Proc` call convention every enumerable intrinsic in
/// `emerald-sema`/`emerald-codegen` expects — see `crates/emerald-
/// codegen/src/lib.rs`'s `call_named_proc` doc comment for the full
/// "why a NAMED Proc, not an inline block literal" rationale this
/// rewrite exists to satisfy without changing either crate: it turns
/// `nums.map do |x: Int64| x * 2 end` into the exact AST a hand-written
/// `__fresh: Proc = do |x: Int64| x * 2 end; nums.map(__fresh)` produces
/// (plan 71's own `"Void"` return-type placeholder aside — see that
/// hoisted `Stmt::Let`'s own construction below, which overwrites it
/// with the real, inferred type before codegen ever sees it), before
/// sema ever runs.
///
/// Deliberately narrow, matching `collect_lambda_infos`' own real,
/// pre-existing "top-level `Let` only" restriction on every `Proc`
/// value in this language (block-attached or not): only a *top-level*
/// `Item::Stmt`'s own, direct `Let`/`Expr`/`Assign`/`SetField` value is
/// rewritten here — a block-attached enumerable call nested inside an
/// `if`/`while`/`for`/`begin` body, inside a binary-operator operand,
/// or inside any function/class/actor method body, is left alone
/// entirely (falls through to sema's existing, correctly-worded
/// rejection) rather than risk moving a captured free variable's read
/// point out of its original, possibly-conditionally-executed scope.
///
/// `.map`/`.reduce`/`.inject`'s own block has no declared return type
/// at all (`do |x: Int64| x * 2 end` — and, since plan 71 deleted the
/// lambda literal's own `-> T` return-type slot outright, NO surface
/// syntax in this language can spell a lambda's return type explicitly
/// any more, block-attached or bare) — `infer_block_result_type` below is
/// a small, self-contained, syntax-only type inferencer (no `env`,
/// only the block's own explicitly-typed params and this program's own
/// top-level function return-type table) that computes it, since
/// `emerald_sema::check_program` takes `&Program` and can never write
/// an inferred type back into this immutable AST for codegen to see —
/// the inference has to happen here, before codegen ever looks at the
/// string. When it can't determine a type (an expression shape this
/// small inferencer doesn't cover), this returns a real, disclosed
/// parse error naming the exact call rather than silently guessing.
fn hoist_enumerable_blocks(
  program: &mut Program,
  name: &str,
  source: &str,
) -> Result<(), ParseError> {
  let top_level_fn_returns: std::collections::HashMap<String, String> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Function(f) => Some((f.name.clone(), f.return_type.clone())),
      _ => None,
    })
    .collect();

  let mut counter: usize = 0;
  let old_items = std::mem::take(&mut program.items);
  let mut new_items = Vec::with_capacity(old_items.len());
  for item in old_items {
    let Item::Stmt(mut stmt) = item else {
      new_items.push(item);
      continue;
    };
    let value = match &mut stmt.node {
      Stmt::Let { value, .. }
      | Stmt::Expr(value)
      | Stmt::Assign { value, .. }
      | Stmt::SetField { value, .. } => Some(value),
      _ => None,
    };
    let mut hoisted: Option<Item> = None;
    if let Some(value) = value {
      if let Expr::MethodCall(_, method, args) = &mut value.node {
        let method_name = method.clone();
        let has_trailing_block = matches!(args.last().map(|a| &a.node), Some(Expr::Lambda { .. }));
        if ENUMERABLE_BLOCK_METHODS.contains(&method_name.as_str()) && has_trailing_block {
          let old = args
            .pop()
            .expect("checked Some above via has_trailing_block");
          let Expr::Lambda { params, body, .. } = old.node else {
            unreachable!("has_trailing_block already matched Expr::Lambda")
          };
          let return_type = match method_name.as_str() {
            "select" | "filter" | "count" => "Boolean".to_string(),
            "each" | "each_with_index" => "Void".to_string(),
            "map" | "reduce" | "inject" => {
              infer_block_result_type(&params, &body, &top_level_fn_returns).ok_or_else(|| {
                hoist_error(
                  name,
                  source,
                  old.span,
                  format!(
                    "can't infer this block's result type for `.{method_name}` — as of plan \
                     71, no surface syntax in this language can state a lambda's return type \
                     explicitly any more (the old `->(...) -> ReturnType {{ ... }}` lambda \
                     literal is gone), so there is currently no rewrite that works around this; \
                     this plan's own block-return-type inference only covers literals, a bare \
                     block-parameter reference, same-typed arithmetic/comparison/logical \
                     operators, and a call to an already-declared top-level function — restate \
                     the block using only those shapes"
                  ),
                )
              })?
            }
            _ => unreachable!("ENUMERABLE_BLOCK_METHODS has no other member"),
          };
          let fresh = format!("__enum_blk_{counter}");
          counter += 1;
          args.push(Spanned {
            span: old.span,
            node: Expr::Ident(fresh.clone()),
          });
          hoisted = Some(Item::Stmt(Spanned {
            span: old.span,
            node: Stmt::Let {
              name: fresh,
              ty: "Proc".to_string(),
              value: Spanned {
                span: old.span,
                node: Expr::Lambda {
                  params,
                  return_type,
                  body,
                },
              },
            },
          }));
        }
      }
    }
    if let Some(hoisted) = hoisted {
      new_items.push(hoisted);
    }
    new_items.push(Item::Stmt(stmt));
  }
  program.items = new_items;
  Ok(())
}

/// `hoist_enumerable_blocks`'s own tiny error constructor — same shape
/// as `to_parse_error`'s `ParseError`, just built from a byte-offset
/// span this pass already has in hand rather than one recovered from a
/// `lalrpop_util::ParseError`.
fn hoist_error(name: &str, source: &str, span: (usize, usize), message: String) -> ParseError {
  ParseError {
    message,
    src: miette::NamedSource::new(name, source.to_string()),
    span: span.into(),
  }
}

/// `hoist_enumerable_blocks`'s own return-type inferencer for a
/// parameter-less-return-type block literal — see that function's own
/// doc comment for the full rationale and real, disclosed coverage
/// limit.
fn infer_block_result_type(
  params: &[Param],
  body: &[Spanned<Stmt>],
  top_level_fn_returns: &std::collections::HashMap<String, String>,
) -> Option<String> {
  let param_types: std::collections::HashMap<&str, &str> = params
    .iter()
    .map(|p| (p.name.as_str(), p.ty.as_str()))
    .collect();
  let tail = match body.last().map(|s| &s.node) {
    Some(Stmt::Expr(e)) => e,
    Some(Stmt::Return(Some(e))) => e,
    _ => return None,
  };
  infer_simple_expr_type(&tail.node, &param_types, top_level_fn_returns)
}

/// `infer_block_result_type`'s own recursive expression walk — see
/// `hoist_enumerable_blocks`'s doc comment for exactly which shapes
/// this intentionally-small inferencer covers and why anything else
/// returns `None` rather than a guess.
fn infer_simple_expr_type(
  expr: &Expr,
  param_types: &std::collections::HashMap<&str, &str>,
  top_level_fn_returns: &std::collections::HashMap<String, String>,
) -> Option<String> {
  match expr {
    Expr::Int(_) => Some("Int64".to_string()),
    Expr::Float(_) => Some("Float64".to_string()),
    Expr::StringLit(_) | Expr::Interpolate(_) => Some("String".to_string()),
    Expr::Bool(_) => Some("Boolean".to_string()),
    Expr::Ident(n) => param_types.get(n.as_str()).map(|t| t.to_string()),
    Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) | Expr::Rem(a, b) => {
      let ta = infer_simple_expr_type(&a.node, param_types, top_level_fn_returns)?;
      let tb = infer_simple_expr_type(&b.node, param_types, top_level_fn_returns)?;
      (ta == tb).then_some(ta)
    }
    Expr::Neg(a) => infer_simple_expr_type(&a.node, param_types, top_level_fn_returns),
    Expr::Compare(..) | Expr::And(..) | Expr::Or(..) | Expr::Not(_) => Some("Boolean".to_string()),
    Expr::Call(fn_name, _) => top_level_fn_returns
      .get(fn_name.as_str())
      .map(|t| t.to_string()),
    // `p.key`/`p.value` on a block param explicitly typed `Pair[K, V]`
    // (`Hash[K,V]`'s own `.each`/`.map`/`.reduce`/`.each_with_index`
    // block parameter shape) — the one `MethodCall` receiver+method
    // combination this small inferencer covers, since `Pair[K, V]`'s
    // own compound type-name string (`grammar.lalrpop`'s `TypeName`
    // rule) already carries `K`/`V` in plain text, no real type
    // resolution needed to read them back out.
    Expr::MethodCall(recv, method, call_args)
      if call_args.is_empty() && (method == "key" || method == "value") =>
    {
      let Expr::Ident(recv_name) = &recv.node else {
        return None;
      };
      let (k, v) = parse_pair_type_parts(param_types.get(recv_name.as_str())?)?;
      Some(if method == "key" { k } else { v })
    }
    _ => None,
  }
}

/// Parses a `Pair[K, V]` compound type-name string (`grammar.lalrpop`'s
/// own `"Pair" "[" <k:Ident> "," <v:Ident> "]" => format!("Pair[{k}, \
/// {v}]")` production) back into its `(K, V)` parts — the exact inverse
/// of that `format!`, kept in sync with it deliberately (both live in
/// this one crate).
fn parse_pair_type_parts(ty: &str) -> Option<(String, String)> {
  let inner = ty.strip_prefix("Pair[")?.strip_suffix(']')?;
  let (k, v) = inner.split_once(", ")?;
  Some((k.to_string(), v.to_string()))
}

pub fn parse_named(src: &str, name: &str) -> Result<Program, Vec<ParseError>> {
  let mut recovered = Vec::new();
  let result = grammar::grammar::ProgramParser::new().parse(&mut recovered, src);
  let mut errors: Vec<ParseError> = recovered
    .into_iter()
    .map(|e| to_parse_error(e.error, name, src))
    .collect();
  match result {
    Ok(mut program) if errors.is_empty() => {
      rewrite_assert_locations(&mut program.items, name, src);
      fill_contract_text(&mut program.items, src);
      hoist_enumerable_blocks(&mut program, name, src).map_err(|e| vec![e])?;
      Ok(program)
    }
    Ok(_) => {
      errors.truncate(MAX_RECOVERED_ERRORS);
      Err(errors)
    }
    Err(e) => {
      errors.push(to_parse_error(e, name, src));
      errors.truncate(MAX_RECOVERED_ERRORS);
      Err(errors)
    }
  }
}

pub fn parse(src: &str) -> Result<Program, Vec<ParseError>> {
  parse_named(src, "<source>")
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Test-only shorthand for `Spanned::synthetic` — a hand-written
  /// expected-value literal has no real source text to derive a span
  /// from, so every nested `Expr`/`Stmt` position in one needs this.
  fn s<T>(node: T) -> Spanned<T> {
    Spanned::synthetic(node)
  }

  const FUNC_ONLY: &str = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend";

  #[test]
  fn parses_add_function() {
    let program = parse(FUNC_ONLY).expect("should parse");
    assert_eq!(program.items.len(), 1);
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
    assert_eq!(f.name, "add");
    assert_eq!(
      f.params,
      vec![
        Param {
          name: "a".into(),
          ty: "Int64".into(),
          default: None
        },
        Param {
          name: "b".into(),
          ty: "Int64".into(),
          default: None
        }
      ]
    );
    assert_eq!(f.return_type, "Int64");
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("a".into()))),
        Box::new(s(Expr::Ident("b".into())))
      ))))]
    );
  }

  #[test]
  fn missing_end_errors() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b";
    // Plan 26: `errs.len()` is 1 here — a syntax error inside a
    // function body (not at a top-level `Item` boundary) still
    // hard-stops the whole parse, unchanged by top-level recovery.
    let errs = parse(src).unwrap_err();
    assert_eq!(errs.len(), 1);
    eprintln!("LALRPOP ERROR: {}", errs[0]);
    assert!(!errs[0].to_string().is_empty());
  }

  #[test]
  fn parse_error_span_points_at_the_offending_token() {
    // Plan 13 AC1: the span isn't just "present" — it points at the
    // token's *actual* byte offset, verified against the source
    // string's own position, not merely asserted to exist.
    let src = "x: Int64 = +\n";
    let mut errs = parse(src).expect_err("`+` alone is not a valid Expr");
    assert_eq!(errs.len(), 1);
    let err = errs.remove(0);
    let expected_offset = src.find('+').unwrap();
    assert_eq!(err.span.offset(), expected_offset);
    // `ParseError` must satisfy `miette::Diagnostic` for `emerald-cli`
    // to render it — proven by actually constructing a `Report`.
    let report: miette::Report = miette::Report::new(err);
    assert!(!format!("{report:?}").is_empty());
  }

  const HELLO_EM: &str = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn parses_hello_em_end_to_end() {
    let program = parse(HELLO_EM).expect("hello.em should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Function(f) = &program.items[0] else {
      panic!(
        "expected item 0 to be the add function, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(f.name, "add");

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be the puts call, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Call(
          "add".into(),
          vec![s(Expr::Int(20)), s(Expr::Int(22))]
        ))]
      )
    );
  }

  const MILESTONE2: &str = "x: Int64 = 10\n\nif x > 5 do\n  puts x\nend\n";

  #[test]
  fn parses_inception_milestone2_end_to_end() {
    let program = parse(MILESTONE2).expect("milestone-2 example should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected item 0 to be `x: Int64 = 10`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "x");
    assert_eq!(ty, "Int64");
    assert_eq!(*value, Expr::Int(10));

    let Item::Stmt(Spanned {
      node: Stmt::If {
        cond,
        then_branch,
        else_branch,
      },
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be an if statement, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *cond,
      Expr::Compare(
        Box::new(s(Expr::Ident("x".into()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(5)))
      )
    );
    assert_eq!(
      then_branch,
      &vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::Ident("x".into()))]
      ))))]
    );
    assert_eq!(else_branch, &None);
  }

  #[test]
  fn parses_while_break_next() {
    let src = "while x < 3 do\n  next\n  break\nend\n";
    let program = parse(src).expect("while/break/next should parse");
    let Item::Stmt(Spanned {
      node: Stmt::While { body, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a while statement, got {:?}", program.items[0]);
    };
    assert_eq!(body, &vec![Stmt::Next, Stmt::Break]);
  }

  #[test]
  fn parses_if_else() {
    let src = "if x > 5 do\n  puts x\nelse\n  puts x\nend\n";
    let program = parse(src).expect("if/else should parse");
    let Item::Stmt(Spanned {
      node: Stmt::If { else_branch, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected an if statement, got {:?}", program.items[0]);
    };
    assert!(else_branch.is_some(), "else branch should be present");
  }

  #[test]
  fn parses_return_with_value() {
    let src = "fn f(a: Int64): Int64 do\n  return a\nend";
    let program = parse(src).expect("return should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert_eq!(
      f.body,
      vec![s(Stmt::Return(Some(s(Expr::Ident("a".into())))))]
    );
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  fn initialize(x: Float64, y: Float64): Void do\n    @x = x\n    @y = y\n  end\n\n  fn sum: Float64 do\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn parses_inception_point_example_end_to_end() {
    let program = parse(POINT_EXAMPLE).expect("Point example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Class(class) = &program.items[0] else {
      panic!(
        "expected item 0 to be the Point class, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(class.name, "Point");
    assert_eq!(
      class.fields,
      vec![
        Param {
          name: "x".into(),
          ty: "Float64".into(),
          default: None
        },
        Param {
          name: "y".into(),
          ty: "Float64".into(),
          default: None
        }
      ]
    );
    assert_eq!(class.methods.len(), 2);
    assert_eq!(class.methods[0].name, "initialize");
    assert_eq!(
      class.methods[0].body,
      vec![
        s(Stmt::SetField {
          name: "x".into(),
          value: s(Expr::Ident("x".into()))
        }),
        s(Stmt::SetField {
          name: "y".into(),
          value: s(Expr::Ident("y".into()))
        }),
      ]
    );
    assert_eq!(class.methods[1].name, "sum");
    assert_eq!(
      class.methods[1].body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::InstanceVar("x".into()))),
        Box::new(s(Expr::InstanceVar("y".into())))
      ))))]
    );

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be `p: Point = Point.new(...)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(name, "p");
    assert_eq!(ty, "Point");
    assert_eq!(
      *value,
      Expr::New(
        "Point".into(),
        vec![s(Expr::Float(2.0)), s(Expr::Float(3.0))]
      )
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts p.sum`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("p".into()))),
          "sum".into(),
          vec![]
        ))]
      )
    );
  }

  const ARRAY_EXAMPLE: &str =
    "arr: Array[Int64] = [10, 20, 30]\narr[1] = 99\nputs arr[1]\nputs arr[0] + arr[2]\n";

  #[test]
  fn parses_array_literal_index_read_and_write() {
    let program = parse(ARRAY_EXAMPLE).expect("array example should parse");
    assert_eq!(program.items.len(), 4);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected item 0 to be `arr: Array[Int64] = [...]`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "arr");
    assert_eq!(ty, "Array[Int64]");
    assert_eq!(
      *value,
      Expr::ArrayLit(vec![s(Expr::Int(10)), s(Expr::Int(20)), s(Expr::Int(30))])
    );

    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::SetIndex {
        array: s(Expr::Ident("arr".into())),
        index: s(Expr::Int(1)),
        value: s(Expr::Int(99)),
      }))
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts arr[1]`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Index(
          Box::new(s(Expr::Ident("arr".into()))),
          Box::new(s(Expr::Int(1)))
        ))]
      )
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[3]
    else {
      panic!(
        "expected item 3 to be `puts arr[0] + arr[2]`, got {:?}",
        program.items[3]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Add(
          Box::new(s(Expr::Index(
            Box::new(s(Expr::Ident("arr".into()))),
            Box::new(s(Expr::Int(0)))
          ))),
          Box::new(s(Expr::Index(
            Box::new(s(Expr::Ident("arr".into()))),
            Box::new(s(Expr::Int(2)))
          )))
        ))]
      )
    );
  }

  const LAMBDA_EXAMPLE: &str =
    "x: Int64 = 10\nadd_x: Proc = do |y: Int64| y + x end\nputs add_x.call(5)\n";

  #[test]
  fn parses_lambda_capture_and_call() {
    let program = parse(LAMBDA_EXAMPLE).expect("lambda example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be `add_x: Proc = ->(...) -> Int64 {{...}}`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(name, "add_x");
    assert_eq!(ty, "Proc");
    assert_eq!(
      *value,
      Expr::Lambda {
        params: vec![Param {
          name: "y".into(),
          ty: "Int64".into(),
          default: None
        }],
        // Plan 71: the bare `do |params| ... end` lambda literal carries
        // no return-type syntax of its own any more (typed by context
        // instead) — the grammar's own placeholder for this position is
        // `"Void"`, the same placeholder `BlockLiteral` already uses.
        return_type: "Void".into(),
        body: vec![s(Stmt::Expr(s(Expr::Add(
          Box::new(s(Expr::Ident("y".into()))),
          Box::new(s(Expr::Ident("x".into())))
        ))))],
      }
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts add_x.call(5)`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("add_x".into()))),
          "call".into(),
          vec![s(Expr::Int(5))]
        ))]
      )
    );
  }

  #[test]
  fn parses_method_call_with_arguments() {
    // Independent of Proc/lambdas: plan 10's grammar gap fix means an
    // ordinary `.method(args)` call — never possible before this plan —
    // now parses with a populated argument list.
    let src = "p.move(1, 2)\n";
    let program = parse(src).expect("method call with args should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected a method-call statement, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(
      *call,
      Expr::MethodCall(
        Box::new(s(Expr::Ident("p".into()))),
        "move".into(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )
    );
  }

  #[test]
  fn parses_raise() {
    let src = "raise MyError.new(99)\n";
    let program = parse(src).expect("raise should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Raise(s(Expr::New(
        "MyError".into(),
        vec![s(Expr::Int(99))]
      )))))
    );
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  fn initialize(code: Int64): Void do\n    @code = code\n  end\n\n  fn code: Int64 do\n    @code\n  end\nend\n\nfn risky(x: Int64): Int64 do\n  if x > 100 do\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn parses_begin_rescue() {
    let program = parse(EXCEPTION_EXAMPLE).expect("exception example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        body,
        rescues,
        ensure,
      },
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be a begin/rescue statement, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(rescues.len(), 1);
    assert_eq!(rescues[0].class_name, Some("MyError".to_string()));
    assert_eq!(rescues[0].var, "e");
    assert_eq!(ensure, &None);
    assert_eq!(
      body,
      &vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::Call("risky".into(), vec![s(Expr::Int(999))]))]
      ))))]
    );
    assert_eq!(
      rescues[0].body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("e".into()))),
          "code".into(),
          vec![]
        ))]
      ))))]
    );
  }

  // Plan 38 (full exception model).

  #[test]
  fn begin_with_multiple_rescues_and_ensure_parses_in_source_order() {
    let src = "begin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        rescues, ensure, ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues.len(), 2);
    assert_eq!(rescues[0].class_name, Some("NotFoundError".to_string()));
    assert_eq!(rescues[1].class_name, Some("TimeoutError".to_string()));
    assert_eq!(
      ensure,
      &Some(vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::StringLit("cleanup".into()))]
      ))))])
    );
  }

  #[test]
  fn begin_with_bare_rescue_and_no_ensure_parses() {
    let src = "begin\n  puts 1\nrescue => e\n  puts 2\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        rescues, ensure, ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues.len(), 1);
    assert_eq!(rescues[0].class_name, None);
    assert_eq!(rescues[0].var, "e");
    assert_eq!(ensure, &None);
  }

  #[test]
  fn retry_parses_to_stmt_retry_wherever_a_stmt_is_legal() {
    let src = "begin\n  puts 1\nrescue => e\n  retry\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin { rescues, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues[0].body, vec![Stmt::Retry]);
  }

  // Plan 39 (function signature completeness).

  #[test]
  fn default_param_value_parses_into_param_default() {
    let src = "fn inc(n: Int64, step: Int64 = 1): Int64 do\n  n + step\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params[0].default, None);
    assert_eq!(f.params[1].default, Some(s(Expr::Int(1))));
  }

  #[test]
  fn default_referencing_another_parameter_is_a_parse_error() {
    let src = "fn bad(n: Int64, step: Int64 = n): Int64 do\n  n + step\nend\n";
    assert!(
      parse(src).is_err(),
      "DefaultLit admits only literal tokens, never an Expr::Ident"
    );
  }

  #[test]
  fn splat_param_parses_into_function_splat_param() {
    let src = "fn sum_all(*xs: Int64): Int64 do\n  0\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert!(f.params.is_empty());
    let splat = f.splat_param.as_ref().expect("splat_param should be Some");
    assert_eq!(splat.name, "xs");
    assert_eq!(splat.ty, "Int64");
  }

  #[test]
  fn ordinary_param_then_splat_param_parses() {
    let src = "fn f(a: Int64, *xs: Int64): Int64 do\n  0\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params.len(), 1);
    assert_eq!(f.params[0].name, "a");
    assert_eq!(f.splat_param.as_ref().unwrap().name, "xs");
  }

  #[test]
  fn keyword_call_parses_to_expr_call_kw() {
    let src = "greet(name: \"yo\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::CallKw(
        "greet".to_string(),
        vec![("name".to_string(), s(Expr::StringLit("yo".to_string())))]
      )))))
    );
  }

  #[test]
  fn positional_call_still_parses_to_expr_call_unchanged() {
    let src = "greet(\"yo\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "greet".to_string(),
        vec![s(Expr::StringLit("yo".to_string()))]
      )))))
    );
  }

  #[test]
  fn return_tuple_parses_into_a_tuple_lit() {
    let src = "fn divmod(a: Int64, b: Int64): (Int64, Int64) do\n  return a / b, a % b\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.return_type, "(Int64, Int64)");
    let Stmt::Return(Some(Spanned {
      node: Expr::TupleLit(elems),
      ..
    })) = &f.body[0].node
    else {
      panic!("expected a tuple-literal return, got {:?}", f.body[0]);
    };
    assert_eq!(elems.len(), 2);
  }

  #[test]
  fn multi_assign_from_a_single_call_value_parses() {
    let src = "q: Int64 = 0\nr: Int64 = 0\nq, r = f(1, 2)\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::MultiAssign { names, values },
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected a MultiAssign statement, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(names, &vec!["q".to_string(), "r".to_string()]);
    assert_eq!(values.len(), 1);
  }

  // Plan 40 (operator overloading).

  fn operator_method_name(src: &str) -> String {
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    c.methods[0].name.clone()
  }

  #[test]
  fn plus_operator_method_parses_to_a_function_named_plus() {
    let src = "class Vector2\n  fn +(other: Vector2): Vector2 do\n    self\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert_eq!(
      c.methods[0],
      Function {
        name: "+".to_string(),
        params: vec![Param {
          name: "other".to_string(),
          ty: "Vector2".to_string(),
          default: None,
        }],
        return_type: "Vector2".to_string(),
        body: vec![s(Stmt::Expr(s(Expr::Ident("self".to_string()))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
      }
    );
  }

  #[test]
  fn all_eight_operator_tokens_parse_as_method_names() {
    assert_eq!(
      operator_method_name("class C\n  fn -(o: C): C do\n    self\n  end\nend\n"),
      "-"
    );
    assert_eq!(
      operator_method_name("class C\n  fn *(o: C): C do\n    self\n  end\nend\n"),
      "*"
    );
    assert_eq!(
      operator_method_name("class C\n  fn /(o: C): C do\n    self\n  end\nend\n"),
      "/"
    );
    assert_eq!(
      operator_method_name("class C\n  fn ==(o: C): Boolean do\n    true\nend\nend\n"),
      "=="
    );
    assert_eq!(
      operator_method_name("class C\n  fn <=>(o: C): Int64 do\n    0\n  end\nend\n"),
      "<=>"
    );
    assert_eq!(
      operator_method_name("class C\n  fn [](i: Int64): Float64 do\n    1.0\n  end\nend\n"),
      "[]"
    );
    assert_eq!(
      operator_method_name(
        "class C\n  fn []=(i: Int64, v: Float64): Void do\n    puts 1\n  end\nend\n"
      ),
      "[]="
    );
  }

  #[test]
  fn top_level_operator_named_def_is_a_parse_error() {
    let src = "fn +(a: Int64, b: Int64): Int64 do\n  a + b\nend\n";
    assert!(
      parse(src).is_err(),
      "operator-named methods stay class-body-only"
    );
  }

  const MODULE_EXAMPLE: &str = "module MathUtils\n  fn double(x: Int64): Int64 do\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";

  #[test]
  fn parses_module_and_namespaced_call() {
    let program = parse(MODULE_EXAMPLE).expect("module example should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Module(m) = &program.items[0] else {
      panic!(
        "expected item 0 to be the MathUtils module, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(m.name, "MathUtils");
    assert_eq!(m.methods.len(), 1);
    assert_eq!(m.methods[0].name, "double");
    assert_eq!(
      m.methods[0].params,
      vec![Param {
        name: "x".into(),
        ty: "Int64".into(),
        default: None
      }]
    );
    assert_eq!(m.methods[0].return_type, "Int64");
    assert_eq!(
      m.methods[0].body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("x".into()))),
        Box::new(s(Expr::Ident("x".into())))
      ))))]
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be `puts MathUtils.double(21)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("MathUtils".into()))),
          "double".into(),
          vec![s(Expr::Int(21))]
        ))]
      )
    );
  }

  // Plan 18 (arithmetic & logical operators) — precedence is real, not
  // just parseable (AC1).

  #[test]
  fn mul_binds_tighter_than_add() {
    let program = parse("puts 2 + 3 * 4\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(s(Expr::Int(2))),
        Box::new(s(Expr::Mul(
          Box::new(s(Expr::Int(3))),
          Box::new(s(Expr::Int(4)))
        )))
      )
    );
  }

  #[test]
  fn unary_minus_binds_tighter_than_add() {
    let program = parse("puts -3 + 10\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(s(Expr::Neg(Box::new(s(Expr::Int(3)))))),
        Box::new(s(Expr::Int(10)))
      )
    );
  }

  #[test]
  fn and_binds_tighter_than_or() {
    let program = parse("puts a > 0 && b > 0 || c > 0\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    let gt = |name: &str, n: i64| {
      Expr::Compare(
        Box::new(s(Expr::Ident(name.into()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(n))),
      )
    };
    assert_eq!(
      args[0],
      Expr::Or(
        Box::new(s(Expr::And(
          Box::new(s(gt("a", 0))),
          Box::new(s(gt("b", 0)))
        ))),
        Box::new(s(gt("c", 0)))
      )
    );
  }

  #[test]
  fn not_binds_tighter_than_compare_not_looser() {
    // AC3: `!` sits at the unary tier, tighter than comparison — `!x > 0`
    // parses as `Compare(Not(x), Gt, 0)`, NOT `Not(Compare(x, Gt, 0))`.
    // Real, Ruby-divergent precedence, documented by this test rather
    // than left unspecified.
    let program = parse("puts !x > 0\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Compare(
        Box::new(s(Expr::Not(Box::new(s(Expr::Ident("x".into())))))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(0)))
      )
    );
  }

  const ARITHMETIC_EXAMPLE: &str = "fn factorial(n: Int64): Int64 do\n  if n <= 1 do\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";

  #[test]
  fn parses_arithmetic_example() {
    let program = parse(ARITHMETIC_EXAMPLE).expect("arithmetic example should parse");
    assert_eq!(program.items.len(), 5);
    let Item::Function(f) = &program.items[0] else {
      panic!(
        "expected the factorial function, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(
      f.body[0],
      Stmt::If {
        cond: s(Expr::Compare(
          Box::new(s(Expr::Ident("n".into()))),
          CompareOp::Le,
          Box::new(s(Expr::Int(1)))
        )),
        then_branch: vec![s(Stmt::Return(Some(s(Expr::Int(1)))))],
        else_branch: None,
      }
    );
    assert_eq!(
      f.body[1],
      Stmt::Return(Some(s(Expr::Mul(
        Box::new(s(Expr::Ident("n".into()))),
        Box::new(s(Expr::Call(
          "factorial".into(),
          vec![s(Expr::Sub(
            Box::new(s(Expr::Ident("n".into()))),
            Box::new(s(Expr::Int(1)))
          ))]
        )))
      ))))
    );
  }

  const SHORT_CIRCUIT_EXAMPLE: &str = "fn noisy(n: Int64): Boolean do\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1) do\n  puts 100\nend\nif x > -10 && noisy(3) do\n  puts 300\nend\nif x < 0 || noisy(2) do\n  puts 200\nend\nif x > 0 || noisy(4) do\n  puts 400\nend\n";

  #[test]
  fn parses_short_circuit_example() {
    let program = parse(SHORT_CIRCUIT_EXAMPLE).expect("short-circuit example should parse");
    // 1 function + 1 let + 4 ifs.
    assert_eq!(program.items.len(), 6);
    let Item::Stmt(Spanned {
      node: Stmt::If { cond, .. },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the first if, got {:?}", program.items[2]);
    };
    assert_eq!(
      *cond,
      Expr::And(
        Box::new(s(Expr::Compare(
          Box::new(s(Expr::Ident("x".into()))),
          CompareOp::Gt,
          Box::new(s(Expr::Int(0)))
        ))),
        Box::new(s(Expr::Call("noisy".into(), vec![s(Expr::Int(1))])))
      )
    );
  }

  #[test]
  fn rejects_bare_unary_minus_as_a_statement() {
    // A fresh statement can't start with unary `-`/`!` (see
    // `StmtUnaryExpr`'s Decision-log comment in grammar.lalrpop) — still
    // fully usable as a Let/return/argument value.
    assert!(parse("-5\n").is_err());
  }

  // Plan 19 (string literals).

  #[test]
  fn parses_plain_string_literal() {
    let program = parse("puts \"hello\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("hello".to_string()));
  }

  #[test]
  fn decodes_escaped_quote() {
    let program = parse("puts \"a\\\"b\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a\"b".to_string()));
  }

  #[test]
  fn decodes_escaped_newline() {
    let program = parse("puts \"line1\\nline2\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("line1\nline2".to_string()));
  }

  #[test]
  fn unterminated_string_literal_errors_not_panics() {
    let src = "puts \"hello\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn unrecognized_escape_errors_not_panics() {
    // `\t` isn't one of the two recognized escapes (`\"`/`\n`) — a real
    // parse error, not a silent pass-through.
    let src = "puts \"a\\tb\"\n";
    assert!(parse(src).is_err());
  }

  // Plan 20 (comments and case/when).

  #[test]
  fn comment_only_file_is_an_empty_program() {
    let program = parse("# just a comment\n").expect("should parse");
    assert_eq!(program.items, vec![]);
  }

  #[test]
  fn trailing_and_leading_comments_dont_change_the_ast() {
    let with_comments = "# classify\nx: Int64 = 10 # ten\nputs x\n";
    let without = "x: Int64 = 10\nputs x\n";
    assert_eq!(
      parse(with_comments).expect("should parse"),
      parse(without).expect("should parse")
    );
  }

  #[test]
  fn hash_inside_string_literal_is_not_eaten_as_a_comment() {
    let program = parse("puts \"a#b\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a#b".to_string()));
  }

  // Plan 71: `case`/`when`/`else` becomes `match`/`do`/`_` — each arm
  // (including the wildcard) closes its own `do ... end`.
  const CASE_EXAMPLE: &str = "# classify an integer by a fixed set of buckets\nn: Int64 = 2\nlabel: Int64 = 0\nmatch n do\n  1 do\n    label: Int64 = 10\n  end\n  2, 3 do\n    label: Int64 = 20\n  end\n  _ do\n    label: Int64 = 99\n  end\nend\nputs label\n";

  #[test]
  fn parses_case_when_example() {
    let program = parse(CASE_EXAMPLE).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case {
        scrutinee,
        arms,
        else_body,
      },
      ..
    }) = &program.items[2]
    else {
      panic!("expected a case statement, got {:?}", program.items[2]);
    };
    assert_eq!(*scrutinee, Expr::Ident("n".into()));
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, CasePattern::Values(vec![s(Expr::Int(1))]));
    assert_eq!(
      arms[0].1,
      vec![s(Stmt::Let {
        name: "label".into(),
        ty: "Int64".into(),
        value: s(Expr::Int(10)),
      })]
    );
    assert_eq!(
      arms[1].0,
      CasePattern::Values(vec![s(Expr::Int(2)), s(Expr::Int(3))])
    );
    assert!(else_body.is_some());
  }

  // Plan 25 (stdlib expansion).

  #[test]
  fn parses_bool_and_nil_literals() {
    let program = parse("puts true\nputs false\nputs nil\n")
      .expect("should parse")
      .items;
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a1),
        ..
      }),
      ..
    }) = &program[0]
    else {
      panic!("expected a puts call, got {:?}", program[0]);
    };
    assert_eq!(a1[0], Expr::Bool(true));
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a2),
        ..
      }),
      ..
    }) = &program[1]
    else {
      panic!("expected a puts call, got {:?}", program[1]);
    };
    assert_eq!(a2[0], Expr::Bool(false));
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a3),
        ..
      }),
      ..
    }) = &program[2]
    else {
      panic!("expected a puts call, got {:?}", program[2]);
    };
    assert_eq!(a3[0], Expr::Nil);
  }

  #[test]
  fn parses_hash_literal_and_indexing() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(name, "h");
    assert_eq!(ty, "Hash[Int64, Int64]");
    assert_eq!(
      *value,
      Expr::HashLit(vec![
        (s(Expr::Int(1)), s(Expr::Int(10))),
        (s(Expr::Int(2)), s(Expr::Int(20))),
        (s(Expr::Int(3)), s(Expr::Int(30))),
      ])
    );
    assert_eq!(
      program.items[2],
      Item::Stmt(s(Stmt::SetIndex {
        array: s(Expr::Ident("h".into())),
        index: s(Expr::Int(2)),
        value: s(Expr::Int(99)),
      }))
    );
  }

  #[test]
  fn parses_array_new() {
    let program = parse("arr: Array[Int64] = Array.new(5)\n").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: s(Expr::ArrayNew(Box::new(s(Expr::Int(5))))),
      }))
    );
  }

  #[test]
  fn parses_array_new_with_runtime_size() {
    let program = parse("n: Int64 = 5\narr: Array[Int64] = Array.new(n)\n").expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: s(Expr::ArrayNew(Box::new(s(Expr::Ident("n".into()))))),
      }))
    );
  }

  #[test]
  fn case_with_no_else_parses() {
    let src = "match n do\n  1 do\n    puts 1\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case { else_body, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a case statement, got {:?}", program.items[0]);
    };
    assert_eq!(*else_body, None);
  }

  // Plan 31 (compound and multiple assignment).

  #[test]
  fn bare_reassignment_parses_as_assign() {
    let src = "x: Int64 = 1\nx = 2\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::Assign {
        name: "x".to_string(),
        value: s(Expr::Int(2)),
      }))
    );
  }

  #[test]
  fn compound_plus_assign_desugars_to_assign_of_add() {
    let src = "total += i\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Assign {
        name: "total".to_string(),
        value: s(Expr::Add(
          Box::new(s(Expr::Ident("total".to_string()))),
          Box::new(s(Expr::Ident("i".to_string()))),
        )),
      }))
    );
  }

  fn assert_compound_assign_desugars_to(src: &str, expected_value: Expr) {
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Assign {
        name: "x".to_string(),
        value: s(expected_value),
      })),
      "mismatched desugaring for {src:?}"
    );
  }

  #[test]
  fn all_compound_assign_operators_desugar_to_the_matching_binop() {
    let x = || Box::new(s(Expr::Ident("x".to_string())));
    let one = || Box::new(s(Expr::Int(1)));
    assert_compound_assign_desugars_to("x -= 1\n", Expr::Sub(x(), one()));
    assert_compound_assign_desugars_to("x *= 1\n", Expr::Mul(x(), one()));
    assert_compound_assign_desugars_to("x /= 1\n", Expr::Div(x(), one()));
    assert_compound_assign_desugars_to("x %= 1\n", Expr::Rem(x(), one()));
  }

  #[test]
  fn multiple_assignment_swap_parses() {
    let src = "a, b = b, a\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::MultiAssign {
        names: vec!["a".to_string(), "b".to_string()],
        values: vec![
          s(Expr::Ident("b".to_string())),
          s(Expr::Ident("a".to_string()))
        ],
      }))
    );
  }

  #[test]
  fn plan_31_worked_example_parses() {
    let src = "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5 do\n  total += i\n  i += 1\nend\nputs total\n\na: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    parse(src).expect("plan 31's worked example must parse cleanly");
  }

  // Plan 34 (blocks and yield).

  #[test]
  fn block_param_marker_parses_separately_from_params() {
    let src = "fn repeat(n: Int64, &blk): Void do\n  yield n\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(
      f.params,
      vec![Param {
        name: "n".to_string(),
        ty: "Int64".to_string(),
        default: None
      }]
    );
    assert_eq!(f.block_param, Some("blk".to_string()));
  }

  #[test]
  fn zero_arg_block_param_parses() {
    let src = "fn once(&blk): Void do\n  yield 1\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params, vec![]);
    assert_eq!(f.block_param, Some("blk".to_string()));
  }

  #[test]
  fn function_with_no_block_param_is_none() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.block_param, None);
  }

  #[test]
  fn trailing_block_literal_desugars_to_an_extra_call_argument() {
    let src = "repeat(3) do |i: Int64| puts i end\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "repeat".to_string(),
        vec![
          s(Expr::Int(3)),
          s(Expr::Lambda {
            params: vec![Param {
              name: "i".to_string(),
              ty: "Int64".to_string(),
              default: None
            }],
            return_type: "Void".to_string(),
            body: vec![s(Stmt::Expr(s(Expr::Call(
              "puts".to_string(),
              vec![s(Expr::Ident("i".to_string()))]
            ))))],
          }),
        ]
      )))))
    );
  }

  #[test]
  fn call_with_no_trailing_block_parses_unchanged() {
    let src = "add(1, 2)\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "add".to_string(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )))))
    );
  }

  #[test]
  fn yield_parses_to_stmt_yield() {
    let src = "yield i\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Yield(vec![s(Expr::Ident("i".to_string()))])))
    );
  }

  #[test]
  fn plan_34_worked_example_parses() {
    let src = "fn repeat(n: Int64, &blk): Void do\n  i: Int64 = 0\n  while i < n do\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) do |i: Int64| puts i end\n";
    parse(src).expect("plan 34's worked example must parse cleanly");
  }

  // Plan 36 (string interpolation and heredocs).

  #[test]
  fn interpolated_string_splits_into_parts() {
    let src = "puts \"Hello, #{name}!\"\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::Interpolate(vec![
          StringPart::Literal("Hello, ".to_string()),
          StringPart::Expr(Box::new(s(Expr::Ident("name".to_string())))),
          StringPart::Literal("!".to_string()),
        ]))]
      )))))
    );
  }

  #[test]
  fn plain_string_with_no_interpolation_still_parses_as_string_lit() {
    let src = "puts \"hello\"\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::StringLit("hello".to_string()))]
      )))))
    );
  }

  #[test]
  fn nested_hash_literal_inside_interpolation_parses_via_brace_depth_scan() {
    let src = "puts \"#{ {1 => 2}[1] }\"\n";
    parse(src).expect("brace-depth scan must not cut the span short at the hash literal's own `}`");
  }

  #[test]
  fn unterminated_interpolation_is_a_real_parse_error() {
    let src = "puts \"unterminated #{name\"\n";
    assert!(
      parse(src).is_err(),
      "an unterminated `#{{` must be a real parse error, not a silent truncation"
    );
  }

  // Plan 23 (multi-file compilation).

  #[test]
  fn require_single_segment_parses() {
    let src = "require helpers\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items[0], Item::Require("helpers".to_string()));
  }

  #[test]
  fn require_multi_segment_path_parses() {
    let src = "require utils/math\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items[0], Item::Require("utils/math".to_string()));
  }

  #[test]
  fn require_inside_a_function_body_is_a_parse_error() {
    let src = "fn f: Void do\n  require helpers\nend\n";
    assert!(
      parse(src).is_err(),
      "`require` must be rejected inside a function body"
    );
  }

  // Plan 33 (field-access sugar).

  #[test]
  fn read_field_synthesizes_a_zero_arg_accessor() {
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    // `fields` is unchanged: two plain, unmarked Params.
    assert_eq!(
      c.fields,
      vec![
        Param {
          name: "x".to_string(),
          ty: "Int64".to_string(),
          default: None
        },
        Param {
          name: "y".to_string(),
          ty: "Int64".to_string(),
          default: None
        },
      ]
    );
    // `methods` contains a synthesized zero-arg accessor for `x`,
    // alongside the hand-written `initialize` — but nothing for `y`.
    let x_accessor = c
      .methods
      .iter()
      .find(|m| m.name == "x")
      .expect("expected a synthesized `x` accessor method");
    assert_eq!(x_accessor.params, vec![]);
    assert_eq!(x_accessor.return_type, "Int64");
    assert_eq!(
      x_accessor.body,
      vec![s(Stmt::Expr(s(Expr::InstanceVar("x".to_string()))))]
    );
    assert!(c.methods.iter().any(|m| m.name == "initialize"));
    assert!(!c.methods.iter().any(|m| m.name == "y"));
  }

  #[test]
  fn plan_33_worked_example_parses() {
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  fn initialize(x: Int64, y: Int64): Void do\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";
    parse(src).expect("plan 33's worked example must parse cleanly");
  }

  // Plan 32 (class inheritance).

  #[test]
  fn class_with_no_superclass_parses_with_none() {
    let src = "class Counter\n  n: Int64\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    assert_eq!(c.superclass, None);
  }

  #[test]
  fn class_with_superclass_parses_the_name() {
    let src = "class Dog < Animal\n  breed_code: Int64\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    assert_eq!(c.superclass, Some("Animal".to_string()));
  }

  #[test]
  fn plan_32_worked_example_parses() {
    let src = "class Animal\n  age: Int64\n\n  fn initialize(age: Int64): Void do\n    @age = age\n  end\n\n  fn age: Int64 do\n    @age\n  end\n\n  fn describe: Int64 do\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  fn initialize(age: Int64, breed_code: Int64): Void do\n    @age = age\n    @breed_code = breed_code\n  end\n\n  fn describe: Int64 do\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";
    parse(src).expect("plan 32's worked example must parse cleanly");
  }

  // Plan 30 (for-in iteration).

  #[test]
  fn for_in_over_a_literal_array_parses() {
    let src = "for x in [10, 20, 30]\n  puts x\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::For {
        var: "x".to_string(),
        elements: vec![s(Expr::Int(10)), s(Expr::Int(20)), s(Expr::Int(30))],
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("x".to_string()))]
        ))))],
      }))
    );
  }

  #[test]
  fn for_in_over_a_bare_identifier_is_a_parse_error() {
    let src = "for x in arr\n  puts x\nend\n";
    assert!(
      parse(src).is_err(),
      "a non-literal scrutinee must be rejected at parse time"
    );
  }

  // Plan 37 (ranges and range-based iteration).

  #[test]
  fn inclusive_range_for_in_parses_to_stmt_for_range() {
    let src = "for i in 1..5\n  puts i\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::ForRange {
        var: "i".to_string(),
        start: s(Expr::Int(1)),
        end: s(Expr::Int(5)),
        exclusive: false,
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("i".to_string()))]
        ))))],
      }))
    );
  }

  #[test]
  fn exclusive_range_for_in_parses_to_stmt_for_range() {
    let src = "for i in 1...5\n  puts i\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::ForRange {
        var: "i".to_string(),
        start: s(Expr::Int(1)),
        end: s(Expr::Int(5)),
        exclusive: true,
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("i".to_string()))]
        ))))],
      }))
    );
  }

  #[test]
  fn bare_range_outside_a_for_in_scrutinee_is_a_parse_error() {
    let src = "r = 1..5\n";
    assert!(
      parse(src).is_err(),
      "a Range has no standalone AST shape — only legal directly after `for <var> in`"
    );
  }

  #[test]
  fn endless_range_for_in_is_a_parse_error() {
    let src = "for i in 5..\n  puts i\nend\n";
    assert!(
      parse(src).is_err(),
      "an endless range (no end operand) must be rejected at parse time"
    );
  }

  // Plan 29 (control-flow completeness).

  #[test]
  fn elsif_chain_desugars_to_nested_if_in_else_branch() {
    let src = "if a do\n  1\nelsif b do\n  2\nelsif c do\n  3\nelse\n  4\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::If {
        then_branch,
        else_branch,
        ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level If statement");
    };
    assert_eq!(*then_branch, vec![s(Stmt::Expr(s(Expr::Int(1))))]);

    // First elsif link.
    let Some(outer_else) = else_branch else {
      panic!("expected an else_branch from the first elsif");
    };
    assert_eq!(outer_else.len(), 1);
    let Spanned {
      node: Stmt::If {
        then_branch: b2,
        else_branch: e2,
        ..
      },
      ..
    } = &outer_else[0]
    else {
      panic!("expected a nested If for the first elsif");
    };
    assert_eq!(*b2, vec![s(Stmt::Expr(s(Expr::Int(2))))]);

    // Second elsif link.
    let Some(inner_else) = e2 else {
      panic!("expected an else_branch from the second elsif");
    };
    assert_eq!(inner_else.len(), 1);
    let Spanned {
      node: Stmt::If {
        then_branch: b3,
        else_branch: e3,
        ..
      },
      ..
    } = &inner_else[0]
    else {
      panic!("expected a nested If for the second elsif");
    };
    assert_eq!(*b3, vec![s(Stmt::Expr(s(Expr::Int(3))))]);

    // Trailing plain else.
    assert_eq!(*e3, Some(vec![s(Stmt::Expr(s(Expr::Int(4))))]));
  }

  #[test]
  fn unless_desugars_to_if_not() {
    let src = "unless x > 0 do\n  return 0\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::If {
        cond,
        then_branch,
        else_branch,
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected an If statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(s(Expr::Compare(
        Box::new(s(Expr::Ident("x".to_string()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(0))),
      ))))
    );
    assert_eq!(*then_branch, vec![s(Stmt::Return(Some(s(Expr::Int(0)))))]);
    assert_eq!(*else_branch, None);
  }

  #[test]
  fn until_desugars_to_while_not() {
    let src = "until i >= 3 do\n  puts i\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::While { cond, body },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a While statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(s(Expr::Compare(
        Box::new(s(Expr::Ident("i".to_string()))),
        CompareOp::Ge,
        Box::new(s(Expr::Int(3))),
      ))))
    );
    assert_eq!(
      *body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::Ident("i".to_string()))]
      ))))]
    );
  }

  #[test]
  fn plan_29_worked_examples_parse() {
    let example_a = "fn grade(score: Int64): Int64 do\n  if score >= 90 do\n    return 4\n  elsif score >= 80 do\n    return 3\n  elsif score >= 70 do\n    return 2\n  else\n    return 1\n  end\nend\n\nputs grade(95)\nputs grade(85)\nputs grade(72)\nputs grade(50)\n";
    parse(example_a).expect("plan 29 example A must parse cleanly");

    let example_b = "fn describe(x: Int64): Int64 do\n  unless x > 0 do\n    return 0\n  end\n  return 1\nend\n\nputs describe(-5)\nputs describe(5)\n\ni: Int64 = 0\nuntil i >= 3 do\n  puts i\n  i: Int64 = i + 1\nend\n";
    parse(example_b).expect("plan 29 example B must parse cleanly");
  }

  // Plan 28 (bitwise operators).

  #[test]
  fn bit_and_binds_tighter_than_bit_or() {
    let src = "x: Int64 = 1 | 2 & 3\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::BitOr(
        Box::new(s(Expr::Int(1))),
        Box::new(s(Expr::BitAnd(
          Box::new(s(Expr::Int(2))),
          Box::new(s(Expr::Int(3)))
        ))),
      )
    );
  }

  #[test]
  fn shift_binds_tighter_than_bit_and() {
    let src = "x: Int64 = 1 << 2 & 3\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::BitAnd(
        Box::new(s(Expr::Shl(
          Box::new(s(Expr::Int(1))),
          Box::new(s(Expr::Int(2)))
        ))),
        Box::new(s(Expr::Int(3))),
      )
    );
  }

  #[test]
  fn bit_and_binds_tighter_than_compare() {
    let src = "x: Boolean = flags & flag == flag\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::Compare(
        Box::new(s(Expr::BitAnd(
          Box::new(s(Expr::Ident("flags".to_string()))),
          Box::new(s(Expr::Ident("flag".to_string()))),
        ))),
        CompareOp::Eq,
        Box::new(s(Expr::Ident("flag".to_string()))),
      )
    );
  }

  #[test]
  fn bit_not_binds_as_tight_as_neg() {
    let src = "x: Int64 = ~0\ny: Int64 = ~x + 1\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items.len(), 2);
    let Item::Stmt(Spanned {
      node: Stmt::Let { value: v0, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(*v0, Expr::BitNot(Box::new(s(Expr::Int(0)))));
    let Item::Stmt(Spanned {
      node: Stmt::Let { value: v1, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *v1,
      Expr::Add(
        Box::new(s(Expr::BitNot(Box::new(s(Expr::Ident("x".to_string())))))),
        Box::new(s(Expr::Int(1))),
      )
    );
  }

  fn as_let(program: &Program) -> &Stmt {
    match &program.items[0] {
      Item::Stmt(
        spanned @ Spanned {
          node: Stmt::Let { .. },
          ..
        },
      ) => &spanned.node,
      other => panic!("expected a Let statement, got {other:?}"),
    }
  }

  #[test]
  fn plan_28_worked_example_parses() {
    let src = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\nfn has_flag(flags: Int64, flag: Int64): Boolean do\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ) do\n  puts 1\nend\nif has_flag(perms, EXEC) do\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";
    parse(src).expect("plan 28's worked example must parse cleanly");
  }

  // Plan 26 (parser error recovery).

  const TWO_BROKEN_LETS: &str = "x: Int64 = +\ny: Int64 = +\n";

  #[test]
  fn reports_two_independent_top_level_syntax_errors_in_one_pass() {
    let errs = parse(TWO_BROKEN_LETS).expect_err("both lets are malformed");
    assert_eq!(
      errs.len(),
      2,
      "expected exactly two recovered errors, got {errs:?}"
    );
    let first_plus = TWO_BROKEN_LETS.find('+').unwrap();
    let second_plus = TWO_BROKEN_LETS.rfind('+').unwrap();
    assert_eq!(errs[0].span.offset(), first_plus);
    assert_eq!(errs[1].span.offset(), second_plus);
  }

  #[test]
  fn error_recovery_does_not_alter_single_error_case() {
    // Regression guard for the plan 26 acceptance criteria: a source with
    // exactly one malformed construct inside an otherwise well-formed
    // function body must still report exactly one error, not more.
    let errs = parse("fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\n\nx: Int64 = +\n")
      .expect_err("the second statement is malformed");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
  }

  #[test]
  fn string_typed_let_and_concat_parse() {
    let src = "s: String = \"hello\"\na: String = \"foo\" + \"bar\"\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "s".into(),
        ty: "String".into(),
        value: s(Expr::StringLit("hello".into())),
      }))
    );
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::Let {
        name: "a".into(),
        ty: "String".into(),
        value: s(Expr::Add(
          Box::new(s(Expr::StringLit("foo".into()))),
          Box::new(s(Expr::StringLit("bar".into())))
        )),
      }))
    );
  }

  // Plan 41 (interfaces and generics).

  const INTERFACES_GENERICS_EXAMPLE: &str = "interface Comparable\n  fn compare_to(other: Self): Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\n\n  fn compare_to(other: Money): Int64 do\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  fn initialize(meters: Int64): Void do\n    @meters = meters\n  end\n\n  fn compare_to(other: Distance): Int64 do\n    @meters - other.meters\n  end\nend\n\nfn max[T: Comparable](a: T, b: T): T do\n  if a.compare_to(b) >= 0 do\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

  #[test]
  fn worked_example_parses_end_to_end_into_the_expected_ast_shapes() {
    let program = parse(INTERFACES_GENERICS_EXAMPLE).expect("should parse");

    let Item::Interface(iface) = &program.items[0] else {
      panic!(
        "expected item 0 to be the `Comparable` interface, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(iface.name, "Comparable");
    assert_eq!(iface.method_name, "compare_to");
    assert_eq!(
      iface.params,
      vec![Param {
        name: "other".to_string(),
        ty: "Self".to_string(),
        default: None,
      }]
    );
    assert_eq!(iface.return_type, "Int64");

    let Item::Class(money) = &program.items[1] else {
      panic!(
        "expected item 1 to be the `Money` class, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(money.name, "Money");
    assert_eq!(money.implements, Some("Comparable".to_string()));

    let Item::Function(max_fn) = &program.items[3] else {
      panic!(
        "expected item 3 to be the `max` function, got {:?}",
        program.items[3]
      );
    };
    assert_eq!(max_fn.name, "max");
    assert_eq!(
      max_fn.type_params,
      vec![TypeParam {
        name: "T".to_string(),
        bound: Some("Comparable".to_string()),
      }]
    );
  }

  // Plan 58 (generic types).

  #[test]
  fn generic_class_with_no_bound_parses_with_a_bound_less_type_param() {
    let src = "class Box[T]\n  value: T\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert_eq!(
      c.type_params,
      vec![TypeParam {
        name: "T".to_string(),
        bound: None,
      }]
    );
  }

  #[test]
  fn generic_class_with_a_bound_parses_with_a_some_bound_type_param() {
    let src = "class Box[T: Comparable]\n  value: T\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert_eq!(
      c.type_params,
      vec![TypeParam {
        name: "T".to_string(),
        bound: Some("Comparable".to_string()),
      }]
    );
  }

  const STACK_GENERIC_EXAMPLE: &str = "class Stack[T]\n  items: Array[T]\n  count: Int64\n\n  fn initialize: Void do\n    @items = Array.new(8)\n    @count = 0\n  end\n\n  fn push(x: T): Void do\n    @items[@count] = x\n    @count = @count + 1\n  end\n\n  fn pop: T do\n    @count = @count - 1\n    @items[@count]\n  end\n\n  fn peek: T do\n    @items[@count - 1]\n  end\nend\n\ns1: Stack[Int64] = Stack.new()\ns1.push(10)\ns1.push(20)\ns1.push(30)\nputs s1.pop\nputs s1.peek\n\ns2: Stack[String] = Stack.new()\ns2.push(\"first\")\ns2.push(\"second\")\nputs s2.pop\nputs s2.peek\n";

  #[test]
  fn stack_generic_worked_example_parses_end_to_end() {
    let program = parse(STACK_GENERIC_EXAMPLE).expect("should parse");
    let Item::Class(stack) = &program.items[0] else {
      panic!("expected the Stack class");
    };
    assert_eq!(
      stack.type_params,
      vec![TypeParam {
        name: "T".to_string(),
        bound: None,
      }]
    );
    let items_field = stack
      .fields
      .iter()
      .find(|p| p.name == "items")
      .expect("items field");
    assert_eq!(items_field.ty, "Array[T]");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected `s1`'s Let statement");
    };
    assert_eq!(ty, "Stack[Int64]");
  }

  #[test]
  fn nested_generic_type_arguments_parse_as_one_compound_typename_string() {
    let src = "class Box[T]\n  value: T\nend\n\nb: Box[Box[Int64]] = Box.new()\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected `b`'s Let statement");
    };
    assert_eq!(ty, "Box[Box[Int64]]");
  }

  #[test]
  fn a_non_generic_class_still_parses_with_empty_type_params() {
    let src = "class Point\n  x: Int64\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert_eq!(c.type_params, vec![]);
  }

  #[test]
  fn malformed_generic_class_header_is_a_parse_error_not_a_panic() {
    // A paren-parameter-list immediately after `class NoBound[T]`, with
    // no intervening field/method shape — a real, disclosed AST-level
    // impossibility (no grammar production accepts this), not a panic.
    let src = "class NoBound[T](x: T)\nend\n";
    let errs = parse(src).expect_err("must be a real parse error");
    assert!(!errs.is_empty());
  }

  #[test]
  fn multiple_type_parameters_parse_the_grammar_does_not_restrict_the_count() {
    // Plan 41's Decision log: the grammar's `TypeParamList` is a general
    // comma list — the single-type-parameter restriction is sema's job.
    let src = "fn bad[T: Comparable, U: Comparable](a: T, b: U): T do\n  a\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert_eq!(f.type_params.len(), 2);
    assert_eq!(f.type_params[0].name, "T");
    assert_eq!(f.type_params[1].name, "U");
  }

  #[test]
  fn interface_with_a_second_method_before_end_is_a_parse_error_not_a_panic() {
    // The grammar structurally admits exactly one method — a second
    // `def` before the outer `end` cannot reduce as this same
    // production, so this is a real parse error.
    let src = "interface Comparable\n  fn compare_to(other: Self): Int64\n  fn other_method(x: Int64): Int64\nend\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  #[test]
  fn interface_missing_the_inner_return_type_is_a_parse_error_not_a_panic() {
    let src = "interface Comparable\n  fn compare_to(other: Self)\nend\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  // Plan 43 (nullable types and safe navigation).

  #[test]
  fn nullable_suffix_parses_into_a_compound_type_string() {
    let src = "g: Greeter? = nil\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(ty, "Greeter?");
  }

  #[test]
  fn safe_call_parses_to_expr_safe_call() {
    let src = "message: String? = g&.shout\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::SafeCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "shout".to_string(),
        vec![]
      )
    );
  }

  #[test]
  fn safe_call_with_args_parses_to_expr_safe_call() {
    let src = "x: Int64? = g&.add(1, 2)\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::SafeCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "add".to_string(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )
    );
  }

  #[test]
  fn ordinary_dot_method_call_still_parses_to_expr_method_call_unchanged() {
    let src = "x: Int64 = g.shout\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::MethodCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "shout".to_string(),
        vec![]
      )
    );
  }

  #[test]
  fn or_assign_parses_to_stmt_or_assign() {
    let src = "message: String? = nil\nmessage ||= \"nobody here\"\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::OrAssign {
        name: "message".to_string(),
        default: s(Expr::StringLit("nobody here".to_string())),
      }))
    );
  }

  #[test]
  fn and_assign_parses_to_stmt_and_assign() {
    let src = "g: Greeter? = nil\ng &&= Greeter.new(\"upgraded\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::AndAssign {
        name: "g".to_string(),
        value: s(Expr::New(
          "Greeter".to_string(),
          vec![s(Expr::StringLit("upgraded".to_string()))]
        )),
      }))
    );
  }

  #[test]
  fn or_assign_on_a_non_ident_target_is_a_parse_error_not_a_panic() {
    let src = "arr[0] ||= 1\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  #[test]
  fn nullable_worked_example_parses_end_to_end() {
    let src = "class Greeter\n  name: String\n\n  fn initialize(name: String): Void do\n    @name = name\n  end\n\n  fn shout: String do\n    @name + \"!\"\n  end\nend\n\nfn find_greeter(id: Int64): Greeter? do\n  if id == 1 do\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\nfn greet(id: Int64): String do\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 5);
  }

  // Plan 44 (symbols).

  #[test]
  fn symbol_literal_parses_to_expr_symbol_lit() {
    let src = ":foo\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::SymbolLit("foo".to_string())))))
    );
  }

  #[test]
  fn symbol_typed_let_parses() {
    let src = "x: Symbol = :foo\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "x".to_string(),
        ty: "Symbol".to_string(),
        value: s(Expr::SymbolLit("foo".to_string())),
      }))
    );
  }

  #[test]
  fn ordinary_spaced_type_annotation_still_parses_identically() {
    let src = "x: Int64 = 1\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "x".to_string(),
        ty: "Int64".to_string(),
        value: s(Expr::Int(1)),
      }))
    );
  }

  #[test]
  fn symbol_keyed_hash_literal_parses() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82}\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::HashLit(vec![
        (s(Expr::SymbolLit("alice".to_string())), s(Expr::Int(90))),
        (s(Expr::SymbolLit("bob".to_string())), s(Expr::Int(82))),
      ])
    );
  }

  #[test]
  fn symbols_worked_example_parses_end_to_end() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo do\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar do\n  puts 1\nelse\n  puts 0\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 6);
  }

  // Plan 45 (stdlib strings and I/O).

  #[test]
  fn dot_read_method_call_parses_despite_read_being_reserved_by_class_field_sugar() {
    // Plan 33's `read <name>: <Type>` class-field sugar already
    // reserves `read` as a distinct terminal, globally — `.read` (e.g.
    // `File.read(path)`) needs `CallMethodName` to widen the
    // method-name slot at every call site, or this fails to parse.
    let src = "content: String = File.read(\"x.txt\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "content".to_string(),
        ty: "String".to_string(),
        value: s(Expr::MethodCall(
          Box::new(s(Expr::Ident("File".to_string()))),
          "read".to_string(),
          vec![s(Expr::StringLit("x.txt".to_string()))],
        )),
      }))
    );
  }

  #[test]
  fn plan_45_worked_example_parses_end_to_end() {
    let src = "input: String = \"hello world foo\"\nupper: String = input.upcase\nFile.write(\"plan45_demo.txt\", upper)\nreadback: String = File.read(\"plan45_demo.txt\")\nputs readback\nn: Int64 = readback.split_count(\" \")\nputs n\nwords: Array[String] = readback.split(\" \")\ni: Int64 = 0\nwhile i < n do\n  puts words[i]\n  i += 1\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 10);
  }

  // Plan 47 (REPL and test framework).

  #[test]
  fn test_block_parses_into_item_test() {
    let src = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 1);
    let Item::Test { description, body } = &program.items[0] else {
      panic!("expected an Item::Test, got {:?}", program.items[0]);
    };
    assert_eq!(description, "addition works");
    assert_eq!(body.len(), 1);
  }

  #[test]
  fn assert_call_rewrites_its_trailing_offset_into_a_file_line_string() {
    // `assert(...)` is on line 2 (1-based) of this named source.
    let src = "fn f(): Void do\n  assert(true)\nend\n";
    let program = parse_named(src, "t.em").expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item");
    };
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "assert".to_string(),
        vec![
          s(Expr::Bool(true)),
          s(Expr::StringLit("t.em:2".to_string()))
        ]
      ))))]
    );
  }

  #[test]
  fn assert_eq_call_rewrites_its_trailing_offset_into_a_file_line_string() {
    let src = "assert_eq(3, 1 + 1)\n";
    let program = parse_named(src, "math_test.em").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "assert_eq".to_string(),
        vec![
          s(Expr::Int(3)),
          s(Expr::Add(
            Box::new(s(Expr::Int(1))),
            Box::new(s(Expr::Int(1)))
          )),
          s(Expr::StringLit("math_test.em:1".to_string())),
        ]
      )))))
    );
  }

  #[test]
  fn assert_inside_a_nested_block_still_gets_its_own_correct_line() {
    // The rewrite pass must recurse into `if`/`while`/etc. bodies, not
    // just top-level statements — this asserts on line 3.
    let src = "x: Int64 = 1\nif x == 1 do\n  assert(x == 1)\nend\n";
    let program = parse_named(src, "nested.em").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::If { then_branch, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected an if statement");
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    } = &then_branch[0]
    else {
      panic!("expected an assert call");
    };
    assert_eq!(args[1], Expr::StringLit("nested.em:3".to_string()));
  }

  #[test]
  fn math_test_worked_example_parses_with_two_test_blocks() {
    let src = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n\ntest \"addition is broken on purpose\" do\n  assert_eq(3, 1 + 1)\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 2);
    assert!(matches!(program.items[0], Item::Test { .. }));
    assert!(matches!(program.items[1], Item::Test { .. }));
  }

  // Plan 22 (sema diagnostic spans) — leaf-ast-spans.

  #[test]
  fn add_rhs_span_points_at_the_real_second_b_not_the_parameter_or_the_whole_fn() {
    // This plan's own concrete-proof program (Decision log): the `rhs`
    // of `a + b` inside `add`'s body must carry the real byte offset of
    // the *second* `b` in the source — the one actually used in `a +
    // b`, not the first `b` (the `b: String` parameter declaration) and
    // not some placeholder/whole-document span.
    let src = "fn add(a: Int64, b: String): Int64 do\n  a + b\nend";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Add(_, rhs),
        ..
      }),
      ..
    } = &f.body[0]
    else {
      panic!("expected `a + b` as add's only statement, got {:?}", f.body);
    };
    let second_b = src
      .match_indices('b')
      .nth(1)
      .expect("source has two occurrences of 'b'")
      .0;
    assert_eq!(rhs.span, (second_b, second_b + 1));
  }

  #[test]
  fn spans_are_non_degenerate_for_both_multi_and_single_character_tokens() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Add(lhs, rhs),
        span: add_span,
      }),
      ..
    } = &f.body[0]
    else {
      panic!("expected `a + b` as add's only statement, got {:?}", f.body);
    };
    // The whole `a + b` expression spans "a + b" — 5 real bytes, a
    // genuinely multi-character span.
    assert!(
      add_span.1 > add_span.0,
      "multi-character span must be non-degenerate: {add_span:?}"
    );
    assert_eq!(
      *add_span,
      (src.find("a + b").unwrap(), src.find("a + b").unwrap() + 5)
    );
    // `a` and `b` are each a single-character token: span.1 ==
    // span.0 + 1, never wider and never degenerate (span.1 == span.0).
    assert_eq!(lhs.span.1, lhs.span.0 + 1);
    assert_eq!(rhs.span.1, rhs.span.0 + 1);
  }

  #[test]
  fn parses_to_an_ast_structurally_equal_before_and_after_this_plan_modulo_spans() {
    // Regression (AC3): every prior worked example in this file still
    // parses; this one specifically proves a real, non-`FUNC_ONLY`
    // program's `.node` shape survives completely unchanged — real
    // spans differ per-node (never asserted equal to each other here),
    // but `Spanned<T>`'s own `PartialEq` (node-only, see ast.rs) is
    // exactly what makes this comparison possible without hand-deriving
    // every nested byte offset.
    let program = parse(HELLO_EM).expect("hello.em should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected the add function");
    };
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("a".into()))),
        Box::new(s(Expr::Ident("b".into())))
      ))))]
    );
  }

  // Plan 52 (algebraic data types and exhaustive pattern matching).

  const SHAPE_ENUM_SRC: &str =
    "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n";

  #[test]
  fn enum_declaration_parses_into_item_enum() {
    let program = parse(SHAPE_ENUM_SRC).expect("should parse");
    let Item::Enum(def) = &program.items[0] else {
      panic!("expected an Item::Enum, got {:?}", program.items[0]);
    };
    assert_eq!(def.name, "Shape");
    assert_eq!(
      def.variants,
      vec![
        EnumVariant {
          name: "Circle".into(),
          fields: vec!["Float64".into()],
        },
        EnumVariant {
          name: "Square".into(),
          fields: vec!["Float64".into()],
        },
        EnumVariant {
          name: "Rectangle".into(),
          fields: vec!["Float64".into(), "Float64".into()],
        },
      ]
    );
  }

  #[test]
  fn case_over_variant_patterns_parses_arms_with_bindings_in_source_order() {
    let src = "match circle do\n  Circle(r) do\n    puts r\n  end\n  Square(s) do\n    puts s\n  end\n  Rectangle(w, h) do\n    puts w\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case { arms, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a case statement, got {:?}", program.items[0]);
    };
    assert_eq!(arms.len(), 3);
    assert_eq!(
      arms[0].0,
      CasePattern::Variant {
        name: "Circle".into(),
        bindings: vec!["r".into()],
      }
    );
    assert_eq!(
      arms[1].0,
      CasePattern::Variant {
        name: "Square".into(),
        bindings: vec!["s".into()],
      }
    );
    assert_eq!(
      arms[2].0,
      CasePattern::Variant {
        name: "Rectangle".into(),
        bindings: vec!["w".into(), "h".into()],
      }
    );
  }

  #[test]
  fn variant_construction_parses_as_an_ordinary_call() {
    // Decision log: no new grammar production for construction — the
    // parser can't tell "call a function" from "construct a variant"
    // apart at all; that's entirely sema's job. `Circle(2.0)` parses
    // exactly like any other bare function call.
    let program = parse("Circle(2.0)\n").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "Circle".to_string(),
        vec![s(Expr::Float(2.0))]
      )))))
    );
  }

  #[test]
  fn plan_52_worked_example_parses_end_to_end() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\n\nmatch circle do\n  Circle(r) do\n    puts r\n  end\n  Square(s) do\n    puts s\n  end\n  Rectangle(w, h) do\n    puts w\n  end\nend\n";
    let program = parse(src).expect("worked example should parse");
    assert!(matches!(program.items[0], Item::Enum(_)));
    assert!(matches!(program.items[1], Item::Stmt(_)));
    assert!(matches!(program.items[2], Item::Stmt(_)));
  }

  // Plan 53 (Result type and error propagation).

  #[test]
  fn ok_and_err_construction_parse_to_their_own_expr_variants() {
    let program = parse("x: Result[Int64, String] = Ok(42)\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(ty, "Result[Int64, String]");
    assert_eq!(value.node, Expr::Ok(Box::new(s(Expr::Int(42)))));

    let program = parse("y: Result[Int64, String] = Err(\"bad\")\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      value.node,
      Expr::Err(Box::new(s(Expr::StringLit("bad".to_string()))))
    );
  }

  #[test]
  fn postfix_try_parses_to_expr_try() {
    let program = parse("n: Int64 = parse_int(s)?\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      value.node,
      Expr::Try(Box::new(s(Expr::Call(
        "parse_int".to_string(),
        vec![s(Expr::Ident("s".to_string()))]
      ))))
    );
  }

  #[test]
  fn ok_err_match_form_parses_to_stmt_match_result() {
    let src =
      "match result do\n  Ok(v) do\n    puts v\n  end\n  Err(e) do\n    puts e\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node:
        Stmt::MatchResult {
          ok_var,
          ok_body,
          err_var,
          err_body,
          ..
        },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected a MatchResult statement, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(ok_var, "v");
    assert_eq!(err_var, "e");
    assert_eq!(ok_body.len(), 1);
    assert_eq!(err_body.len(), 1);
  }

  // Plan 54 (actor declarations and isolated heaps).

  // Plan 55's Decision log tightened this worked example (originally
  // authored under plan 54, before that rule existed): an actor
  // method other than `initialize` may no longer declare a return
  // type — `value` now prints `@count` itself (`puts @count`) rather
  // than returning it for a top-level `puts a.value` to print.
  const COUNTER_ACTOR_EXAMPLE: &str = "actor Counter\n  count: Int64\n\n  fn initialize(start: Int64): Void do\n    @count = start\n  end\n\n  fn increment: Void do\n    @count = @count + 1\n  end\n\n  fn value: Void do\n    puts @count\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n\na.increment\na.increment\nb.increment\n\na.value\nb.value\n";

  #[test]
  fn actor_worked_example_parses_into_expected_shapes() {
    let program = parse(COUNTER_ACTOR_EXAMPLE).expect("worked example should parse");
    let Item::Actor(a) = &program.items[0] else {
      panic!("expected Item::Actor, got {:?}", program.items[0]);
    };
    assert_eq!(a.name, "Counter");
    assert_eq!(a.fields.len(), 1);
    assert_eq!(a.fields[0].name, "count");
    assert_eq!(a.methods.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected a Let statement, got {:?}", program.items[1]);
    };
    assert_eq!(
      value.node,
      Expr::Spawn("Counter".to_string(), vec![s(Expr::Int(0))])
    );

    // `a.increment`/`a.value` parse as the *existing* Expr::MethodCall/
    // Stmt::Expr shapes — no new AST node for the call sites themselves.
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::MethodCall(..),
        ..
      }),
      ..
    }) = &program.items[3]
    else {
      panic!(
        "expected a MethodCall statement, got {:?}",
        program.items[3]
      );
    };
  }

  #[test]
  fn actor_inheritance_is_a_real_parse_error() {
    let src = "actor Dog < Animal\nend\n";
    assert!(
      parse(src).is_err(),
      "actor grammar has no superclass clause at all"
    );
  }

  #[test]
  fn a_class_spawn_called_and_an_actor_new_called_both_parse_at_this_leaf() {
    // Grammar-only leaf: rejecting these two shapes is leaf-sema-actor's
    // job, not this one's — both must parse successfully here.
    let src1 = "class Foo\nend\n\nf: Foo = Foo.spawn()\n";
    assert!(parse(src1).is_ok());
    let src2 = "actor Bar\nend\n\nb: Bar = Bar.new()\n";
    assert!(parse(src2).is_ok());
  }

  #[test]
  fn plan_53_worked_example_parses_end_to_end() {
    let src = "fn parse_int(s: String): Result[Int64, String] do\n  if is_valid_int(s) do\n    return Ok(parse_digits(s))\n  end\n  return Err(\"not a number\")\nend\n\nfn try_parse(s: String): Result[Int64, String] do\n  n: Int64 = parse_int(s)?\n  return Ok(n * 2)\nend\n\nresult: Result[Int64, String] = try_parse(\"21\")\nmatch result do\n  Ok(v) do\n    puts v\n  end\n  Err(e) do\n    puts e\n  end\nend\n";
    let program = parse(src).expect("worked example should parse");
    assert!(matches!(program.items[0], Item::Function(_)));
    assert!(matches!(program.items[1], Item::Function(_)));
    assert!(matches!(program.items[2], Item::Stmt(_)));
    assert!(matches!(
      program.items[3],
      Item::Stmt(Spanned {
        node: Stmt::MatchResult { .. },
        ..
      })
    ));
  }

  // Plan 59 (C FFI).

  #[test]
  fn unsafe_extern_c_block_parses_into_an_extern_item_with_both_fns_populated() {
    let src =
      "unsafe extern \"C\" {\n  fn llabs(x: Int64): Int64\n  fn strlen(s: String): Int64\n}\n";
    let program = parse(src).expect("should parse");
    let Item::Extern(block) = &program.items[0] else {
      panic!("expected Item::Extern, got {:?}", program.items[0]);
    };
    assert_eq!(block.abi, "C");
    assert_eq!(block.fns.len(), 2);
    assert_eq!(block.fns[0].name, "llabs");
    assert_eq!(block.fns[0].params[0].name, "x");
    assert_eq!(block.fns[0].params[0].ty, "Int64");
    assert_eq!(block.fns[0].return_type, "Int64");
    assert_eq!(block.fns[1].name, "strlen");
    assert_eq!(block.fns[1].params[0].ty, "String");
  }

  #[test]
  fn extern_c_block_without_the_leading_unsafe_keyword_is_a_real_parse_error() {
    let src = "extern \"C\" {\n  fn llabs(x: Int64): Int64\n}\n";
    assert!(
      parse(src).is_err(),
      "`unsafe` is a mandatory, grammar-level marker, not a sema-optional one"
    );
  }

  // Plan 60 (distributed, location-transparent actors).

  #[test]
  fn dot_remote_parses_into_an_expr_remote_node() {
    let src =
      "actor Counter\nend\n\nc: Counter = Counter.remote(\"127.0.0.1:9000\", \"counter1\")\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected a top-level Let, got {:?}", program.items[1]);
    };
    assert!(matches!(value.node, Expr::Remote { .. }));
  }

  #[test]
  fn dot_register_parses_via_the_existing_ordinary_method_call_grammar() {
    let src =
      "actor Counter\nend\n\nc: Counter = Counter.spawn()\nc.register(\"counter1\", 9000)\n";
    let program = parse(src).expect("should parse");
    assert!(matches!(
      &program.items[2],
      Item::Stmt(Spanned {
        node: Stmt::Expr(Spanned {
          node: Expr::MethodCall(_, method, _),
          ..
        }),
        ..
      }) if method == "register"
    ));
  }

  #[test]
  fn comptime_prefixed_def_parses_with_is_comptime_true() {
    let src = "comptime fn fact(n: Int64): Int64 do\n  n\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert!(f.is_comptime);
  }

  #[test]
  fn an_ordinary_def_has_is_comptime_false() {
    let src = "fn fact(n: Int64): Int64 do\n  n\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert!(!f.is_comptime);
  }

  #[test]
  fn comptime_expr_parses_as_a_top_level_lets_value() {
    let src = "FACT10: Int64 = comptime fact(10)\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level Let, got {:?}", program.items[0]);
    };
    assert!(matches!(value.node, Expr::Comptime(_)));
  }

  #[test]
  fn comptime_expr_parses_as_array_news_size_argument() {
    let src = "a: Array[Int64] = Array.new(comptime fact(5))\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level Let, got {:?}", program.items[0]);
    };
    let Expr::ArrayNew(size) = &value.node else {
      panic!("expected Expr::ArrayNew, got {:?}", value.node);
    };
    assert!(matches!(size.node, Expr::Comptime(_)));
  }

  #[test]
  fn comptime_expr_parses_as_a_bare_top_level_statement() {
    let src = "comptime fact(5)\n";
    let program = parse(src).expect("should parse");
    assert!(matches!(
      &program.items[0],
      Item::Stmt(Spanned {
        node: Stmt::Expr(Spanned {
          node: Expr::Comptime(_),
          ..
        }),
        ..
      })
    ));
  }

  #[test]
  fn comptime_binds_tighter_than_add_the_same_tier_as_unary_minus() {
    // `comptime` binds at `UnaryExpr`'s tight tier (see this plan's
    // Decision log for why any looser tier is genuinely ambiguous), so
    // `comptime a + b` parses as `Add(Comptime(a), b)`, not
    // `Comptime(Add(a, b))` — the same precedence `-a + b` already has.
    let src = "x: Int64 = comptime a + b\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level Let, got {:?}", program.items[0]);
    };
    let Expr::Add(lhs, _rhs) = &value.node else {
      panic!("expected Expr::Add at the top, got {:?}", value.node);
    };
    assert!(matches!(lhs.node, Expr::Comptime(_)));
  }

  #[test]
  fn derive_comparable_clause_parses_into_class_def_derive() {
    let src = "class Point derive Comparable\n  x: Int64\n  y: Int64\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class, got {:?}", program.items[0]);
    };
    assert_eq!(c.derive, Some("Comparable".to_string()));
  }

  #[test]
  fn a_class_with_no_derive_clause_parses_with_none() {
    let src = "class Point\n  x: Int64\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class, got {:?}", program.items[0]);
    };
    assert_eq!(c.derive, None);
  }

  #[test]
  fn expand_derives_synthesizes_read_accessors_and_an_alphabetically_ordered_eq() {
    let src = "class Point derive Comparable\n  x: Int64\n  y: Int64\nend\n";
    let mut program = parse(src).expect("should parse");
    crate::expand_derives(&mut program).expect("should expand");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert!(c
      .methods
      .iter()
      .any(|m| m.name == "x" && m.params.is_empty()));
    assert!(c
      .methods
      .iter()
      .any(|m| m.name == "y" && m.params.is_empty()));
    let eq = c
      .methods
      .iter()
      .find(|m| m.name == "==")
      .expect("expected a synthesized ==");
    assert_eq!(
      eq.params,
      vec![Param {
        name: "other".to_string(),
        ty: "Point".to_string(),
        default: None
      }]
    );
    assert_eq!(eq.return_type, "Boolean");
    let expected_body = vec![s(Stmt::Return(Some(s(Expr::And(
      Box::new(s(Expr::Compare(
        Box::new(s(Expr::InstanceVar("x".to_string()))),
        CompareOp::Eq,
        Box::new(s(Expr::MethodCall(
          Box::new(s(Expr::Ident("other".to_string()))),
          "x".to_string(),
          vec![],
        ))),
      ))),
      Box::new(s(Expr::Compare(
        Box::new(s(Expr::InstanceVar("y".to_string()))),
        CompareOp::Eq,
        Box::new(s(Expr::MethodCall(
          Box::new(s(Expr::Ident("other".to_string()))),
          "y".to_string(),
          vec![],
        ))),
      ))),
    )))))];
    assert_eq!(eq.body, expected_body);
  }

  #[test]
  fn expand_derives_rejects_an_unknown_derive_target() {
    let src = "class Point derive Serializable\n  x: Int64\nend\n";
    let mut program = parse(src).expect("should parse");
    let err = crate::expand_derives(&mut program).expect_err("should reject");
    assert!(err.contains("Serializable"));
  }

  #[test]
  fn expand_derives_rejects_a_class_that_already_hand_writes_eq() {
    let src = "class Point derive Comparable\n  x: Int64\n  fn ==(other: Point): Boolean do\n    true\n  end\nend\n";
    let mut program = parse(src).expect("should parse");
    let err = crate::expand_derives(&mut program).expect_err("should reject");
    assert!(err.contains("Point"));
    assert!(err.contains("=="));
  }

  #[test]
  fn expand_derives_on_a_subclass_covers_inherited_fields_too() {
    let src = "class Point derive Comparable\n  x: Int64\n  y: Int64\nend\n\nclass Point3D < Point derive Comparable\n  z: Int64\nend\n";
    let mut program = parse(src).expect("should parse");
    crate::expand_derives(&mut program).expect("should expand");
    let Item::Class(c) = &program.items[1] else {
      panic!("expected the subclass");
    };
    let eq = c
      .methods
      .iter()
      .find(|m| m.name == "==")
      .expect("expected a synthesized ==");
    let Stmt::Return(Some(body)) = &eq.body[0].node else {
      panic!("expected a Return");
    };
    let mut fields_compared = Vec::new();
    let mut cur = body;
    loop {
      match &cur.node {
        Expr::And(l, r) => {
          if let Expr::Compare(lhs, _, _) = &r.node {
            if let Expr::InstanceVar(n) = &lhs.node {
              fields_compared.push(n.clone());
            }
          }
          cur = l;
        }
        Expr::Compare(lhs, _, _) => {
          if let Expr::InstanceVar(n) = &lhs.node {
            fields_compared.push(n.clone());
          }
          break;
        }
        _ => break,
      }
    }
    fields_compared.sort();
    assert_eq!(
      fields_compared,
      vec!["x".to_string(), "y".to_string(), "z".to_string()]
    );
  }

  #[test]
  fn expand_derives_is_a_strict_no_op_for_a_class_with_no_derive_clause() {
    let src = "class Point\n  x: Int64\nend\n";
    let mut program = parse(src).expect("should parse");
    let before = program.clone();
    crate::expand_derives(&mut program).expect("should be a no-op");
    assert_eq!(program, before);
  }

  // Plan 62 (design-by-contract).

  #[test]
  fn requires_and_ensures_clauses_parse_with_the_expected_ast_shape_and_text() {
    let src = "fn divide(a: Int64, b: Int64): Int64\n  requires b != 0\n  ensures result * b <= a\n  do\n  return a / b\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function");
    };
    assert_eq!(f.requires.len(), 1);
    assert_eq!(
      f.requires[0].expr.node,
      Expr::Compare(
        Box::new(s(Expr::Ident("b".to_string()))),
        CompareOp::Ne,
        Box::new(s(Expr::Int(0))),
      )
    );
    assert_eq!(f.requires[0].text, "b != 0");
    assert_eq!(f.ensures.len(), 1);
    assert_eq!(f.ensures[0].text, "result * b <= a");
  }

  #[test]
  fn a_function_with_no_contracts_parses_with_both_lists_empty() {
    let src = "fn add(a: Int64, b: Int64): Int64 do\n  return a + b\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function");
    };
    assert!(f.requires.is_empty());
    assert!(f.ensures.is_empty());
  }

  #[test]
  fn requires_used_as_an_ordinary_identifier_is_a_real_parse_error() {
    let src = "requires: Int64 = 1\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn ensures_used_as_an_ordinary_identifier_is_a_real_parse_error() {
    let src = "ensures: Int64 = 1\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn a_requires_clause_on_a_class_method_is_a_real_parse_error_not_a_sema_diagnostic() {
    let src = "class Point\n  x: Int64\n\n  fn get_x(): Int64 do\n    requires true\n    return @x\n  end\nend\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn pure_def_parses_with_is_pure_true() {
    let src = "pure fn square(x: Int64): Int64 do\n  return x * x\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function");
    };
    assert!(f.is_pure);
  }

  #[test]
  fn an_ordinary_def_parses_with_is_pure_false() {
    let src = "fn square(x: Int64): Int64 do\n  return x * x\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function");
    };
    assert!(!f.is_pure);
  }

  #[test]
  fn pure_def_parses_identically_inside_a_class_body() {
    let src = "class Point\n  x: Int64\n\n  pure fn get_x(): Int64 do\n    return @x\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert!(c.methods[0].is_pure);
  }

  #[test]
  fn pure_and_comptime_compose_on_the_same_function() {
    let src = "pure comptime fn square(x: Int64): Int64 do\n  return x * x\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function");
    };
    assert!(f.is_pure);
    assert!(f.is_comptime);
  }

  #[test]
  fn pure_used_as_an_ordinary_identifier_is_a_real_parse_error() {
    let src = "pure: Int64 = 1\n";
    assert!(parse(src).is_err());
  }

  // Plan 70 (enumerable stdlib completion): block-attached-call parsing
  // + `hoist_enumerable_blocks`.

  #[test]
  fn a_block_attached_select_call_parses_as_a_lets_rhs() {
    let src =
      "nums: Array[Int64] = [1, 2, 3]\nevens: Array[Int64] = nums.select do |x: Int64| x > 1 end\n";
    let program = parse(src).expect("should parse");
    // The block is hoisted to a fresh top-level `Proc` `Let` immediately
    // before the `evens` statement, so there are 4 top-level items, not
    // 2: `nums`, the hoisted proc, `evens`, in that order.
    assert_eq!(program.items.len(), 3);
    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected the hoisted Proc Let, got {:?}", program.items[1]);
    };
    assert_eq!(name, "__enum_blk_0");
    assert_eq!(ty, "Proc");
    let Item::Stmt(Spanned {
      node:
        Stmt::Let {
          name: evens_name,
          value:
            Spanned {
              node: Expr::MethodCall(_, method, args),
              ..
            },
          ..
        },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the `evens` Let, got {:?}", program.items[2]);
    };
    assert_eq!(evens_name, "evens");
    assert_eq!(method, "select");
    assert_eq!(
      args,
      &vec![Spanned::synthetic(Expr::Ident("__enum_blk_0".to_string()))]
    );
  }

  #[test]
  fn a_block_attached_map_call_infers_its_blocks_return_type_from_the_body() {
    let src =
      "nums: Array[Int64] = [1, 2, 3]\ndoubled: Array[Int64] = nums.map do |x: Int64| x * 2 end\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, value, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected the hoisted Proc Let, got {:?}", program.items[1]);
    };
    assert_eq!(ty, "Proc");
    let Expr::Lambda { return_type, .. } = &value.node else {
      panic!("expected a Lambda value, got {:?}", value.node);
    };
    assert_eq!(return_type, "Int64");
  }

  #[test]
  fn a_block_attached_reduce_call_parses_with_args_and_a_block_together() {
    let src = "nums: Array[Int64] = [1, 2, 3]\ntotal: Int64 = nums.reduce(0) do |acc: Int64, x: Int64| acc + x end\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 3);
    let Item::Stmt(Spanned {
      node:
        Stmt::Let {
          value:
            Spanned {
              node: Expr::MethodCall(_, method, args),
              ..
            },
          ..
        },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the `total` Let, got {:?}", program.items[2]);
    };
    assert_eq!(method, "reduce");
    // `0` (the explicit initial-value arg) plus the hoisted Proc name —
    // the block did not silently disappear, and the initial value's own
    // position (before the block) is preserved.
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], Spanned::synthetic(Expr::Int(0)));
  }

  #[test]
  fn a_bare_no_parens_block_attached_call_parses_as_a_statement_too() {
    let src = "nums: Array[Int64] = [1, 2, 3]\nnums.each do |x: Int64| puts x end\n";
    let program = parse(src).expect("should parse");
    // `nums`, the hoisted proc, the `.each` statement.
    assert_eq!(program.items.len(), 3);
  }

  #[test]
  fn a_block_whose_return_type_cannot_be_inferred_is_a_real_disclosed_parse_error() {
    // `File.read` isn't a top-level function this inferencer's own
    // small `Call`-arm table can look up, and it isn't one of the
    // arithmetic/comparison/literal/param shapes it covers either — a
    // real, disclosed inference-coverage boundary, not a crash.
    let src = "nums: Array[Int64] = [1, 2, 3]\ndoubled: Array[Int64] = nums.map do |x: Int64| File.read(\"a\") end\n";
    let err = parse(src).expect_err("should fail to infer the block's return type");
    assert!(!err.is_empty());
  }

  #[test]
  fn a_block_attached_call_inside_an_if_body_is_left_unhoisted_and_reports_semas_own_error() {
    // Deliberately narrow scope (this plan's own Decision log): only a
    // top-level statement's own, direct value is hoisted — nested
    // inside an `if` body, the block is left as a raw, un-hoisted
    // `Expr::Lambda`, which still parses (this plan's grammar change is
    // receiver/context-agnostic) but is `emerald-sema`'s problem, not
    // this pass's.
    let src = "nums: Array[Int64] = [1, 2, 3]\nif true do\n  evens: Array[Int64] = nums.select do |x: Int64| x > 1 end\nend\n";
    let program = parse(src).expect("should still parse");
    let Item::Stmt(Spanned {
      node: Stmt::If { then_branch, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected the `if`, got {:?}", program.items[1]);
    };
    let Stmt::Let { value, .. } = &then_branch[0].node else {
      panic!("expected a Let");
    };
    assert!(
      matches!(&value.node, Expr::MethodCall(_, _, args) if matches!(args.last().map(|a| &a.node), Some(Expr::Lambda { .. })))
    );
  }

  // Plan 87 (do...end exclusive: braces removed as block syntax).

  #[test]
  fn plan_87_do_end_block_attaches_in_statement_position() {
    let src = "nums: Array[Int64] = [1, 2, 3]\nnums.each do |x: Int64| puts x end\n";
    let program =
      parse(src).expect("a bare, no-parens do...end-attached call must parse as a statement");
    // `nums`, the hoisted proc, the `.each` statement.
    assert_eq!(program.items.len(), 3);
  }

  #[test]
  fn plan_87_do_end_block_attaches_in_expression_position() {
    let src =
      "nums: Array[Int64] = [1, 2, 3]\nevens: Array[Int64] = nums.select do |x: Int64| x > 1 end\n";
    let program = parse(src).expect("a do...end-attached call must parse as a Let's RHS");
    assert_eq!(program.items.len(), 3);
    let Item::Stmt(Spanned {
      node: Stmt::Let { name, value, .. },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the `evens` Let, got {:?}", program.items[2]);
    };
    assert_eq!(name, "evens");
    assert!(matches!(&value.node, Expr::MethodCall(_, method, _) if method == "select"));
  }

  #[test]
  fn plan_87_chain_of_at_least_three_do_end_calls_binds_tight_and_chains() {
    // Chain-local tight binding (this plan's Decision log): each
    // `do...end` block binds to exactly the `.method` call it's written
    // on, and the whole result is itself a legal receiver for the next
    // `.method` in the chain — three links here, `.select`, `.map`,
    // `.count`, none of them parenthesized.
    let src = "nums: Array[Int64] = [1, 2, 3]\nresult: Int64 = nums.select do |x: Int64| x > 1 end.map do |x: Int64| x * 2 end.count do |x: Int64| x > 1 end\n";
    let program = parse(src)
      .expect("a do...end chain of three .method calls should parse, each block binding tight to its own call");
    assert_eq!(program.items.len(), 3);
    let Item::Stmt(Spanned {
      node: Stmt::Let { name, value, .. },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the `result` Let, got {:?}", program.items[2]);
    };
    assert_eq!(name, "result");
    // Outermost link (`.count`): this is the top-level statement's own
    // direct value, so `hoist_enumerable_blocks` rewrites its trailing
    // block into a fresh, named `Proc` `Ident` — see that pass's own
    // Decision log for why only this outermost link qualifies.
    let Expr::MethodCall(recv2, method2, args2) = &value.node else {
      panic!(
        "expected the outermost `.count` MethodCall, got {:?}",
        value.node
      );
    };
    assert_eq!(method2, "count");
    assert!(
      matches!(args2.last().map(|a| &a.node), Some(Expr::Ident(n)) if n.starts_with("__enum_blk_"))
    );
    // Middle link (`.map`): nested inside a receiver, so it keeps its
    // own raw, un-hoisted `Expr::Lambda` block (out of that pass's
    // deliberately narrow, top-level-only scope).
    let Expr::MethodCall(recv1, method1, args1) = &recv2.node else {
      panic!(
        "expected the middle `.map` MethodCall, got {:?}",
        recv2.node
      );
    };
    assert_eq!(method1, "map");
    assert!(matches!(
      args1.last().map(|a| &a.node),
      Some(Expr::Lambda { .. })
    ));
    // Innermost link (`.select`) on the bare `nums` receiver — the base
    // case of `ChainCallExpr`'s own recursion.
    let Expr::MethodCall(recv0, method0, args0) = &recv1.node else {
      panic!(
        "expected the innermost `.select` MethodCall, got {:?}",
        recv1.node
      );
    };
    assert_eq!(method0, "select");
    assert_eq!(recv0.node, Expr::Ident("nums".to_string()));
    assert!(matches!(
      args0.last().map(|a| &a.node),
      Some(Expr::Lambda { .. })
    ));
  }

  #[test]
  fn plan_87_bare_block_attached_call_as_a_while_condition_is_a_real_parse_error() {
    // No parens around the block-attached `.select` call — this plan's
    // own condition-position restriction (Decision log) makes this a
    // genuine parse error (the grammar has no derivation for it at all,
    // once `CondPrimaryExpr` excludes a bare `ChainCallExpr`), not a
    // silent misparse and not a hang.
    let src = "nums: Array[Int64] = [1, 2, 3]\nwhile nums.select do |x: Int64| x > 100 end.length > 0 do\n  puts 1\nend\n";
    let err = parse(src)
      .expect_err("an unparenthesized block-attached call must not be usable as a while condition");
    assert!(!err.is_empty());
  }

  #[test]
  fn plan_87_bare_block_attached_call_as_an_if_condition_is_a_real_parse_error() {
    // Same restriction, `if` instead of `while` — both route through
    // the identical `CondExpr` nonterminal, so this must fail exactly
    // the same way.
    let src = "nums: Array[Int64] = [1, 2, 3]\nif nums.select do |x: Int64| x > 100 end.length > 0 do\n  puts 1\nend\n";
    assert!(
      parse(src).is_err(),
      "an unparenthesized block-attached call must not be usable as an if condition"
    );
  }

  #[test]
  fn plan_87_parenthesized_block_attached_call_is_legal_as_a_while_condition() {
    // The one legal escape hatch this plan adds: wrapping the block-
    // attached call in explicit parens closes off the ambiguity (see
    // `CondPrimaryExpr`'s own header comment) and lets the chain be
    // extended by one more, final, non-block `.length` call.
    let src = "nums: Array[Int64] = [1, 2, 3]\nwhile (nums.select do |x: Int64| x > 100 end).length > 0 do\n  puts 1\nend\n";
    parse(src).expect("a parenthesized block-attached call must be legal as a while condition");
  }
}
