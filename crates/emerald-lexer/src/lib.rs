//! Prototype lexer for `02 toolchain-prototype` (plan-of-plans row 02).
//!
//! Token kinds map onto `spec/GRAMMAR.md`'s KEEP/MODIFY rows for the
//! inception §17 milestone-1 slice. Newlines are treated as insignificant
//! whitespace in this prototype; statement-termination semantics are a
//! parser-level decision deferred to `04 milestone1-front-end`.

use logos::Logos;

#[derive(Logos, Debug, Clone, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum Token<'src> {
  // Plan 71: `def` is deleted outright (no coexistence period) — `fn`
  // is the one reserved keyword for a function-shaped declaration now,
  // matching plan 59's `extern fn` spelling that this plan generalizes
  // to every function-shaped construct in the language.
  #[token("fn")]
  Fn,
  #[token("end")]
  End,

  #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
  Ident(&'src str),

  #[regex(r"[0-9]+")]
  Int(&'src str),

  #[regex(r#""[^"]*""#)]
  Str(&'src str),

  #[token("(")]
  LParen,
  #[token(")")]
  RParen,
  #[token(",")]
  Comma,
  #[token(":")]
  Colon,
  // Plan 71: `->` is removed entirely — every return-type position
  // (a function/method/interface signature) now uses `Colon` instead,
  // and the lambda literal's own leading `->` introducer is deleted
  // too (a lambda is a bare `do |params: T| ... end` block).
  #[token("+")]
  Plus,

  #[regex(r"@[A-Za-z_][A-Za-z0-9_]*")]
  InstanceVar(&'src str),
}

/// Lexes `src` into a token stream, or returns the byte offset of the
/// first character that matched no token pattern.
pub fn lex(src: &str) -> Result<Vec<Token<'_>>, usize> {
  let mut tokens = Vec::new();
  for (result, span) in Token::lexer(src).spanned() {
    match result {
      Ok(tok) => tokens.push(tok),
      Err(()) => return Err(span.start),
    }
  }
  Ok(tokens)
}

#[cfg(test)]
mod tests {
  use super::*;

  const HELLO_EM: &str = "fn add(a: Int64, b: Int64): Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn tokenizes_hello_em() {
    let tokens = lex(HELLO_EM).expect("hello.em should lex cleanly");
    assert_eq!(
      tokens,
      vec![
        Token::Fn,
        Token::Ident("add"),
        Token::LParen,
        Token::Ident("a"),
        Token::Colon,
        Token::Ident("Int64"),
        Token::Comma,
        Token::Ident("b"),
        Token::Colon,
        Token::Ident("Int64"),
        Token::RParen,
        Token::Colon,
        Token::Ident("Int64"),
        Token::Ident("a"),
        Token::Plus,
        Token::Ident("b"),
        Token::End,
        Token::Ident("puts"),
        Token::Ident("add"),
        Token::LParen,
        Token::Int("20"),
        Token::Comma,
        Token::Int("22"),
        Token::RParen,
      ]
    );
  }

  #[test]
  fn instance_var_lexes_but_class_var_errors() {
    // `@foo` is KEEP (spec/GRAMMAR.md §2); `@@foo` (class variable) is
    // REMOVE — the second `@` is not a valid instance-var start, so it
    // must fail to lex rather than silently succeeding.
    assert_eq!(lex("@foo"), Ok(vec![Token::InstanceVar("@foo")]));
    assert!(lex("@@foo").is_err(), "@@ (class variable) must not lex");
  }

  #[test]
  fn unterminated_string_errors_without_panicking() {
    assert!(lex("\"unterminated").is_err());
  }
}
