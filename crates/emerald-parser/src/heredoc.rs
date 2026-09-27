//! Plan 36: `<<~IDENT` squiggly heredocs.
//!
//! A raw-source preprocessing pass, run over the *entire* file before
//! `grammar.lalrpop` ever sees it — mirroring `collect_doc_comments`'s
//! own precedent in `lib.rs` (a pass over raw text, computed once, up
//! front). It has to be a raw-text pass rather than a `match {}` token
//! like every other terminal: matching `<<~IDENT ... IDENT` requires a
//! *backreference* (the closing line has to repeat whatever identifier
//! text followed `<<~`, not some fixed pattern), and Rust's `regex`
//! crate — what LALRPOP's own internal lexer is built on — has no
//! backreference support at all. No single regex can express this
//! token, at any priority, in that `match {}` block.
//!
//! # Syntax
//! `<<~IDENT` opens a heredoc, where `IDENT` is an ordinary identifier
//! (`grammar.lalrpop`'s own `Ident: String = <s:r"[A-Za-z_][A-Za-z0-9_]*">`
//! pattern, reused here verbatim for consistency). The body is every
//! line following the opener line, up to (not including) the first
//! line whose *trimmed* content exactly equals `IDENT` — the closing
//! marker may be indented, unlike Ruby's plain `<<IDENT` (flush-left
//! only). Only this one Ruby heredoc form is implemented: no `<<IDENT`,
//! no `<<-IDENT`, no quote-controlled interpolation suppression
//! (`<<~'IDENT'`).
//!
//! # Dedent
//! The minimum leading-whitespace-*character* count across all
//! non-blank body lines is stripped from every body line (mixed
//! space/tab indentation is counted in raw characters, not expanded tab
//! width — a real, narrow simplification against Ruby's own `<<~`,
//! which is otherwise byte-for-byte identical: this only differs from
//! Ruby when a heredoc mixes tabs and spaces across its own lines, an
//! edge case no example or test here exercises). Blank lines
//! (all-whitespace or empty) are left blank rather than dedented
//! against — and rather than raised as a "blank lines don't count
//! toward the minimum" special case for the *minimum computation*
//! either, exactly mirroring Ruby's own `<<~`.
//!
//! # Interpolation
//! The dedented body is re-encoded as the same textual form
//! `grammar.lalrpop`'s own `StringLitTok` regex
//! (`"([^"\\]|\\[n"])*"`) already accepts — every literal `"` becomes
//! `\"`, every real newline becomes the two-character escape `\n`
//! (`decode_string_lit`'s own supported escapes, see `ast.rs`) — then
//! wrapped in `"..."` and spliced back into the source AS TEXT at the
//! heredoc's position. `#{...}` spans are left untouched by this
//! escaping (neither `#` nor `{`/`}` are touched), so they survive
//! completely intact and the result flows through the entirely
//! unmodified normal `StringLitTok` -> `interpolate::
//! parse_interpolated_string` path, with zero grammar/AST changes.
//!
//! # Line-number preservation
//! Replacing a multi-line heredoc with a single rewritten literal
//! would shift every following line's number, unless compensated. This
//! pass pads its replacement with exactly as many `\n` characters as
//! the original heredoc block (opener line + every body line + the
//! terminator line) itself consumed, so the byte offset of every
//! character *after* the heredoc lands on the same line number it
//! would have without this pass. `lib.rs::parse_named` relies on this:
//! it reassigns its own `src` to this pass's output and uses that one
//! (expanded) string for everything downstream — `collect_doc_
//! comments`, the grammar parse itself, and every `ParseError` span —
//! so line numbers reported anywhere outside a heredoc's own body stay
//! byte-for-byte correct against the original file.
//!
//! # Disclosed limitations
//! - **No literal backslash in a heredoc body.** This is not a new gap
//!   heredocs introduce — `StringLitTok`'s own regex recognizes only
//!   `\"` and `\n` as escapes (see `decode_string_lit`'s doc comment in
//!   `ast.rs`); there has never been a way to put a literal `\` inside
//!   *any* `"..."` string literal in this language. A heredoc body
//!   containing a raw `\` is passed through unescaped by this pass
//!   (not doubled — doubling it would produce `\\`, which this
//!   grammar's regex doesn't accept as an escape either) and will fail
//!   to lex as part of the rewritten literal, exactly as it would if
//!   typed directly inside an ordinary `"..."` literal today.
//! - **`<<~` is always a heredoc opener, even mid-expression.** This
//!   pass has no operator-precedence context — it's pure raw-text
//!   scanning — so `a<<~b` (shift-left of `a` by the bitwise-NOT of
//!   `b`, with no surrounding whitespace) is indistinguishable from a
//!   heredoc opener and is always treated as one. Real Emerald source
//!   always spaces binary operators (`a << ~b`), so this isn't expected
//!   to misfire in practice; a real, narrow, disclosed ambiguity rather
//!   than a hidden one.
//! - **Diagnostics whose span falls inside a heredoc's own rewritten
//!   literal** (e.g. a malformed `#{...}` inside the body) report the
//!   correct LINE (the heredoc's own opening line, where the rewritten
//!   literal now lives), but any `miette` source snippet renders the
//!   rewritten, single-line, escaped form — not the original multi-line
//!   body — so column positions inside that snippet won't visually
//!   match the original body's layout. A real, disclosed column/
//!   snippet-fidelity tradeoff, not a line-number bug.
//! - **Only one heredoc opener per physical line.** Ruby allows several
//!   stacked heredocs on one line (`f(<<~A, <<~B)`); not implemented
//!   here — a second `<<~` on the same opener line is left untouched
//!   and scanned again as ordinary source text once this heredoc's
//!   consumed region ends.

