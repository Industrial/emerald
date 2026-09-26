//! `emerald-design-tools`: rasterizes Emerald's SVG visual-identity sources
//! (mark, banner, avatar) into the fixed set of platform assets described in
//! `manifest.rs`, and verifies a previously generated output tree.

mod ico;
mod manifest;
mod render;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
  name = "emerald-design-tools",
  version,
  about = "Rasterizes Emerald's SVG identity sources to platform-specific assets."
)]
struct Cli {
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Rasterize SVG sources into the full manifest of platform assets.
  Generate {
    /// Directory containing the named source SVGs (mark.svg, banner.svg,
    /// avatar.svg). Mutually exclusive with --input.
    #[arg(long)]
    input_dir: Option<PathBuf>,
    /// Single SVG file to rasterize at every manifest size (single-file
    /// mode, used by this tool's own tests). Mutually exclusive with
    /// --input-dir.
    #[arg(long)]
    input: Option<PathBuf>,
    /// Directory to write rasterized PNGs and favicon.ico into.
    #[arg(long)]
    out_dir: PathBuf,
  },
  /// Re-check that every manifest-expected file exists in --dir with the
  /// correct pixel dimensions.
  Verify {
    /// Directory to check against the manifest.
    #[arg(long)]
    dir: PathBuf,
  },
}

fn main() -> Result<()> {
  let cli = Cli::parse();
  match cli.command {
    Command::Generate {
      input_dir,
      input,
      out_dir,
    } => generate(input_dir.as_deref(), input.as_deref(), &out_dir),
    Command::Verify { dir } => verify(&dir),
  }
}

fn generate(input_dir: Option<&Path>, input: Option<&Path>, out_dir: &Path) -> Result<()> {
  std::fs::create_dir_all(out_dir)
    .with_context(|| format!("creating output directory {}", out_dir.display()))?;

  match (input_dir, input) {
    (Some(dir), None) => generate_from_manifest(dir, out_dir),
    (None, Some(file)) => generate_single_file(file, out_dir),
    (Some(_), Some(_)) => bail!("--input-dir and --input are mutually exclusive"),
    (None, None) => bail!("one of --input-dir or --input is required"),
  }
}

/// Multi-file mode: each manifest entry's outputs are rendered from its own
/// named source SVG in `input_dir`.
fn generate_from_manifest(input_dir: &Path, out_dir: &Path) -> Result<()> {
  for entry in manifest::entries() {
    let svg_path = input_dir.join(entry.source);
    for output in &entry.outputs {
      let out_path = out_dir.join(&output.filename);
      render::render_svg_to_png(&svg_path, &out_path, output.width, output.height)?;
      println!("wrote {}", out_path.display());
    }
  }
  pack_favicon(out_dir)
}

/// Single-file mode: the whole manifest's size table is applied to one SVG,
/// so the tool's own tests can exercise the full pipeline (including
/// favicon.ico packing) against a placeholder fixture rather than the real
/// brand sources.
fn generate_single_file(svg_path: &Path, out_dir: &Path) -> Result<()> {
  for output in manifest::all_outputs() {
    let out_path = out_dir.join(&output.filename);
    render::render_svg_to_png(svg_path, &out_path, output.width, output.height)?;
    println!("wrote {}", out_path.display());
  }
  pack_favicon(out_dir)
}

fn pack_favicon(out_dir: &Path) -> Result<()> {
  let png_paths: Vec<PathBuf> = manifest::favicon_ico_source_filenames()
    .into_iter()
    .map(|filename| out_dir.join(filename))
    .collect();
  let ico_path = out_dir.join(manifest::FAVICON_ICO_FILENAME);
  ico::pack_favicon_ico(&png_paths, &ico_path)?;
  println!("wrote {}", ico_path.display());
  Ok(())
}

fn verify(dir: &Path) -> Result<()> {
  let mut failures: Vec<String> = Vec::new();

  for output in manifest::all_outputs() {
    let path = dir.join(&output.filename);
    if let Err(e) = check_png_dimensions(&path, output.width, output.height) {
      failures.push(format!("{}: {e}", path.display()));
    }
  }

  let ico_path = dir.join(manifest::FAVICON_ICO_FILENAME);
  if !ico_path.is_file() {
    failures.push(format!("{}: missing", ico_path.display()));
  }

  if failures.is_empty() {
    println!(
      "verify: all {} manifest output(s) present and correctly sized in {}",
      manifest::all_outputs().len() + 1,
      dir.display()
    );
    Ok(())
  } else {
    for failure in &failures {
      eprintln!("verify: {failure}");
    }
    bail!(
      "{} manifest output(s) missing or mismatched in {}",
      failures.len(),
      dir.display()
    );
  }
}

fn check_png_dimensions(path: &Path, expected_width: u32, expected_height: u32) -> Result<()> {
  if !path.is_file() {
    bail!("missing");
  }
  let (width, height) = image::ImageReader::open(path)
    .with_context(|| format!("opening {}", path.display()))?
    .with_guessed_format()
    .with_context(|| format!("guessing format of {}", path.display()))?
    .into_dimensions()
    .with_context(|| format!("reading dimensions of {}", path.display()))?;
  if (width, height) != (expected_width, expected_height) {
    bail!("expected {expected_width}x{expected_height}, found {width}x{height}");
  }
  Ok(())
}
