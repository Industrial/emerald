//! Path/git dependency resolution (plan 46's Decision log). A path
//! dependency is canonicalized in place; a git dependency is cloned
//! into `.emerald/deps/<name>-<sha-prefix>/` and checked out at the
//! requested `rev`. Either way, `<manifest_dir>/deps/<name>` is
//! created (or replaced) as a symlink to the resolved root — the only
//! thing plan 23's `require`-resolution rule (see `require.rs`) needs
//! to find it, since a dot-free `deps/<name>/...` path is already an
//! ordinary path that rule can walk unmodified.
//!
//! No transitive resolution: a dependency's own `[dependencies]` table
//! is never read, only the package currently being built's own.

use crate::lockfile::{LockedPackage, LockedSource, Lockfile};
use crate::manifest::{DependencySpec, Manifest, ManifestError};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedDependency {
  pub name: String,
  pub root: PathBuf,
}

#[derive(Debug)]
pub enum DepsError {
  MissingPath(PathBuf),
  Git(String),
  Io(std::io::Error),
  /// A dependency's own `emerald.toml` (read only for validation — its
  /// `[dependencies]` table is never followed, see this module's own
  /// doc comment) is malformed.
  DependencyManifest(String, ManifestError),
}

impl std::fmt::Display for DepsError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      DepsError::MissingPath(p) => write!(f, "dependency path `{}` does not exist", p.display()),
      DepsError::Git(msg) => write!(f, "{msg}"),
      DepsError::Io(e) => write!(f, "{e}"),
      DepsError::DependencyManifest(name, e) => {
        write!(f, "dependency `{name}`'s own emerald.toml is invalid: {e}")
      }
    }
  }
}

impl std::error::Error for DepsError {}

/// Resolves every dependency in `manifest`, unconditionally — no
/// lockfile involved. `resolve_or_reuse` (below) is what a real build
/// should call; this is exposed directly for the plain "fetch
/// everything now" case and for `resolve_or_reuse`'s own re-resolve
/// path.
pub fn resolve_dependencies(
  manifest_dir: &Path,
  manifest: &Manifest,
) -> Result<Vec<ResolvedDependency>, DepsError> {
  if manifest.dependencies.is_empty() {
    return Ok(Vec::new());
  }
  let deps_dir = manifest_dir.join("deps");
  std::fs::create_dir_all(&deps_dir).map_err(DepsError::Io)?;

  let mut out = Vec::new();
  for (name, spec) in &manifest.dependencies {
    let root = resolve_one(manifest_dir, name, spec)?;
    link_dependency(&deps_dir, name, &root)?;
    out.push(ResolvedDependency {
      name: name.clone(),
      root,
    });
  }
  Ok(out)
}

/// Resolves each dependency, reusing the lockfile's recorded
/// path/resolved-SHA (no `git`/filesystem-canonicalize call at all)
/// whenever the manifest's spec for that name still matches what's
/// locked; a changed or newly-added spec is fully re-resolved. Writes
/// `<manifest_dir>/emerald.lock` with the result either way.
pub fn resolve_or_reuse(
  manifest_dir: &Path,
  manifest: &Manifest,
  existing_lock: Option<&Lockfile>,
) -> Result<Vec<ResolvedDependency>, DepsError> {
  let (resolved, locked_packages) = match existing_lock {
    // No lockfile to consult at all (first build) — every dependency
    // is freshly resolved either way, so this is exactly what
    // `resolve_dependencies` already does; reuse it rather than
    // duplicating the "resolve everything" loop.
    None => resolve_all_fresh(manifest_dir, manifest)?,
    Some(lock) => resolve_reusing_lock(manifest_dir, manifest, lock)?,
  };
  Lockfile {
    version: 1,
    packages: locked_packages,
  }
  .save(manifest_dir)
  .map_err(DepsError::Io)?;
  Ok(resolved)
}

fn resolve_all_fresh(
  manifest_dir: &Path,
  manifest: &Manifest,
) -> Result<(Vec<ResolvedDependency>, Vec<LockedPackage>), DepsError> {
  let resolved = resolve_dependencies(manifest_dir, manifest)?;
  let mut locked_packages = Vec::with_capacity(resolved.len());
  for dep in &resolved {
    let spec = &manifest.dependencies[&dep.name];
    locked_packages.push(LockedPackage {
      name: dep.name.clone(),
      source: locked_source_for(spec, &dep.root)?,
      resolved_path: dep.root.to_string_lossy().into_owned(),
    });
  }
  Ok((resolved, locked_packages))
}

