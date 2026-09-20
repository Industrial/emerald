//! `emerald format <file.em>|<directory>` (plan-of-plans row 78,
//! `canonical-formatter`) — a real pretty-printer for Emerald source,
//! rewriting each file in place with canonically-formatted output.
//! All of the actual formatting logic lives in the `emerald-fmt`
//! crate (a pure AST pretty-printer over `emerald-parser`'s own AST —
//! see that crate's own module doc comment for the chosen approach and
//! its one disclosed limitation, ordinary comments are lost); this
//! module is CLI-only glue: argument parsing, walking a directory
//! (mirroring `emerald lint`'s own `collect_em_files` convention, via
//! `emerald_fmt::collect_em_files`, so both subcommands treat a
//! project directory identically), reading/writing each file, and
//! rendering a parse failure through the same miette `fancy` graphical
//! handler `run_legacy`/`lint`/`doc` already use.
//!
//! A file whose formatted output is byte-for-byte identical to what's
//! already on disk is left untouched (no needless write, no misleading
//! "formatted" line) — `emerald format` run twice in a row on an
//! already-formatted project reports zero files changed both times,
//! the same idempotency this crate's own automated tests check
//! directly against `emerald_fmt::format_source`.
//!
//! Exit code convention (matching `emerald lint`'s own): `0` when every
//! file parsed successfully (whether or not any needed reformatting);
//! `1` when at least one file failed to parse; `2` for a usage/path
//! error.

use std::path::PathBuf;
use std::process;

pub fn run(args: &[String]) {
  let Some(target) = args.get(2) else {
    eprintln!("usage: emerald format <file.em>|<directory>");
    process::exit(2);
  };

  let path = PathBuf::from(target);
  let files = match emerald_fmt::collect_em_files(&path) {
    Ok(files) if !files.is_empty() => files,
    Ok(_) => {
      eprintln!("error: no `.em` files found at `{target}`");
      process::exit(2);
    }
    Err(e) => {
      eprintln!("error: cannot read `{target}`: {e}");
      process::exit(2);
    }
  };

  let mut had_error = false;
  let mut changed = 0usize;

  for file in &files {
    let source = match std::fs::read_to_string(file) {
      Ok(s) => s,
      Err(e) => {
        eprintln!("error: cannot read `{}`: {e}", file.display());
        had_error = true;
        continue;
      }
    };
    let name = file.display().to_string();
    match emerald_fmt::format_source(&source, &name) {
      Ok(formatted) => {
        if formatted != source {
          if let Err(e) = std::fs::write(file, &formatted) {
            eprintln!("error: cannot write `{}`: {e}", file.display());
            had_error = true;
            continue;
          }
          println!("formatted {}", file.display());
          changed += 1;
        }
      }
      Err(errs) => {
        had_error = true;
        for e in errs {
          eprintln!("{:?}", miette::Report::new(e));
        }
      }
    }
  }

  if files.len() > 1 || changed == 0 {
    println!(
      "emerald format: {} file{} checked, {} reformatted",
      files.len(),
      if files.len() == 1 { "" } else { "s" },
      changed
    );
  }

  if had_error {
    process::exit(1);
  }
}
