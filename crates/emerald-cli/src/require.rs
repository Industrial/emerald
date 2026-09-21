//! Multi-file `require` resolution (plan 23's design), extended by
//! plan 76's `import-export-module-visibility` with real, checked
//! `export`/`import` symbol-level visibility on top of it. Plan 23
//! shipped `Item::Require`'s grammar/AST/parse layer only —
//! `emerald-driver`'s `resolve_program` (plan 17, still unextracted)
//! that actually splices multi-file `require`s together does not exist
//! anywhere in this workspace yet. Plan 46 needs `require deps/<name>/
//! <entry>` to really compile, so this module builds that resolver
//! directly in `emerald-cli` — the same "no second caller yet, so it
//! stays here" precedent `main.rs`'s own doc comment already states
//! for the rest of the pipeline.
//!
//! ## Plan 76's own two-pass design
//!
//! Splicing (plan 23) and visibility enforcement (plan 76) are two
//! separate passes over the same underlying graph, not one fused walk:
//!
//! 1. `build_graph` parses every reachable file exactly once (the
//!    identical DFS/cycle-detection/dedup plan 23 always had,
//!    unchanged), additionally recording each file's own top-level
//!    name set, its `export` set (`None` if it declares zero —
//!    plan 76's backward-compatibility rule, see `emerald_parser::
//!    visibility`'s own doc comment), and its `require`/`import` edges.
//!    An `import path { Name }` is validated right here, at
//!    graph-build time — naming a nonexistent or unexported symbol is
//!    a real, immediate `RequireError::Visibility`, not deferred to a
//!    confusing "unknown function" sema diagnostic later.
//! 2. `check_visibility` walks every file's own body once, checking
//!    every cross-file identifier-shaped reference it finds against
//!    that file's own computed visibility set (`compute_visible`,
//!    propagated transitively through every plain `require` of a
//!    file that itself declares no exports — the exact mechanism that
//!    keeps a program with zero `export` declarations behaving
//!    identically to plan 23's original, unrestricted global
//!    namespace).
//! 3. `flatten` is plan 23's original splicer, unchanged in behavior
//!    (dependency-first, diamond-deduped) — `Item::Export` is stripped
//!    (the wrapped declaration compiles exactly like a non-exported
//!    one) and `Item::Import` disappears the same way `Item::Require`
//!    already does, once its target's items have been spliced in.
//!
//! See `emerald_driver::require_graph` for the identical design's
//! graph-shaped (plan 49, `--jobs`-parallel) counterpart — both share
//! `emerald_parser::visibility`'s own AST-walking primitives
//! (`own_names`, `export_names`, `find_violations`, `strip_export`);
//! each owns its own graph-shaped orchestration around them (visible-
//! set propagation, error formatting) rather than a third, shared
//! graph abstraction neither crate otherwise needs.

use emerald_driver::cache::{raw_hash, CacheKey};
use emerald_parser::{visibility, Item, ParseError, Program};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum RequireError {
  Parse(PathBuf, Vec<ParseError>),
  Io(PathBuf, std::io::Error),
  Cycle(PathBuf),
  /// Plan 76's Decision log: a real, clear compile error — referencing
  /// an unexported symbol from another file (once that file declares
  /// at least one `export`), or `import`ing a name a target file never
  /// declares at all / never exports. Pre-formatted (`file:line:col`,
  /// symbol, defining file) text rather than a structured payload —
  /// this enum's own `Display` impl already renders every other
  /// variant as plain text this way, and there is no single coherent
  /// multi-file "source" for `miette`'s snippet rendering, the same
  /// reason `report_driver_error` (`emerald-cli/src/main.rs`) already
  /// gives for a require-spliced `Program`.
  Visibility(String),
}

impl std::fmt::Display for RequireError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      RequireError::Parse(p, errs) => {
        write!(f, "{} parse error(s) in `{}`", errs.len(), p.display())
      }
      RequireError::Io(p, e) => write!(f, "cannot read `{}`: {e}", p.display()),
      RequireError::Cycle(p) => write!(f, "require cycle detected at `{}`", p.display()),
      RequireError::Visibility(msg) => write!(f, "{msg}"),
    }
  }
}

impl std::error::Error for RequireError {}

