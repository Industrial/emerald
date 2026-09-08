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

pub use ast::{ClassDef, CompareOp, Expr, Function, Item, ModuleDef, Param, Program, Stmt};

/// A parse failure, carrying enough of `lalrpop_util::ParseError`'s own
/// byte-offset location info (plan `13`) to render a real source
/// snippet + caret via `miette` — not just a bare message string. This
/// is the parser's ONLY diagnostic with a real span; sema/codegen
/// diagnostics stay text-only (see plan `13`'s Decision log — spans
/// there would mean threading a span through every `Expr`/`Stmt`
/// variant, out of this plan's scope).
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{message}")]
pub struct ParseError {
  message: String,
  #[source_code]
  src: miette::NamedSource<String>,
  #[label("here")]
  span: miette::SourceSpan,
}

/// Extracts a byte-offset span from whichever `lalrpop_util::ParseError`
/// variant matched. `User` (this grammar's one fallible `=>?` action,
/// the assignment-target check) carries no location at all — see plan
/// 13's Decision log for that one disclosed gap; every other variant
/// already tracks a real offset LALRPOP computed during parsing.
fn to_parse_error<T: std::fmt::Display>(
  e: lalrpop_util::ParseError<usize, T, String>,
  name: &str,
  src: &str,
) -> ParseError {
  let message = e.to_string();
  let (start, end) = match &e {
    lalrpop_util::ParseError::InvalidToken { location } => (*location, *location),
    lalrpop_util::ParseError::UnrecognizedEof { location, .. } => (*location, *location),
    lalrpop_util::ParseError::UnrecognizedToken { token, .. } => (token.0, token.2),
    lalrpop_util::ParseError::ExtraToken { token } => (token.0, token.2),
    lalrpop_util::ParseError::User { .. } => (0, 0),
  };
  let len = end.saturating_sub(start).max(1);
  ParseError {
    message,
    src: miette::NamedSource::new(name, src.to_string()),
    span: (start, len).into(),
  }
}

/// Plan 26's Decision log: a fixed, disclosed robustness bound —
/// panic-mode recovery on a badly malformed file can cascade into a
/// flood of low-quality secondary errors, so this cap exists purely to
/// bound worst-case output, not because any real fixture needs it.
const MAX_RECOVERED_ERRORS: usize = 50;

/// Same as [`parse`], but names the source (shown in the rendered
/// diagnostic's snippet header) as `name` instead of the generic
/// `"<source>"` placeholder — `emerald-cli` uses this with the real file
/// path it read `src` from.
///
/// Returns every top-level `Item` boundary's syntax error in one pass
/// (plan 26), not just the first — LALRPOP's own `!` error-recovery
/// marker on the `Item` production (`grammar.lalrpop`) resynchronizes
/// at the next top-level construct instead of aborting the whole parse.
/// `Ok(program)` is returned only when *zero* errors were recovered
/// (`program.items` then contains no `Item::Error` either, by
/// construction) — the moment one or more exist, this returns
/// `Err(Vec<ParseError>)` instead, exactly the same all-or-nothing
/// shape `parse_named` had before this plan, just now potentially
/// carrying more than one error.
pub fn parse_named(src: &str, name: &str) -> Result<Program, Vec<ParseError>> {
  let mut recovered = Vec::new();
  let result = grammar::grammar::ProgramParser::new().parse(&mut recovered, src);
  let mut errors: Vec<ParseError> = recovered
    .into_iter()
    .map(|e| to_parse_error(e.error, name, src))
    .collect();
  match result {
    Ok(program) if errors.is_empty() => Ok(program),
    Ok(_) => {
      errors.truncate(MAX_RECOVERED_ERRORS);
      Err(errors)
    }
    Err(e) => {
      errors.push(to_parse_error(e, name, src));
      errors.truncate(MAX_RECOVERED_ERRORS);
      Err(errors)
    }
  }
}

