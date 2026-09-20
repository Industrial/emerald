//! `emerald doc <file.em|dir>` (plan 77's `leaf-doc-comment-extraction`)
//! — extracts every `##` doc comment (design brief §36) the parser
//! captured (`emerald_parser::collect_doc_comments`, plumbed onto
//! `Function`/`ClassDef`/`InterfaceDef`/`ModuleDef`/`EnumDef`/
//! `ActorDef.doc`) and renders it as Markdown.
//!
//! **Format decision, disclosed**: ONE combined Markdown document per
//! invocation, printed to stdout by default (or written to `-o <path>`)
//! — not one file per declaration/kind. A single file is real,
//! low-effort, and immediately readable top to bottom or pasted into a
//! wiki/README; splitting per-declaration would mean inventing a
//! multi-file naming/linking scheme this plan's own scope doesn't call
//! for. Grouped by source file, then by declaration kind (Functions /
//! Classes / Interfaces / Modules / Enums / Actors), each documented
//! declaration rendered as its own heading with its real signature (in
//! Emerald's own concrete syntax, not a paraphrase) followed by its doc
//! text. A class/module/actor is included only if it (or at least one
//! of its methods) is documented; a bare function/interface/enum is
//! included only if it itself is documented — this tool extracts `##`
//! comments (the design brief's own phrasing), it does not exhaustively
//! document an entire API surface the way a `pub`-item-scanning rustdoc
//! would.
//!
//! Parsing only, never compiling/running — `emerald doc` never invokes
//! `emerald_driver` at all.

use emerald_parser::ast::{ActorDef, ClassDef, EnumDef, Function, InterfaceDef, ModuleDef};
use emerald_parser::{Item, Program};
use std::path::{Path, PathBuf};
use std::process;

pub fn run(args: &[String]) {
  let Some(path_arg) = args.get(2) else {
    eprintln!("usage: emerald doc <file.em|dir> [-o <output.md>]");
    process::exit(2);
  };
  let output_path = args
    .iter()
    .position(|a| a == "-o")
    .and_then(|i| args.get(i + 1))
    .map(PathBuf::from);

  let path = Path::new(path_arg);
  let files = if path.is_dir() {
    collect_em_files(path)
  } else {
    vec![path.to_path_buf()]
  };

  if files.is_empty() {
    eprintln!("error: no `.em` files found at `{}`", path.display());
    process::exit(1);
  }

  let mut sections = Vec::new();
  for file in &files {
    let source = std::fs::read_to_string(file).unwrap_or_else(|e| {
      eprintln!("error: cannot read `{}`: {e}", file.display());
      process::exit(1);
    });
    let file_label = file.display().to_string();
    match emerald_parser::parse_named(&source, &file_label) {
      Ok(program) => {
        if let Some(section) = render_file_docs(&program, &file_label) {
          sections.push(section);
        }
      }
      Err(errs) => {
        for e in errs {
          eprintln!("{:?}", miette::Report::new(e));
        }
        process::exit(1);
      }
    }
  }

  let output = if sections.is_empty() {
    "# Emerald API Documentation\n\nNo `##` doc comments found.\n".to_string()
  } else {
    format!("# Emerald API Documentation\n\n{}", sections.join("\n"))
  };

  match output_path {
    Some(p) => {
      std::fs::write(&p, &output).unwrap_or_else(|e| {
        eprintln!("error: cannot write `{}`: {e}", p.display());
        process::exit(1);
      });
      eprintln!("wrote {}", p.display());
    }
    None => print!("{output}"),
  }
}

/// Recursively collects every `.em` file under `dir`, skipping the same
/// non-source directories `require.rs`'s own multi-file resolution
/// already has no reason to walk into — `.emerald/` (plan 46's cache/
/// deps namespace), `target/` (Cargo's own build output, relevant only
/// when `emerald doc` is pointed at this very repository), and any
/// dotfile directory (`.git/` included). Sorted for deterministic
/// output across runs/platforms.
fn collect_em_files(dir: &Path) -> Vec<PathBuf> {
  let mut out = Vec::new();
  collect_em_files_into(dir, &mut out);
  out.sort();
  out
}

fn collect_em_files_into(dir: &Path, out: &mut Vec<PathBuf>) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    let path = entry.path();
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if path.is_dir() {
      if name.starts_with('.') || name == "target" {
        continue;
      }
      collect_em_files_into(&path, out);
    } else if path.extension().is_some_and(|ext| ext == "em") {
      out.push(path);
    }
  }
}

