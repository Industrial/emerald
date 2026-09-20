//! Plan 76's `import-export-module-visibility`: explicit `import`/
//! `export` symbol-level visibility, alongside (not replacing) plan
//! 23's file-based `require`. This module holds the parts of that
//! design that only need `emerald_parser`'s own AST (`Item`/`Stmt`/
//! `Expr`) — no type information, no `emerald-sema` dependency — so
//! both of this workspace's two require-resolution call sites
//! (`emerald-cli`'s own flattening `require.rs`, plan 23/46/48, and
//! `emerald-driver`'s graph-shaped `require_graph.rs`, plan 49) can
//! share the identical, single implementation instead of each hand-
//! rolling their own AST walk.
//!
//! ## Backward compatibility (plan 76's own Decision log)
//!
//! A file with zero `export` declarations exports **everything** —
//! `export_names` returns `None` for it, and every call site in this
//! module treats `None` as "no restriction, matches plan 23's original,
//! unrestricted `require`-merge behavior exactly." Only once a file
//! writes at least one `export` does it switch to explicit-export-only
//! for *that file* — every other, non-exporting file in the same
//! program is completely unaffected. This is what keeps `examples/
//! parallel/`, `examples/packages/`, and every pre-existing `Item::
//! Require`-using fixture compiling and running identically to before
//! this plan, with no changes to those files at all.
//!
//! ## What `export`/`import` actually restrict
//!
//! `export` never removes a declaration from the compiled program —
//! flattening still splices *every* item from a required file into the
//! merged `Program` (a private helper an exported function calls
//! internally must still be compiled). `export` only restricts which
//! *names* another file's own source is allowed to *reference*
//! directly: `find_violations` walks a file's own (pre-flatten) items
//! for identifier-shaped references (bare calls, `.new`/`.spawn`,
//! module-static calls, etc.) that resolve to a name declared in a
//! DIFFERENT file, and rejects any such reference whose target file (a)
//! declares at least one export and (b) didn't put that particular name
//! in its export set — unless the referencing file's own visibility set
//! (built from its `require`/`import` edges, see the two call sites'
//! own graph-walk code) already grants it.
//!
//! ## A disclosed scope limitation
//!
//! This walk covers every `Stmt`/`Expr` position an ordinary function/
//! method/module-level body can reference a cross-file name from
//! (calls, `.new`/`.spawn`/`.remote`/`.locate`, module-static calls,
//! lambda bodies, `rescue`/`case`/`for` bodies). It does **not** walk
//! `TypeExpr` positions (a `Param`'s declared type, a function's
//! return type, a class field's type) — a function whose *signature*
//! names an unexported class from another file, but whose *body* never
//! constructs or calls anything cross-file, is not caught by this
//! check. Real, not silently pretended otherwise: this is the design's
//! one deliberate scope cut, made to keep this plan's own AST walk a
//! single, generic-recursion pass rather than a second, parallel
//! type-annotation walker.

