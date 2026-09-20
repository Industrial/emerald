//! `emerald-fmt` — a canonical pretty-printer for Emerald source
//! (`emerald format <file>.em`, plan 78's `canonical-formatter`, row 78
//! of the plan-of-plans).
//!
//! # Chosen approach: a pure AST pretty-printer
//!
//! `format_source` parses the input with `emerald_parser::parse_named`
//! and re-emits the resulting `Program` as canonical text
//! (`format_program`) — every stylistic decision (2-space indentation,
//! spacing around operators/colons/commas, blank lines between
//! top-level items) is derived fresh from the AST shape alone. This is
//! NOT a whitespace-preserving/normalizing pass over the original
//! token stream; the alternative (a conservative pass that only
//! touches whitespace/indentation and otherwise preserves the original
//! text byte-for-byte) was considered and rejected for this first cut
//! — a real formatter needs to move contract clauses, `read`-sugared
//! fields, `+=`/`unless`/`until`/`elsif` desugaring, and generic-type
//! spelling onto one canonical shape, all of which are much simpler to
//! get right from a clean AST-driven emission than by patching a
//! token stream in place.
//!
//! # Disclosed limitation: comments are lost
//!
//! `emerald_parser`'s lexer treats an ordinary `#`-comment as pure
//! whitespace (`grammar.lalrpop`'s own `match {}` block) and it has NO
//! AST representation at all — parsing and re-emitting a file WILL
//! drop every such comment. This is a real, disclosed gap, not a
//! silently-swallowed one: threading general comments through would
//! mean either a second, comment-aware lexing pass correlated back
//! onto the AST by byte offset, or promoting comments to real AST
//! nodes — genuinely more work than this task's scope, and not
//! attempted here.
//!
//! The ONE exception: a `##` doc-comment block immediately preceding a
//! `fn`/`class`/`interface`/`module`/`enum`/`actor` declaration (plan
//! 77) IS captured by the parser itself, as a real `doc: Option<String>`
//! field on the corresponding AST node — and this formatter DOES
//! round-trip those faithfully (re-emitted as a `##` block directly
//! above the declaration, byte-for-byte the same text `##`-comments
//! decode to). Every other comment, and any `##` run not immediately
//! attached to one of those six declaration kinds, is gone the moment
//! `parse_named` returns, before this formatter ever sees the source.
//!
//! # Canonicalized sugar
//!
//! The AST retains no marker of which surface spelling produced a
//! given node, so reformatting necessarily collapses sugar onto one
//! canonical spelling:
//!
//! - A `Stmt::Assign` whose value is exactly `BinOp(Ident(same name),
//!   rhs)` for `Add`/`Sub`/`Mul`/`Div`/`Rem` prints as `name <op>=
//!   rhs` — collapsing TOWARD the compound-assignment spelling, not
//!   away from it (`x = x + 1` and `x += 1` are indistinguishable at
//!   the AST level, so either could be "canonical" — but `+=`
//!   specifically is also the ONLY surface form able to correctly
//!   round-trip every possible `rhs` shape; see `write_assign`'s own
//!   doc comment for why the fully-expanded spelling is actually
//!   UNSAFE here, not just less pretty, whenever `rhs` is itself a
//!   looser expression like `a + b`).
//! - `unless cond do ... end` / `until cond do ... end` print back out
//!   AS `unless`/`until` (never expanded into `if !cond`/`while
//!   !cond`) — see `write_cond_head`'s own doc comment for why the
//!   `if !`/`while !` spelling is actually UNSAFE here too, for the
//!   same "this grammar has no parentheses" reason `+=` is.
//! - `read name: Type` class/actor field sugar prints as the fully
//!   expanded field declaration plus its synthesized zero-arg accessor
//!   method — indistinguishable, after parsing, from a class that
//!   declared the plain field and hand-wrote that exact accessor.
//! - An `if`/`elsif`/`else` chain IS reconstructed on the way back out
//!   (see `write_else_chain`): the grammar itself desugars `elsif`
//!   into an `else` branch containing exactly one nested `Stmt::If`,
//!   and this formatter detects that exact shape and re-emits it as
//!   `elsif`, since the two spellings are indistinguishable at the AST
//!   level and the `elsif` spelling reads far better.
//! - A trailing `Expr::Lambda` argument on an ordinary (non-hoisted)
//!   `Call`/`MethodCall`/`SafeCall` is always printed in block-attached
//!   `do |params| ... end` style, never as an inline literal
//!   argument — again a pure style choice with zero AST-equivalence
//!   risk, since both surface spellings parse to the identical AST.
//!
//! # A real, disclosed, PRE-EXISTING AST transformation this formatter
//! must reverse, not just tolerate
//!
//! `emerald_parser::parse_named` itself hoists a block attached to one
//! of eight specific enumerable methods (`each`/`map`/`select`/
//! `filter`/`reduce`/`inject`/`each_with_index`/`count`) when that call
//! is the direct value of a *top-level* `Let`/`Expr`/`Assign`/
//! `SetField` statement — `nums.map do |x: Int64| x * 2 end` used as a
//! top-level `Let`'s value is rewritten, before this crate ever sees
//! the `Program`, into a synthesized `__enum_blk_N: Proc = do |x:
//! Int64| x * 2 end` statement immediately followed by an ordinary
//! `nums.map(__enum_blk_N)` call — and, critically, that hoist ALSO
//! overwrites the block's placeholder `Void` return type with a real
//! INFERRED type (`infer_block_result_type`), which has NO surface
//! syntax at all (this language has no way to write a lambda's return
//! type explicitly). Printing the two hoisted statements separately —
//! this crate's first, naive design — silently loses that inferred
//! type on reparse (the fresh `__enum_blk_N` Let's own bare `do
//! |x: Int64| ... end` reparses with the `Void` placeholder again,
//! never re-inferred, since re-inference only happens when the
//! hoisting pass ITSELF fires, which requires seeing an inline
//! attached block, not an already-hoisted named `Ident` argument) — a
//! real, found-during-testing AST-equivalence bug, not merely an
//! ugly-output cosmetic issue.
//!
//! `unhoist_enumerable_blocks` (below) reverses this before printing:
//! it detects a run of `__enum_blk_N: Proc = do ... end` statements
//! immediately followed by the one statement whose call chain consumes
//! every one of them, by name, as each link's trailing argument, and
//! substitutes the original `Lambda` literal back into that exact
//! trailing-argument position — reconstructing the pretty, original
//! `nums.map do |x: Int64| x * 2 end` inline-block spelling. Printing
//! that spelling, reparsed, re-triggers the identical hoist +
//! inference fresh, reproducing the exact original inferred type (and,
//! since a freshly re-hoisted `__enum_blk_N` counter always starts
//! from `0` and counts up in the same left-to-right order this crate
//! also preserves, the exact original synthesized names too) —
//! `examples/enumerable.em` (this repo's own worked example of this
//! exact syntax) reformats back to its own original inline-block
//! style, not the hoisted two-statement form.
//!
//! Guarded conservatively: a run is only recognized (and only ever
//! collapsed) when EVERY synthesized `Proc` in it is fully consumed by
//! the following statement's own enumerable-method chain, by the exact
//! `__enum_blk_<digits>` name shape this pass alone ever produces — a
//! user-declared `Proc` that merely happens to share that literal name
//! is not a realistic collision to guard against further, but the
//! "every candidate must be consumed, or none are collapsed" rule
//! means a partial/ambiguous match is never silently forced through.
//!
//! # Correctness contract
//!
//! `format_program` is a pure function of the `Program` alone (it
//! never looks at the original source text), which gives two
//! properties for free, both covered by this crate's own tests:
//!
//! - **Idempotent**: formatting is deterministic and depends only on
//!   the parsed `Program`, so `format(format(src)) == format(src)`.
//! - **Structurally faithful**: reparsing the formatted output
//!   produces a `Program` that is `==` (in the span-blind sense
//!   `Spanned<T>`'s own `PartialEq` already gives every AST node in
//!   this crate — see `emerald_parser::ast::Spanned`'s own doc comment)
//!   to the `Program` that was formatted.
//!
//! Parenthesization note: this language's grammar has NO general
//! parenthesized-expression syntax at all (the one narrow exception —
//! `"(" ChainCallExpr ")" "." method` — is irrelevant to operator
//! nesting), so this formatter never inserts parentheses anywhere.
//! Every ORDINARY binary/unary grammar production restricts its own
//! operand(s) to the next tighter precedence tier, so a real, parsed
//! `Expr` tree built that way already nests in an order that reprints
//! correctly with no parentheses at all. The two exceptions are both
//! Rust-code-constructed, not grammar-recursion-constructed: `unless`/
//! `until`'s synthesized `Expr::Not`, and `+=`/`-=`/`*=`/`/=`/`%=`'s
//! synthesized right-nested `BinOp` — both take an UNRESTRICTED
//! operand precisely because they're built directly in a grammar
//! *action*, not via the ordinary recursive-descent tiers, so a naive
//! precedence-based printer would occasionally reparse them into a
//! different tree. Since parentheses aren't available as a fix, this
//! formatter instead always reprints both using the one surface
//! keyword/operator spelling that's actually capable of expressing an
//! arbitrary operand in that position (`unless`/`until` themselves;
//! the compound-assignment operator itself) — see `write_cond_head`'s
//! and `write_assign`'s own doc comments for the full argument.

