//! Plan 49's `leaf-require-graph-leveling`: turns plan 23's DFS
//! require-splice into a queryable graph (canonical-path nodes,
//! require edges) instead of a single flattened `Program`, so later
//! leaves can schedule per-file parse/typecheck/codegen work onto
//! worker threads. Reuses plan 23's own `visited`/`in_progress`
//! cycle-detection and diamond-dedup semantics verbatim (mirrored from
//! `emerald-cli/src/require.rs`'s `resolve_file`, not re-derived) — a
//! cycle or a missing file still produces a real, descriptive error,
//! now `DriverError::Require` rather than `emerald-cli`'s own
//! `RequireError` (this graph lives in `emerald-driver`, not
//! `emerald-cli`, since it's scheduling infrastructure plan 21/47's
//! future callers should get for free through `emerald_driver::check`/
//! `compile`, the same reasoning the plan's own Decision log gives —
//! `emerald-cli`'s `require.rs` keeps its own separate, unchanged
//! `resolve_program`/`resolve_program_with_hashes` for the existing
//! single-flattened-`Program` path plan 46/48 already built against).
//!
//! No speculative parallelism: `build_require_graph` is a single-
//! threaded DFS that must confirm the *whole* graph acyclic before
//! `compute_levels` (Kahn's algorithm) ever runs — a cycle is detected
//! during graph construction itself, never during leveling.

use crate::DriverError;
use crate::cache::{CacheKey, raw_hash};
use emerald_parser::{Function, Item};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One item from a file's own source, or a resolved (canonical-path)
/// require edge in its original source position — keeping requires
/// positional (rather than a separate up-front list) is what lets
/// `closure_items` reproduce plan 23's exact dependency-first splicing
/// order without re-touching the filesystem.
// `Item` is a large, deeply-nested AST enum (many `Vec`/`Box`-carrying
// variants already) — boxing it here just to shrink `NodeEntry` would
// move an allocation cost around, not remove one, for a `Vec<NodeEntry>`
// built once per file during graph construction and otherwise only
// ever read, never a hot loop.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum NodeEntry {
  Item(Item),
  Require(PathBuf),
}

