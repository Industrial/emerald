//! Integration test driving the `generate` and `verify` subcommands against
//! the placeholder fixture (`tests/fixtures/sample.svg`), exercising the
//! full manifest pipeline end to end.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
  Command::new(env!("CARGO_BIN_EXE_emerald-design-tools"))
}

fn fixture_path() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.svg")
}

/// Every filename + expected pixel size the manifest is supposed to
/// produce, mirrored here (independent of `src/manifest.rs`) so the test
/// actually pins down the CLI's observable behavior rather than just
/// re-checking the manifest against itself.
fn expected_outputs() -> Vec<(&'static str, u32, u32)> {
  vec![
    ("favicon-16.png", 16, 16),
    ("favicon-32.png", 32, 32),
    ("favicon-48.png", 48, 48),
    ("favicon-64.png", 64, 64),
    ("favicon-128.png", 128, 128),
    ("favicon-192.png", 192, 192),
    ("favicon-256.png", 256, 256),
    ("favicon-512.png", 512, 512),
    ("apple-touch-icon-180.png", 180, 180),
    ("social-preview-1280x640.png", 1280, 640),
    ("avatar-192.png", 192, 192),
    ("avatar-512.png", 512, 512),
  ]
}

#[test]
fn generate_single_file_produces_every_manifest_output_at_correct_dimensions() {
  let out_dir = tempfile::tempdir().expect("create temp out-dir");

  let status = bin()
    .args(["generate", "--input"])
    .arg(fixture_path())
    .args(["--out-dir"])
    .arg(out_dir.path())
    .status()
    .expect("run generate");
  assert!(status.success(), "generate exited non-zero: {status:?}");

  for (filename, expected_width, expected_height) in expected_outputs() {
    let path = out_dir.path().join(filename);
    assert!(
      path.is_file(),
      "expected output file missing: {}",
      path.display()
    );

    let (width, height) = image::ImageReader::open(&path)
      .unwrap_or_else(|e| panic!("opening {}: {e}", path.display()))
      .with_guessed_format()
      .unwrap_or_else(|e| panic!("guessing format of {}: {e}", path.display()))
      .into_dimensions()
      .unwrap_or_else(|e| panic!("reading dimensions of {}: {e}", path.display()));
    assert_eq!(
      (width, height),
      (expected_width, expected_height),
      "{} has wrong dimensions",
      path.display()
    );
  }

  // favicon.ico: a valid multi-resolution icon packing the 16/32/48 set.
  let ico_path = out_dir.path().join("favicon.ico");
  assert!(ico_path.is_file(), "favicon.ico missing");
  let ico_file = std::fs::File::open(&ico_path).expect("open favicon.ico");
  let icon_dir = ico::IconDir::read(ico_file).expect("parse favicon.ico");
  let mut sizes: Vec<u32> = icon_dir.entries().iter().map(|e| e.width()).collect();
  sizes.sort_unstable();
  assert_eq!(
    sizes,
    vec![16, 32, 48],
    "favicon.ico entries don't match the expected size set"
  );

  // Favicon and avatar PNGs must carry a genuine alpha channel — the
  // fixture's rect doesn't fill the full canvas, so corner pixels outside
  // its rounded silhouette must be transparent.
  for filename in ["favicon-64.png", "avatar-192.png"] {
    let path = out_dir.path().join(filename);
    let img = image::ImageReader::open(&path)
      .unwrap_or_else(|e| panic!("opening {}: {e}", path.display()))
      .decode()
      .unwrap_or_else(|e| panic!("decoding {}: {e}", path.display()))
      .into_rgba8();
    let corner = img.get_pixel(0, 0);
    assert_eq!(
      corner[3],
      0,
      "{} corner pixel should be fully transparent (alpha=0), got {:?}",
      path.display(),
      corner
    );
  }
}

#[test]
fn verify_passes_on_a_complete_set_and_fails_naming_a_missing_file() {
  let out_dir = tempfile::tempdir().expect("create temp out-dir");

  let status = bin()
    .args(["generate", "--input"])
    .arg(fixture_path())
    .args(["--out-dir"])
    .arg(out_dir.path())
    .status()
    .expect("run generate");
  assert!(status.success());

  let verify_ok = bin()
    .args(["verify", "--dir"])
    .arg(out_dir.path())
    .status()
    .expect("run verify");
  assert!(verify_ok.success(), "verify should pass on a complete set");

  // Remove one expected file and confirm verify fails, naming it.
  let missing = out_dir.path().join("avatar-512.png");
  std::fs::remove_file(&missing).expect("remove avatar-512.png");

  let verify_output = bin()
    .args(["verify", "--dir"])
    .arg(out_dir.path())
    .output()
    .expect("run verify after removing a file");
  assert!(
    !verify_output.status.success(),
    "verify should fail on an incomplete set"
  );
  let stderr = String::from_utf8_lossy(&verify_output.stderr);
  assert!(
    stderr.contains("avatar-512.png"),
    "verify stderr should name the missing file, got: {stderr}"
  );
}