fn resolve_reusing_lock(
  manifest_dir: &Path,
  manifest: &Manifest,
  lock: &Lockfile,
) -> Result<(Vec<ResolvedDependency>, Vec<LockedPackage>), DepsError> {
  let mut resolved = Vec::new();
  let mut locked_packages = Vec::new();
  if manifest.dependencies.is_empty() {
    return Ok((resolved, locked_packages));
  }

  let deps_dir = manifest_dir.join("deps");
  std::fs::create_dir_all(&deps_dir).map_err(DepsError::Io)?;

  for (name, spec) in &manifest.dependencies {
    let reused = lock
      .find(name)
      .filter(|locked| spec_matches(spec, &locked.source));
    let (root, source) = match reused {
      Some(locked) => (PathBuf::from(&locked.resolved_path), locked.source.clone()),
      None => {
        let root = resolve_one(manifest_dir, name, spec)?;
        let source = locked_source_for(spec, &root)?;
        (root, source)
      }
    };

    link_dependency(&deps_dir, name, &root)?;
    locked_packages.push(LockedPackage {
      name: name.clone(),
      source,
      resolved_path: root.to_string_lossy().into_owned(),
    });
    resolved.push(ResolvedDependency {
      name: name.clone(),
      root,
    });
  }
  Ok((resolved, locked_packages))
}

fn locked_source_for(spec: &DependencySpec, root: &Path) -> Result<LockedSource, DepsError> {
  Ok(match spec {
    DependencySpec::Path { path } => LockedSource::Path { path: path.clone() },
    DependencySpec::Git { git, rev } => LockedSource::Git {
      git: git.clone(),
      rev: rev.clone(),
      resolved_rev: run_git_capture(root, &["rev-parse", "HEAD"])?,
    },
  })
}

fn spec_matches(spec: &DependencySpec, source: &LockedSource) -> bool {
  match (spec, source) {
    (DependencySpec::Path { path }, LockedSource::Path { path: locked_path }) => {
      path == locked_path
    }
    (
      DependencySpec::Git { git, rev },
      LockedSource::Git {
        git: locked_git,
        rev: locked_rev,
        ..
      },
    ) => git == locked_git && rev == locked_rev,
    _ => false,
  }
}

fn resolve_one(
  manifest_dir: &Path,
  name: &str,
  spec: &DependencySpec,
) -> Result<PathBuf, DepsError> {
  let root = match spec {
    DependencySpec::Path { path } => {
      let candidate = manifest_dir.join(path);
      std::fs::canonicalize(&candidate).map_err(|_| DepsError::MissingPath(candidate))?
    }
    DependencySpec::Git { git, rev } => {
      let cache_dir = manifest_dir.join(".emerald").join("deps");
      std::fs::create_dir_all(&cache_dir).map_err(DepsError::Io)?;
      clone_and_checkout(git, rev, &cache_dir, name)?
    }
  };
  // Read only for validation — a dependency's own `[dependencies]`
  // table is never opened (see this module's doc comment). A
  // dependency with no `emerald.toml` at all (a bare directory of
  // `.em` files) is perfectly legal, so this is skipped, not required.
  if root.join("emerald.toml").exists() {
    Manifest::load_as_dependency(&root)
      .map_err(|e| DepsError::DependencyManifest(name.to_string(), e))?;
  }
  Ok(root)
}

fn clone_and_checkout(
  url: &str,
  rev: &str,
  cache_dir: &Path,
  name: &str,
) -> Result<PathBuf, DepsError> {
  let scratch = cache_dir.join(format!("{name}-scratch"));
  if scratch.exists() {
    std::fs::remove_dir_all(&scratch).map_err(DepsError::Io)?;
  }
  run_git(
    None,
    &[
      "clone",
      url,
      scratch.to_str().expect("scratch path is utf8"),
    ],
  )?;
  run_git(Some(&scratch), &["checkout", rev])?;
  let sha = run_git_capture(&scratch, &["rev-parse", "HEAD"])?;
  let sha_prefix = &sha[..sha.len().min(7)];
  let final_dir = cache_dir.join(format!("{name}-{sha_prefix}"));
  if final_dir.exists() {
    // Another branch/tag already resolved to this same commit —
    // share its cache directory instead of keeping a second copy.
    std::fs::remove_dir_all(&scratch).ok();
  } else {
    std::fs::rename(&scratch, &final_dir).map_err(DepsError::Io)?;
  }
  std::fs::canonicalize(&final_dir).map_err(DepsError::Io)
}

