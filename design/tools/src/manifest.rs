//! Fixed table of platform assets this tool knows how to produce.
//!
//! Every size this project rasterizes to lives here, and nowhere else — both
//! `generate` and `verify` read from this single source of truth so the two
//! subcommands can never drift apart.

/// One rasterized output: a filename (written verbatim into `--out-dir`) and
/// the exact pixel dimensions it must be rendered at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
  pub filename: String,
  pub width: u32,
  pub height: u32,
}

/// One source SVG and the set of rasterized outputs derived from it.
#[derive(Debug, Clone)]
pub struct Entry {
  /// Source SVG filename, resolved relative to `--input-dir`.
  pub source: &'static str,
  pub outputs: Vec<Output>,
}

/// Favicon PNG sizes packed into `favicon.ico` (a subset of `mark.svg`'s
/// full favicon PNG set below).
pub const FAVICON_ICO_SIZES: [u32; 3] = [16, 32, 48];

/// Filename of the packed multi-resolution favicon, written into `--out-dir`.
pub const FAVICON_ICO_FILENAME: &str = "favicon.ico";

/// Full set of standalone favicon PNG sizes rendered from `mark.svg`.
const FAVICON_PNG_SIZES: [u32; 8] = [16, 32, 48, 64, 128, 192, 256, 512];

fn favicon_png_filename(size: u32) -> String {
  format!("favicon-{size}.png")
}

/// The fixed manifest: every source SVG this tool expects, and every
/// rasterized output derived from it.
///
/// - `mark.svg`: favicon PNGs at 16/32/48/64/128/192/256/512px, plus
///   `apple-touch-icon-180.png` at 180px. The 16/32/48 set is additionally
///   packed into `favicon.ico` (see [`FAVICON_ICO_SIZES`]).
/// - `banner.svg`: `social-preview-1280x640.png` at exactly 1280x640.
/// - `avatar.svg`: `avatar-192.png` and `avatar-512.png`.
pub fn entries() -> Vec<Entry> {
  let mut mark_outputs: Vec<Output> = FAVICON_PNG_SIZES
    .iter()
    .map(|&size| Output {
      filename: favicon_png_filename(size),
      width: size,
      height: size,
    })
    .collect();
  mark_outputs.push(Output {
    filename: "apple-touch-icon-180.png".to_string(),
    width: 180,
    height: 180,
  });

  vec![
    Entry {
      source: "mark.svg",
      outputs: mark_outputs,
    },
    Entry {
      source: "banner.svg",
      outputs: vec![Output {
        filename: "social-preview-1280x640.png".to_string(),
        width: 1280,
        height: 640,
      }],
    },
    Entry {
      source: "avatar.svg",
      outputs: vec![
        Output {
          filename: "avatar-192.png".to_string(),
          width: 192,
          height: 192,
        },
        Output {
          filename: "avatar-512.png".to_string(),
          width: 512,
          height: 512,
        },
      ],
    },
  ]
}

/// The flattened union of every entry's outputs, independent of which
/// source SVG produced them.
///
/// Used by single-file `generate --input` (apply the whole size table to
/// one SVG) and by `verify` (check the whole size table regardless of
/// source, since a rasterized PNG carries no record of its origin).
pub fn all_outputs() -> Vec<Output> {
  entries().into_iter().flat_map(|e| e.outputs).collect()
}

/// Paths (relative to `--out-dir`) of the PNGs packed into `favicon.ico`.
pub fn favicon_ico_source_filenames() -> Vec<String> {
  FAVICON_ICO_SIZES
    .iter()
    .copied()
    .map(favicon_png_filename)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn favicon_ico_sources_are_a_subset_of_marks_favicon_pngs() {
    let mark_filenames: Vec<String> = entries()
      .into_iter()
      .find(|e| e.source == "mark.svg")
      .expect("mark.svg entry present")
      .outputs
      .into_iter()
      .map(|o| o.filename)
      .collect();
    for filename in favicon_ico_source_filenames() {
      assert!(
        mark_filenames.contains(&filename),
        "favicon.ico source {filename} must be one of mark.svg's rendered outputs"
      );
    }
  }

  #[test]
  fn all_output_filenames_are_unique() {
    let outputs = all_outputs();
    let mut filenames: Vec<&str> = outputs.iter().map(|o| o.filename.as_str()).collect();
    filenames.sort_unstable();
    let mut deduped = filenames.clone();
    deduped.dedup();
    assert_eq!(
      filenames, deduped,
      "manifest has duplicate output filenames"
    );
  }
}
