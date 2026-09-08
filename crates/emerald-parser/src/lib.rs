//! Emerald's parser, built on LALRPOP — chosen in `02 toolchain-prototype`
//! over Chumsky (see `spec/COMPILER.md` for the decision record). Parses a
//! full `Program` of `Item`s, each a function definition or a top-level
//! `Stmt` (plan `07` extended the milestone-1-only single-expr shape to a
//! real statement/block language); grows as later milestones extend
//! `grammar.lalrpop`.

pub mod ast;

#[allow(clippy::all)]
mod grammar {
  lalrpop_util::lalrpop_mod!(pub grammar, "/grammar.rs");
}

pub use ast::{ClassDef, CompareOp, Expr, Function, Item, Param, Program, Stmt};

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
      vec![Stmt::Expr(Expr::Add(
        Box::new(Expr::Ident("a".into())),
        Box::new(Expr::Ident("b".into()))
      ))]
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

    let Item::Stmt(Stmt::Expr(call)) = &program.items[1] else {
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

  const MILESTONE2: &str = "x: Int64 = 10\n\nif x > 5\n  puts x\nend\n";

  #[test]
  fn parses_inception_milestone2_end_to_end() {
    let program = parse(MILESTONE2).expect("milestone-2 example should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Stmt(Stmt::Let { name, ty, value }) = &program.items[0] else {
      panic!(
        "expected item 0 to be `x: Int64 = 10`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "x");
    assert_eq!(ty, "Int64");
    assert_eq!(*value, Expr::Int(10));

    let Item::Stmt(Stmt::If {
      cond,
      then_branch,
      else_branch,
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be an if statement, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *cond,
      Expr::Compare(
        Box::new(Expr::Ident("x".into())),
        CompareOp::Gt,
        Box::new(Expr::Int(5))
      )
    );
    assert_eq!(
      then_branch,
      &vec![Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Ident("x".into())]
      ))]
    );
    assert_eq!(else_branch, &None);
  }

  #[test]
  fn parses_while_break_next() {
    let src = "while x < 3\n  next\n  break\nend\n";
    let program = parse(src).expect("while/break/next should parse");
    let Item::Stmt(Stmt::While { body, .. }) = &program.items[0] else {
      panic!("expected a while statement, got {:?}", program.items[0]);
    };
    assert_eq!(body, &vec![Stmt::Next, Stmt::Break]);
  }

  #[test]
  fn parses_if_else() {
    let src = "if x > 5\n  puts x\nelse\n  puts x\nend\n";
    let program = parse(src).expect("if/else should parse");
    let Item::Stmt(Stmt::If { else_branch, .. }) = &program.items[0] else {
      panic!("expected an if statement, got {:?}", program.items[0]);
    };
    assert!(else_branch.is_some(), "else branch should be present");
  }

  #[test]
  fn parses_return_with_value() {
    let src = "def f(a: Int64) -> Int64\n  return a\nend";
    let program = parse(src).expect("return should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert_eq!(f.body, vec![Stmt::Return(Some(Expr::Ident("a".into())))]);
  }

  const POINT_EXAMPLE: &str = "class Point\n  x: Float64\n  y: Float64\n\n  def initialize(x: Float64, y: Float64) -> Void\n    @x = x\n    @y = y\n  end\n\n  def sum -> Float64\n    @x + @y\n  end\nend\n\np: Point = Point.new(2.0, 3.0)\nputs p.sum\n";

  #[test]
  fn parses_inception_point_example_end_to_end() {
    let program = parse(POINT_EXAMPLE).expect("Point example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Class(class) = &program.items[0] else {
      panic!(
        "expected item 0 to be the Point class, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(class.name, "Point");
    assert_eq!(
      class.fields,
      vec![
        Param {
          name: "x".into(),
          ty: "Float64".into()
        },
        Param {
          name: "y".into(),
          ty: "Float64".into()
        }
      ]
    );
    assert_eq!(class.methods.len(), 2);
    assert_eq!(class.methods[0].name, "initialize");
    assert_eq!(
      class.methods[0].body,
      vec![
        Stmt::SetField {
          name: "x".into(),
          value: Expr::Ident("x".into())
        },
        Stmt::SetField {
          name: "y".into(),
          value: Expr::Ident("y".into())
        },
      ]
    );
    assert_eq!(class.methods[1].name, "sum");
    assert_eq!(
      class.methods[1].body,
      vec![Stmt::Expr(Expr::Add(
        Box::new(Expr::InstanceVar("x".into())),
        Box::new(Expr::InstanceVar("y".into()))
      ))]
    );

    let Item::Stmt(Stmt::Let { name, ty, value }) = &program.items[1] else {
      panic!(
        "expected item 1 to be `p: Point = Point.new(...)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(name, "p");
    assert_eq!(ty, "Point");
    assert_eq!(
      *value,
      Expr::New("Point".into(), vec![Expr::Float(2.0), Expr::Float(3.0)])
    );

    let Item::Stmt(Stmt::Expr(call)) = &program.items[2] else {
      panic!(
        "expected item 2 to be `puts p.sum`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::MethodCall(
          Box::new(Expr::Ident("p".into())),
          "sum".into(),
          vec![]
        )]
      )
    );
  }
}
