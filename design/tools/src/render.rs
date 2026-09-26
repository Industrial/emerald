//! SVG -> PNG rasterization at a specific pixel size.
//!
//! Uses `resvg` + `usvg` for parsing/layout and `tiny-skia`'s default RGBA
//! render surface, which preserves a genuine alpha channel outside the
//! source SVG's opaque silhouette (needed for favicon/avatar transparency).

use std::path::Path;

use anyhow::{bail, Context, Result};

/// Render the SVG at `svg_path` to a PNG at exactly `width` x `height`
/// pixels, written to `out_path`. Non-uniform scaling is applied if the
/// source's own aspect ratio doesn't match the target's.
pub fn render_svg_to_png(svg_path: &Path, out_path: &Path, width: u32, height: u32) -> Result<()> {
  if width == 0 || height == 0 {
    bail!("target size must be non-zero, got {width}x{height}");
  }

  let svg_data = std::fs::read(svg_path)
    .with_context(|| format!("reading SVG source {}", svg_path.display()))?;

  let mut options = usvg::Options {
    resources_dir: svg_path.parent().map(Path::to_path_buf),
    ..usvg::Options::default()
  };
  // Sources are expected to be path-only (no live <text>), but load
  // system fonts defensively in case a source SVG does carry text.
  options.fontdb_mut().load_system_fonts();

  let tree = usvg::Tree::from_data(&svg_data, &options)
    .with_context(|| format!("parsing SVG {}", svg_path.display()))?;

  let source_size = tree.size();
  let (source_width, source_height) = (source_size.width(), source_size.height());
  if source_width <= 0.0 || source_height <= 0.0 {
    bail!(
      "SVG {} has a non-positive size ({source_width}x{source_height})",
      svg_path.display()
    );
  }

  let mut pixmap = tiny_skia::Pixmap::new(width, height)
    .with_context(|| format!("allocating a {width}x{height} render surface"))?;

  let transform =
    tiny_skia::Transform::from_scale(width as f32 / source_width, height as f32 / source_height);
  resvg::render(&tree, transform, &mut pixmap.as_mut());

  if let Some(parent) = out_path.parent() {
    std::fs::create_dir_all(parent)
      .with_context(|| format!("creating output directory {}", parent.display()))?;
  }

  pixmap
    .save_png(out_path)
    .with_context(|| format!("writing PNG {}", out_path.display()))?;

  Ok(())
}
