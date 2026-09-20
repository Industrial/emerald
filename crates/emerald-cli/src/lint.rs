//! `emerald lint <file.em>|<directory>` (plan-of-plans row 79,
//! `canonical-linter`) — a first-party static-analysis/style checker
//! for Emerald source, operating purely on `emerald-parser`'s AST (no
//! `emerald-sema` dependency: every rule below is checkable from
//! syntax shape alone, so `emerald lint` never runs the type checker
//! and never blocks on a program that merely fails to compile for
//! unrelated reasons — it reports whatever it can about whatever
//! parses).
//!
//! Four rules, each picked because it's a REAL, checkable consequence
//! of a feature this language actually has (not a generic clippy-style
//! catch-all):
//!   - `unused-local-variable` — a `name: Type = expr`/`var name: Type
//!     = expr` local (`Stmt::Let`) never read again. Checkable for
//!     free because plan 72 already gives every `Let` a real `is_var`
//!     flag and this language's scoping is genuinely flat (no block
//!     scope — see `emerald-sema::check_stmt`'s `Stmt::If`/`Stmt::
//!     While` arms, which thread the SAME `&mut env` into both
//!     branches, letting a `Let` inside one branch's body leak into
//!     whatever follows; confirmed against every real `examples/*.em`
//!     file, several of which rely on exactly this to "reassign" a
//!     loop counter via redeclaration instead of `var`).
//!   - `needless-var` — a `var`-declared local never actually targeted
//!     by `Stmt::Assign`/`Stmt::MultiAssign` — plan 72's own immutable-
//!     by-default philosophy says this should have been a plain
//!     binding.
//!   - `empty-rescue` — a `rescue` clause with an empty body: Ruby's
//!     classic silently-swallowed-exception anti-pattern, real in this
//!     language too since plan 38 gave it the identical `begin ...
//!     rescue ... end` shape.
//!   - `empty-class` — a `class` with no fields, no methods, and no
//!     superclass: structurally inert (the one legitimate look-alike,
//!     an empty subclass used purely to name a distinct exception
//!     type, is deliberately excluded via the `superclass.is_none()`
//!     guard).
//!
//! Both `unused-local-variable` and `needless-var` are checked per
//! FLAT SCOPE (a `Function`/method/module-fn/actor-method body, a
//! `test`/`property`/`benchmark` body, or the program's own top-level
//! statement sequence — each independent, mirroring `emerald-sema`'s
//! own per-function fresh `env`), using the most conservative
//! evaluation this codebase's real idioms actually require: a
//! declaration is "used"/"reassigned" if that name appears ANYWHERE
//! else in its own scope's `Stmt`/`Expr` tree at all, not just
//! "textually after" the declaration. This is a deliberate choice, not
//! a shortcut — it can only ever cost a false NEGATIVE (missing a
//! genuinely dead redeclaration in a same-named shadowing chain),
//! never a false positive, and it's what makes the redeclaration-as-
//! reassignment idiom (`i: Int64 = i + 1`, all over `examples/
//! control_flow.em`/`closures.em`/`collections.em`/`benchmark_example.
//! em`) and by-value lambda capture (`examples/closures.em`'s `add_x:
//! Proc = do |y: Int64| y + x end`, which reads the enclosing `x`)
//! both lint clean with zero false positives.

use emerald_parser::{CasePattern, ClassDef, Expr, Item, Program, Spanned, StringPart};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process;

// `emerald_parser::Stmt` collides in name with nothing else here, but
// is imported separately from the group above purely so the `use`
// block above stays sorted the way the rest of this codebase's `use`
// blocks are (rustfmt would otherwise interleave it mid-alphabet —
// kept as one clean `use` line instead by just listing it there too).
use emerald_parser::Stmt;

/// One lint finding: a stable rule name (shown as miette's `code`),
/// a human message, and the real byte span into the checked file's own
/// source that `emerald lint`'s snippet rendering points a caret at.
#[derive(Debug)]
struct Finding {
  rule: &'static str,
  message: String,
  span: (usize, usize),
}

impl Finding {
  fn new(rule: &'static str, message: impl Into<String>, span: (usize, usize)) -> Self {
    Self {
      rule,
      message: message.into(),
      span,
    }
  }
}

