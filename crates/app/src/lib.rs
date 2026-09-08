//! Minimal placeholder crate for the workspace.
//!
//! Replace with real application code as the project grows.

/// Adds two numbers.
pub fn add(left: u64, right: u64) -> u64 {
  left + right
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn it_adds() {
    assert_eq!(add(2, 2), 4);
  }
}
