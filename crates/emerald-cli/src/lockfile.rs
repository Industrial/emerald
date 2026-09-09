//! `emerald.lock` — records the exact resolved path/git-commit-SHA per
//! dependency so a rebuild with an unchanged manifest re-resolves
//! nothing (plan 46's Decision log). `emerald update` is explicitly
//! not implemented here — see `main.rs`'s dispatch.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LockedSource {
  Path {
    path: String,
  },
  Git {
    git: String,
    rev: String,
    /// Always the fully resolved commit SHA, even when `rev` in the
    /// manifest is a loose ref (a branch or tag name).
    resolved_rev: String,
  },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedPackage {
  pub name: String,
  pub source: LockedSource,
  /// The resolved, canonicalized absolute path `deps/<name>` was last
  /// symlinked to.
  pub resolved_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lockfile {
  pub version: u32,
  #[serde(default)]
  pub packages: Vec<LockedPackage>,
}

impl Lockfile {
  /// `None` for a missing or unparseable lockfile — either way, the
  /// caller falls back to resolving every dependency fresh.
  pub fn load(dir: &Path) -> Option<Lockfile> {
    let text = std::fs::read_to_string(dir.join("emerald.lock")).ok()?;
    toml::from_str(&text).ok()
  }

  pub fn save(&self, dir: &Path) -> std::io::Result<()> {
    let text = toml::to_string_pretty(self).expect("Lockfile always serializes");
    std::fs::write(dir.join("emerald.lock"), text)
  }

  pub fn find(&self, name: &str) -> Option<&LockedPackage> {
    self.packages.iter().find(|p| p.name == name)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
      "emerald-cli-lockfile-test-{tag}-{}",
      std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  #[test]
  fn round_trips_a_path_and_a_git_entry() {
    let dir = fresh_dir("roundtrip");
    let lock = Lockfile {
      version: 1,
      packages: vec![
        LockedPackage {
          name: "mathutils".into(),
          source: LockedSource::Path {
            path: "../mathutils".into(),
          },
          resolved_path: "/abs/mathutils".into(),
        },
        LockedPackage {
          name: "other".into(),
          source: LockedSource::Git {
            git: "https://example.invalid/other.git".into(),
            rev: "main".into(),
            resolved_rev: "deadbeef".into(),
          },
          resolved_path: "/abs/.emerald/deps/other-deadbee".into(),
        },
      ],
    };
    lock.save(&dir).unwrap();
    let loaded = Lockfile::load(&dir).unwrap();
    assert_eq!(loaded, lock);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn missing_lockfile_loads_as_none() {
    let dir = fresh_dir("missing");
    assert!(Lockfile::load(&dir).is_none());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn find_looks_up_by_package_name() {
    let lock = Lockfile {
      version: 1,
      packages: vec![LockedPackage {
        name: "mathutils".into(),
        source: LockedSource::Path {
          path: "../mathutils".into(),
        },
        resolved_path: "/abs/mathutils".into(),
      }],
    };
    assert!(lock.find("mathutils").is_some());
    assert!(lock.find("nope").is_none());
  }
}
