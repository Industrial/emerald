//! Multi-file `require` resolution (plan 23's design). Plan 23 shipped
//! `Item::Require`'s grammar/AST/parse layer only — `emerald-driver`'s
//! `resolve_program` (plan 17, still unextracted) that actually splices
//! required files together does not exist anywhere in this workspace
//! yet. Plan 46 needs `require deps/<name>/<entry>` to really compile,
//! so this module builds that resolver directly in `emerald-cli` — the
//! same "no second caller yet, so it stays here" precedent `main.rs`'s
//! own doc comment already states for the rest of the pipeline.

use emerald_parser::{Item, ParseError, Program};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum RequireError {
  Parse(PathBuf, Vec<ParseError>),
  Io(PathBuf, std::io::Error),
  Cycle(PathBuf),
}

impl std::fmt::Display for RequireError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      RequireError::Parse(p, errs) => {
        write!(f, "{} parse error(s) in `{}`", errs.len(), p.display())
      }
      RequireError::Io(p, e) => write!(f, "cannot read `{}`: {e}", p.display()),
      RequireError::Cycle(p) => write!(f, "require cycle detected at `{}`", p.display()),
    }
  }
}

impl std::error::Error for RequireError {}

/// Resolves `entry_path`'s own `require`s, recursively, into one flat
/// `Program` with every `Item::Require` replaced in place by its
/// target file's (recursively resolved) items. Canonicalized-path
/// dedup: a file required more than once contributes its items only
/// the first time it's reached (plan 23's Decision log).
pub fn resolve_program(entry_path: &Path) -> Result<Program, RequireError> {
  let mut in_progress = Vec::new();
  let mut done = HashSet::new();
  let items = resolve_file(entry_path, &mut in_progress, &mut done)?;
  Ok(Program { items })
}

fn resolve_file(
  path: &Path,
  in_progress: &mut Vec<PathBuf>,
  done: &mut HashSet<PathBuf>,
) -> Result<Vec<Item>, RequireError> {
  let canonical =
    std::fs::canonicalize(path).map_err(|e| RequireError::Io(path.to_path_buf(), e))?;
  if in_progress.contains(&canonical) {
    return Err(RequireError::Cycle(canonical));
  }
  if done.contains(&canonical) {
    return Ok(Vec::new());
  }

  let source =
    std::fs::read_to_string(&canonical).map_err(|e| RequireError::Io(canonical.clone(), e))?;
  let name = canonical.to_string_lossy().to_string();
  let program = emerald_parser::parse_named(&source, &name)
    .map_err(|errs| RequireError::Parse(canonical.clone(), errs))?;

  in_progress.push(canonical.clone());
  let dir = canonical
    .parent()
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."));

  let mut out = Vec::new();
  for item in program.items {
    match item {
      Item::Require(rel) => {
        let target = dir.join(format!("{rel}.em"));
        out.extend(resolve_file(&target, in_progress, done)?);
      }
      other => out.push(other),
    }
  }
  in_progress.pop();
  done.insert(canonical);
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-cli-require-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn splices_a_single_required_file_in_place() {
    let dir = fresh_dir("single");
    std::fs::write(dir.join("helper.em"), "def helper() -> Int64\n  5\nend\n").unwrap();
    std::fs::write(dir.join("main.em"), "require helper\nputs helper()\n").unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    assert!(!program.items.iter().any(|i| matches!(i, Item::Require(_))));
    assert_eq!(program.items.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn requiring_the_same_file_twice_contributes_it_once() {
    let dir = fresh_dir("dedup");
    std::fs::write(dir.join("a.em"), "def a() -> Int64\n  1\nend\n").unwrap();
    std::fs::write(
      dir.join("b.em"),
      "require a\ndef b() -> Int64\n  a()\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.em"), "require a\nrequire b\nputs b()\n").unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    // a() defined once, b() defined once, puts b() once = 3 items.
    assert_eq!(program.items.len(), 3);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_require_cycle_errors_instead_of_recursing_forever() {
    let dir = fresh_dir("cycle");
    std::fs::write(dir.join("a.em"), "require b\n").unwrap();
    std::fs::write(dir.join("b.em"), "require a\n").unwrap();
    let err = resolve_program(&dir.join("a.em")).unwrap_err();
    assert!(matches!(err, RequireError::Cycle(_)));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_missing_required_file_errors_not_panics() {
    let dir = fresh_dir("missing");
    std::fs::write(dir.join("main.em"), "require nope\n").unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    assert!(matches!(err, RequireError::Io(_, _)));
    std::fs::remove_dir_all(&dir).ok();
  }
}