use crate::ParseError;

/// Scans `src` once, left to right, copying everything through
/// unchanged except for `<<~IDENT ... IDENT` heredoc blocks, which are
/// rewritten in place into an ordinary escaped `"..."` string literal
/// (see the module doc comment). Skips over existing `"..."` string
/// literals and `#`-comments while scanning — using the exact same
/// character-class rules `grammar.lalrpop`'s own `StringLitTok`/`#[^\n]*`
/// patterns use — so a `<<~`-looking substring that's actually inside a
/// string literal or a comment is never mistaken for a real heredoc
/// opener (mirrors `collect_doc_comments`'s own precedent of scanning
/// raw text while staying aware of those two constructs).
///
/// Returns the rewritten source on success. An unterminated heredoc
/// (an opener with no line, before EOF, whose trimmed text exactly
/// matches the marker) is a real `ParseError` — carrying a real span
/// pointing at the `<<~IDENT` opener itself — not a panic.
pub fn expand_heredocs(src: &str, name: &str) -> Result<String, ParseError> {
  let len = src.len();
  let mut out = String::with_capacity(src.len());
  let mut i = 0usize;

  while i < len {
    let rest = &src[i..];
    let b = rest.as_bytes()[0];

    if b == b'#' {
      // Comment: `#[^\n]*` — copy through to (excluding) the next
      // newline, unchanged.
      let end = rest.find('\n').map_or(len, |p| i + p);
      out.push_str(&src[i..end]);
      i = end;
      continue;
    }

    if b == b'"' {
      // String literal: mirror `StringLitTok`'s own regex —
      // `"([^"\\]|\\[n"])*"` — exactly, so this scan never stops in
      // the middle of a real string literal's content (which could
      // otherwise contain a `<<~`-looking substring, or a `#`).
      let str_end = scan_string_literal(src, i);
      out.push_str(&src[i..str_end]);
      i = str_end;
      continue;
    }

    if rest.starts_with("<<~") {
      let after_tilde = i + 3;
      let ident_end = ident_end_at(src, after_tilde);
      if ident_end > after_tilde {
        let marker = &src[after_tilde..ident_end];
        let (replacement, consumed_end) = expand_one_heredoc(src, name, i, ident_end, marker)?;
        out.push_str(&replacement);
        i = consumed_end;
        continue;
      }
    }

    // Ordinary character — copy through unchanged. `chars().next()` is
    // used (rather than assuming one byte) so multi-byte UTF-8 content
    // elsewhere in the file is never split mid-codepoint.
    let ch_len = rest.chars().next().map_or(1, char::len_utf8);
    out.push_str(&src[i..i + ch_len]);
    i += ch_len;
  }

  Ok(out)
}

/// Scans a `"..."` string literal starting at `start` (which must
/// point at the opening `"`), returning the byte offset just past its
/// closing `"` — or `src.len()` if the literal is unterminated (an
/// already-invalid file; the real grammar parse a few lines later in
/// `parse_named` reports that properly, so this just stops scanning
/// for heredocs inside the runaway rest of the file rather than
/// guessing further).
fn scan_string_literal(src: &str, start: usize) -> usize {
  let bytes = src.as_bytes();
  let len = bytes.len();
  let mut j = start + 1;
  while j < len {
    if bytes[j] == b'\\' && j + 1 < len && (bytes[j + 1] == b'n' || bytes[j + 1] == b'"') {
      j += 2;
    } else if bytes[j] == b'"' {
      return j + 1;
    } else {
      j += 1;
    }
  }
  len
}