/// One `require`/`import` edge out of a file, in source order — plan
/// 76's `EdgeKind::Import` carries its own explicit name list so
/// `compute_visible` can grant exactly that subset rather than a
/// whole-file grant, even when the target itself exports more names
/// than were actually named.
#[derive(Debug, Clone)]
enum EdgeKind {
  Require,
  Import(Vec<String>),
}

/// One parsed file's own metadata — plan 23's splicing needs `entries`
/// alone; plan 76's visibility enforcement additionally needs
/// `raw_items`/`own_names`/`export_names`/`edges`.
struct FileNode {
  source: String,
  /// Splicing order, `Item::Export` already stripped and `Item::
  /// Require`/`Item::Import` already resolved to a target reference —
  /// the exact shape `flatten` walks. Mirrors `emerald_driver::
  /// require_graph::GraphNode`'s own `entries` field/`NodeEntry` type.
  entries: Vec<NodeEntry>,
  /// This file's own, unflattened, `Item::Export`/`Item::Import`-intact
  /// items — kept purely for `visibility::find_violations`'s own walk
  /// (run once for the whole graph by `check_visibility`).
  raw_items: Vec<Item>,
  own_names: HashSet<String>,
  export_names: Option<HashSet<String>>,
  edges: Vec<(PathBuf, EdgeKind)>,
}

// `Item` is a large, deeply-nested AST enum (many `Vec`/`Box`-carrying
// variants already) — boxing it here just to shrink `NodeEntry` would
// move an allocation cost around, not remove one, for a `Vec<NodeEntry>`
// built once per file during graph construction and otherwise only
// ever read, never a hot loop. Mirrors `emerald_driver::require_graph`'s
// own identical `NodeEntry`/`#[allow]` precedent.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum NodeEntry {
  Item(Item),
  Require(PathBuf),
}

/// Resolves `entry_path`'s own `require`s/`import`s, recursively, into
/// one flat `Program` with every `Item::Require`/`Item::Import`
/// replaced in place by its target file's (recursively resolved)
/// items. Canonicalized-path dedup: a file required more than once
/// contributes its items only the first time it's reached (plan 23's
/// Decision log, unchanged). Plan 76 additionally rejects (before ever
/// returning `Ok`) any cross-file reference plan 76's own visibility
/// rules don't allow — see this module's own doc comment.
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
  let mut nodes: HashMap<PathBuf, FileNode> = HashMap::new();
  let mut hashes = Vec::new();
  let entry = build_graph(entry_path, &mut in_progress, &mut nodes, &mut hashes)?;

  check_visibility(&nodes)?;

  let mut done = HashSet::new();
  let items = flatten(&entry, &nodes, &mut done);
  Ok((Program { items }, hashes))
}

fn build_graph(
  path: &Path,
  in_progress: &mut Vec<PathBuf>,
  nodes: &mut HashMap<PathBuf, FileNode>,
  hashes: &mut Vec<(PathBuf, CacheKey)>,
) -> Result<PathBuf, RequireError> {
  let canonical =
    std::fs::canonicalize(path).map_err(|e| RequireError::Io(path.to_path_buf(), e))?;
  if in_progress.contains(&canonical) {
    return Err(RequireError::Cycle(canonical));
  }
  if nodes.contains_key(&canonical) {
    return Ok(canonical);
  }

  let source =
    std::fs::read_to_string(&canonical).map_err(|e| RequireError::Io(canonical.clone(), e))?;
  hashes.push((canonical.clone(), raw_hash(source.as_bytes())));
  let name = canonical.to_string_lossy().to_string();
  let program = emerald_parser::parse_named(&source, &name)
    .map_err(|errs| RequireError::Parse(canonical.clone(), errs))?;

  let own_names = visibility::own_names(&program.items);
  let export_names = visibility::export_names(&program.items);

  in_progress.push(canonical.clone());
  let dir = canonical
    .parent()
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."));

  let mut entries = Vec::with_capacity(program.items.len());
  let mut raw_items = Vec::with_capacity(program.items.len());
  let mut edges = Vec::new();
  for item in program.items {
    match item {
      Item::Require(rel) => {
        let target = dir.join(format!("{rel}.em"));
        let target_canonical = build_graph(&target, in_progress, nodes, hashes)?;
        edges.push((target_canonical.clone(), EdgeKind::Require));
        entries.push(NodeEntry::Require(target_canonical));
        raw_items.push(Item::Require(rel));
      }
      Item::Import { path: rel, names } => {
        let target = dir.join(format!("{rel}.em"));
        let target_canonical = build_graph(&target, in_progress, nodes, hashes)?;
        validate_import(&target_canonical, &names, nodes)?;
        edges.push((target_canonical.clone(), EdgeKind::Import(names.clone())));
        entries.push(NodeEntry::Require(target_canonical));
        raw_items.push(Item::Import { path: rel, names });
      }
      other => {
        raw_items.push(other.clone());
        entries.push(NodeEntry::Item(visibility::strip_export(other)));
      }
    }
  }
  in_progress.pop();

  nodes.insert(
    canonical.clone(),
    FileNode {
      source,
      entries,
      raw_items,
      own_names,
      export_names,
      edges,
    },
  );
  Ok(canonical)
}