use emerald_parser::{
  ActorDef, CasePattern, ClassDef, CompareOp, EnumDef, Expr, ExternBlock, Function, InterfaceDef,
  Item, ModuleDef, NewtypeDef, Param, ParseError, Program, RescueClause, Spanned, Stmt, StringPart,
  TypeExpr, TypeParam,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parses `source` (named `name` for diagnostics) and re-emits it as
/// canonically-formatted Emerald source. See this crate's own module
/// doc comment for the exact formatting rules and disclosed
/// limitations.
pub fn format_source(source: &str, name: &str) -> Result<String, Vec<ParseError>> {
  let program = emerald_parser::parse_named(source, name)?;
  Ok(format_program(&program))
}

/// Re-emits an already-parsed `Program` as canonical Emerald source. A
/// pure function of `program` alone — see this crate's own module doc
/// comment's "Correctness contract" section. Runs
/// `unhoist_enumerable_blocks` first — see that function's own doc
/// comment, and this crate's own module doc comment's "A real,
/// disclosed, PRE-EXISTING AST transformation" section, for why that's
/// necessary for correctness, not just prettier output.
pub fn format_program(program: &Program) -> String {
  let mut out = String::new();
  let items = unhoist_enumerable_blocks(&program.items);
  write_items(&mut out, &items);
  out
}

/// The eight enumerable methods `emerald_parser::hoist_enumerable_
/// blocks` hoists a trailing block from — copied here (not imported;
/// it's a private `const` of that crate) since this pass needs the
/// exact same list to recognize which trailing `Ident` arguments are
/// safe to substitute a `Lambda` back into.
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

/// `true` only for the exact `__enum_blk_<digits>` shape
/// `emerald_parser::hoist_enumerable_blocks_in_chain` alone ever
/// synthesizes (`format!("__enum_blk_{counter}")`) — see this crate's
/// own module doc comment for why matching on this exact shape (rather
/// than, say, any `Proc`-typed `Let` immediately followed by a
/// consuming call) is the deliberately conservative choice here.
fn is_hoisted_proc_name(name: &str) -> bool {
  name
    .strip_prefix("__enum_blk_")
    .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Reverses `emerald_parser::parse_named`'s own enumerable-block-hoist
/// pass — see this crate's own module doc comment's "A real, disclosed,
/// PRE-EXISTING AST transformation" section for the full rationale.
/// Returns a fresh `Vec<Item>`; `items` itself is untouched.
fn unhoist_enumerable_blocks(items: &[Item]) -> Vec<Item> {
  type Lambda = (Vec<Param>, Vec<Spanned<Stmt>>);

  let mut out: Vec<Item> = Vec::with_capacity(items.len());
  let mut i = 0;
  while i < items.len() {
    // Collect a maximal run of "hoisted Proc" candidate lets starting
    // at `i`.
    let mut lambdas: HashMap<String, Lambda> = HashMap::new();
    let mut j = i;
    while let Some(Item::Stmt(s)) = items.get(j) {
      let Stmt::Let {
        name,
        ty,
        value,
        is_var: false,
      } = &s.node
      else {
        break;
      };
      if !is_hoisted_proc_name(name) || !matches!(ty, TypeExpr::Named(t) if t == "Proc") {
        break;
      }
      let Expr::Lambda { params, body, .. } = &value.node else {
        break;
      };
      lambdas.insert(name.clone(), (params.clone(), body.clone()));
      j += 1;
    }

    if j > i {
      if let Some(Item::Stmt(consumer)) = items.get(j) {
        let mut candidate = consumer.clone();
        let value = match &mut candidate.node {
          Stmt::Let { value, .. }
          | Stmt::Assign { value, .. }
          | Stmt::SetField { value, .. }
          | Stmt::Expr(value) => Some(value),
          _ => None,
        };
        if let Some(value) = value {
          let mut remaining = lambdas.clone();
          if reinline_enumerable_chain(&mut value.node, &mut remaining) && remaining.is_empty() {
            out.push(Item::Stmt(candidate));
            i = j + 1;
            continue;
          }
        }
      }
    }

    out.push(items[i].clone());
    i += 1;
  }
  out
}

/// `unhoist_enumerable_blocks`'s own chain walk: recurses into a
/// `MethodCall`'s receiver first (mirroring `hoist_enumerable_blocks_
/// in_chain`'s identical receiver-first order, though the two walks
/// are independent per name lookup either way), then — only for a
/// method actually in `ENUMERABLE_BLOCK_METHODS` — checks whether this
/// link's own trailing argument is an `Ident` naming one of the
/// still-unconsumed candidates in `lambdas`, substituting the original
/// `Lambda` back in and removing it from `lambdas` on a match. Returns
/// `true` if this call (or anything in its receiver chain) substituted
/// at least one candidate.
fn reinline_enumerable_chain(
  expr: &mut Expr,
  lambdas: &mut HashMap<String, (Vec<Param>, Vec<Spanned<Stmt>>)>,
) -> bool {
  let Expr::MethodCall(recv, method, args) = expr else {
    return false;
  };
  let mut matched = reinline_enumerable_chain(&mut recv.node, lambdas);
  if ENUMERABLE_BLOCK_METHODS.contains(&method.as_str()) {
    if let Some(last) = args.last() {
      if let Expr::Ident(name) = &last.node {
        if let Some((params, body)) = lambdas.remove(name) {
          let idx = args.len() - 1;
          let span = args[idx].span;
          args[idx] = Spanned {
            span,
            node: Expr::Lambda {
              params,
              return_type: TypeExpr::Named("Void".to_string()),
              body,
            },
          };
          matched = true;
        }
      }
    }
  }
  matched
}

/// Recursively collects every `*.em` file under `path` (or, if `path`
/// itself is a file, just that one file), sorted for deterministic
/// output ordering — the walk `emerald format <directory>` needs to
/// format a whole project in one invocation. Mirrors `emerald lint`'s
/// own identical `collect_em_files`/`collect_em_files_rec` convention
/// (skip `deps/` — plan 46's `emerald build` vendors an exact copy of
/// each path/git dependency's own source there, so a project-wide
/// `emerald format .` reformats each real file exactly once, not a
/// second time against its own vendored copy — and skip ordinary
/// dot-directories like `.git`/`.emerald`), so all three subcommands
/// walk a project directory identically.
pub fn collect_em_files(path: &Path) -> std::io::Result<Vec<PathBuf>> {
  if path.is_file() {
    return Ok(vec![path.to_path_buf()]);
  }
  let mut out = Vec::new();
  collect_em_files_into(path, &mut out)?;
  out.sort();
  Ok(out)
}

fn collect_em_files_into(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
  let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .collect();
  entries.sort();
  for entry in entries {
    let file_name = entry
      .file_name()
      .map(|n| n.to_string_lossy().into_owned())
      .unwrap_or_default();
    if entry.is_dir() {
      if file_name == "deps" || file_name.starts_with('.') {
        continue;
      }
      collect_em_files_into(&entry, out)?;
    } else if entry.extension().and_then(|e| e.to_str()) == Some("em") {
      out.push(entry);
    }
  }
  Ok(())
}

// ---------------------------------------------------------------------
// Indentation helper
// ---------------------------------------------------------------------

fn push_indent(out: &mut String, indent: usize) {
  for _ in 0..indent {
    out.push_str("  ");
  }
}

// ---------------------------------------------------------------------
// Top-level items
// ---------------------------------------------------------------------

/// A top-level `Item`/`Stmt` counts as "standalone" (blank-line
/// separated from its neighbors) unless it's a plain, single-line
/// statement or a `require` — those pack together the way a block of
/// imports or a run of simple assignments conventionally does.
fn item_is_standalone(item: &Item) -> bool {
  match item {
    Item::Stmt(s) => is_block_stmt(&s.node),
    Item::Require(_) => false,
    // Plan 76's `import-export-module-visibility`: `import path {
    // Names }` packs together with `require`/other `import`s the same
    // way `Item::Require` already does above — both are one-line
    // "bring another file's names into scope" declarations.
    Item::Import { .. } => false,
    // `export` wraps a declaration without changing its own
    // standalone-ness — an exported function/class/etc. gets exactly
    // the same blank-line treatment it would without the `export`
    // prefix.
    Item::Export(inner) => item_is_standalone(inner),
    Item::Error => false,
    _ => true,
  }
}

fn is_block_stmt(stmt: &Stmt) -> bool {
  matches!(
    stmt,
    Stmt::If { .. }
      | Stmt::While { .. }
      | Stmt::Begin { .. }
      | Stmt::Case { .. }
      | Stmt::For { .. }
      | Stmt::ForRange { .. }
      | Stmt::MatchResult { .. }
  )
}

fn write_items(out: &mut String, items: &[Item]) {
  let mut prev_standalone = false;
  for (i, item) in items.iter().enumerate() {
    let standalone = item_is_standalone(item);
    if i > 0 && (standalone || prev_standalone) {
      out.push('\n');
    }
    write_item(out, item);
    prev_standalone = standalone;
  }
}

fn write_item(out: &mut String, item: &Item) {
  match item {
    Item::Function(f) => write_function(out, f, 0),
    Item::Class(c) => write_class(out, c, 0),
    Item::Module(m) => write_module(out, m, 0),
    Item::Actor(a) => write_actor(out, a, 0),
    Item::Enum(e) => write_enum(out, e, 0),
    Item::Newtype(n) => write_newtype(out, n, 0),
    Item::Interface(i) => write_interface(out, i, 0),
    Item::Extern(x) => write_extern(out, x, 0),
    Item::Stmt(s) => write_stmt(out, s, 0),
    Item::Require(path) => {
      out.push_str("require ");
      out.push_str(path);
      out.push('\n');
    }
    // Plan 76's `import-export-module-visibility`. `export` reprints
    // as the keyword directly in front of its wrapped declaration's
    // own canonical rendering (`write_item` recurses at the same
    // indent 0 every one of the six exportable top-level kinds already
    // writes at) — a real, round-tripping reprint, not a stub.
    Item::Export(inner) => {
      out.push_str("export ");
      write_item(out, inner);
    }
    // `import path { Name, Name2 }` — one line, comma-space-joined,
    // mirroring `Item::Require`'s own bare-path rendering immediately
    // above.
    Item::Import { path, names } => {
      out.push_str("import ");
      out.push_str(path);
      out.push_str(" { ");
      out.push_str(&names.join(", "));
      out.push_str(" }\n");
    }
    Item::Test { description, body } => write_named_block(out, "test", description, body, 0),
    Item::Property { description, body } => {
      write_named_block(out, "property", description, body, 0)
    }
    Item::Benchmark { description, body } => {
      write_named_block(out, "benchmark", description, body, 0)
    }
    // `parse_named` only ever returns `Ok(program)` when zero errors
    // were recovered, which by construction means no `Item::Error`
    // exists anywhere in it (see `emerald_parser`'s own doc comment on
    // this variant) — `format_source`'s only caller of `format_program`
    // never hands it a `Program` containing one.
    Item::Error => {}
  }
}

fn write_doc(out: &mut String, doc: &Option<String>, indent: usize) {
  let Some(doc) = doc else { return };
  for line in doc.split('\n') {
    push_indent(out, indent);
    out.push_str("##");
    if !line.is_empty() {
      out.push(' ');
      out.push_str(line);
    }
    out.push('\n');
  }
}

fn write_named_block(
  out: &mut String,
  keyword: &str,
  description: &str,
  body: &[Spanned<Stmt>],
  indent: usize,
) {
  push_indent(out, indent);
  out.push_str(keyword);
  out.push(' ');
  out.push_str(&encode_string_lit(description));
  out.push_str(" do\n");
  write_stmts(out, body, indent + 1);
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_type_params(out: &mut String, tps: &[TypeParam]) {
  let parts: Vec<String> = tps
    .iter()
    .map(|tp| {
      if tp.bounds.is_empty() {
        tp.name.clone()
      } else {
        format!("{}: {}", tp.name, tp.bounds.join(" + "))
      }
    })
    .collect();
  out.push_str(&parts.join(", "));
}

fn write_function(out: &mut String, f: &Function, indent: usize) {
  write_doc(out, &f.doc, indent);
  push_indent(out, indent);
  if f.is_pure {
    out.push_str("pure ");
  }
  if f.is_comptime {
    out.push_str("comptime ");
  }
  out.push_str("fn ");
  out.push_str(&f.name);
  if !f.type_params.is_empty() {
    out.push('[');
    write_type_params(out, &f.type_params);
    out.push(']');
  }
  // `ParenParams` (the shared `FuncDef`/`MethodDef` parameter-list
  // production) has THREE alternatives: parenthesized with a trailing
  // `&blk`, parenthesized without one, or — via its own epsilon
  // alternative — no parens AT ALL, reachable only when there are zero
  // ordinary params, no splat, and no block param (`fn value: Int64 do`,
  // every zero-arg accessor in this repo's own `examples/classes.em`).
  // A written `fn value(): Int64 do` (explicit empty parens) parses to
  // the identical AST either way, so omitting them here whenever
  // there's truly nothing to put inside is a safe, canonical choice.
  if f.params.is_empty() && f.splat_param.is_none() && f.block_param.is_none() {
    out.push_str(": ");
  } else {
    out.push('(');
    write_func_params(out, f);
    out.push_str("): ");
  }
  out.push_str(&f.return_type.to_string());
  for c in &f.requires {
    out.push_str(" requires ");
    write_expr(out, &c.expr, indent);
  }
  for c in &f.ensures {
    out.push_str(" ensures ");
    write_expr(out, &c.expr, indent);
  }
  out.push_str(" do\n");
  write_stmts(out, &f.body, indent + 1);
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_func_params(out: &mut String, f: &Function) {
  let mut parts: Vec<String> = Vec::new();
  for p in &f.params {
    let mut s = format!("{}: {}", p.name, p.ty);
    if let Some(d) = &p.default {
      s.push_str(" = ");
      s.push_str(&ex(d, 0));
    }
    parts.push(s);
  }
  if let Some(sp) = &f.splat_param {
    parts.push(format!("*{}: {}", sp.name, sp.ty));
  }
  out.push_str(&parts.join(", "));
  if let Some(blk) = &f.block_param {
    if !f.params.is_empty() || f.splat_param.is_some() {
      out.push_str(", ");
    }
    out.push('&');
    out.push_str(blk);
  }
}

fn write_class(out: &mut String, c: &ClassDef, indent: usize) {
  write_doc(out, &c.doc, indent);
  push_indent(out, indent);
  out.push_str("class ");
  out.push_str(&c.name);
  if !c.type_params.is_empty() {
    out.push('[');
    write_type_params(out, &c.type_params);
    out.push(']');
  }
  if let Some(sup) = &c.superclass {
    out.push_str(" < ");
    out.push_str(sup);
  }
  if let Some((iface, args)) = &c.implements {
    out.push_str(" implements ");
    out.push_str(iface);
    if !args.is_empty() {
      out.push('[');
      out.push_str(
        &args
          .iter()
          .map(|a| a.to_string())
          .collect::<Vec<_>>()
          .join(", "),
      );
      out.push(']');
    }
  }
  if let Some(d) = &c.derive {
    out.push_str(" derive ");
    out.push_str(d);
  }
  out.push('\n');
  write_fields_and_methods(out, &c.fields, &c.methods, indent);
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_actor(out: &mut String, a: &ActorDef, indent: usize) {
  write_doc(out, &a.doc, indent);
  push_indent(out, indent);
  out.push_str("actor ");
  out.push_str(&a.name);
  out.push('\n');
  write_fields_and_methods(out, &a.fields, &a.methods, indent);
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_fields_and_methods(
  out: &mut String,
  fields: &[Param],
  methods: &[Function],
  indent: usize,
) {
  for field in fields {
    push_indent(out, indent + 1);
    out.push_str(&field.name);
    out.push_str(": ");
    out.push_str(&field.ty.to_string());
    out.push('\n');
  }
  if !fields.is_empty() && !methods.is_empty() {
    out.push('\n');
  }
  for (i, m) in methods.iter().enumerate() {
    if i > 0 {
      out.push('\n');
    }
    write_function(out, m, indent + 1);
  }
}

fn write_module(out: &mut String, m: &ModuleDef, indent: usize) {
  write_doc(out, &m.doc, indent);
  push_indent(out, indent);
  out.push_str("module ");
  out.push_str(&m.name);
  out.push('\n');
  for (i, f) in m.methods.iter().enumerate() {
    if i > 0 {
      out.push('\n');
    }
    write_function(out, f, indent + 1);
  }
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_interface(out: &mut String, i: &InterfaceDef, indent: usize) {
  write_doc(out, &i.doc, indent);
  push_indent(out, indent);
  out.push_str("interface ");
  out.push_str(&i.name);
  if !i.type_params.is_empty() {
    out.push('[');
    write_type_params(out, &i.type_params);
    out.push(']');
  }
  out.push('\n');
  for m in &i.methods {
    push_indent(out, indent + 1);
    out.push_str("fn ");
    out.push_str(&m.method_name);
    if !m.type_params.is_empty() {
      out.push('[');
      write_type_params(out, &m.type_params);
      out.push(']');
    }
    out.push('(');
    out.push_str(
      &m.params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.ty))
        .collect::<Vec<_>>()
        .join(", "),
    );
    out.push_str("): ");
    out.push_str(&m.return_type.to_string());
    out.push('\n');
  }
  push_indent(out, indent);
  out.push_str("end\n");
}

fn write_enum(out: &mut String, e: &EnumDef, indent: usize) {
  write_doc(out, &e.doc, indent);
  push_indent(out, indent);
  out.push_str("enum ");
  out.push_str(&e.name);
  if !e.type_params.is_empty() {
    out.push('[');
    write_type_params(out, &e.type_params);
    out.push(']');
  }
  out.push_str(" = ");
  let variants: Vec<String> = e
    .variants
    .iter()
    .map(|v| {
      let fields: Vec<String> = v.fields.iter().map(|f| f.to_string()).collect();
      format!("{}({})", v.name, fields.join(", "))
    })
    .collect();
  out.push_str(&variants.join(" | "));
  out.push('\n');
}

fn write_newtype(out: &mut String, n: &NewtypeDef, indent: usize) {
  write_doc(out, &n.doc, indent);
  push_indent(out, indent);
  out.push_str("newtype ");
  out.push_str(&n.name);
  out.push_str(": ");
  out.push_str(&n.underlying.to_string());
  out.push('\n');
}

fn write_extern(out: &mut String, e: &ExternBlock, indent: usize) {
  push_indent(out, indent);
  out.push_str("unsafe extern ");
  out.push_str(&encode_string_lit(&e.abi));
  out.push_str(" {\n");
  for f in &e.fns {
    push_indent(out, indent + 1);
    out.push_str("fn ");
    out.push_str(&f.name);
    out.push('(');
    out.push_str(
      &f.params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.ty))
        .collect::<Vec<_>>()
        .join(", "),
    );
    out.push_str("): ");
    out.push_str(&f.return_type.to_string());
    out.push('\n');
  }
  push_indent(out, indent);
  out.push_str("}\n");
}

// ---------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------

fn write_stmts(out: &mut String, stmts: &[Spanned<Stmt>], indent: usize) {
  let mut prev_block = false;
  for (i, s) in stmts.iter().enumerate() {
    let block = is_block_stmt(&s.node);
    if i > 0 && (block || prev_block) {
      out.push('\n');
    }
    write_stmt(out, s, indent);
    prev_block = block;
  }
}

fn write_stmt(out: &mut String, s: &Spanned<Stmt>, indent: usize) {
  match &s.node {
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      push_indent(out, indent);
      write_cond_head(out, "if", "unless", cond, indent);
      out.push_str(" do\n");
      write_stmts(out, then_branch, indent + 1);
      write_else_chain(out, else_branch, indent);
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::While { cond, body } => {
      push_indent(out, indent);
      write_cond_head(out, "while", "until", cond, indent);
      out.push_str(" do\n");
      write_stmts(out, body, indent + 1);
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      push_indent(out, indent);
      out.push_str("begin\n");
      write_stmts(out, body, indent + 1);
      write_rescues(out, rescues, indent);
      if let Some(ens) = ensure {
        push_indent(out, indent);
        out.push_str("ensure\n");
        write_stmts(out, ens, indent + 1);
      }
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      push_indent(out, indent);
      out.push_str("match ");
      write_expr(out, scrutinee, indent);
      out.push_str(" do\n");
      for (pattern, body) in arms {
        push_indent(out, indent + 1);
        write_case_pattern(out, pattern, indent);
        out.push_str(" do\n");
        write_stmts(out, body, indent + 2);
        push_indent(out, indent + 1);
        out.push_str("end\n");
      }
      if let Some(eb) = else_body {
        push_indent(out, indent + 1);
        out.push_str("_ do\n");
        write_stmts(out, eb, indent + 2);
        push_indent(out, indent + 1);
        out.push_str("end\n");
      }
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::For {
      var,
      elements,
      body,
    } => {
      push_indent(out, indent);
      out.push_str("for ");
      out.push_str(var);
      out.push_str(" in [");
      out.push_str(
        &elements
          .iter()
          .map(|e| ex(e, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
      out.push_str("]\n");
      write_stmts(out, body, indent + 1);
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::ForRange {
      var,
      start,
      end,
      exclusive,
      body,
    } => {
      push_indent(out, indent);
      out.push_str("for ");
      out.push_str(var);
      out.push_str(" in ");
      write_expr(out, start, indent);
      out.push_str(if *exclusive { "..." } else { ".." });
      write_expr(out, end, indent);
      out.push('\n');
      write_stmts(out, body, indent + 1);
      push_indent(out, indent);
      out.push_str("end\n");
    }
    Stmt::MatchResult {
      scrutinee,
      ok_var,
      ok_body,
      err_var,
      err_body,
    } => {
      push_indent(out, indent);
      out.push_str("match ");
      write_expr(out, scrutinee, indent);
      out.push_str(" do\n");
      push_indent(out, indent + 1);
      out.push_str("Ok(");
      out.push_str(ok_var);
      out.push_str(") do\n");
      write_stmts(out, ok_body, indent + 2);
      push_indent(out, indent + 1);
      out.push_str("end\n");
      push_indent(out, indent + 1);
      out.push_str("Err(");
      out.push_str(err_var);
      out.push_str(") do\n");
      write_stmts(out, err_body, indent + 2);
      push_indent(out, indent + 1);
      out.push_str("end\n");
      push_indent(out, indent);
      out.push_str("end\n");
    }
    _ => {
      push_indent(out, indent);
      write_simple_stmt(out, &s.node, indent);
      out.push('\n');
    }
  }
}

fn write_case_pattern(out: &mut String, pattern: &CasePattern, indent: usize) {
  match pattern {
    CasePattern::Values(vs) => {
      out.push_str(
        &vs
          .iter()
          .map(|e| ex(e, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
    }
    CasePattern::Variant { name, bindings } => {
      out.push_str(name);
      if !bindings.is_empty() {
        out.push('(');
        out.push_str(&bindings.join(", "));
        out.push(')');
      }
    }
  }
}

fn write_rescues(out: &mut String, rescues: &[RescueClause], indent: usize) {
  for r in rescues {
    push_indent(out, indent);
    out.push_str("rescue ");
    if let Some(cn) = &r.class_name {
      out.push_str(cn);
      out.push_str(" => ");
    } else {
      out.push_str("=> ");
    }
    out.push_str(&r.var);
    out.push('\n');
    write_stmts(out, &r.body, indent + 1);
  }
}

/// `if`/`while` print their condition head via this helper rather than
/// unconditionally writing `<keyword> <cond>` — see the doc comment
/// below for why a top-level `Expr::Not` cond specifically needs the
/// negated keyword (`unless`/`until`) rather than a literal `!`.
///
/// `unless cond do .. end` / `until cond do .. end` desugar at parse
/// time into `Stmt::If`/`Stmt::While` whose `cond` is `Expr::Not`
/// wrapping the FULL, unrestricted `CondExpr` the user wrote (see
/// `grammar.lalrpop`'s own `"unless"`/`"until"` productions) — unlike
/// every ordinary `!expr` reachable through the ordinary
/// `CondUnaryExpr`/`UnaryExpr` grammar tier (whose operand is
/// grammar-restricted to the next tighter tier, i.e. a bare
/// call/index/primary), this specific `Not` can wrap something as
/// loose as a whole `a > b && c` comparison. Since this grammar has NO
/// general parenthesized-expression syntax at all, printing such a
/// `Not` as a literal `!` prefix (`!a > b && c`) would silently
/// reparse into a COMPLETELY different tree (`!` binds at the tightest
/// tier, so that text means `(!a) > b && c`, not `!(a > b && c)`) —
/// there is no way to write the intended meaning back out other than
/// via the one surface form that's actually built to hold an arbitrary
/// `CondExpr` operand: `unless`/`until` themselves. So: whenever `cond`
/// is exactly `Expr::Not(inner)`, this ALWAYS prints using the negated
/// keyword with `inner` (never `!inner`) — safe and correct regardless
/// of how tight `inner` happens to be, and, for the common case where
/// `inner` genuinely is tight (a hand-written `if !flag do` is
/// indistinguishable from `unless flag do` at the AST level), a pure,
/// harmless cosmetic canonicalization exactly like this crate's other
/// sugar-collapsing choices.
fn write_cond_head(
  out: &mut String,
  normal_kw: &str,
  negated_kw: &str,
  cond: &Spanned<Expr>,
  indent: usize,
) {
  if let Expr::Not(inner) = &cond.node {
    out.push_str(negated_kw);
    out.push(' ');
    write_expr(out, inner, indent);
  } else {
    out.push_str(normal_kw);
    out.push(' ');
    write_expr(out, cond, indent);
  }
}

/// Reconstructs an `elsif` chain: the grammar itself desugars `elsif
/// cond do ... end` into an `else` branch containing exactly one
/// nested `Stmt::If` (see `grammar.lalrpop`'s own `ElseClause`
/// production) — this detects that exact shape and re-emits it as
/// `elsif`, recursing so a whole chain collapses correctly. A plain
/// `else` containing some other single statement (including a
/// hand-written, non-`elsif` `if` as the ONLY statement in an `else`
/// block) is indistinguishable from a real `elsif` at the AST level —
/// both spellings parse identically — so collapsing it here is a safe,
/// purely cosmetic choice, not a correctness risk either way.
///
/// Deliberately does NOT collapse into `elsif` when that nested `If`'s
/// own `cond` is `Expr::Not(_)` — there is no `elsif`-shaped negated
/// keyword (only bare `if`/`while` have an `unless`/`until`
/// counterpart), so falling through to a plain `else` block (whose own
/// nested `write_stmt` call correctly reprints that `If` via
/// `write_cond_head`'s `unless` path) is the only always-correct
/// choice here.
fn write_else_chain(out: &mut String, else_branch: &Option<Vec<Spanned<Stmt>>>, indent: usize) {
  let Some(body) = else_branch else { return };
  if body.len() == 1 {
    if let Stmt::If {
      cond,
      then_branch,
      else_branch,
    } = &body[0].node
    {
      if !matches!(cond.node, Expr::Not(_)) {
        push_indent(out, indent);
        out.push_str("elsif ");
        write_expr(out, cond, indent);
        out.push_str(" do\n");
        write_stmts(out, then_branch, indent + 1);
        write_else_chain(out, else_branch, indent);
        return;
      }
    }
  }
  push_indent(out, indent);
  out.push_str("else\n");
  write_stmts(out, body, indent + 1);
}

/// `x += rhs` / `-=` / `*=` / `/=` / `%=` desugar at parse time into
/// `Stmt::Assign { name, value: Expr::Add(Ident(name), rhs) }` (etc. —
/// see `grammar.lalrpop`'s own `"+="` production), where `rhs` is
/// captured as the FULL, unrestricted `Expr`. That matters because
/// EVERY ordinary parsed binary-operator tree in this language is
/// always LEFT-associated, by construction of the grammar's
/// left-recursive precedence tiers (`a + b + c` can only ever parse as
/// `Add(Add(a, b), c)`) — but this one specific desugaring can produce
/// a RIGHT-nested tree instead (`total += a + b` really does produce
/// `Add(Ident(total), Add(a, b))`, putting the freshly-synthesized
/// `Ident(total)` on the OUTSIDE, not folded in on the left the way an
/// ordinary left-to-right parse would). Since this grammar has no
/// general parenthesized-expression syntax at all, naively printing
/// that as `total = total + a + b` would silently reparse into the
/// WRONG tree (`Add(Add(total, a), b)` — different shape, even though
/// mathematically equal) — there is no way to spell the right-nested
/// shape back out other than through the one surface form that's
/// actually built to hold an arbitrary `rhs`: the compound-assignment
/// operator itself. So: whenever `value` is exactly `BinOp(Ident(name),
/// rhs)` for the SAME `name` being assigned, this reprints it as
/// `name <op>= rhs` — correct for any `rhs` shape, and, for the common
/// case where a human just happened to hand-write `x = x + 1` instead
/// of `x += 1` (the two are indistinguishable at the AST level), a
/// harmless cosmetic canonicalization exactly like this crate's other
/// sugar-collapsing choices.
fn write_assign(out: &mut String, name: &str, value: &Spanned<Expr>, indent: usize) {
  if let Some((op, rhs)) = compound_assign_parts(name, &value.node) {
    out.push_str(name);
    out.push(' ');
    out.push_str(op);
    out.push_str("= ");
    write_expr(out, rhs, indent);
    return;
  }
  out.push_str(name);
  out.push_str(" = ");
  write_expr(out, value, indent);
}

/// Matches `write_assign`'s own `BinOp(Ident(name), rhs)` shape for one
/// of the five operators this grammar actually has compound-assignment
/// sugar for (no `&=`/`|=`/`^=`/`<<=`/`>>=` exist in this language, so
/// a `Stmt::Assign` whose value is `BitAnd`/`BitOr`/etc. is always an
/// ordinary, safely-left-nested assignment — never this shape — and is
/// correctly left alone by not matching here at all).
fn compound_assign_parts<'a>(
  name: &str,
  expr: &'a Expr,
) -> Option<(&'static str, &'a Spanned<Expr>)> {
  let (op, lhs, rhs) = match expr {
    Expr::Add(l, r) => ("+", l, r),
    Expr::Sub(l, r) => ("-", l, r),
    Expr::Mul(l, r) => ("*", l, r),
    Expr::Div(l, r) => ("/", l, r),
    Expr::Rem(l, r) => ("%", l, r),
    _ => return None,
  };
  match &lhs.node {
    Expr::Ident(n) if n == name => Some((op, rhs.as_ref())),
    _ => None,
  }
}

/// Every `Stmt` variant EXCEPT the seven multi-line block constructs
/// `write_stmt` handles directly above — shared verbatim between a
/// top-level statement (indent + trailing newline added by the caller)
/// and a single-statement lambda body printed inline (see
/// `write_lambda`, which calls this with neither).
fn write_simple_stmt(out: &mut String, stmt: &Stmt, indent: usize) {
  match stmt {
    Stmt::Let {
      name,
      ty,
      value,
      is_var,
    } => {
      if *is_var {
        out.push_str("var ");
      }
      out.push_str(name);
      out.push_str(": ");
      out.push_str(&ty.to_string());
      out.push_str(" = ");
      write_expr(out, value, indent);
    }
    Stmt::SetField { name, value } => {
      out.push('@');
      out.push_str(name);
      out.push_str(" = ");
      write_expr(out, value, indent);
    }
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      write_expr(out, array, indent);
      out.push('[');
      write_expr(out, index, indent);
      out.push_str("] = ");
      write_expr(out, value, indent);
    }
    Stmt::Assign { name, value } => write_assign(out, name, value, indent),
    Stmt::MultiAssign { names, values } => {
      out.push_str(&names.join(", "));
      out.push_str(" = ");
      out.push_str(
        &values
          .iter()
          .map(|v| ex(v, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
    }
    Stmt::Return(Some(e)) => {
      out.push_str("return ");
      match &e.node {
        Expr::TupleLit(es) => {
          out.push_str(
            &es
              .iter()
              .map(|x| ex(x, indent))
              .collect::<Vec<_>>()
              .join(", "),
          );
        }
        _ => write_expr(out, e, indent),
      }
    }
    // Grammatically unreachable (`Stmt::Return` always requires a
    // value — see `grammar.lalrpop`'s own comment on why a bare
    // `return` is genuinely LALR(1)-ambiguous here), kept only so this
    // match stays exhaustive against future AST changes rather than
    // panicking outright.
    Stmt::Return(None) => out.push_str("return"),
    Stmt::Break => out.push_str("break"),
    Stmt::Next => out.push_str("next"),
    Stmt::Expr(e) => write_stmt_expr(out, e, indent),
    Stmt::Raise(e) => {
      out.push_str("raise ");
      write_expr(out, e, indent);
    }
    Stmt::Yield(args) => {
      out.push_str("yield ");
      out.push_str(
        &args
          .iter()
          .map(|a| ex(a, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
    }
    Stmt::Retry => out.push_str("retry"),
    Stmt::If { .. }
    | Stmt::While { .. }
    | Stmt::Begin { .. }
    | Stmt::Case { .. }
    | Stmt::For { .. }
    | Stmt::ForRange { .. }
    | Stmt::MatchResult { .. } => {
      unreachable!("block statements are handled by write_stmt directly")
    }
  }
}

/// `puts <expr>` is the ONLY legal surface spelling for
/// `Expr::Call("puts", [arg])` — `puts` is a reserved grammar keyword,
/// never matched by the ordinary `Ident` regex (see
/// `grammar.lalrpop`'s own comment on why a general "Ident followed by
/// an expression" rule can't coexist with a bare-`Expr`-statement
/// rule), so printing `puts(arg)` here would fail to reparse. Every
/// other `Stmt::Expr` prints as an ordinary expression.
fn write_stmt_expr(out: &mut String, e: &Spanned<Expr>, indent: usize) {
  if let Expr::Call(name, args) = &e.node {
    if name == "puts" && args.len() == 1 {
      out.push_str("puts ");
      write_expr(out, &args[0], indent);
      return;
    }
  }
  write_expr(out, e, indent);
}

// ---------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------

/// Renders `e` to a fresh `String` — a convenience for the many
/// comma-joined argument-list call sites above.
fn ex(e: &Spanned<Expr>, indent: usize) -> String {
  let mut s = String::new();
  write_expr(&mut s, e, indent);
  s
}

fn compare_op_str(op: CompareOp) -> &'static str {
  match op {
    CompareOp::Lt => "<",
    CompareOp::Gt => ">",
    CompareOp::Le => "<=",
    CompareOp::Ge => ">=",
    CompareOp::Eq => "==",
    CompareOp::Ne => "!=",
  }
}

fn write_binop(out: &mut String, a: &Spanned<Expr>, op: &str, b: &Spanned<Expr>, indent: usize) {
  write_expr(out, a, indent);
  out.push(' ');
  out.push_str(op);
  out.push(' ');
  write_expr(out, b, indent);
}

/// `write_expr` never needs to insert parentheses for precedence —
/// see this crate's own module doc comment's "Parenthesization note".
fn write_expr(out: &mut String, e: &Spanned<Expr>, indent: usize) {
  match &e.node {
    Expr::Ident(n) => out.push_str(n),
    Expr::Int(n) => out.push_str(&n.to_string()),
    Expr::Float(f) => out.push_str(&format_float(*f)),
    Expr::StringLit(s) => out.push_str(&encode_string_lit(s)),
    Expr::Interpolate(parts) => write_interpolate(out, parts, indent),
    Expr::SymbolLit(s) => {
      out.push(':');
      out.push_str(s);
    }
    Expr::Add(a, b) => write_binop(out, a, "+", b, indent),
    Expr::Sub(a, b) => write_binop(out, a, "-", b, indent),
    Expr::Mul(a, b) => write_binop(out, a, "*", b, indent),
    Expr::Div(a, b) => write_binop(out, a, "/", b, indent),
    Expr::Rem(a, b) => write_binop(out, a, "%", b, indent),
    Expr::Neg(a) => {
      out.push('-');
      write_expr(out, a, indent);
    }
    Expr::Not(a) => {
      out.push('!');
      write_expr(out, a, indent);
    }
    Expr::And(a, b) => write_binop(out, a, "&&", b, indent),
    Expr::Or(a, b) => write_binop(out, a, "||", b, indent),
    Expr::Compare(a, op, b) => write_binop(out, a, compare_op_str(*op), b, indent),
    Expr::Call(name, args) => write_call(out, name, args, indent),
    Expr::CallKw(name, kwargs) => {
      out.push_str(name);
      out.push('(');
      out.push_str(
        &kwargs
          .iter()
          .map(|(k, v)| format!("{}: {}", k, ex(v, indent)))
          .collect::<Vec<_>>()
          .join(", "),
      );
      out.push(')');
    }
    Expr::New(name, args) => write_dotted_call(out, name, "new", args, indent),
    Expr::MethodCall(recv, name, args) => write_method_call(out, recv, name, args, false, indent),
    Expr::SafeCall(recv, name, args) => write_method_call(out, recv, name, args, true, indent),
    Expr::Coalesce(a, b) => write_binop(out, a, "??", b, indent),
    Expr::InstanceVar(n) => {
      out.push('@');
      out.push_str(n);
    }
    Expr::ArrayLit(es) => {
      out.push('[');
      out.push_str(
        &es
          .iter()
          .map(|e| ex(e, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
      out.push(']');
    }
    Expr::Index(base, idx) => {
      write_expr(out, base, indent);
      out.push('[');
      write_expr(out, idx, indent);
      out.push(']');
    }
    Expr::Lambda { params, body, .. } => write_lambda(out, params, body, indent),
    Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    Expr::HashLit(pairs) => {
      out.push('{');
      out.push_str(
        &pairs
          .iter()
          .map(|(k, v)| format!("{} => {}", ex(k, indent), ex(v, indent)))
          .collect::<Vec<_>>()
          .join(", "),
      );
      out.push('}');
    }
    Expr::ArrayNew(size) => {
      out.push_str("Array.new(");
      write_expr(out, size, indent);
      out.push(')');
    }
    Expr::BitAnd(a, b) => write_binop(out, a, "&", b, indent),
    Expr::BitOr(a, b) => write_binop(out, a, "|", b, indent),
    Expr::BitXor(a, b) => write_binop(out, a, "^", b, indent),
    Expr::BitNot(a) => {
      out.push('~');
      write_expr(out, a, indent);
    }
    Expr::Shl(a, b) => write_binop(out, a, "<<", b, indent),
    Expr::Shr(a, b) => write_binop(out, a, ">>", b, indent),
    // Only ever legal as `Stmt::Return`'s direct argument (handled
    // specially in `write_simple_stmt`/lambda-inline printing above) —
    // reached here only if some other AST-construction path builds one
    // elsewhere; a bare comma list is the best available fallback.
    Expr::TupleLit(es) => {
      out.push_str(
        &es
          .iter()
          .map(|e| ex(e, indent))
          .collect::<Vec<_>>()
          .join(", "),
      );
    }
    Expr::Ok(e) => {
      out.push_str("Ok(");
      write_expr(out, e, indent);
      out.push(')');
    }
    Expr::Err(e) => {
      out.push_str("Err(");
      write_expr(out, e, indent);
      out.push(')');
    }
    Expr::Try(e) => {
      write_expr(out, e, indent);
      out.push('?');
    }
    Expr::Spawn(name, args) => write_dotted_call(out, name, "spawn", args, indent),
    Expr::Supervise(body) => {
      out.push_str("supervise do\n");
      write_stmts(out, body, indent + 1);
      push_indent(out, indent);
      out.push_str("end");
    }
    Expr::Remote { class, addr, name } => {
      out.push_str(class);
      out.push_str(".remote(");
      write_expr(out, addr, indent);
      out.push_str(", ");
      write_expr(out, name, indent);
      out.push(')');
    }
    Expr::Locate { class, key, args } => {
      out.push_str(class);
      out.push_str(".locate(");
      write_expr(out, key, indent);
      for a in args {
        out.push_str(", ");
        write_expr(out, a, indent);
      }
      out.push(')');
    }
    Expr::Comptime(e) => {
      out.push_str("comptime ");
      write_expr(out, e, indent);
    }
  }
}

/// Splits off a trailing `Expr::Lambda` argument, if any — used to
/// print it in block-attached `do |params| ... end` style rather than
/// as an inline literal argument (see this crate's own module doc
/// comment on why that's a safe, purely cosmetic canonicalization).
fn split_trailing_block(args: &[Spanned<Expr>]) -> (&[Spanned<Expr>], Option<&Spanned<Expr>>) {
  if let Some(last) = args.last() {
    if matches!(last.node, Expr::Lambda { .. }) {
      return (&args[..args.len() - 1], Some(last));
    }
  }
  (args, None)
}

/// `assert`/`assert_eq` are reserved-keyword-headed grammar productions
/// (never reachable via the ordinary `Ident "(" Args ")"` call shape)
/// that synthesize an extra trailing offset argument at parse time,
/// later rewritten into a `"name:line"` string literal
/// (`rewrite_assert_locations`) — printing that synthesized argument
/// back out would neither round-trip (the dedicated grammar production
/// accepts exactly one/two `Expr`s, not two/three) nor mean anything to
/// a human, so both are special-cased here to drop it.
fn write_call(out: &mut String, name: &str, args: &[Spanned<Expr>], indent: usize) {
  if name == "assert" && args.len() == 2 {
    out.push_str("assert(");
    write_expr(out, &args[0], indent);
    out.push(')');
    return;
  }
  if name == "assert_eq" && args.len() == 3 {
    out.push_str("assert_eq(");
    write_expr(out, &args[0], indent);
    out.push_str(", ");
    write_expr(out, &args[1], indent);
    out.push(')');
    return;
  }
  let (plain, block) = split_trailing_block(args);
  out.push_str(name);
  out.push('(');
  out.push_str(
    &plain
      .iter()
      .map(|a| ex(a, indent))
      .collect::<Vec<_>>()
      .join(", "),
  );
  out.push(')');
  if let Some(b) = block {
    out.push(' ');
    write_expr(out, b, indent);
  }
}

/// `.new`/`.spawn` — always parenthesized, never block-attachable (no
/// `DoBlock?` slot in either grammar production).
fn write_dotted_call(
  out: &mut String,
  recv: &str,
  method: &str,
  args: &[Spanned<Expr>],
  indent: usize,
) {
  out.push_str(recv);
  out.push('.');
  out.push_str(method);
  out.push('(');
  out.push_str(
    &args
      .iter()
      .map(|a| ex(a, indent))
      .collect::<Vec<_>>()
      .join(", "),
  );
  out.push(')');
}

/// `.method`/`.method(args)`/`.method do...end`/`.method(args) do...end`
/// — parens are omitted only when there are no non-block arguments at
/// all (matching `ChainCallExpr`'s own zero-paren block form).
fn write_method_call(
  out: &mut String,
  recv: &Spanned<Expr>,
  name: &str,
  args: &[Spanned<Expr>],
  safe: bool,
  indent: usize,
) {
  write_expr(out, recv, indent);
  out.push_str(if safe { "?." } else { "." });
  out.push_str(name);
  let (plain, block) = split_trailing_block(args);
  // A parenless zero-arg dotted call (`recv.method`) is a distinct AST
  // shape from an explicit-call `recv.method()` in this grammar, and only
  // the former is accepted for a user class's own zero-arg methods; a
  // builtin-type intrinsic like `Int64.abs()` is rejected without the
  // parens ("method call on non-class type"). The formatter has no type
  // information at this stage to know which case a given zero-arg call is,
  // so it always emits explicit parens -- the one spelling valid in both
  // cases -- rather than guessing and risking an unparseable-as-intended
  // (or differently-typed) round trip.
  out.push('(');
  out.push_str(
    &plain
      .iter()
      .map(|a| ex(a, indent))
      .collect::<Vec<_>>()
      .join(", "),
  );
  out.push(')');
  if let Some(b) = block {
    out.push(' ');
    write_expr(out, b, indent);
  }
}

/// `do |p1: T1, p2: T2| body end` — `"|"` pipes are mandatory in the
/// grammar even for a zero-parameter block (`do || ... end`), so they
/// are never omitted here. A single-statement, non-block-construct body
/// prints inline on one line (matching every real example in this
/// repo's own `examples/` directory); anything else prints as a proper
/// multi-line block.
fn write_lambda(out: &mut String, params: &[Param], body: &[Spanned<Stmt>], indent: usize) {
  out.push_str("do |");
  out.push_str(
    &params
      .iter()
      .map(|p| format!("{}: {}", p.name, p.ty))
      .collect::<Vec<_>>()
      .join(", "),
  );
  out.push('|');
  if body.len() == 1 && !is_block_stmt(&body[0].node) {
    out.push(' ');
    write_simple_stmt(out, &body[0].node, indent);
    out.push_str(" end");
  } else {
    out.push('\n');
    write_stmts(out, body, indent + 1);
    push_indent(out, indent);
    out.push_str("end");
  }
}

/// `emerald_parser::decode_string_lit` only ever recognizes `\"`/`\n`
/// as escapes, and the lexer's own `StringLitTok` regex only ever
/// admits those two escapes in the first place — so a raw `\` can
/// never actually survive into a decoded `String` from real parsed
/// source (any `\` in the source is always consumed as the start of
/// one of the two recognized escapes, or the token fails to lex at
/// all). This encoder therefore only ever needs to re-escape `"` and a
/// literal newline; it does not attempt to handle a hypothetical
/// decoded value containing a raw `\`, since one can't arise from this
/// grammar.
fn escape_string_body(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    match c {
      '"' => out.push_str("\\\""),
      '\n' => out.push_str("\\n"),
      _ => out.push(c),
    }
  }
  out
}

fn encode_string_lit(s: &str) -> String {
  format!("\"{}\"", escape_string_body(s))
}

fn write_interpolate(out: &mut String, parts: &[StringPart], indent: usize) {
  out.push('"');
  for p in parts {
    match p {
      StringPart::Literal(s) => out.push_str(&escape_string_body(s)),
      StringPart::Expr(e) => {
        out.push_str("#{");
        write_expr(out, e, indent);
        out.push('}');
      }
    }
  }
  out.push('"');
}

/// `FloatLit`'s own grammar regex (`[0-9]+\.[0-9]+`) has no exponent
/// notation at all and always requires at least one digit on each side
/// of the `.` — Rust's own `Display` for `f64` omits the `.0` for a
/// whole number (`format!("{}", 2.0)` => `"2"`), so that case is
/// patched up here. A magnitude large/small enough that `Display`
/// falls back to scientific notation is a real, disclosed gap this
/// formatter does not attempt to handle — no example or test fixture
/// in this repository uses one, and the language's own literal syntax
/// couldn't spell it back out regardless.
fn format_float(f: f64) -> String {
  let s = format!("{f}");
  if s.contains('.') {
    s
  } else {
    format!("{s}.0")
  }
}