pub fn parse(src: &str) -> Result<Program, Vec<ParseError>> {
  parse_named(src, "<source>")
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
    // Plan 26: `errs.len()` is 1 here — a syntax error inside a
    // function body (not at a top-level `Item` boundary) still
    // hard-stops the whole parse, unchanged by top-level recovery.
    let errs = parse(src).unwrap_err();
    assert_eq!(errs.len(), 1);
    eprintln!("LALRPOP ERROR: {}", errs[0]);
    assert!(!errs[0].to_string().is_empty());
  }

  #[test]
  fn parse_error_span_points_at_the_offending_token() {
    // Plan 13 AC1: the span isn't just "present" — it points at the
    // token's *actual* byte offset, verified against the source
    // string's own position, not merely asserted to exist.
    let src = "x: Int64 = +\n";
    let mut errs = parse(src).expect_err("`+` alone is not a valid Expr");
    assert_eq!(errs.len(), 1);
    let err = errs.remove(0);
    let expected_offset = src.find('+').unwrap();
    assert_eq!(err.span.offset(), expected_offset);
    // `ParseError` must satisfy `miette::Diagnostic` for `emerald-cli`
    // to render it — proven by actually constructing a `Report`.
    let report: miette::Report = miette::Report::new(err);
    assert!(!format!("{report:?}").is_empty());
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

  const ARRAY_EXAMPLE: &str =
    "arr: Array[Int64] = [10, 20, 30]\narr[1] = 99\nputs arr[1]\nputs arr[0] + arr[2]\n";

  #[test]
  fn parses_array_literal_index_read_and_write() {
    let program = parse(ARRAY_EXAMPLE).expect("array example should parse");
    assert_eq!(program.items.len(), 4);

    let Item::Stmt(Stmt::Let { name, ty, value }) = &program.items[0] else {
      panic!(
        "expected item 0 to be `arr: Array[Int64] = [...]`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "arr");
    assert_eq!(ty, "Array[Int64]");
    assert_eq!(
      *value,
      Expr::ArrayLit(vec![Expr::Int(10), Expr::Int(20), Expr::Int(30)])
    );

    assert_eq!(
      program.items[1],
      Item::Stmt(Stmt::SetIndex {
        array: Expr::Ident("arr".into()),
        index: Expr::Int(1),
        value: Expr::Int(99),
      })
    );

    let Item::Stmt(Stmt::Expr(call)) = &program.items[2] else {
      panic!(
        "expected item 2 to be `puts arr[1]`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::Index(
          Box::new(Expr::Ident("arr".into())),
          Box::new(Expr::Int(1))
        )]
      )
    );

    let Item::Stmt(Stmt::Expr(call)) = &program.items[3] else {
      panic!(
        "expected item 3 to be `puts arr[0] + arr[2]`, got {:?}",
        program.items[3]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::Add(
          Box::new(Expr::Index(
            Box::new(Expr::Ident("arr".into())),
            Box::new(Expr::Int(0))
          )),
          Box::new(Expr::Index(
            Box::new(Expr::Ident("arr".into())),
            Box::new(Expr::Int(2))
          ))
        )]
      )
    );
  }

  const LAMBDA_EXAMPLE: &str =
    "x: Int64 = 10\nadd_x: Proc = ->(y: Int64) -> Int64 { y + x }\nputs add_x.call(5)\n";

  #[test]
  fn parses_lambda_capture_and_call() {
    let program = parse(LAMBDA_EXAMPLE).expect("lambda example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Stmt::Let { name, ty, value }) = &program.items[1] else {
      panic!(
        "expected item 1 to be `add_x: Proc = ->(...) -> Int64 {{...}}`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(name, "add_x");
    assert_eq!(ty, "Proc");
    assert_eq!(
      *value,
      Expr::Lambda {
        params: vec![Param {
          name: "y".into(),
          ty: "Int64".into()
        }],
        return_type: "Int64".into(),
        body: vec![Stmt::Expr(Expr::Add(
          Box::new(Expr::Ident("y".into())),
          Box::new(Expr::Ident("x".into()))
        ))],
      }
    );

    let Item::Stmt(Stmt::Expr(call)) = &program.items[2] else {
      panic!(
        "expected item 2 to be `puts add_x.call(5)`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::MethodCall(
          Box::new(Expr::Ident("add_x".into())),
          "call".into(),
          vec![Expr::Int(5)]
        )]
      )
    );
  }

  #[test]
  fn parses_method_call_with_arguments() {
    // Independent of Proc/lambdas: plan 10's grammar gap fix means an
    // ordinary `.method(args)` call — never possible before this plan —
    // now parses with a populated argument list.
    let src = "p.move(1, 2)\n";
    let program = parse(src).expect("method call with args should parse");
    let Item::Stmt(Stmt::Expr(call)) = &program.items[0] else {
      panic!(
        "expected a method-call statement, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(
      *call,
      Expr::MethodCall(
        Box::new(Expr::Ident("p".into())),
        "move".into(),
        vec![Expr::Int(1), Expr::Int(2)]
      )
    );
  }

  #[test]
  fn parses_raise() {
    let src = "raise MyError.new(99)\n";
    let program = parse(src).expect("raise should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::Raise(Expr::New(
        "MyError".into(),
        vec![Expr::Int(99)]
      )))
    );
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn parses_begin_rescue() {
    let program = parse(EXCEPTION_EXAMPLE).expect("exception example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Stmt::Begin {
      body,
      rescue_type,
      rescue_var,
      rescue_body,
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be a begin/rescue statement, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(rescue_type, "MyError");
    assert_eq!(rescue_var, "e");
    assert_eq!(
      body,
      &vec![Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::Call("risky".into(), vec![Expr::Int(999)])]
      ))]
    );
    assert_eq!(
      rescue_body,
      &vec![Stmt::Expr(Expr::Call(
        "puts".into(),
        vec![Expr::MethodCall(
          Box::new(Expr::Ident("e".into())),
          "code".into(),
          vec![]
        )]
      ))]
    );
  }

  const MODULE_EXAMPLE: &str = "module MathUtils\n  def double(x: Int64) -> Int64\n    x + x\n  end\nend\n\nputs MathUtils.double(21)\n";

  #[test]
  fn parses_module_and_namespaced_call() {
    let program = parse(MODULE_EXAMPLE).expect("module example should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Module(m) = &program.items[0] else {
      panic!(
        "expected item 0 to be the MathUtils module, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(m.name, "MathUtils");
    assert_eq!(m.methods.len(), 1);
    assert_eq!(m.methods[0].name, "double");
    assert_eq!(
      m.methods[0].params,
      vec![Param {
        name: "x".into(),
        ty: "Int64".into()
      }]
    );
    assert_eq!(m.methods[0].return_type, "Int64");
    assert_eq!(
      m.methods[0].body,
      vec![Stmt::Expr(Expr::Add(
        Box::new(Expr::Ident("x".into())),
        Box::new(Expr::Ident("x".into()))
      ))]
    );

    let Item::Stmt(Stmt::Expr(call)) = &program.items[1] else {
      panic!(
        "expected item 1 to be `puts MathUtils.double(21)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![Expr::MethodCall(
          Box::new(Expr::Ident("MathUtils".into())),
          "double".into(),
          vec![Expr::Int(21)]
        )]
      )
    );
  }

  // Plan 18 (arithmetic & logical operators) — precedence is real, not
  // just parseable (AC1).

  #[test]
  fn mul_binds_tighter_than_add() {
    let program = parse("puts 2 + 3 * 4\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(Expr::Int(2)),
        Box::new(Expr::Mul(Box::new(Expr::Int(3)), Box::new(Expr::Int(4))))
      )
    );
  }

  #[test]
  fn unary_minus_binds_tighter_than_add() {
    let program = parse("puts -3 + 10\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(Expr::Neg(Box::new(Expr::Int(3)))),
        Box::new(Expr::Int(10))
      )
    );
  }

  #[test]
  fn and_binds_tighter_than_or() {
    let program = parse("puts a > 0 && b > 0 || c > 0\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    let gt = |name: &str, n: i64| {
      Expr::Compare(
        Box::new(Expr::Ident(name.into())),
        CompareOp::Gt,
        Box::new(Expr::Int(n)),
      )
    };
    assert_eq!(
      args[0],
      Expr::Or(
        Box::new(Expr::And(Box::new(gt("a", 0)), Box::new(gt("b", 0)))),
        Box::new(gt("c", 0))
      )
    );
  }

  #[test]
  fn not_binds_tighter_than_compare_not_looser() {
    // AC3: `!` sits at the unary tier, tighter than comparison — `!x > 0`
    // parses as `Compare(Not(x), Gt, 0)`, NOT `Not(Compare(x, Gt, 0))`.
    // Real, Ruby-divergent precedence, documented by this test rather
    // than left unspecified.
    let program = parse("puts !x > 0\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Compare(
        Box::new(Expr::Not(Box::new(Expr::Ident("x".into())))),
        CompareOp::Gt,
        Box::new(Expr::Int(0))
      )
    );
  }

  const ARITHMETIC_EXAMPLE: &str = "def factorial(n: Int64) -> Int64\n  if n <= 1\n    return 1\n  end\n  return n * factorial(n - 1)\nend\n\nputs factorial(5)\nputs 17 / 5\nputs 17 % 5\nputs -3 + 10\n";

  #[test]
  fn parses_arithmetic_example() {
    let program = parse(ARITHMETIC_EXAMPLE).expect("arithmetic example should parse");
    assert_eq!(program.items.len(), 5);
    let Item::Function(f) = &program.items[0] else {
      panic!(
        "expected the factorial function, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(
      f.body[0],
      Stmt::If {
        cond: Expr::Compare(
          Box::new(Expr::Ident("n".into())),
          CompareOp::Le,
          Box::new(Expr::Int(1))
        ),
        then_branch: vec![Stmt::Return(Some(Expr::Int(1)))],
        else_branch: None,
      }
    );
    assert_eq!(
      f.body[1],
      Stmt::Return(Some(Expr::Mul(
        Box::new(Expr::Ident("n".into())),
        Box::new(Expr::Call(
          "factorial".into(),
          vec![Expr::Sub(
            Box::new(Expr::Ident("n".into())),
            Box::new(Expr::Int(1))
          )]
        ))
      )))
    );
  }

  const SHORT_CIRCUIT_EXAMPLE: &str = "def noisy(n: Int64) -> Boolean\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1)\n  puts 100\nend\nif x > -10 && noisy(3)\n  puts 300\nend\nif x < 0 || noisy(2)\n  puts 200\nend\nif x > 0 || noisy(4)\n  puts 400\nend\n";

  #[test]
  fn parses_short_circuit_example() {
    let program = parse(SHORT_CIRCUIT_EXAMPLE).expect("short-circuit example should parse");
    // 1 function + 1 let + 4 ifs.
    assert_eq!(program.items.len(), 6);
    let Item::Stmt(Stmt::If { cond, .. }) = &program.items[2] else {
      panic!("expected the first if, got {:?}", program.items[2]);
    };
    assert_eq!(
      *cond,
      Expr::And(
        Box::new(Expr::Compare(
          Box::new(Expr::Ident("x".into())),
          CompareOp::Gt,
          Box::new(Expr::Int(0))
        )),
        Box::new(Expr::Call("noisy".into(), vec![Expr::Int(1)]))
      )
    );
  }

  #[test]
  fn rejects_bare_unary_minus_as_a_statement() {
    // A fresh statement can't start with unary `-`/`!` (see
    // `StmtUnaryExpr`'s Decision-log comment in grammar.lalrpop) — still
    // fully usable as a Let/return/argument value.
    assert!(parse("-5\n").is_err());
  }

  // Plan 19 (string literals).

  #[test]
  fn parses_plain_string_literal() {
    let program = parse("puts \"hello\"\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("hello".to_string()));
  }

  #[test]
  fn decodes_escaped_quote() {
    let program = parse("puts \"a\\\"b\"\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a\"b".to_string()));
  }

  #[test]
  fn decodes_escaped_newline() {
    let program = parse("puts \"line1\\nline2\"\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("line1\nline2".to_string()));
  }

  #[test]
  fn unterminated_string_literal_errors_not_panics() {
    let src = "puts \"hello\n";
    assert!(parse(src).is_err());
  }

  #[test]
  fn unrecognized_escape_errors_not_panics() {
    // `\t` isn't one of the two recognized escapes (`\"`/`\n`) — a real
    // parse error, not a silent pass-through.
    let src = "puts \"a\\tb\"\n";
    assert!(parse(src).is_err());
  }

  // Plan 20 (comments and case/when).

  #[test]
  fn comment_only_file_is_an_empty_program() {
    let program = parse("# just a comment\n").expect("should parse");
    assert_eq!(program.items, vec![]);
  }

  #[test]
  fn trailing_and_leading_comments_dont_change_the_ast() {
    let with_comments = "# classify\nx: Int64 = 10 # ten\nputs x\n";
    let without = "x: Int64 = 10\nputs x\n";
    assert_eq!(
      parse(with_comments).expect("should parse"),
      parse(without).expect("should parse")
    );
  }

  #[test]
  fn hash_inside_string_literal_is_not_eaten_as_a_comment() {
    let program = parse("puts \"a#b\"\n").expect("should parse");
    let Item::Stmt(Stmt::Expr(Expr::Call(_, args))) = &program.items[0] else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a#b".to_string()));
  }

  const CASE_EXAMPLE: &str = "# classify an integer by a fixed set of buckets\nn: Int64 = 2\nlabel: Int64 = 0\ncase n\nwhen 1\n  label: Int64 = 10\nwhen 2, 3\n  label: Int64 = 20\nelse\n  label: Int64 = 99\nend\nputs label\n";

  #[test]
  fn parses_case_when_example() {
    let program = parse(CASE_EXAMPLE).expect("should parse");
    let Item::Stmt(Stmt::Case {
      scrutinee,
      arms,
      else_body,
    }) = &program.items[2]
    else {
      panic!("expected a case statement, got {:?}", program.items[2]);
    };
    assert_eq!(*scrutinee, Expr::Ident("n".into()));
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, vec![Expr::Int(1)]);
    assert_eq!(
      arms[0].1,
      vec![Stmt::Let {
        name: "label".into(),
        ty: "Int64".into(),
        value: Expr::Int(10),
      }]
    );
    assert_eq!(arms[1].0, vec![Expr::Int(2), Expr::Int(3)]);
    assert!(else_body.is_some());
  }

  // Plan 25 (stdlib expansion).

  #[test]
  fn parses_bool_and_nil_literals() {
    let program = parse("puts true\nputs false\nputs nil\n")
      .expect("should parse")
      .items;
    let Item::Stmt(Stmt::Expr(Expr::Call(_, a1))) = &program[0] else {
      panic!("expected a puts call, got {:?}", program[0]);
    };
    assert_eq!(a1[0], Expr::Bool(true));
    let Item::Stmt(Stmt::Expr(Expr::Call(_, a2))) = &program[1] else {
      panic!("expected a puts call, got {:?}", program[1]);
    };
    assert_eq!(a2[0], Expr::Bool(false));
    let Item::Stmt(Stmt::Expr(Expr::Call(_, a3))) = &program[2] else {
      panic!("expected a puts call, got {:?}", program[2]);
    };
    assert_eq!(a3[0], Expr::Nil);
  }

  #[test]
  fn parses_hash_literal_and_indexing() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Stmt::Let { name, ty, value }) = &program.items[0] else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(name, "h");
    assert_eq!(ty, "Hash[Int64, Int64]");
    assert_eq!(
      *value,
      Expr::HashLit(vec![
        (Expr::Int(1), Expr::Int(10)),
        (Expr::Int(2), Expr::Int(20)),
        (Expr::Int(3), Expr::Int(30)),
      ])
    );
    assert_eq!(
      program.items[2],
      Item::Stmt(Stmt::SetIndex {
        array: Expr::Ident("h".into()),
        index: Expr::Int(2),
        value: Expr::Int(99),
      })
    );
  }

  #[test]
  fn parses_array_new() {
    let program = parse("arr: Array[Int64] = Array.new(5)\n").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: Expr::ArrayNew(Box::new(Expr::Int(5))),
      })
    );
  }

  #[test]
  fn parses_array_new_with_runtime_size() {
    let program = parse("n: Int64 = 5\narr: Array[Int64] = Array.new(n)\n").expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: Expr::ArrayNew(Box::new(Expr::Ident("n".into()))),
      })
    );
  }

  #[test]
  fn case_with_no_else_parses() {
    let src = "case n\nwhen 1\n  puts 1\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Stmt::Case { else_body, .. }) = &program.items[0] else {
      panic!("expected a case statement, got {:?}", program.items[0]);
    };
    assert_eq!(*else_body, None);
  }

  // Plan 31 (compound and multiple assignment).

  #[test]
  fn bare_reassignment_parses_as_assign() {
    let src = "x: Int64 = 1\nx = 2\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[1],
      Item::Stmt(Stmt::Assign {
        name: "x".to_string(),
        value: Expr::Int(2),
      })
    );
  }

  #[test]
  fn compound_plus_assign_desugars_to_assign_of_add() {
    let src = "total += i\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::Assign {
        name: "total".to_string(),
        value: Expr::Add(
          Box::new(Expr::Ident("total".to_string())),
          Box::new(Expr::Ident("i".to_string())),
        ),
      })
    );
  }

  fn assert_compound_assign_desugars_to(src: &str, expected_value: Expr) {
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::Assign {
        name: "x".to_string(),
        value: expected_value,
      }),
      "mismatched desugaring for {src:?}"
    );
  }

  #[test]
  fn all_compound_assign_operators_desugar_to_the_matching_binop() {
    let x = || Box::new(Expr::Ident("x".to_string()));
    let one = || Box::new(Expr::Int(1));
    assert_compound_assign_desugars_to("x -= 1\n", Expr::Sub(x(), one()));
    assert_compound_assign_desugars_to("x *= 1\n", Expr::Mul(x(), one()));
    assert_compound_assign_desugars_to("x /= 1\n", Expr::Div(x(), one()));
    assert_compound_assign_desugars_to("x %= 1\n", Expr::Rem(x(), one()));
  }

  #[test]
  fn multiple_assignment_swap_parses() {
    let src = "a, b = b, a\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::MultiAssign {
        names: vec!["a".to_string(), "b".to_string()],
        values: vec![Expr::Ident("b".to_string()), Expr::Ident("a".to_string())],
      })
    );
  }

  #[test]
  fn plan_31_worked_example_parses() {
    let src = "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5\n  total += i\n  i += 1\nend\nputs total\n\na: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    parse(src).expect("plan 31's worked example must parse cleanly");
  }

  // Plan 30 (for-in iteration).

  #[test]
  fn for_in_over_a_literal_array_parses() {
    let src = "for x in [10, 20, 30]\n  puts x\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::For {
        var: "x".to_string(),
        elements: vec![Expr::Int(10), Expr::Int(20), Expr::Int(30)],
        body: vec![Stmt::Expr(Expr::Call(
          "puts".to_string(),
          vec![Expr::Ident("x".to_string())]
        ))],
      })
    );
  }

  #[test]
  fn for_in_over_a_bare_identifier_is_a_parse_error() {
    let src = "for x in arr\n  puts x\nend\n";
    assert!(
      parse(src).is_err(),
      "a non-literal scrutinee must be rejected at parse time"
    );
  }

  // Plan 29 (control-flow completeness).

  #[test]
  fn elsif_chain_desugars_to_nested_if_in_else_branch() {
    let src = "if a\n  1\nelsif b\n  2\nelsif c\n  3\nelse\n  4\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Stmt::If {
      then_branch,
      else_branch,
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level If statement");
    };
    assert_eq!(*then_branch, vec![Stmt::Expr(Expr::Int(1))]);

    // First elsif link.
    let Some(outer_else) = else_branch else {
      panic!("expected an else_branch from the first elsif");
    };
    assert_eq!(outer_else.len(), 1);
    let Stmt::If {
      then_branch: b2,
      else_branch: e2,
      ..
    } = &outer_else[0]
    else {
      panic!("expected a nested If for the first elsif");
    };
    assert_eq!(*b2, vec![Stmt::Expr(Expr::Int(2))]);

    // Second elsif link.
    let Some(inner_else) = e2 else {
      panic!("expected an else_branch from the second elsif");
    };
    assert_eq!(inner_else.len(), 1);
    let Stmt::If {
      then_branch: b3,
      else_branch: e3,
      ..
    } = &inner_else[0]
    else {
      panic!("expected a nested If for the second elsif");
    };
    assert_eq!(*b3, vec![Stmt::Expr(Expr::Int(3))]);

    // Trailing plain else.
    assert_eq!(*e3, Some(vec![Stmt::Expr(Expr::Int(4))]));
  }

  #[test]
  fn unless_desugars_to_if_not() {
    let src = "unless x > 0\n  return 0\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Stmt::If {
      cond,
      then_branch,
      else_branch,
    }) = &program.items[0]
    else {
      panic!("expected an If statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(Expr::Compare(
        Box::new(Expr::Ident("x".to_string())),
        CompareOp::Gt,
        Box::new(Expr::Int(0)),
      )))
    );
    assert_eq!(*then_branch, vec![Stmt::Return(Some(Expr::Int(0)))]);
    assert_eq!(*else_branch, None);
  }

  #[test]
  fn until_desugars_to_while_not() {
    let src = "until i >= 3\n  puts i\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Stmt::While { cond, body }) = &program.items[0] else {
      panic!("expected a While statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(Expr::Compare(
        Box::new(Expr::Ident("i".to_string())),
        CompareOp::Ge,
        Box::new(Expr::Int(3)),
      )))
    );
    assert_eq!(
      *body,
      vec![Stmt::Expr(Expr::Call(
        "puts".to_string(),
        vec![Expr::Ident("i".to_string())]
      ))]
    );
  }

  #[test]
  fn plan_29_worked_examples_parse() {
    let example_a = "def grade(score: Int64) -> Int64\n  if score >= 90\n    return 4\n  elsif score >= 80\n    return 3\n  elsif score >= 70\n    return 2\n  else\n    return 1\n  end\nend\n\nputs grade(95)\nputs grade(85)\nputs grade(72)\nputs grade(50)\n";
    parse(example_a).expect("plan 29 example A must parse cleanly");

    let example_b = "def describe(x: Int64) -> Int64\n  unless x > 0\n    return 0\n  end\n  return 1\nend\n\nputs describe(-5)\nputs describe(5)\n\ni: Int64 = 0\nuntil i >= 3\n  puts i\n  i: Int64 = i + 1\nend\n";
    parse(example_b).expect("plan 29 example B must parse cleanly");
  }

  // Plan 28 (bitwise operators).

  #[test]
  fn bit_and_binds_tighter_than_bit_or() {
    let src = "x: Int64 = 1 | 2 & 3\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::BitOr(
        Box::new(Expr::Int(1)),
        Box::new(Expr::BitAnd(Box::new(Expr::Int(2)), Box::new(Expr::Int(3)))),
      )
    );
  }

  #[test]
  fn shift_binds_tighter_than_bit_and() {
    let src = "x: Int64 = 1 << 2 & 3\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::BitAnd(
        Box::new(Expr::Shl(Box::new(Expr::Int(1)), Box::new(Expr::Int(2)))),
        Box::new(Expr::Int(3)),
      )
    );
  }

  #[test]
  fn bit_and_binds_tighter_than_compare() {
    let src = "x: Boolean = flags & flag == flag\n";
    let program = parse(src).unwrap();
    let Stmt::Let { value, .. } = as_let(&program) else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *value,
      Expr::Compare(
        Box::new(Expr::BitAnd(
          Box::new(Expr::Ident("flags".to_string())),
          Box::new(Expr::Ident("flag".to_string())),
        )),
        CompareOp::Eq,
        Box::new(Expr::Ident("flag".to_string())),
      )
    );
  }

  #[test]
  fn bit_not_binds_as_tight_as_neg() {
    let src = "x: Int64 = ~0\ny: Int64 = ~x + 1\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items.len(), 2);
    let Item::Stmt(Stmt::Let { value: v0, .. }) = &program.items[0] else {
      panic!("expected a Let statement");
    };
    assert_eq!(*v0, Expr::BitNot(Box::new(Expr::Int(0))));
    let Item::Stmt(Stmt::Let { value: v1, .. }) = &program.items[1] else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *v1,
      Expr::Add(
        Box::new(Expr::BitNot(Box::new(Expr::Ident("x".to_string())))),
        Box::new(Expr::Int(1)),
      )
    );
  }

  fn as_let(program: &Program) -> &Stmt {
    match &program.items[0] {
      Item::Stmt(s @ Stmt::Let { .. }) => s,
      other => panic!("expected a Let statement, got {other:?}"),
    }
  }

  #[test]
  fn plan_28_worked_example_parses() {
    let src = "READ: Int64 = 1\nWRITE: Int64 = 2\nEXEC: Int64 = 4\n\ndef has_flag(flags: Int64, flag: Int64) -> Boolean\n  return flags & flag == flag\nend\n\nperms: Int64 = READ | WRITE\nputs perms\nif has_flag(perms, READ)\n  puts 1\nend\nif has_flag(perms, EXEC)\n  puts 0\nend\nputs perms ^ WRITE\nputs ~0\nputs 1 << 4\nputs 256 >> 4\n";
    parse(src).expect("plan 28's worked example must parse cleanly");
  }

  // Plan 26 (parser error recovery).

  const TWO_BROKEN_LETS: &str = "x: Int64 = +\ny: Int64 = +\n";

  #[test]
  fn reports_two_independent_top_level_syntax_errors_in_one_pass() {
    let errs = parse(TWO_BROKEN_LETS).expect_err("both lets are malformed");
    assert_eq!(
      errs.len(),
      2,
      "expected exactly two recovered errors, got {errs:?}"
    );
    let first_plus = TWO_BROKEN_LETS.find('+').unwrap();
    let second_plus = TWO_BROKEN_LETS.rfind('+').unwrap();
    assert_eq!(errs[0].span.offset(), first_plus);
    assert_eq!(errs[1].span.offset(), second_plus);
  }

  #[test]
  fn error_recovery_does_not_alter_single_error_case() {
    // Regression guard for the plan 26 acceptance criteria: a source with
    // exactly one malformed construct inside an otherwise well-formed
    // function body must still report exactly one error, not more.
    let errs = parse("def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n\nx: Int64 = +\n")
      .expect_err("the second statement is malformed");
    assert_eq!(errs.len(), 1, "expected exactly one error, got {errs:?}");
  }

  #[test]
  fn string_typed_let_and_concat_parse() {
    let src = "s: String = \"hello\"\na: String = \"foo\" + \"bar\"\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(Stmt::Let {
        name: "s".into(),
        ty: "String".into(),
        value: Expr::StringLit("hello".into()),
      })
    );
    assert_eq!(
      program.items[1],
      Item::Stmt(Stmt::Let {
        name: "a".into(),
        ty: "String".into(),
        value: Expr::Add(
          Box::new(Expr::StringLit("foo".into())),
          Box::new(Expr::StringLit("bar".into()))
        ),
      })
    );
  }
}