/// `import path { Names }`'s own eager validation — every named symbol
/// must be a real top-level declaration in `target`, and, if `target`
/// restricts its exports, must be in that export set. A real, clear
/// error naming the symbol and the file, not a deferred "unknown
/// function" sema diagnostic once the import silently contributed
/// nothing.
fn validate_import(
  target: &Path,
  names: &[String],
  nodes: &HashMap<PathBuf, FileNode>,
) -> Result<(), RequireError> {
  let target_node = &nodes[target];
  for n in names {
    if !target_node.own_names.contains(n) {
      return Err(RequireError::Visibility(format!(
        "cannot import `{n}` from `{}`: no such top-level declaration",
        target.display()
      )));
    }
    if let Some(exported) = &target_node.export_names {
      if !exported.contains(n) {
        return Err(RequireError::Visibility(format!(
          "cannot import `{n}` from `{}`: not exported (add `export` before its declaration in that file)",
          target.display()
        )));
      }
    }
  }
  Ok(())
}

/// Plan 76: every top-level name any file in the graph declares,
/// mapped to its defining file — the whole-program name resolution
/// table `check_visibility`'s cross-file reference check is built on.
fn check_visibility(nodes: &HashMap<PathBuf, FileNode>) -> Result<(), RequireError> {
  let mut all_names: HashMap<String, PathBuf> = HashMap::new();
  for (path, node) in nodes {
    for name in &node.own_names {
      all_names.insert(name.clone(), path.clone());
    }
  }

  let mut visible_cache: HashMap<PathBuf, HashSet<String>> = HashMap::new();
  let mut errors = Vec::new();
  for (path, node) in nodes {
    let visible = compute_visible(path, nodes, &mut visible_cache);
    let foreign = visibility::foreign_names_excluding(&all_names, path);
    for v in visibility::find_violations(&node.raw_items, &foreign, &visible) {
      errors.push(format_violation(path, &node.source, &v));
    }
  }

  if errors.is_empty() {
    Ok(())
  } else {
    errors.sort();
    Err(RequireError::Visibility(errors.join("\n")))
  }
}

/// The set of foreign top-level names `path`'s own file may reference
/// — memoized, recursive (safe: `build_graph`'s own cycle detection
/// already guarantees the require/import graph is acyclic). A plain
/// `require` of a file with no `export` declarations grants that
/// file's entire own name set PLUS everything visible through it in
/// turn (recreating plan 23's original, fully transitive, unrestricted
/// global namespace exactly when nobody in the whole program ever
/// writes `export` — plan 76's own backward-compatibility rule). A
/// `require`/`import` of a file that DOES restrict its exports grants
/// only the relevant export/import name subset, and does NOT propagate
/// further — an exporting file is a real visibility firewall, not a
/// transparent relay.
fn compute_visible(
  path: &Path,
  nodes: &HashMap<PathBuf, FileNode>,
  cache: &mut HashMap<PathBuf, HashSet<String>>,
) -> HashSet<String> {
  if let Some(v) = cache.get(path) {
    return v.clone();
  }
  let node = &nodes[path];
  let mut visible = HashSet::new();
  for (target, kind) in &node.edges {
    let target_node = &nodes[target];
    match &target_node.export_names {
      None => {
        visible.extend(target_node.own_names.iter().cloned());
        visible.extend(compute_visible(target, nodes, cache));
      }
      Some(set) => match kind {
        EdgeKind::Require => visible.extend(set.iter().cloned()),
        EdgeKind::Import(names) => visible.extend(names.iter().cloned()),
      },
    }
  }
  cache.insert(path.to_path_buf(), visible.clone());
  visible
}

