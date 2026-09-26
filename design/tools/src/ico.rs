//! Pack a set of already-rasterized PNGs into a multi-resolution `.ico`.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Read each PNG in `png_paths` (already rendered to disk) and pack them
/// together into a single multi-resolution favicon at `out_path`.
pub fn pack_favicon_ico(png_paths: &[PathBuf], out_path: &Path) -> Result<()> {
  let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

  for png_path in png_paths {
    let file = File::open(png_path)
      .with_context(|| format!("opening {} to pack into favicon.ico", png_path.display()))?;
    let image = ico::IconImage::read_png(file)
      .with_context(|| format!("decoding {} as PNG for ico packing", png_path.display()))?;
    let entry = ico::IconDirEntry::encode(&image)
      .with_context(|| format!("encoding ico entry from {}", png_path.display()))?;
    icon_dir.add_entry(entry);
  }

  if let Some(parent) = out_path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("creating output directory {}", parent.display()))?;
  }

  let out_file =
    File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
  icon_dir
    .write(out_file)
    .with_context(|| format!("writing {}", out_path.display()))?;

  Ok(())
}