/// Wraps one `Finding` for miette's `fancy`-handler graphical
/// rendering — the same `NamedSource` + one labeled `SourceSpan` shape
/// `emerald-cli::main::SemaError` already uses for sema diagnostics,
/// just reported at `Severity::Warning` (a lint finding never blocks
/// compilation the way a parse/sema error does) and tagged with the
/// rule's own name as `code()`, so the rendered output reads
/// `warning[unused-local-variable]: ...` the same way `rustc`/clippy's
/// own diagnostics carry a lint name.
#[derive(Debug)]
struct LintReport {
  rule: &'static str,
  message: String,
  src: miette::NamedSource<String>,
  span: miette::SourceSpan,
}

impl std::fmt::Display for LintReport {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl std::error::Error for LintReport {}

impl miette::Diagnostic for LintReport {
  fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
    Some(Box::new(self.rule))
  }
  fn severity(&self) -> Option<miette::Severity> {
    Some(miette::Severity::Warning)
  }
  fn source_code(&self) -> Option<&dyn miette::SourceCode> {
    Some(&self.src)
  }
  fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
    Some(Box::new(std::iter::once(
      miette::LabeledSpan::new_with_span(Some("here".to_string()), self.span),
    )))
  }
}

/// `emerald lint <file.em>|<directory>` — parses each `.em` file found
/// at the given path (recursing into a directory), lints each
/// independently, and renders every finding through the same miette
/// `fancy` graphical handler `run_legacy`'s own parse/sema errors use.
/// Exit code convention (matching ordinary lint-tool practice — `cargo
/// clippy`/`eslint`/etc. all exit non-zero on any finding, not just a
/// hard failure): `0` only when every file parsed AND produced zero
/// findings; `1` when at least one file either failed to parse or
/// produced at least one finding; `2` for a usage/path error (no
/// `<file.em>|<directory>` argument, or the argument names neither).
pub fn run(args: &[String]) {
  let Some(target) = args.get(2) else {
    eprintln!("usage: emerald lint <file.em>|<directory>");
    process::exit(2);
  };

  let path = PathBuf::from(target);
  let files = match collect_em_files(&path) {
    Ok(files) if !files.is_empty() => files,
    Ok(_) => {
      eprintln!("error: no `.em` files found at `{target}`");
      process::exit(2);
    }
    Err(e) => {
      eprintln!("error: {e}");
      process::exit(2);
    }
  };

  let mut total_findings = 0usize;
  let mut had_error = false;

  for file in &files {
    let source = match std::fs::read_to_string(file) {
      Ok(s) => s,
      Err(e) => {
        eprintln!("error: cannot read `{}`: {e}", file.display());
        had_error = true;
        continue;
      }
    };
    let name = file.display().to_string();
    match emerald_parser::parse_named(&source, &name) {
      Ok(program) => {
        let findings = lint_program(&program, &source);
        total_findings += findings.len();
        for finding in findings {
          let span_len = finding.span.1.saturating_sub(finding.span.0).max(1);
          let report = LintReport {
            rule: finding.rule,
            message: finding.message,
            src: miette::NamedSource::new(&name, source.clone()),
            span: (finding.span.0, span_len).into(),
          };
          eprintln!("{:?}", miette::Report::new(report));
        }
      }
      Err(errs) => {
        had_error = true;
        for e in errs {
          eprintln!("{:?}", miette::Report::new(e));
        }
      }
    }
  }

  if files.len() > 1 {
    eprintln!(
      "emerald lint: {} file{} checked, {} finding{}",
      files.len(),
      if files.len() == 1 { "" } else { "s" },
      total_findings,
      if total_findings == 1 { "" } else { "s" }
    );
  }

  if had_error || total_findings > 0 {
    process::exit(1);
  }
}

/// A single file lints itself; a directory is walked recursively for
/// every `*.em` file it (transitively) contains.
fn collect_em_files(path: &Path) -> Result<Vec<PathBuf>, String> {
  if path.is_file() {
    return Ok(vec![path.to_path_buf()]);
  }
  if !path.is_dir() {
    return Err(format!("`{}` is not a file or directory", path.display()));
  }
  let mut out = Vec::new();
  collect_em_files_rec(path, &mut out)?;
  out.sort();
  Ok(out)
}

