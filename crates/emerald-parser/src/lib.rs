//! Emerald's parser, built on LALRPOP — chosen in `02 toolchain-prototype`
//! over Chumsky (see `spec/COMPILER.md` for the decision record). Parses a
//! full `Program` (inception §17 milestone-1 scope: one function
//! definition plus one top-level command-call statement); grows as later
//! milestones extend `grammar.lalrpop`.

pub mod ast;

#[allow(clippy::all)]
mod grammar {
  lalrpop_util::lalrpop_mod!(pub grammar, "/grammar.rs");
}

pub use ast::{Expr, Function, Item, Param, Program};

pub fn parse(src: &str) -> Result<Program, String> {
  grammar::grammar::ProgramParser::new()
    .parse(src)
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  const FUNC_ONLY: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend";

  #[test]
  fn parses_add_function() {
    let program = parse(FUNC_ONLY).expect("should parse");
    assert_eq!(program.items.len(), 1);
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
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

  const HELLO_EM: &str = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nputs add(20, 22)\n";

  #[test]
  fn parses_hello_em_end_to_end() {
    let program = parse(HELLO_EM).expect("hello.em should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Function(f) = &program.items[0] else {
      panic!(
        "expected item 0 to be the add function, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(f.name, "add");

    let Item::Expr(call) = &program.items[1] else {
      panic!(
        "expected item 1 to be the puts call, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::Call("add".into(), vec![Expr::Int(20), Expr::Int(22)])]
      )
    );
  }
}
