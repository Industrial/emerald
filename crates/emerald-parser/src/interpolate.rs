//! Plan 36: string interpolation is a *compile-time* parse, never a
//! runtime `eval` — each `#{...}` span found inside a decoded string
//! literal is re-parsed by the exact same static expression grammar
//! that parses the rest of the program (a second, independent LALRPOP
//! entry point, `grammar::grammar::ExprParser`), not deferred to
//! anything dynamic. See `grammar.lalrpop`'s `StringLitTok` call site
//! and this plan's own Decision log.

use crate::ast::{Expr, StringPart};
use crate::grammar;

/// `decoded` is already quote-stripped and `\"`/`\n`-decoded (`String
/// LitTok`'s own action already did that before calling this) — this
/// function's only job is finding `#{...}` spans inside it. A brace-
/// depth scan (not naive first-`}` matching) means a nested hash
/// literal like `#{ {1 => 2}[1] }` is handled correctly. A string with
/// zero `#{` spans returns a plain `Expr::StringLit`, unchanged from
/// plan 19's own behavior — the backward-compatibility guarantee this
/// plan's Decision log calls for, so the overwhelmingly common
/// non-interpolating case pays no new codegen cost.
pub fn parse_interpolated_string(decoded: &str) -> Result<Expr, String> {
  let mut parts = Vec::new();
  let mut literal = String::new();
  let mut has_interpolation = false;
  let mut chars = decoded.char_indices();

  while let Some((idx, c)) = chars.next() {
    if c == '#' && decoded[idx..].starts_with("#{") {
      has_interpolation = true;
      if !literal.is_empty() {
        parts.push(StringPart::Literal(std::mem::take(&mut literal)));
      }
      chars.next(); // consume the '{' the `starts_with` check above found
      let expr_start = idx + 2; // '#' and '{' are both one ASCII byte
      let mut depth = 1usize;
      let mut expr_end = None;
      for (j, cc) in chars.by_ref() {
        match cc {
          '{' => depth += 1,
          '}' => {
            depth -= 1;
            if depth == 0 {
              expr_end = Some(j);
              break;
            }
          }
          _ => {}
        }
      }
      let Some(expr_end) = expr_end else {
        return Err(format!(
          "unterminated `#{{...}}` interpolation in string literal (opened at byte {idx})"
        ));
      };
      let expr_src = &decoded[expr_start..expr_end];
      // `ExprParser::parse` still takes the grammar-level `errors`
      // accumulator plan 26 threads into every generated entry point
      // (LALRPOP's parameterized-grammar mechanism applies uniformly,
      // even though `Expr`'s own productions never reach a `!` marker —
      // that's `Item`-only) — a fresh, always-empty `Vec` here, since a
      // malformed `#{...}` span is a real hard error, not recovered.
      // Plan 77's `docs` map is threaded the same uniform way — always
      // empty here too, since `Expr`'s own productions never look one
      // up (only `Item`-level declaration productions do).
      let mut errors = Vec::new();
      let docs = std::collections::HashMap::new();
      let parsed_expr = grammar::grammar::ExprParser::new()
        .parse(&mut errors, &docs, expr_src)
        .map_err(|e| {
          format!("invalid expression in string interpolation `#{{{expr_src}}}`: {e}")
        })?;
      parts.push(StringPart::Expr(Box::new(parsed_expr)));
    } else {
      literal.push(c);
    }
  }

  if !has_interpolation {
    return Ok(Expr::StringLit(literal));
  }
  if !literal.is_empty() {
    parts.push(StringPart::Literal(literal));
  }
  Ok(Expr::Interpolate(parts))
}