use crate::ast::{
  ActorDef, CasePattern, ClassDef, Expr, Function, Item, ModuleDef, Spanned, Stmt, StringPart,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The top-level name `item` declares — `None` for anything `export`/
/// `import` can never name (`Require`, `Import`, `Stmt`, `Test`/
/// `Property`/`Benchmark`, `Extern`, `Error`). Recurses through
/// `Item::Export` so callers never need to unwrap it themselves.
pub fn item_name(item: &Item) -> Option<&str> {
  match item {
    Item::Function(f) => Some(f.name.as_str()),
    Item::Class(c) => Some(c.name.as_str()),
    Item::Module(m) => Some(m.name.as_str()),
    Item::Actor(a) => Some(a.name.as_str()),
    Item::Enum(e) => Some(e.name.as_str()),
    Item::Interface(i) => Some(i.name.as_str()),
    Item::Export(inner) => item_name(inner),
    _ => None,
  }
}

/// Every top-level name one file's own (unflattened) `items` declares,
/// `export`-wrapped or not. Used both to build the whole-program
/// name -> defining-file map and to validate `import path { Name }`'s
/// `Name`s are real declarations in `path` (a real, clear "no such
/// top-level declaration" error otherwise — see the two call sites).
pub fn own_names(items: &[Item]) -> HashSet<String> {
  items
    .iter()
    .filter_map(item_name)
    .map(str::to_string)
    .collect()
}

/// `Some(exported_set)` once at least one `Item::Export` appears
/// anywhere in `items`; `None` when it doesn't — the exact predicate
/// this module's own backward-compatibility rule is built on (see the
/// module doc comment above).
pub fn export_names(items: &[Item]) -> Option<HashSet<String>> {
  let mut set = HashSet::new();
  let mut any_export = false;
  for item in items {
    if let Item::Export(inner) = item {
      any_export = true;
      if let Some(name) = item_name(inner) {
        set.insert(name.to_string());
      }
    }
  }
  if any_export {
    Some(set)
  } else {
    None
  }
}

/// Unwraps every `Item::Export` in place, recursively through nested
/// wrapping (the grammar never actually nests one, but this stays
/// correct even if a future grammar change did). The flattened
/// `Program` a require-merge produces compiles identically whether or
/// not its constituent declarations were exported — `export` is a
/// cross-file-visibility concern this module enforces separately, not
/// a change to what `emerald-sema`/`emerald-codegen` themselves see.
pub fn strip_exports(items: Vec<Item>) -> Vec<Item> {
  items.into_iter().map(strip_export).collect()
}

/// The single-item form of `strip_exports` above — `emerald-cli`'s
/// `require.rs`/`emerald-driver`'s `require_graph.rs` both build their
/// own splicing entries one source item at a time, so a per-item
/// unwrap is the shape they actually need.
pub fn strip_export(item: Item) -> Item {
  match item {
    Item::Export(inner) => strip_export(*inner),
    other => other,
  }
}

/// One real, clear visibility violation: `name` (referenced at `span`,
/// this file's own byte-offset span) resolves to a top-level
/// declaration in `defining_file`, which restricts its exports and
/// didn't include `name` — or the referencing file never required/
/// imported `defining_file` at all.
#[derive(Debug, Clone)]
pub struct VisibilityError {
  pub span: (usize, usize),
  pub name: String,
  pub defining_file: PathBuf,
}

/// Walks `items` (one file's own, unflattened, pre-export-stripped
/// items) for references to any name present in `foreign` (the
/// whole-program name -> defining-file map, already excluding this
/// file's own names) that isn't also present in `visible` (the set of
/// foreign names this file's own `require`/`import` edges grant it —
/// built by each call site's own graph walk, see `emerald-cli::require`
/// and `emerald_driver::require_graph`). Reports every violation found,
/// not just the first.
pub fn find_violations(
  items: &[Item],
  foreign: &HashMap<String, PathBuf>,
  visible: &HashSet<String>,
) -> Vec<VisibilityError> {
  let mut out = Vec::new();
  {
    let mut checker = Checker {
      foreign,
      visible,
      out: &mut out,
      top_locals: HashSet::new(),
    };
    for item in items {
      checker.walk_item(item);
    }
  }
  out
}

struct Checker<'a> {
  foreign: &'a HashMap<String, PathBuf>,
  visible: &'a HashSet<String>,
  out: &'a mut Vec<VisibilityError>,
  /// Plan 76: top-level `Item::Stmt`s share ONE flat scope across the
  /// whole file, in source order (`x: Int64 = 10` then a later
  /// `puts x` needs `x` visible) — mirrors `emerald-sema`'s own
  /// `top_env` accumulation exactly, so a locally-declared top-level
  /// binding never false-positives as a cross-file reference just
  /// because its name happens to collide with a restricted foreign
  /// one.
  top_locals: HashSet<String>,
}

impl<'a> Checker<'a> {
  fn check_name(&mut self, name: &str, span: (usize, usize), locals: &HashSet<String>) {
    if locals.contains(name) {
      return;
    }
    if let Some(file) = self.foreign.get(name) {
      if !self.visible.contains(name) {
        self.out.push(VisibilityError {
          span,
          name: name.to_string(),
          defining_file: file.clone(),
        });
      }
    }
  }