fn collect_em_files_rec(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
  let entries =
    std::fs::read_dir(dir).map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
  for entry in entries {
    let entry = entry.map_err(|e| format!("cannot read `{}`: {e}", dir.display()))?;
    let entry_path = entry.path();
    let file_name = entry.file_name();
    let file_name = file_name.to_string_lossy();
    if entry_path.is_dir() {
      // Plan 46's `emerald build` vendors an exact copy of each
      // path/git dependency's own source under `deps/` (see
      // `emerald-cli/src/deps.rs`) — skipped here so `emerald lint
      // <project-dir>` reports each real finding exactly once, against
      // the dependency's own source tree, not a second time against
      // `deps/`'s vendored copy. Ordinary dot-directories (`.git`,
      // `.emerald`) are skipped for the same "don't double/needlessly
      // scan" reason.
      if file_name == "deps" || file_name.starts_with('.') {
        continue;
      }
      collect_em_files_rec(&entry_path, out)?;
    } else if entry_path.extension().is_some_and(|ext| ext == "em") {
      out.push(entry_path);
    }
  }
  Ok(())
}

/// Every local declared with `Stmt::Let` inside one flat scope, plus
/// enough about that scope (every name read as an `Expr::Ident`, every
/// name targeted by `Stmt::Assign`/`Stmt::MultiAssign`) to decide
/// `unused-local-variable`/`needless-var` for each one. Borrows
/// straight out of the parsed `Program` — never cloned — so its
/// lifetime `'a` is tied to that `Program`.
#[derive(Default)]
struct ScopeInfo<'a> {
  lets: Vec<LetInfo<'a>>,
  reads: HashMap<&'a str, usize>,
  assigned: HashSet<&'a str>,
}

struct LetInfo<'a> {
  name: &'a str,
  is_var: bool,
  span: (usize, usize),
}

/// Lints one flat scope's already-collected `ScopeInfo` for `unused-
/// local-variable`/`needless-var`. A name starting with `_` is treated
/// as a deliberate "I know this is unused" marker (the same convention
/// Rust itself uses) and skipped by both rules — this language has no
/// such documented convention, but honoring it can only ever suppress
/// a true positive a real author marked on purpose, never manufacture
/// a false one.
fn lint_scope(info: &ScopeInfo<'_>, findings: &mut Vec<Finding>) {
  for l in &info.lets {
    if l.name.starts_with('_') {
      continue;
    }
    let reads = info.reads.get(l.name).copied().unwrap_or(0);
    if reads == 0 {
      findings.push(Finding::new(
        "unused-local-variable",
        format!(
          "local variable `{}` is declared but never read again",
          l.name
        ),
        l.span,
      ));
    }
    if l.is_var && !info.assigned.contains(l.name) {
      findings.push(Finding::new(
        "needless-var",
        format!(
          "`var {}` is never reassigned — declare it without `var` (immutable by default, per this language's own binding philosophy)",
          l.name
        ),
        l.span,
      ));
    }
  }
}

/// Recursively walks one flat scope's own statement list, accumulating
/// `ScopeInfo` and — in the same pass — every `empty-rescue` finding
/// reachable from it (a `Stmt::Begin` can appear nested arbitrarily
/// deep: inside an `if`, a `while`, even a lambda literal's own body,
/// so this has to be a real recursive walk, not a top-level-only scan).
fn walk_stmt<'a>(stmt: &'a Spanned<Stmt>, info: &mut ScopeInfo<'a>, findings: &mut Vec<Finding>) {
  match &stmt.node {
    Stmt::Let {
      name,
      value,
      is_var,
      ..
    } => {
      info.lets.push(LetInfo {
        name,
        is_var: *is_var,
        span: stmt.span,
      });
      walk_expr(value, info, findings);
    }
    Stmt::SetField { value, .. } => walk_expr(value, info, findings),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      walk_expr(array, info, findings);
      walk_expr(index, info, findings);
      walk_expr(value, info, findings);
    }
    Stmt::Assign { name, value } => {
      info.assigned.insert(name);
      walk_expr(value, info, findings);
    }
    Stmt::MultiAssign { names, values } => {
      for n in names {
        info.assigned.insert(n);
      }
      for v in values {
        walk_expr(v, info, findings);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      walk_expr(cond, info, findings);
      walk_stmts(then_branch, info, findings);
      if let Some(b) = else_branch {
        walk_stmts(b, info, findings);
      }
    }
    Stmt::While { cond, body } => {
      walk_expr(cond, info, findings);
      walk_stmts(body, info, findings);
    }
    Stmt::Return(Some(e)) => walk_expr(e, info, findings),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Expr(e) => walk_expr(e, info, findings),
    Stmt::Raise(e) => walk_expr(e, info, findings),
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      walk_stmts(body, info, findings);
      for r in rescues {
        if r.body.is_empty() {
          let who = r.class_name.as_deref().unwrap_or("_ (catch-all)");
          findings.push(Finding::new(
            "empty-rescue",
            format!(
              "`rescue {who} => {}` has an empty body — the exception is silently swallowed",
              r.var
            ),
            stmt.span,
          ));
        }
        walk_stmts(&r.body, info, findings);
      }
      if let Some(e) = ensure {
        walk_stmts(e, info, findings);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      walk_expr(scrutinee, info, findings);
      for (pattern, body) in arms {
        if let CasePattern::Values(vs) = pattern {
          for v in vs {
            walk_expr(v, info, findings);
          }
        }
        walk_stmts(body, info, findings);
      }
      if let Some(b) = else_body {
        walk_stmts(b, info, findings);
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        walk_expr(e, info, findings);
      }
      walk_stmts(body, info, findings);
    }
    Stmt::Yield(args) => {
      for a in args {
        walk_expr(a, info, findings);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      walk_expr(start, info, findings);
      walk_expr(end, info, findings);
      walk_stmts(body, info, findings);
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      walk_expr(scrutinee, info, findings);
      walk_stmts(ok_body, info, findings);
      walk_stmts(err_body, info, findings);
    }
  }
}

