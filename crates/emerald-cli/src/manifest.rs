//! `emerald.toml` manifest parsing (plan 46's Decision log): `[package]`
//! `{name, version, entry}` and `[dependencies]` as `{path = "..."}` or
//! `{git = "...", rev = "..."}`. No registry, no semver constraint
//! solving — `version` is metadata only.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct PackageMeta {
  pub name: String,
  pub version: String,
  pub entry: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DependencySpec {
  Path { path: String },
  Git { git: String, rev: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
  pub package: PackageMeta,
  pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug)]
pub enum ManifestError {
  Io(std::io::Error),
  Parse(toml::de::Error),
  /// A `[dependencies]` entry that is neither exactly `{path = "..."}`
  /// nor exactly `{git = "...", rev = "..."}` — both keys present, or
  /// neither.
  InvalidDependency(String),
}

impl std::fmt::Display for ManifestError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      ManifestError::Io(e) => write!(f, "cannot read emerald.toml: {e}"),
      ManifestError::Parse(e) => write!(f, "malformed emerald.toml: {e}"),
      ManifestError::InvalidDependency(name) => write!(
        f,
        "dependency `{name}` must be exactly one of `{{ path = \"...\" }}` or `{{ git = \"...\", rev = \"...\" }}`"
      ),
    }
  }
}

impl std::error::Error for ManifestError {}

#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
  package: RawPackageMeta,
  #[serde(default)]
  dependencies: BTreeMap<String, RawDependencySpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPackageMeta {
  name: String,
  version: String,
  entry: Option<String>,
}

// Deliberately not `#[serde(untagged)]` on a two-variant enum: an
// untagged enum matched against a table carrying *both* `path` and
// `git`/`rev` would silently pick the first variant it fits (`Path`)
// and ignore the rest. Parsing into this flat, all-optional shape and
// validating the exact-one-of relationship by hand (see `Manifest::
// load_with_default_entry`) catches "both keys present" and "neither
// key present" identically and explicitly.
#[derive(Debug, Clone, Deserialize)]
struct RawDependencySpec {
  path: Option<String>,
  git: Option<String>,
  rev: Option<String>,
}

impl Manifest {
  /// Loads `<dir>/emerald.toml` as the top-level package being built —
  /// a missing `[package].entry` defaults to `"main.em"`.
  pub fn load(dir: &Path) -> Result<Manifest, ManifestError> {
    Self::load_with_default_entry(dir, "main.em")
  }

  /// Loads `<dir>/emerald.toml` as a dependency's own manifest — a
  /// missing `[package].entry` defaults to `"lib.em"`.
  pub fn load_as_dependency(dir: &Path) -> Result<Manifest, ManifestError> {
    Self::load_with_default_entry(dir, "lib.em")
  }

  fn load_with_default_entry(dir: &Path, default_entry: &str) -> Result<Manifest, ManifestError> {
    let text = std::fs::read_to_string(dir.join("emerald.toml")).map_err(ManifestError::Io)?;
    let raw: RawManifest = toml::from_str(&text).map_err(ManifestError::Parse)?;

    let mut dependencies = BTreeMap::new();
    for (name, spec) in raw.dependencies {
      let resolved = match (spec.path, spec.git, spec.rev) {
        (Some(path), None, None) => DependencySpec::Path { path },
        (None, Some(git), Some(rev)) => DependencySpec::Git { git, rev },
        _ => return Err(ManifestError::InvalidDependency(name)),
      };
      dependencies.insert(name, resolved);
    }

    Ok(Manifest {
      package: PackageMeta {
        name: raw.package.name,
        version: raw.package.version,
        entry: raw
          .package
          .entry
          .unwrap_or_else(|| default_entry.to_string()),
      },
      dependencies,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn write_manifest(dir: &Path, body: &str) {
    std::fs::write(dir.join("emerald.toml"), body).unwrap();
  }

  fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-cli-manifest-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn parses_a_path_dependency() {
    let dir = fresh_dir("path-dep");
    write_manifest(
      &dir,
      r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
mathutils = { path = "../mathutils" }
"#,
    );
    let manifest = Manifest::load(&dir).unwrap();
    assert_eq!(
      manifest.dependencies.get("mathutils"),
      Some(&DependencySpec::Path {
        path: "../mathutils".into()
      })
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn parses_a_git_dependency() {
    let dir = fresh_dir("git-dep");
    write_manifest(
      &dir,
      r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
mathutils = { git = "https://example.invalid/mathutils.git", rev = "v1.0.0" }
"#,
    );
    let manifest = Manifest::load(&dir).unwrap();
    assert_eq!(
      manifest.dependencies.get("mathutils"),
      Some(&DependencySpec::Git {
        git: "https://example.invalid/mathutils.git".into(),
        rev: "v1.0.0".into(),
      })
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn missing_entry_defaults_differ_by_role() {
    let dir = fresh_dir("default-entry");
    write_manifest(
      &dir,
      r#"
[package]
name = "app"
version = "0.1.0"
"#,
    );
    assert_eq!(Manifest::load(&dir).unwrap().package.entry, "main.em");
    assert_eq!(
      Manifest::load_as_dependency(&dir).unwrap().package.entry,
      "lib.em"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn rejects_malformed_toml_without_panicking() {
    let dir = fresh_dir("malformed-toml");
    write_manifest(&dir, "this is not [ valid toml");
    assert!(matches!(Manifest::load(&dir), Err(ManifestError::Parse(_))));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn rejects_a_dependency_table_with_both_path_and_git_keys() {
    let dir = fresh_dir("both-keys");
    write_manifest(
      &dir,
      r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
mathutils = { path = "../mathutils", git = "https://example.invalid/x.git", rev = "main" }
"#,
    );
    assert!(matches!(
      Manifest::load(&dir),
      Err(ManifestError::InvalidDependency(name)) if name == "mathutils"
    ));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn rejects_a_dependency_table_with_neither_path_nor_git_keys() {
    let dir = fresh_dir("neither-key");
    write_manifest(
      &dir,
      r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
mathutils = {}
"#,
    );
    assert!(matches!(
      Manifest::load(&dir),
      Err(ManifestError::InvalidDependency(name)) if name == "mathutils"
    ));
    std::fs::remove_dir_all(&dir).ok();
  }
}
