//! Emerald's parser, built on LALRPOP — chosen in `02 toolchain-prototype`
//! over Chumsky (see `spec/COMPILER.md` for the decision record). Currently
//! covers only the inception §17 milestone-1 grammar slice; later
//! milestones (`04`+) extend `grammar.lalrpop` as the real grammar grows.

pub mod ast;

#[allow(clippy::all)]
mod grammar {
  lalrpop_util::lalrpop_mod!(pub grammar, "/grammar.rs");
}

pub use ast::{Expr, Function, Param};

pub fn parse(src: &str) -> Result<Function, String> {
  grammar::grammar::FuncParser::new()
    .parse(src)
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  const SRC: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend";

  #[test]
  fn parses_add_function() {
    let f = parse(SRC).expect("should parse");
    assert_eq!(f.name, "add");
    assert_eq!(
      f.params,
      vec![
        Param {
          name: "a".into(),
          ty: "Int64".into()
        },
        Param {
          name: "b".into(),
          ty: "Int64".into()
        }
      ]
    );
    assert_eq!(f.return_type, "Int64");
    assert_eq!(
      f.body,
      Expr::Add(
        Box::new(Expr::Ident("a".into())),
        Box::new(Expr::Ident("b".into()))
      )
    );
  }

  #[test]
  fn missing_end_errors() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b";
    let err = parse(src).unwrap_err();
    eprintln!("LALRPOP ERROR: {err}");
    assert!(!err.is_empty());
  }
}