fn walk_stmts<'a>(
  stmts: &'a [Spanned<Stmt>],
  info: &mut ScopeInfo<'a>,
  findings: &mut Vec<Finding>,
) {
  for s in stmts {
    walk_stmt(s, info, findings);
  }
}

/// `walk_stmt`'s expression-side counterpart — visits every `Expr`
/// variant's own sub-expressions, recording each `Expr::Ident` read and
/// recursing into a `Lambda`/`Supervise` body's own statement list (by-
/// value capture means a lambda reading an enclosing local is a real
/// use of it — see this module's own top doc comment).
fn walk_expr<'a>(expr: &'a Spanned<Expr>, info: &mut ScopeInfo<'a>, findings: &mut Vec<Finding>) {
  match &expr.node {
    Expr::Ident(name) => {
      *info.reads.entry(name).or_insert(0) += 1;
    }
    Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::InstanceVar(_) => {}
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          walk_expr(e, info, findings);
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
    | Expr::Coalesce(a, b)
    | Expr::Index(a, b) => {
      walk_expr(a, info, findings);
      walk_expr(b, info, findings);
    }
    Expr::Compare(a, _, b) => {
      walk_expr(a, info, findings);
      walk_expr(b, info, findings);
    }
    Expr::Neg(e)
    | Expr::Not(e)
    | Expr::BitNot(e)
    | Expr::Comptime(e)
    | Expr::ArrayNew(e)
    | Expr::Ok(e)
    | Expr::Err(e)
    | Expr::Try(e) => walk_expr(e, info, findings),
    Expr::Call(_, args) | Expr::New(_, args) | Expr::Spawn(_, args) => {
      for a in args {
        walk_expr(a, info, findings);
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, v) in kwargs {
        walk_expr(v, info, findings);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      walk_expr(recv, info, findings);
      for a in args {
        walk_expr(a, info, findings);
      }
    }
    Expr::ArrayLit(elems) | Expr::TupleLit(elems) => {
      for e in elems {
        walk_expr(e, info, findings);
      }
    }
    Expr::Lambda { body, .. } => walk_stmts(body, info, findings),
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        walk_expr(k, info, findings);
        walk_expr(v, info, findings);
      }
    }
    Expr::Supervise(stmts) => walk_stmts(stmts, info, findings),
    Expr::Remote { addr, name, .. } => {
      walk_expr(addr, info, findings);
      walk_expr(name, info, findings);
    }
    Expr::Locate { key, args, .. } => {
      walk_expr(key, info, findings);
      for a in args {
        walk_expr(a, info, findings);
      }
    }
  }
}

/// One independent flat scope's worth of `unused-local-variable`/
/// `needless-var`/`empty-rescue` checking — a `Function`/method/
/// module-fn/actor-method/`test`/`property`/`benchmark` body, or (via
/// `lint_program`'s own separate top-level call) the program's flat
/// top-level statement sequence.
fn lint_scope_body(body: &[Spanned<Stmt>], findings: &mut Vec<Finding>) {
  let mut info = ScopeInfo::default();
  walk_stmts(body, &mut info, findings);
  lint_scope(&info, findings);
}