/// Returns the byte offset just past the identifier starting at
/// `start` (`grammar.lalrpop`'s own `[A-Za-z_][A-Za-z0-9_]*`), or
/// `start` itself if there's no identifier there at all.
fn ident_end_at(src: &str, start: usize) -> usize {
  let bytes = src.as_bytes();
  let len = bytes.len();
  if start >= len {
    return start;
  }
  let first = bytes[start];
  if !(first.is_ascii_alphabetic() || first == b'_') {
    return start;
  }
  let mut j = start + 1;
  while j < len && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
    j += 1;
  }
  j
}

/// Expands one `<<~IDENT` heredoc, given `heredoc_start` (the byte
/// offset of its leading `<`) and `marker_end` (the byte offset just
/// past `IDENT`). Returns `(replacement_text, consumed_end)`:
/// `replacement_text` is what should be spliced in starting at
/// `heredoc_start`, and `consumed_end` is the byte offset in the
/// ORIGINAL `src` where the caller's scan should resume (just past the
/// terminator line, or `src.len()` if the terminator was the file's
/// last line with no trailing newline).
fn expand_one_heredoc(
  src: &str,
  name: &str,
  heredoc_start: usize,
  marker_end: usize,
  marker: &str,
) -> Result<(String, usize), ParseError> {
  let len = src.len();

  // Whatever follows `IDENT` on the opener line itself (rare — usually
  // nothing but the newline) is preserved verbatim after the rewritten
  // literal.
  let opener_nl = src[marker_end..].find('\n').map(|p| marker_end + p);
  let trailing = match opener_nl {
    Some(nl) => &src[marker_end..nl],
    None => &src[marker_end..len],
  };
  let Some(opener_nl) = opener_nl else {
    return Err(unterminated_heredoc_error(
      name,
      src,
      heredoc_start,
      marker_end,
      marker,
    ));
  };

  let mut body_lines: Vec<&str> = Vec::new();
  let mut line_start = opener_nl + 1;
  let (terminator_end, terminator_has_nl) = loop {
    if line_start > len {
      return Err(unterminated_heredoc_error(
        name,
        src,
        heredoc_start,
        marker_end,
        marker,
      ));
    }
    let line_nl = src[line_start..].find('\n').map(|p| line_start + p);
    let (line, next_line_start, has_nl) = match line_nl {
      Some(nl) => (&src[line_start..nl], nl + 1, true),
      None => (&src[line_start..len], len + 1, false),
    };
    if line.trim() == marker {
      break (line_start, has_nl);
    }
    if line_nl.is_none() {
      // Last line of the file, and it isn't the terminator: no
      // terminator exists anywhere before EOF.
      return Err(unterminated_heredoc_error(
        name,
        src,
        heredoc_start,
        marker_end,
        marker,
      ));
    }
    body_lines.push(line);
    line_start = next_line_start;
  };

  // Dedent: the minimum leading-whitespace-character count across all
  // non-blank body lines, stripped from every body line; blank lines
  // stay blank.
  let min_indent = body_lines
    .iter()
    .filter(|l| !l.trim().is_empty())
    .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
    .min()
    .unwrap_or(0);

  let mut escaped_body = String::new();
  for line in &body_lines {
    let dedented = if line.trim().is_empty() {
      ""
    } else {
      &line[min_indent.min(line.len())..]
    };
    for c in dedented.chars() {
      match c {
        '"' => escaped_body.push_str("\\\""),
        other => escaped_body.push(other),
      }
    }
    // Ruby's own `<<~` includes a trailing newline after every body
    // line, the last one included — encoded as the `\n` escape
    // `StringLitTok`'s regex already understands, so the rewritten
    // literal stays a single physical line.
    escaped_body.push_str("\\n");
  }

  let quoted_literal = format!("\"{escaped_body}\"");

  // `terminator_end` (from the loop above) is the terminator line's own
  // START offset, not its end — recover the end of its own trailing
  // newline (one byte past the `\n` this exact line has) via the same
  // search.
  let terminator_line_start = terminator_end;
  let terminator_line_span_end = if terminator_has_nl {
    src[terminator_line_start..]
      .find('\n')
      .map_or(len, |p| terminator_line_start + p + 1)
  } else {
    len
  };

  // Every `\n` consumed by the ORIGINAL heredoc block (opener line's
  // own newline, one per body line, and the terminator line's newline
  // if it has one) must be reproduced by the replacement, so every
  // line number after this point stays exactly where it was.
  let newlines_removed = 1 + body_lines.len() + usize::from(terminator_has_nl);
  let pad = "\n".repeat(newlines_removed);

  let replacement = format!("{quoted_literal}{trailing}{pad}");
  Ok((replacement, terminator_line_span_end))
}