/// Renders one source file's documented declarations, or `None` when
/// nothing in it is documented at all (so an undocumented file
/// contributes no empty, noisy heading to the combined output).
fn render_file_docs(program: &Program, file_label: &str) -> Option<String> {
  let mut functions = Vec::new();
  let mut classes = Vec::new();
  let mut interfaces = Vec::new();
  let mut modules = Vec::new();
  let mut enums = Vec::new();
  let mut actors = Vec::new();

  for item in &program.items {
    match item {
      Item::Function(f) if f.doc.is_some() => functions.push(f),
      Item::Class(c) if c.doc.is_some() || c.methods.iter().any(|m| m.doc.is_some()) => {
        classes.push(c)
      }
      Item::Interface(i) if i.doc.is_some() => interfaces.push(i),
      Item::Module(m) if m.doc.is_some() || m.methods.iter().any(|f| f.doc.is_some()) => {
        modules.push(m)
      }
      Item::Enum(e) if e.doc.is_some() => enums.push(e),
      Item::Actor(a) if a.doc.is_some() || a.methods.iter().any(|m| m.doc.is_some()) => {
        actors.push(a)
      }
      _ => {}
    }
  }

  if functions.is_empty()
    && classes.is_empty()
    && interfaces.is_empty()
    && modules.is_empty()
    && enums.is_empty()
    && actors.is_empty()
  {
    return None;
  }

  let mut out = format!("## {file_label}\n\n");

  if !functions.is_empty() {
    out.push_str("### Functions\n\n");
    for f in functions {
      render_function(&mut out, f, "####");
    }
  }
  if !classes.is_empty() {
    out.push_str("### Classes\n\n");
    for c in classes {
      render_class(&mut out, c);
    }
  }
  if !interfaces.is_empty() {
    out.push_str("### Interfaces\n\n");
    for i in interfaces {
      render_interface(&mut out, i);
    }
  }
  if !modules.is_empty() {
    out.push_str("### Modules\n\n");
    for m in modules {
      render_module(&mut out, m);
    }
  }
  if !enums.is_empty() {
    out.push_str("### Enums\n\n");
    for e in enums {
      render_enum(&mut out, e);
    }
  }
  if !actors.is_empty() {
    out.push_str("### Actors\n\n");
    for a in actors {
      render_actor(&mut out, a);
    }
  }

  Some(out)
}

fn render_params(params: &[emerald_parser::ast::Param]) -> String {
  params
    .iter()
    .map(|p| format!("{}: {}", p.name, p.ty))
    .collect::<Vec<_>>()
    .join(", ")
}

fn function_signature(f: &Function) -> String {
  format!(
    "fn {}({}): {}",
    f.name,
    render_params(&f.params),
    f.return_type
  )
}

fn render_function(out: &mut String, f: &Function, heading: &str) {
  out.push_str(&format!("{heading} `{}`\n\n", function_signature(f)));
  if let Some(doc) = &f.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
}

fn render_class(out: &mut String, c: &ClassDef) {
  let mut sig = format!("class {}", c.name);
  if let Some(super_name) = &c.superclass {
    sig.push_str(&format!(" < {super_name}"));
  }
  if let Some((iface, _)) = &c.implements {
    sig.push_str(&format!(" implements {iface}"));
  }
  out.push_str(&format!("#### `{sig}`\n\n"));
  if let Some(doc) = &c.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
  for m in &c.methods {
    if m.doc.is_some() {
      render_function(out, m, "#####");
    }
  }
}

fn render_interface(out: &mut String, i: &InterfaceDef) {
  out.push_str(&format!("#### `interface {}`\n\n", i.name));
  if let Some(doc) = &i.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
  for m in &i.methods {
    let sig = format!(
      "fn {}({}): {}",
      m.method_name,
      render_params(&m.params),
      m.return_type
    );
    out.push_str(&format!("- `{sig}`\n"));
  }
  if !i.methods.is_empty() {
    out.push('\n');
  }
}

fn render_module(out: &mut String, m: &ModuleDef) {
  out.push_str(&format!("#### `module {}`\n\n", m.name));
  if let Some(doc) = &m.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
  for f in &m.methods {
    if f.doc.is_some() {
      render_function(out, f, "#####");
    }
  }
}

fn render_enum(out: &mut String, e: &EnumDef) {
  let variants: Vec<String> = e
    .variants
    .iter()
    .map(|v| {
      if v.fields.is_empty() {
        v.name.clone()
      } else {
        let fields = v
          .fields
          .iter()
          .map(|f| f.to_string())
          .collect::<Vec<_>>()
          .join(", ");
        format!("{}({fields})", v.name)
      }
    })
    .collect();
  out.push_str(&format!(
    "#### `enum {} = {}`\n\n",
    e.name,
    variants.join(" | ")
  ));
  if let Some(doc) = &e.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
}

fn render_actor(out: &mut String, a: &ActorDef) {
  out.push_str(&format!("#### `actor {}`\n\n", a.name));
  if let Some(doc) = &a.doc {
    out.push_str(doc);
    out.push_str("\n\n");
  }
  for m in &a.methods {
    if m.doc.is_some() {
      render_function(out, m, "#####");
    }
  }
}