/// `empty-class`: a `ClassDef` with no fields, no methods, and no
/// superclass. The superclass guard is deliberate, not an
/// afterthought — an empty SUBCLASS (`class NotFoundError < AppError
/// end`) is a genuine, common idiom (distinguishing an exception by
/// its own name/type alone, inheriting every real field/method from
/// its parent), not a structural smell the way a fully freestanding
/// empty class is.
fn lint_class_shape(c: &ClassDef, source: &str, findings: &mut Vec<Finding>) {
  if c.superclass.is_none() && c.fields.is_empty() && c.methods.is_empty() {
    let span = find_class_decl_span(source, &c.name);
    findings.push(Finding::new(
      "empty-class",
      format!(
        "class `{}` declares no fields and no methods — an empty, inert shell",
        c.name
      ),
      span,
    ));
  }
}

/// `ClassDef` carries no span of its own — only `Spanned<Stmt>`/
/// `Spanned<Expr>` do (see `emerald_parser::ast`: every top-level
/// `Item` variant except `Item::Stmt` is span-free by construction,
/// plan 13's own scope never extended that far). Recovers a real,
/// honest byte offset the same way `emerald_parser::lib.rs`'s own
/// `collect_doc_comments`/`rewrite_assert_locations` already do post-
/// parse — a direct scan of the raw source text for this exact class's
/// own `class <Name>` header — rather than fabricating `(0, 0)`.
fn find_class_decl_span(source: &str, name: &str) -> (usize, usize) {
  let needle = format!("class {name}");
  let bytes = source.as_bytes();
  let mut from = 0;
  while let Some(rel) = source.get(from..).and_then(|s| s.find(&needle)) {
    let start = from + rel;
    let end = start + needle.len();
    let before_ok = start == 0 || bytes[start - 1] == b'\n' || bytes[start - 1] == b' ';
    let after_ok = match bytes.get(end) {
      None => true,
      Some(&b) => matches!(b, b' ' | b'\n' | b'\r' | b'\t' | b'['),
    };
    if before_ok && after_ok {
      return (start, end);
    }
    from = end;
  }
  (0, 1)
}

/// Runs every rule over a freshly parsed `Program`, returning every
/// finding in traversal order (top-level scope first, then each
/// top-level item in source order).
fn lint_program(program: &Program, source: &str) -> Vec<Finding> {
  let mut findings = Vec::new();

  // Top-level statements share ONE flat scope across the whole file —
  // mirrors `emerald-sema`'s own top-level `env`, threaded across every
  // top-level `Item::Stmt` in source order, never reset between them.
  let top_level: Vec<&Spanned<Stmt>> = program
    .items
    .iter()
    .filter_map(|item| match item {
      Item::Stmt(s) => Some(s),
      _ => None,
    })
    .collect();
  if !top_level.is_empty() {
    let mut info = ScopeInfo::default();
    for s in &top_level {
      walk_stmt(s, &mut info, &mut findings);
    }
    lint_scope(&info, &mut findings);
  }

  for item in &program.items {
    match item {
      Item::Function(f) => lint_scope_body(&f.body, &mut findings),
      Item::Class(c) => {
        lint_class_shape(c, source, &mut findings);
        for m in &c.methods {
          lint_scope_body(&m.body, &mut findings);
        }
      }
      Item::Module(m) => {
        for f in &m.methods {
          lint_scope_body(&f.body, &mut findings);
        }
      }
      Item::Actor(a) => {
        for m in &a.methods {
          lint_scope_body(&m.body, &mut findings);
        }
      }
      Item::Test { body, .. } | Item::Property { body, .. } | Item::Benchmark { body, .. } => {
        lint_scope_body(body, &mut findings);
      }
      Item::Interface(_)
      | Item::Extern(_)
      | Item::Enum(_)
      | Item::Require(_)
      | Item::Stmt(_)
      | Item::Error => {}
    }
  }

  findings
}

#[cfg(test)]
mod tests {
  use super::*;

  fn findings_for(src: &str) -> Vec<Finding> {
    let program = emerald_parser::parse(src).expect("test source should parse");
    lint_program(&program, src)
  }