/// Builds the real, spanned `ParseError` for an opener with no
/// matching terminator line before EOF — same shape `lib.rs`'s own
/// `hoist_error` uses (a byte-offset span this pass already has in
/// hand, not one recovered from a `lalrpop_util::ParseError`).
fn unterminated_heredoc_error(
  name: &str,
  src: &str,
  heredoc_start: usize,
  marker_end: usize,
  marker: &str,
) -> ParseError {
  ParseError {
    message: format!(
      "unterminated heredoc: no line matching `{marker}` found before end of file (opened by `<<~{marker}`)"
    ),
    src: miette::NamedSource::new(name, src.to_string()),
    span: (heredoc_start, marker_end - heredoc_start).into(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::parse;

  #[test]
  fn basic_heredoc_expands_and_dedents() {
    let src = "text: String = <<~GREETING\n  Hello there\nGREETING\nputs text\n";
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    assert!(expanded.contains("\"Hello there\\n\""));
    // Line count preserved.
    assert_eq!(expanded.matches('\n').count(), src.matches('\n').count());
  }

  #[test]
  fn ragged_indentation_dedents_to_the_minimum() {
    let src = "t: String = <<~T\n    a\n  b\n      c\nT\n";
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    // Minimum indent across body lines is 2 (the "  b" line) — so
    // dedented lines are "  a", "b", "    c".
    assert!(expanded.contains("\"  a\\nb\\n    c\\n\""));
  }

  #[test]
  fn blank_lines_stay_blank_and_dont_affect_the_minimum() {
    let src = "t: String = <<~T\n    a\n\n    b\nT\n";
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    assert!(expanded.contains("\"a\\n\\nb\\n\""));
  }

  #[test]
  fn interpolation_inside_heredoc_survives_untouched() {
    let src = "n: String = \"x\"\nt: String = <<~T\n  hi #{n}!\nT\n";
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    assert!(expanded.contains("\"hi #{n}!\\n\""));
    // And it really compiles end-to-end through the full parser.
    let program = parse(&expanded).expect("expanded source should parse");
    assert!(!program.items.is_empty());
  }

  #[test]
  fn a_body_line_that_merely_resembles_the_terminator_is_not_mistaken_for_it() {
    // "GREETINGS" (plural) must not match the "GREETING" marker, and
    // "  GREETING" (extra leading text before trimming would still
    // just be "GREETING" after trim, so that one - `not GREETING` -
    // below is the real near-miss case: after trimming it's still not
    // an exact match).
    let src = "t: String = <<~GREETING\nGREETINGS\nnot GREETING\nGREETING\nT2\n";
    // (Trailing stray "T2" line just proves the real terminator, found
    // one line earlier, is the one actually used — this line 5 is
    // simply left over as ordinary trailing source text.)
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    assert!(expanded.contains("\"GREETINGS\\nnot GREETING\\n\""));
  }

  #[test]
  fn unterminated_heredoc_is_a_real_error_not_a_panic() {
    let src = "text: String = <<~GREETING\nHello\nputs text\n";
    let err = expand_heredocs(src, "<test>");
    assert!(err.is_err(), "expected an unterminated-heredoc error");
  }

  #[test]
  fn heredoc_preserves_line_numbers_for_code_after_it() {
    // A genuinely malformed top-level construct (`fn (` — a missing
    // function name) on line 5, right after a 2-body-line heredoc on
    // lines 1-4. `ParseError`'s `span` field is a byte offset, not a
    // line number, so this recomputes the line the same way `lib.rs`'s
    // own (private) `line_at` does — counting `\n` bytes up to that
    // offset — against the heredoc-EXPANDED source (what the real
    // `ParseError` span is actually measured against), then confirms
    // that lands on line 5, exactly where the malformed `fn` sits in
    // the ORIGINAL source. This is the real proof that expansion's
    // newline-padding keeps every line number after a heredoc correct,
    // not merely a spot-check of the message text.
    let src = "t: String = <<~T\n  body one\n  body two\nT\nfn (: Int64 do\nend\n";
    let errors = parse(src).expect_err("malformed fn on line 5 should be a real parse error");
    let err = errors.into_iter().next().unwrap();
    let offset = err.span.offset();
    let expanded = expand_heredocs(src, "<test>").expect("should expand");
    let line = 1
      + expanded.as_bytes()[..offset]
        .iter()
        .filter(|&&b| b == b'\n')
        .count();
    assert_eq!(
      line, 5,
      "expected the error on line 5 (after a 2-body-line heredoc), got line {line} \
       (offset {offset} in expanded source {expanded:?})"
    );
  }
}
