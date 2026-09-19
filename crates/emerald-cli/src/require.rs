//! Multi-file `require` resolution (plan 23's design). Plan 23 shipped
//! `Item::Require`'s grammar/AST/parse layer only — `emerald-driver`'s
//! `resolve_program` (plan 17, still unextracted) that actually splices
//! required files together does not exist anywhere in this workspace
//! yet. Plan 46 needs `require deps/<name>/<entry>` to really compile,
//! so this module builds that resolver directly in `emerald-cli` — the
//! same "no second caller yet, so it stays here" precedent `main.rs`'s
//! own doc comment already states for the rest of the pipeline.

use emerald_driver::cache::{raw_hash, CacheKey};
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
///
/// Plan 48: `cmd_build` (`main.rs`) now always calls `resolve_program_
/// with_hashes` instead (its own per-file hashes are needed whenever
/// `--verbose-cache` is active, and calling it unconditionally costs
/// nothing when it isn't) — this plain wrapper is kept for its own
/// pre-existing plan 23 test coverage and as a simpler API for any
/// future caller that has no use for the hash list.
#[allow(dead_code)]
pub fn resolve_program(entry_path: &Path) -> Result<Program, RequireError> {
  let (program, _hashes) = resolve_program_with_hashes(entry_path)?;
  Ok(program)
}

/// Plan 48's `leaf-require-graph-cache-keys`: the same DFS/dedup/
/// cycle-detection algorithm as `resolve_program` above (unchanged —
/// this is a small, additive change to what's *returned*, not a
/// redesign of how the graph is walked), additionally returning every
/// visited file's own canonical path paired with its raw content hash,
/// in first-visit order. `emerald-cli`'s cache-aware call sites fold
/// this list into one merged `CacheKey` via `QueryCache::key_for_many`
/// — touching one file changes its own hash, which changes every
/// merged key that includes it, and no other (`resolve_program`'s own
/// plain callers, and its pre-existing tests, are unaffected: they
/// just discard the second element).
pub fn resolve_program_with_hashes(
  entry_path: &Path,
) -> Result<(Program, Vec<(PathBuf, CacheKey)>), RequireError> {
  let mut in_progress = Vec::new();
  let mut done = HashSet::new();
  let mut hashes = Vec::new();
  let items = resolve_file(entry_path, &mut in_progress, &mut done, &mut hashes)?;
  Ok((Program { items }, hashes))
}

fn resolve_file(
  path: &Path,
  in_progress: &mut Vec<PathBuf>,
  done: &mut HashSet<PathBuf>,
  hashes: &mut Vec<(PathBuf, CacheKey)>,
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
  hashes.push((canonical.clone(), raw_hash(source.as_bytes())));
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
        out.extend(resolve_file(&target, in_progress, done, hashes)?);
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
    std::fs::write(dir.join("helper.em"), "fn helper(): Int64 do\n  5\nend\n").unwrap();
    std::fs::write(dir.join("main.em"), "require helper\nputs helper()\n").unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    assert!(!program.items.iter().any(|i| matches!(i, Item::Require(_))));
    assert_eq!(program.items.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn requiring_the_same_file_twice_contributes_it_once() {
    let dir = fresh_dir("dedup");
    std::fs::write(dir.join("a.em"), "fn a(): Int64 do\n  1\nend\n").unwrap();
    std::fs::write(
      dir.join("b.em"),
      "require a\nfn b(): Int64 do\n  a()\nend\n",
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

  // Plan 48's `leaf-require-graph-cache-keys`.

  fn diamond_dir(tag: &str) -> PathBuf {
    let dir = fresh_dir(tag);
    std::fs::write(dir.join("utils.em"), "fn util(): Int64 do\n  4\nend\n").unwrap();
    std::fs::write(dir.join("helpers.em"), "fn helper(): Int64 do\n  10\nend\n").unwrap();
    std::fs::write(
      dir.join("main.em"),
      "require helpers\nrequire utils\nputs helper() + util()\n",
    )
    .unwrap();
    dir
  }

  #[test]
  fn resolve_program_with_hashes_returns_one_hash_per_visited_file_in_first_visit_order() {
    let dir = diamond_dir("hashes-order");
    let (_program, hashes) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();
    let names: Vec<String> = hashes
      .iter()
      .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
      .collect();
    assert_eq!(names, vec!["main.em", "helpers.em", "utils.em"]);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn recompiling_the_diamond_unchanged_reports_the_same_hashes_both_times() {
    let dir = diamond_dir("hashes-stable");
    let (_p1, hashes1) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();
    let (_p2, hashes2) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();
    assert_eq!(hashes1, hashes2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn editing_one_required_file_changes_only_its_own_hash() {
    let dir = diamond_dir("hashes-selective");
    let (_p1, hashes1) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();

    std::fs::write(dir.join("helpers.em"), "fn helper(): Int64 do\n  99\nend\n").unwrap();
    let (_p2, hashes2) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();

    let by_name = |hashes: &[(PathBuf, CacheKey)], name: &str| {
      hashes
        .iter()
        .find(|(p, _)| p.file_name().unwrap().to_string_lossy() == name)
        .map(|(_, h)| *h)
        .unwrap()
    };
    assert_ne!(
      by_name(&hashes1, "helpers.em"),
      by_name(&hashes2, "helpers.em"),
      "the edited file's own hash must change"
    );
    assert_eq!(
      by_name(&hashes1, "main.em"),
      by_name(&hashes2, "main.em"),
      "main.em's own bytes didn't change, so its own hash must not either"
    );
    assert_eq!(
      by_name(&hashes1, "utils.em"),
      by_name(&hashes2, "utils.em"),
      "utils.em is untouched and doesn't require helpers.em — its hash must not change"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn editing_one_required_file_changes_the_merged_key_for_entries_that_require_it_but_not_others() {
    use emerald_driver::cache::QueryCache;

    let dir = diamond_dir("merged-key-isolation");
    // A second, independent entry point in the same directory that
    // requires only utils.em, never helpers.em.
    std::fs::write(dir.join("utils_only.em"), "require utils\nputs util()\n").unwrap();

    let cache = QueryCache::with_fingerprint(dir.clone(), raw_hash(b"test-fingerprint"));

    let (_p1, hashes1) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();
    let main_key_before = cache.key_for_many(&hashes1.iter().map(|(_, h)| *h).collect::<Vec<_>>());
    let (_u1, uhashes1) = resolve_program_with_hashes(&dir.join("utils_only.em")).unwrap();
    let utils_only_key_before =
      cache.key_for_many(&uhashes1.iter().map(|(_, h)| *h).collect::<Vec<_>>());

    std::fs::write(dir.join("helpers.em"), "fn helper(): Int64 do\n  99\nend\n").unwrap();

    let (_p2, hashes2) = resolve_program_with_hashes(&dir.join("main.em")).unwrap();
    let main_key_after = cache.key_for_many(&hashes2.iter().map(|(_, h)| *h).collect::<Vec<_>>());
    let (_u2, uhashes2) = resolve_program_with_hashes(&dir.join("utils_only.em")).unwrap();
    let utils_only_key_after =
      cache.key_for_many(&uhashes2.iter().map(|(_, h)| *h).collect::<Vec<_>>());

    assert_ne!(
      main_key_before, main_key_after,
      "main.em requires helpers.em — its merged key must change"
    );
    assert_eq!(
      utils_only_key_before, utils_only_key_after,
      "utils_only.em never requires helpers.em — its merged key must stay isolated"
    );
    std::fs::remove_dir_all(&dir).ok();
  }
}