fn run_git(cwd: Option<&Path>, args: &[&str]) -> Result<(), DepsError> {
  run_git_command(cwd, args).map(|_| ())
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Result<String, DepsError> {
  run_git_command(Some(cwd), args)
}

fn run_git_command(cwd: Option<&Path>, args: &[&str]) -> Result<String, DepsError> {
  let mut cmd = Command::new("git");
  cmd.args(args);
  if let Some(dir) = cwd {
    cmd.current_dir(dir);
  }
  let output = cmd
    .output()
    .map_err(|e| DepsError::Git(format!("failed to invoke git: {e}")))?;
  if !output.status.success() {
    return Err(DepsError::Git(format!(
      "git {} failed (exit {}): {}",
      args.join(" "),
      output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".into()),
      String::from_utf8_lossy(&output.stderr).trim()
    )));
  }
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn link_dependency(deps_dir: &Path, name: &str, target: &Path) -> Result<(), DepsError> {
  let link_path = deps_dir.join(name);
  if let Ok(meta) = std::fs::symlink_metadata(&link_path) {
    if meta.file_type().is_dir() {
      std::fs::remove_dir_all(&link_path).map_err(DepsError::Io)?;
    } else {
      std::fs::remove_file(&link_path).map_err(DepsError::Io)?;
    }
  }
  std::os::unix::fs::symlink(target, &link_path).map_err(DepsError::Io)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::BTreeMap;

  fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-cli-deps-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  fn manifest_with(deps: BTreeMap<String, DependencySpec>) -> Manifest {
    Manifest {
      package: crate::manifest::PackageMeta {
        name: "app".into(),
        version: "0.1.0".into(),
        entry: "main.em".into(),
      },
      dependencies: deps,
    }
  }

  fn git_fixture_repo(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let run = |args: &[&str]| {
      let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .status()
        .unwrap();
      assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("lib.em"), "def five() -> Int64\n  5\nend\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "initial"]);
    dir.to_string_lossy().into_owned()
  }

  #[test]
  fn resolves_a_path_dependency_and_symlinks_it() {
    let root = fresh_dir("path");
    let lib_dir = root.join("mathutils");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(lib_dir.join("lib.em"), "def add() -> Int64\n  8\nend\n").unwrap();

    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Path {
        path: "../mathutils".into(),
      },
    );
    let manifest = manifest_with(deps);

    let resolved = resolve_dependencies(&app_dir, &manifest).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].root, std::fs::canonicalize(&lib_dir).unwrap());

    let link_target = std::fs::read_link(app_dir.join("deps").join("mathutils")).unwrap();
    assert!(link_target.join("lib.em").exists());
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn resolves_a_git_dependency_from_a_local_fixture_repo() {
    let root = fresh_dir("git");
    let remote = root.join("remote");
    git_fixture_repo(&remote);

    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Git {
        git: remote.to_string_lossy().into_owned(),
        rev: "main".to_string(),
      },
    );
    let manifest = manifest_with(deps);

    let resolved = resolve_dependencies(&app_dir, &manifest).unwrap();
    assert_eq!(resolved.len(), 1);
    assert!(
      resolved[0]
        .root
        .to_string_lossy()
        .contains(".emerald/deps/mathutils-")
    );

    let link_target = std::fs::read_link(app_dir.join("deps").join("mathutils")).unwrap();
    assert!(link_target.join("lib.em").exists());
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn missing_path_dependency_errors_not_panics() {
    let app_dir = fresh_dir("missing-path");
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Path {
        path: "../does-not-exist".into(),
      },
    );
    let manifest = manifest_with(deps);
    let err = resolve_dependencies(&app_dir, &manifest).unwrap_err();
    assert!(matches!(err, DepsError::MissingPath(_)));
    std::fs::remove_dir_all(&app_dir).ok();
  }

  #[test]
  fn failed_git_clone_errors_with_gits_own_status() {
    let app_dir = fresh_dir("git-fail");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Git {
        git: app_dir
          .join("no-such-remote")
          .to_string_lossy()
          .into_owned(),
        rev: "main".into(),
      },
    );
    let manifest = manifest_with(deps);
    let err = resolve_dependencies(&app_dir, &manifest).unwrap_err();
    assert!(matches!(err, DepsError::Git(_)));
    std::fs::remove_dir_all(&app_dir).ok();
  }

  #[test]
  fn empty_dependencies_resolves_to_nothing_and_creates_no_deps_dir() {
    let app_dir = fresh_dir("empty");
    std::fs::create_dir_all(&app_dir).unwrap();
    let manifest = manifest_with(BTreeMap::new());
    let resolved = resolve_dependencies(&app_dir, &manifest).unwrap();
    assert!(resolved.is_empty());
    assert!(!app_dir.join("deps").exists());
    std::fs::remove_dir_all(&app_dir).ok();
  }

  #[test]
  fn first_build_creates_a_lockfile_pinning_the_resolved_path() {
    let root = fresh_dir("lock-first");
    let lib_dir = root.join("mathutils");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(lib_dir.join("lib.em"), "").unwrap();
    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Path {
        path: "../mathutils".into(),
      },
    );
    let manifest = manifest_with(deps);

    resolve_or_reuse(&app_dir, &manifest, None).unwrap();
    let lock = Lockfile::load(&app_dir).unwrap();
    let locked = lock.find("mathutils").unwrap();
    assert_eq!(
      locked.resolved_path,
      std::fs::canonicalize(&lib_dir).unwrap().to_string_lossy()
    );
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn a_matching_lock_entry_for_git_skips_re_invoking_git() {
    let root = fresh_dir("lock-skip-git");
    let remote = root.join("remote");
    git_fixture_repo(&remote);

    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Git {
        git: remote.to_string_lossy().into_owned(),
        rev: "main".to_string(),
      },
    );
    let manifest = manifest_with(deps);

    resolve_or_reuse(&app_dir, &manifest, None).unwrap();
    let lock = Lockfile::load(&app_dir).unwrap();

    // Make the remote unreachable — a second, spec-unchanged resolve
    // must still succeed, proving it never re-invoked `git clone`.
    std::fs::remove_dir_all(&remote).unwrap();

    let resolved = resolve_or_reuse(&app_dir, &manifest, Some(&lock)).unwrap();
    assert_eq!(resolved.len(), 1);
    std::fs::remove_dir_all(&root).ok();
  }

  #[test]
  fn editing_a_dependencys_spec_re_resolves_only_that_dependency() {
    let root = fresh_dir("lock-respec");
    let lib_v1 = root.join("mathutils_v1");
    let lib_v2 = root.join("mathutils_v2");
    std::fs::create_dir_all(&lib_v1).unwrap();
    std::fs::create_dir_all(&lib_v2).unwrap();
    std::fs::write(lib_v1.join("lib.em"), "").unwrap();
    std::fs::write(lib_v2.join("lib.em"), "").unwrap();

    let app_dir = root.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let mut deps = BTreeMap::new();
    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Path {
        path: "../mathutils_v1".into(),
      },
    );
    let manifest_v1 = manifest_with(deps.clone());
    resolve_or_reuse(&app_dir, &manifest_v1, None).unwrap();
    let lock_v1 = Lockfile::load(&app_dir).unwrap();

    deps.insert(
      "mathutils".to_string(),
      DependencySpec::Path {
        path: "../mathutils_v2".into(),
      },
    );
    let manifest_v2 = manifest_with(deps);
    let resolved_v2 = resolve_or_reuse(&app_dir, &manifest_v2, Some(&lock_v1)).unwrap();
    assert_eq!(resolved_v2[0].root, std::fs::canonicalize(&lib_v2).unwrap());

    let lock_v2 = Lockfile::load(&app_dir).unwrap();
    assert_eq!(
      lock_v2.find("mathutils").unwrap().resolved_path,
      std::fs::canonicalize(&lib_v2).unwrap().to_string_lossy()
    );
    std::fs::remove_dir_all(&root).ok();
  }
}