  fn walk_item(&mut self, item: &Item) {
    match item {
      Item::Function(f) => self.walk_function(f),
      Item::Class(c) => self.walk_class(c),
      Item::Module(m) => self.walk_module(m),
      Item::Actor(a) => self.walk_actor(a),
      Item::Enum(_) | Item::Interface(_) | Item::Newtype(_) => {}
      Item::Export(inner) => self.walk_item(inner),
      Item::Stmt(s) => {
        let mut locals = std::mem::take(&mut self.top_locals);
        self.walk_stmt(s, &mut locals);
        self.top_locals = locals;
      }
      Item::Require(_) | Item::Import { .. } | Item::Extern(_) | Item::Error => {}
      Item::Test { body, .. } | Item::Property { body, .. } | Item::Benchmark { body, .. } => {
        self.walk_stmts(body, &HashSet::new());
      }
    }
  }

  fn walk_class(&mut self, c: &ClassDef) {
    for m in &c.methods {
      self.walk_function(m);
    }
  }

  fn walk_module(&mut self, m: &ModuleDef) {
    for f in &m.methods {
      self.walk_function(f);
    }
  }

  fn walk_actor(&mut self, a: &ActorDef) {
    for m in &a.methods {
      self.walk_function(m);
    }
  }

  fn walk_function(&mut self, f: &Function) {
    let mut locals: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
    if let Some(blk) = &f.block_param {
      locals.insert(blk.clone());
    }
    if let Some(splat) = &f.splat_param {
      locals.insert(splat.name.clone());
    }
    self.walk_stmts(&f.body, &locals);
  }

  fn walk_stmts(&mut self, stmts: &[Spanned<Stmt>], locals: &HashSet<String>) {
    let mut scope = locals.clone();
    for s in stmts {
      self.walk_stmt(s, &mut scope);
    }
  }

  fn walk_stmt(&mut self, stmt: &Spanned<Stmt>, locals: &mut HashSet<String>) {
    match &stmt.node {
      Stmt::Let { name, value, .. } => {
        self.walk_expr(value, locals);
        locals.insert(name.clone());
      }
      Stmt::SetField { value, .. } => self.walk_expr(value, locals),
      Stmt::SetIndex {
        array,
        index,
        value,
      } => {
        self.walk_expr(array, locals);
        self.walk_expr(index, locals);
        self.walk_expr(value, locals);
      }
      Stmt::Assign { value, .. } => self.walk_expr(value, locals),
      Stmt::MultiAssign { values, .. } => {
        for v in values {
          self.walk_expr(v, locals);
        }
      }
      Stmt::If {
        cond,
        then_branch,
        else_branch,
      } => {
        self.walk_expr(cond, locals);
        self.walk_stmts(then_branch, locals);
        if let Some(e) = else_branch {
          self.walk_stmts(e, locals);
        }
      }
      Stmt::While { cond, body } => {
        self.walk_expr(cond, locals);
        self.walk_stmts(body, locals);
      }
      Stmt::Return(e) => {
        if let Some(e) = e {
          self.walk_expr(e, locals);
        }
      }
      Stmt::Break | Stmt::Next | Stmt::Retry => {}
      Stmt::Expr(e) => self.walk_expr(e, locals),
      Stmt::Raise(e) => self.walk_expr(e, locals),
      Stmt::Begin {
        body,
        rescues,
        ensure,
      } => {
        self.walk_stmts(body, locals);
        for r in rescues {
          let mut scope = locals.clone();
          scope.insert(r.var.clone());
          self.walk_stmts(&r.body, &scope);
        }
        if let Some(e) = ensure {
          self.walk_stmts(e, locals);
        }
      }
      Stmt::Case {
        scrutinee,
        arms,
        else_body,
      } => {
        self.walk_expr(scrutinee, locals);
        for (pattern, body) in arms {
          let mut scope = locals.clone();
          if let CasePattern::Values(vs) = pattern {
            for v in vs {
              self.walk_expr(v, locals);
            }
          } else if let CasePattern::Variant { bindings, .. } = pattern {
            for b in bindings {
              scope.insert(b.clone());
            }
          }
          self.walk_stmts(body, &scope);
        }
        if let Some(e) = else_body {
          self.walk_stmts(e, locals);
        }
      }
      Stmt::For {
        var,
        elements,
        body,
      } => {
        for e in elements {
          self.walk_expr(e, locals);
        }
        let mut scope = locals.clone();
        scope.insert(var.clone());
        self.walk_stmts(body, &scope);
      }
      Stmt::Yield(args) => {
        for a in args {
          self.walk_expr(a, locals);
        }
      }
      Stmt::ForRange {
        var,
        start,
        end,
        body,
        ..
      } => {
        self.walk_expr(start, locals);
        self.walk_expr(end, locals);
        let mut scope = locals.clone();
        scope.insert(var.clone());
        self.walk_stmts(body, &scope);
      }
      Stmt::MatchResult {
        scrutinee,
        ok_var,
        ok_body,
        err_var,
        err_body,
      } => {
        self.walk_expr(scrutinee, locals);
        let mut ok_scope = locals.clone();
        ok_scope.insert(ok_var.clone());
        self.walk_stmts(ok_body, &ok_scope);
        let mut err_scope = locals.clone();
        err_scope.insert(err_var.clone());
        self.walk_stmts(err_body, &err_scope);
      }
    }
  }