#[derive(Debug)]
pub struct GraphNode {
  pub path: PathBuf,
  pub source: String,
  pub hash: CacheKey,
  entries: Vec<NodeEntry>,
  /// Direct dependencies, canonical paths, first-occurrence order,
  /// deduped — the edges `compute_levels` ranks.
  pub requires: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct RequireGraph {
  pub entry: PathBuf,
  pub nodes: HashMap<PathBuf, GraphNode>,
}

/// Builds the full require graph rooted at `entry` — parses every
/// reachable file exactly once (dedup via `nodes`' own keys doubling
/// as the "done" set), confirms it's cycle-free, and returns it as a
/// queryable structure rather than plan 23's flattened `Program`.
pub fn build_require_graph(entry: &Path) -> Result<RequireGraph, DriverError> {
  let mut nodes = HashMap::new();
  let mut in_progress = Vec::new();
  let canonical_entry = visit(entry, &mut in_progress, &mut nodes)?;
  Ok(RequireGraph {
    entry: canonical_entry,
    nodes,
  })
}

fn visit(
  path: &Path,
  in_progress: &mut Vec<PathBuf>,
  nodes: &mut HashMap<PathBuf, GraphNode>,
) -> Result<PathBuf, DriverError> {
  let canonical = std::fs::canonicalize(path)
    .map_err(|e| DriverError::Require(format!("cannot read `{}`: {e}", path.display())))?;
  if in_progress.contains(&canonical) {
    return Err(DriverError::Require(format!(
      "require cycle detected at `{}`",
      canonical.display()
    )));
  }
  if nodes.contains_key(&canonical) {
    return Ok(canonical);
  }

  let source = std::fs::read_to_string(&canonical)
    .map_err(|e| DriverError::Require(format!("cannot read `{}`: {e}", canonical.display())))?;
  let hash = raw_hash(source.as_bytes());
  let name = canonical.to_string_lossy().to_string();
  let program = emerald_parser::parse_named(&source, &name).map_err(DriverError::Parse)?;

  in_progress.push(canonical.clone());
  let dir = canonical
    .parent()
    .map(Path::to_path_buf)
    .unwrap_or_else(|| PathBuf::from("."));

  let mut entries = Vec::with_capacity(program.items.len());
  let mut requires = Vec::new();
  for item in program.items {
    match item {
      Item::Require(rel) => {
        let target = dir.join(format!("{rel}.em"));
        let target_canonical = visit(&target, in_progress, nodes)?;
        if !requires.contains(&target_canonical) {
          requires.push(target_canonical.clone());
        }
        entries.push(NodeEntry::Require(target_canonical));
      }
      other => entries.push(NodeEntry::Item(other)),
    }
  }
  in_progress.pop();

  nodes.insert(
    canonical.clone(),
    GraphNode {
      path: canonical.clone(),
      source,
      hash,
      entries,
      requires,
    },
  );
  Ok(canonical)
}

/// Kahn's algorithm over `graph`'s require edges, run only once the
/// graph is already confirmed acyclic by `build_require_graph` itself
/// — a file with no unresolved dependency forms level 0; a file enters
/// level N once every file it requires has been assigned a level less
/// than N. Order within a level is unspecified by the plan's own AC,
/// but sorted here for deterministic test assertions and reporting.
pub fn compute_levels(graph: &RequireGraph) -> Vec<Vec<PathBuf>> {
  let mut remaining: HashMap<&PathBuf, usize> = graph
    .nodes
    .iter()
    .map(|(p, n)| (p, n.requires.len()))
    .collect();
  let mut dependents: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();
  for (p, n) in &graph.nodes {
    for dep in &n.requires {
      dependents.entry(dep).or_default().push(p);
    }
  }

  let mut levels = Vec::new();
  let mut frontier: Vec<PathBuf> = remaining
    .iter()
    .filter(|(_, &c)| c == 0)
    .map(|(p, _)| (*p).clone())
    .collect();
  frontier.sort();

  while !frontier.is_empty() {
    levels.push(frontier.clone());
    let mut next = Vec::new();
    for p in &frontier {
      if let Some(deps) = dependents.get(p) {
        for d in deps {
          let c = remaining.get_mut(*d).expect("every node has an entry");
          *c -= 1;
          if *c == 0 {
            next.push((*d).clone());
          }
        }
      }
    }
    next.sort();
    frontier = next;
  }
  levels
}

/// Expands `path`'s node and its full transitive dependency closure
/// into one flat item list, dependency-first, in exactly the order
/// plan 23's own inline splicing would produce (a require is replaced
/// in place by its target's own expansion) — this is the "closure
/// Program" leaf 2/3's per-node typecheck/codegen queries compile
/// against, reusing `emerald_sema::check_program`/
/// `emerald_codegen`'s existing whole-`Program` shape unchanged.
pub fn closure_items(graph: &RequireGraph, path: &Path) -> Vec<Item> {
  let mut done = HashSet::new();
  let mut out = Vec::new();
  expand(graph, path, &mut done, &mut out);
  out
}

fn expand(graph: &RequireGraph, path: &Path, done: &mut HashSet<PathBuf>, out: &mut Vec<Item>) {
  if done.contains(path) {
    return;
  }
  done.insert(path.to_path_buf());
  let node = &graph.nodes[path];
  for entry in &node.entries {
    match entry {
      NodeEntry::Require(target) => expand(graph, target, done, out),
      NodeEntry::Item(item) => out.push(item.clone()),
    }
  }
}

/// The raw content hashes of `path`'s full transitive closure, in
/// `closure_items`' own dependency-first order — the merged
/// `QueryCache::key_for_many` input for that node's typecheck/codegen
/// queries (leaf-require-graph-cache-keys' per-entry-point design,
/// applied per node rather than only at the top-level entry).
pub fn closure_hashes(graph: &RequireGraph, path: &Path) -> Vec<CacheKey> {
  let mut done = HashSet::new();
  let mut out = Vec::new();
  fn walk(graph: &RequireGraph, path: &Path, done: &mut HashSet<PathBuf>, out: &mut Vec<CacheKey>) {
    if done.contains(path) {
      return;
    }
    done.insert(path.to_path_buf());
    let node = &graph.nodes[path];
    for entry in &node.entries {
      if let NodeEntry::Require(target) = entry {
        walk(graph, target, done, out);
      }
    }
    out.push(node.hash);
  }
  walk(graph, path, &mut done, &mut out);
  out
}

/// The set of top-level names `path`'s own file (not its dependencies)
/// defines — `Function`/`Class`/`Module` names only; a bare, non-
/// generic, non-block-param `Function` is the only construct this
/// plan's `leaf-parallel-codegen-and-jobs-flag` actually splits into
/// declare-in-every-file/define-in-one-file across multiple object
/// files (see `own_construct_support`, which flags anything else).
pub fn own_function_names(graph: &RequireGraph, path: &Path) -> HashSet<String> {
  let node = &graph.nodes[path];
  node
    .entries
    .iter()
    .filter_map(|e| match e {
      NodeEntry::Item(Item::Function(f)) if is_plain_function(f) => Some(f.name.clone()),
      _ => None,
    })
    .collect()
}

fn is_plain_function(f: &Function) -> bool {
  f.type_params.is_empty() && f.block_param.is_none()
}

/// Real, disclosed scope narrowing (see this plan's own commit
/// message): the per-file `Context`/`Module` codegen split
/// (`leaf-parallel-codegen-and-jobs-flag`) is only wired up for plain
/// top-level functions. A required file declaring a class, a module, a
/// generic function, a block-param function, or a top-level `Proc`
/// `Let` needs real cross-file method/lambda-body placement this pass
/// doesn't implement — rather than silently mis-emit (a body missing
/// from every `.o`, or defined in more than one and duplicate-symbol
/// at link time), the whole multi-file parallel compile refuses with a
/// clear, named error naming the file and the unsupported construct.
/// Single-file compiles (no `require` at all) are entirely unaffected
/// — they never reach this path.
pub fn unsupported_construct(graph: &RequireGraph, path: &Path) -> Option<String> {
  let node = &graph.nodes[path];
  for entry in &node.entries {
    if let NodeEntry::Item(item) = entry {
      match item {
        Item::Function(f) if !is_plain_function(f) => {
          return Some(format!(
            "`{}` in `{}`: generic/block-param functions aren't supported in `--jobs`-parallel multi-file compiles yet",
            f.name,
            node.path.display()
          ));
        }
        Item::Class(c) => {
          return Some(format!(
            "class `{}` in `{}`: classes aren't supported in `--jobs`-parallel multi-file compiles yet",
            c.name,
            node.path.display()
          ));
        }
        Item::Module(m) => {
          return Some(format!(
            "module `{}` in `{}`: modules aren't supported in `--jobs`-parallel multi-file compiles yet",
            m.name,
            node.path.display()
          ));
        }
        Item::Stmt(s) if is_top_level_lambda_let(s) => {
          return Some(format!(
            "top-level lambda `Let` in `{}`: not supported in `--jobs`-parallel multi-file compiles yet",
            node.path.display()
          ));
        }
        _ => {}
      }
    }
  }
  None
}

fn is_top_level_lambda_let(stmt: &emerald_parser::Spanned<emerald_parser::Stmt>) -> bool {
  matches!(
    &stmt.node,
    emerald_parser::Stmt::Let {
      ty,
      value,
      ..
    } if ty == "Proc" && matches!(value.node, emerald_parser::Expr::Lambda { .. })
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-driver-require-graph-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn worked_example_levels_to_deps_then_entry() {
    let dir = fresh_dir("levels");
    std::fs::write(dir.join("b.em"), "def b_value -> Int64\n  10\nend\n").unwrap();
    std::fs::write(dir.join("c.em"), "def c_value -> Int64\n  20\nend\n").unwrap();
    std::fs::write(
      dir.join("main.em"),
      "require b\nrequire c\nputs b_value + c_value\n",
    )
    .unwrap();
    let graph = build_require_graph(&dir.join("main.em")).unwrap();
    let levels = compute_levels(&graph);
    assert_eq!(levels.len(), 2, "{levels:?}");
    let mut level0: Vec<String> = levels[0]
      .iter()
      .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
      .collect();
    level0.sort();
    assert_eq!(level0, vec!["b.em", "c.em"]);
    assert_eq!(levels[1].len(), 1);
    assert_eq!(
      levels[1][0].file_name().unwrap().to_string_lossy(),
      "main.em"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_cycle_never_reaches_compute_levels() {
    let dir = fresh_dir("cycle");
    std::fs::write(dir.join("a.em"), "require b\ndef a_fn -> Int64\n  1\nend\n").unwrap();
    std::fs::write(dir.join("b.em"), "require a\ndef b_fn -> Int64\n  2\nend\n").unwrap();
    let result = build_require_graph(&dir.join("a.em"));
    assert!(matches!(result, Err(DriverError::Require(_))), "{result:?}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn diamond_dependency_is_visited_once() {
    let dir = fresh_dir("diamond");
    std::fs::write(dir.join("base.em"), "def base_fn -> Int64\n  1\nend\n").unwrap();
    std::fs::write(
      dir.join("left.em"),
      "require base\ndef left_fn -> Int64\n  base_fn()\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("right.em"),
      "require base\ndef right_fn -> Int64\n  base_fn()\nend\n",
    )
    .unwrap();
    std::fs::write(
      dir.join("main.em"),
      "require left\nrequire right\nputs left_fn() + right_fn()\n",
    )
    .unwrap();
    let graph = build_require_graph(&dir.join("main.em")).unwrap();
    assert_eq!(graph.nodes.len(), 4);
    let closure = closure_items(&graph, &graph.entry.clone());
    // base_fn's own Function item appears exactly once in the fully
    // expanded closure, even though both left.em and right.em require
    // base.em — plan 23's diamond-dedup semantics, preserved.
    let base_fn_count = closure
      .iter()
      .filter(|i| matches!(i, Item::Function(f) if f.name == "base_fn"))
      .count();
    assert_eq!(base_fn_count, 1);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn missing_required_file_errors_not_panics() {
    let dir = fresh_dir("missing");
    std::fs::write(dir.join("main.em"), "require nope\nputs 1\n").unwrap();
    let result = build_require_graph(&dir.join("main.em"));
    assert!(matches!(result, Err(DriverError::Require(_))), "{result:?}");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_two_file_real_program_resolves_and_levels() {
    let dir = fresh_dir("twofile");
    std::fs::write(dir.join("helper.em"), "def helper() -> Int64\n  5\nend\n").unwrap();
    std::fs::write(dir.join("main.em"), "require helper\nputs helper()\n").unwrap();
    let graph = build_require_graph(&dir.join("main.em")).unwrap();
    let levels = compute_levels(&graph);
    assert_eq!(levels.len(), 2);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn worked_example_has_no_unsupported_constructs() {
    let dir = fresh_dir("unsupported-check");
    std::fs::write(dir.join("b.em"), "def b_value -> Int64\n  10\nend\n").unwrap();
    std::fs::write(dir.join("c.em"), "def c_value -> Int64\n  20\nend\n").unwrap();
    std::fs::write(
      dir.join("main.em"),
      "require b\nrequire c\nputs b_value + c_value\n",
    )
    .unwrap();
    let graph = build_require_graph(&dir.join("main.em")).unwrap();
    for path in graph.nodes.keys() {
      assert!(unsupported_construct(&graph, path).is_none());
    }
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn a_class_in_a_required_file_is_flagged_unsupported() {
    let dir = fresh_dir("class-unsupported");
    std::fs::write(
      dir.join("shapes.em"),
      "class Point\n  x: Int64\n\n  def initialize(x: Int64) -> Void\n    @x = x\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.em"), "require shapes\nputs 1\n").unwrap();
    let graph = build_require_graph(&dir.join("main.em")).unwrap();
    let shapes_path = graph
      .nodes
      .keys()
      .find(|p| p.file_name().unwrap() == "shapes.em")
      .unwrap()
      .clone();
    assert!(unsupported_construct(&graph, &shapes_path).is_some());
    std::fs::remove_dir_all(&dir).ok();
  }
}