fn format_violation(path: &Path, source: &str, v: &visibility::VisibilityError) -> String {
  let (line, col) = line_col(source, v.span.0);
  format!(
    "{}:{line}:{col}: `{}` is not visible here — it is defined in `{}`, which restricts its exports and does not export `{}`",
    path.display(),
    v.name,
    v.defining_file.display(),
    v.name
  )
}

fn line_col(source: &str, offset: usize) -> (usize, usize) {
  let mut line = 1;
  let mut col = 1;
  for ch in source[..offset.min(source.len())].chars() {
    if ch == '\n' {
      line += 1;
      col = 1;
    } else {
      col += 1;
    }
  }
  (line, col)
}

/// Plan 23's original splicer, unchanged in behavior: dependency-first,
/// diamond-deduped (`done` — a file already spliced in contributes
/// nothing a second time). `NodeEntry::Require` covers both a `require`
/// edge and an `import` edge alike — both need their target's items
/// spliced in for codegen; only the earlier visibility pass (already
/// run by the time this is called) distinguishes what each one
/// permitted a *reference* to see.
fn flatten(
  path: &Path,
  nodes: &HashMap<PathBuf, FileNode>,
  done: &mut HashSet<PathBuf>,
) -> Vec<Item> {
  if done.contains(path) {
    return Vec::new();
  }
  done.insert(path.to_path_buf());
  let node = &nodes[path];
  let mut out = Vec::new();
  for entry in &node.entries {
    match entry {
      NodeEntry::Require(target) => out.extend(flatten(target, nodes, done)),
      NodeEntry::Item(item) => out.push(item.clone()),
    }
  }
  out
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

  // Plan 76's `import-export-module-visibility`.

  #[test]
  fn a_file_with_no_exports_still_exports_everything_backward_compat() {
    let dir = fresh_dir("no-export-backcompat");
    std::fs::write(dir.join("helper.em"), "fn helper(): Int64 do\n  5\nend\n").unwrap();
    std::fs::write(dir.join("main.em"), "require helper\nputs helper()\n").unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    assert_eq!(program.items.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn requiring_an_exported_symbol_is_accepted() {
    let dir = fresh_dir("export-accept");
    std::fs::write(
      dir.join("greeter.em"),
      "export fn greet(): Int64 do\n  1\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.em"), "require greeter\nputs greet()\n").unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    assert_eq!(program.items.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn referencing_a_non_exported_symbol_is_a_real_clear_error() {
    let dir = fresh_dir("export-reject");
    std::fs::write(
      dir.join("greeter.em"),
      "export fn greet(): Int64 do\n  1\nend\nfn secret_helper(): Int64 do\n  2\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "require greeter\nputs secret_helper()\n",
    )
    .unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    let RequireError::Visibility(msg) = err else {
      panic!("expected a Visibility error, got {err:?}");
    };
    assert!(msg.contains("secret_helper"), "{msg}");
    assert!(
      msg.contains("not exported") || msg.contains("does not export"),
      "{msg}"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn import_pulls_in_only_the_named_exported_symbols() {
    let dir = fresh_dir("import-accept");
    std::fs::write(
      dir.join("mathutils.em"),
      "export fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\nexport fn sub(a: Int64, b: Int64): Int64 do\n  a - b\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "import mathutils { add }\nputs add(1, 2)\n",
    )
    .unwrap();
    let program = resolve_program(&dir.join("main.em")).unwrap();
    // add + sub (both spliced in, codegen needs the whole file) + puts.
    assert_eq!(program.items.len(), 3);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn importing_a_name_the_target_does_not_export_is_rejected() {
    let dir = fresh_dir("import-reject-unexported");
    std::fs::write(
      dir.join("mathutils.em"),
      "export fn add(a: Int64, b: Int64): Int64 do\n  a + b\nend\nfn internal(): Int64 do\n  0\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "import mathutils { internal }\nputs internal()\n",
    )
    .unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    let RequireError::Visibility(msg) = err else {
      panic!("expected a Visibility error, got {err:?}");
    };
    assert!(msg.contains("internal"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn importing_a_name_that_does_not_exist_at_all_is_rejected() {
    let dir = fresh_dir("import-reject-missing");
    std::fs::write(
      dir.join("mathutils.em"),
      "export fn add(): Int64 do\n  1\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "import mathutils { nope }\nputs nope()\n",
    )
    .unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    let RequireError::Visibility(msg) = err else {
      panic!("expected a Visibility error, got {err:?}");
    };
    assert!(msg.contains("nope"), "{msg}");
    assert!(msg.contains("no such top-level declaration"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_using_only_names_it_imported_from_b_which_requires_c_does_not_see_c() {
    // A real firewall check: `main` imports only `add` from `mathutils`,
    // and `mathutils` itself bare-`require`s a third, exportless file
    // `internal_only` — `main` must NOT transitively see `internal_
    // only`'s own names just because `mathutils` can.
    let dir = fresh_dir("firewall");
    std::fs::write(
      dir.join("internal_only.em"),
      "fn deep_helper(): Int64 do\n  7\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("mathutils.em"),
      "require internal_only\nexport fn add(a: Int64, b: Int64): Int64 do\n  a + b + deep_helper()\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "import mathutils { add }\nputs deep_helper()\n",
    )
    .unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    let RequireError::Visibility(msg) = err else {
      panic!("expected a Visibility error, got {err:?}");
    };
    assert!(msg.contains("deep_helper"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
  }

  // Found and closed 2026-09-21 (this session's "find all bugs" sweep,
  // `crates/emerald-parser/src/visibility.rs`'s own disclosed scope
  // limitation): the visibility walk used to cover only `Stmt`/`Expr`
  // positions (calls, `.new`, ...), never a `TypeExpr` position — a
  // function whose own PARAMETER type names an unexported class from
  // another file, but whose BODY never constructs or calls anything
  // cross-file, slipped through silently. Confirmed as a real, pre-
  // existing bug (not assumed) by reverting the fix on a stashed copy
  // of `emerald-parser` and confirming this exact program compiled AND
  // ran (printing `1`) without complaint before restoring it.
  #[test]
  fn a_function_signature_naming_an_unexported_cross_file_type_is_rejected() {
    let dir = fresh_dir("type-position-leak");
    std::fs::write(
      dir.join("types.em"),
      "export class Public\n  value: Int64\n\n  fn initialize(v: Int64): Void do\n    @value = v\n  end\nend\n\nclass Secret\n  value: Int64\n\n  fn initialize(v: Int64): Void do\n    @value = v\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("middle.em"),
      "require types\n\nexport fn leaks_secret_type(s: Secret): Int64 do\n  0\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.em"), "require middle\nputs 1\n").unwrap();
    let err = resolve_program(&dir.join("main.em")).unwrap_err();
    let RequireError::Visibility(msg) = err else {
      panic!("expected a Visibility error, got {err:?}");
    };
    assert!(msg.contains("Secret"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
  }

  // The identical scenario, but `Secret` IS exported this time — must
  // compile with zero complaint, proving the fix above doesn't
  // false-positive on a legitimately visible cross-file type.
  #[test]
  fn a_function_signature_naming_an_exported_cross_file_type_is_accepted() {
    let dir = fresh_dir("type-position-accept");
    std::fs::write(
      dir.join("types.em"),
      "export class Public\n  value: Int64\n\n  fn initialize(v: Int64): Void do\n    @value = v\n  end\nend\n\nexport class Secret\n  value: Int64\n\n  fn initialize(v: Int64): Void do\n    @value = v\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("middle.em"),
      "require types\n\nexport fn uses_secret_type(s: Secret): Int64 do\n  0\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.em"), "require middle\nputs 1\n").unwrap();
    resolve_program(&dir.join("main.em")).expect("Secret is exported — this must compile");
    std::fs::remove_dir_all(&dir).ok();
  }
}