  fn walk_expr(&mut self, expr: &Spanned<Expr>, locals: &HashSet<String>) {
    let span = expr.span;
    match &expr.node {
      Expr::Ident(name) => self.check_name(name, span, locals),
      Expr::Int(_)
      | Expr::Float(_)
      | Expr::StringLit(_)
      | Expr::SymbolLit(_)
      | Expr::Bool(_)
      | Expr::InstanceVar(_) => {}
      Expr::Interpolate(parts) => {
        for part in parts {
          if let StringPart::Expr(e) = part {
            self.walk_expr(e, locals);
          }
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
      | Expr::Coalesce(l, r) => {
        self.walk_expr(l, locals);
        self.walk_expr(r, locals);
      }
      Expr::Compare(l, _, r) => {
        self.walk_expr(l, locals);
        self.walk_expr(r, locals);
      }
      Expr::Neg(e) | Expr::Not(e) | Expr::BitNot(e) | Expr::Comptime(e) => {
        self.walk_expr(e, locals)
      }
      Expr::Call(name, args) => {
        self.check_name(name, span, locals);
        for a in args {
          self.walk_expr(a, locals);
        }
      }
      Expr::CallKw(name, kwargs) => {
        self.check_name(name, span, locals);
        for (_, v) in kwargs {
          self.walk_expr(v, locals);
        }
      }
      Expr::New(name, args) => {
        self.check_name(name, span, locals);
        for a in args {
          self.walk_expr(a, locals);
        }
      }
      Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
        self.walk_expr(recv, locals);
        for a in args {
          self.walk_expr(a, locals);
        }
      }
      Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
        for e in elems {
          self.walk_expr(e, locals);
        }
      }
      Expr::Index(a, i) => {
        self.walk_expr(a, locals);
        self.walk_expr(i, locals);
      }
      Expr::Lambda { params, body, .. } => {
        let mut inner = locals.clone();
        for p in params {
          inner.insert(p.name.clone());
        }
        self.walk_stmts(body, &inner);
      }
      Expr::HashLit(pairs) => {
        for (k, v) in pairs {
          self.walk_expr(k, locals);
          self.walk_expr(v, locals);
        }
      }
      Expr::ArrayNew(size) => self.walk_expr(size, locals),
      Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => self.walk_expr(e, locals),
      Expr::Spawn(name, args) => {
        self.check_name(name, span, locals);
        for a in args {
          self.walk_expr(a, locals);
        }
      }
      Expr::Supervise(body) => self.walk_stmts(body, locals),
      Expr::Remote { class, addr, name } => {
        self.check_name(class, span, locals);
        self.walk_expr(addr, locals);
        self.walk_expr(name, locals);
      }
      Expr::Locate { class, key, args } => {
        self.check_name(class, span, locals);
        self.walk_expr(key, locals);
        for a in args {
          self.walk_expr(a, locals);
        }
      }
    }
  }
}

/// Small, shared helper both `emerald-cli::require` and
/// `emerald_driver::require_graph` use to build `find_violations`'
/// `foreign`/`visible` inputs for one file: given the whole program's
/// `name -> defining-file` map and one file's own set of directly
/// visible foreign names, filters `foreign` down to exclude `self_path`
/// itself (a file's own names are never "foreign" to it) before handing
/// both back for `find_violations` to consume.
pub fn foreign_names_excluding(
  all_names: &HashMap<String, PathBuf>,
  self_path: &Path,
) -> HashMap<String, PathBuf> {
  all_names
    .iter()
    .filter(|(_, file)| file.as_path() != self_path)
    .map(|(n, f)| (n.clone(), f.clone()))
    .collect()
}