  fn has_rule<'a>(findings: &'a [Finding], rule: &str) -> Option<&'a Finding> {
    findings.iter().find(|f| f.rule == rule)
  }

  // -- unused-local-variable ------------------------------------------

  #[test]
  fn unused_local_variable_is_flagged_when_never_read_again() {
    let findings = findings_for("x: Int64 = 5\nputs 1\n");
    assert!(
      has_rule(&findings, "unused-local-variable").is_some(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_local_read_later_is_not_flagged_as_unused() {
    let findings = findings_for("x: Int64 = 5\nputs x\n");
    assert!(has_rule(&findings, "unused-local-variable").is_none());
  }

  #[test]
  fn a_local_captured_by_a_later_lambda_is_not_flagged_as_unused() {
    // Mirrors examples/closures.em's own worked pattern exactly.
    let findings =
      findings_for("x: Int64 = 10\nadd_x: Proc = do |y: Int64| y + x end\nputs add_x.call(5)\n");
    assert!(
      has_rule(&findings, "unused-local-variable").is_none(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_redeclared_loop_counter_is_not_flagged_as_unused() {
    // Mirrors examples/control_flow.em's own redeclaration-as-
    // reassignment idiom (no `var`, no `Assign` — a fresh `Let` of the
    // same name every iteration).
    let findings =
      findings_for("i: Int64 = 0\nwhile i < 5 do\n  puts i\n  i: Int64 = i + 1\nend\n");
    assert!(
      has_rule(&findings, "unused-local-variable").is_none(),
      "{findings:?}"
    );
  }

  #[test]
  fn an_underscore_prefixed_local_is_never_flagged_as_unused() {
    let findings = findings_for("_ignored: Int64 = 5\nputs 1\n");
    assert!(has_rule(&findings, "unused-local-variable").is_none());
  }

  // -- needless-var -----------------------------------------------------

  #[test]
  fn a_var_never_reassigned_is_flagged() {
    let findings = findings_for("var x: Int64 = 5\nputs x\n");
    assert!(
      has_rule(&findings, "needless-var").is_some(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_var_reassigned_via_assign_is_not_flagged() {
    let findings = findings_for("var x: Int64 = 5\nx = x + 1\nputs x\n");
    assert!(
      has_rule(&findings, "needless-var").is_none(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_var_reassigned_via_multi_assign_is_not_flagged() {
    // Mirrors examples/bitwise_and_assignment.em's `a, b = b, a` swap.
    let findings =
      findings_for("var a: Int64 = 1\nvar b: Int64 = 2\na, b = b, a\nputs a\nputs b\n");
    assert!(
      has_rule(&findings, "needless-var").is_none(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_plain_immutable_redeclaration_is_never_flagged_as_needless_var() {
    // No `var` at all here — `needless-var` must never fire on a
    // binding that was never declared mutable in the first place.
    let findings = findings_for("i: Int64 = 0\nwhile i < 5 do\n  i: Int64 = i + 1\nend\nputs i\n");
    assert!(
      has_rule(&findings, "needless-var").is_none(),
      "{findings:?}"
    );
  }

  // -- empty-rescue -----------------------------------------------------

  #[test]
  fn an_empty_rescue_body_is_flagged() {
    let findings = findings_for(
      "class Boom\nend\n\nfn risky(): Void do\n  raise Boom.new()\nend\n\nbegin\n  risky()\nrescue Boom => e\nend\n",
    );
    assert!(
      has_rule(&findings, "empty-rescue").is_some(),
      "{findings:?}"
    );
  }

  #[test]
  fn a_rescue_with_a_real_body_is_not_flagged() {
    let findings = findings_for(
      "class Boom\nend\n\nfn risky(): Void do\n  raise Boom.new()\nend\n\nbegin\n  risky()\nrescue Boom => e\n  puts 1\nend\n",
    );
    assert!(
      has_rule(&findings, "empty-rescue").is_none(),
      "{findings:?}"
    );
  }

  // -- empty-class ------------------------------------------------------

  #[test]
  fn a_fully_empty_class_is_flagged() {
    let findings = findings_for("class Empty\nend\n");
    assert!(has_rule(&findings, "empty-class").is_some(), "{findings:?}");
  }

  #[test]
  fn a_class_with_a_field_is_not_flagged() {
    let findings = findings_for("class HasField\n  x: Int64\nend\n");
    assert!(has_rule(&findings, "empty-class").is_none(), "{findings:?}");
  }

  #[test]
  fn a_class_with_a_method_is_not_flagged() {
    let findings = findings_for("class HasMethod\n  fn go: Int64 do\n    1\n  end\nend\n");
    assert!(has_rule(&findings, "empty-class").is_none(), "{findings:?}");
  }

  #[test]
  fn an_empty_subclass_is_not_flagged_the_marker_exception_idiom() {
    let findings = findings_for("class Sub < Base\nend\n");
    assert!(has_rule(&findings, "empty-class").is_none(), "{findings:?}");
  }
}
