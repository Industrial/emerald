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

// Plan 36: string interpolation's `#{...}`-splitting logic — needs to
// call back into `grammar`'s second, independent `pub Expr` entry
// point, a real dependency `ast.rs` (a pure-data module, no dependency
// on the generated parser) deliberately doesn't have.
mod interpolate;

pub use ast::{
  ActorDef, CaseArm, CasePattern, ClassDef, CompareOp, EnumDef, EnumVariant, Expr, Function,
  InterfaceDef, Item, ModuleDef, Param, Program, RescueClause, Spanned, Stmt, StringPart,
  TypeParam,
};

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
/// Plan 47's Decision log: `assert(cond)`/`assert_eq(expected, actual)`
/// capture a raw `@L` byte offset at parse time (as a placeholder
/// `Expr::Int`, the grammar's own action) — this targeted post-parse
/// pass is the only place that offset is ever converted into the real
/// `Expr::StringLit("{name}:{line}")` a test failure's `.message`
/// reads. Deliberately not plan 22's general `Spanned<T>` overhaul
/// (out of scope, would touch every `Expr`/`Stmt` variant); this walks
/// only to find `Expr::Call("assert"|"assert_eq", _)` shapes.
fn line_at(source: &str, offset: usize) -> usize {
  1 + source.as_bytes()[..offset.min(source.len())]
    .iter()
    .filter(|&&b| b == b'\n')
    .count()
}

fn rewrite_assert_locations(items: &mut [Item], name: &str, source: &str) {
  for item in items {
    match item {
      Item::Function(f) => rewrite_stmts(&mut f.body, name, source),
      Item::Class(c) => {
        for m in &mut c.methods {
          rewrite_stmts(&mut m.body, name, source);
        }
      }
      Item::Module(m) => {
        for f in &mut m.methods {
          rewrite_stmts(&mut f.body, name, source);
        }
      }
      Item::Actor(a) => {
        for m in &mut a.methods {
          rewrite_stmts(&mut m.body, name, source);
        }
      }
      Item::Interface(_) | Item::Require(_) | Item::Error | Item::Enum(_) => {}
      Item::Stmt(s) => rewrite_stmt(s, name, source),
      Item::Test { body, .. } => rewrite_stmts(body, name, source),
    }
  }
}

fn rewrite_stmts(stmts: &mut [Spanned<Stmt>], name: &str, source: &str) {
  for s in stmts {
    rewrite_stmt(s, name, source);
  }
}

fn rewrite_stmt(stmt: &mut Spanned<Stmt>, name: &str, source: &str) {
  match &mut stmt.node {
    Stmt::Let { value, .. }
    | Stmt::SetField { value, .. }
    | Stmt::Assign { value, .. }
    | Stmt::Raise(value)
    | Stmt::OrAssign { default: value, .. }
    | Stmt::AndAssign { value, .. }
    | Stmt::Expr(value) => rewrite_expr(value, name, source),
    Stmt::SetIndex {
      array,
      index,
      value,
    } => {
      rewrite_expr(array, name, source);
      rewrite_expr(index, name, source);
      rewrite_expr(value, name, source);
    }
    Stmt::MultiAssign { values, .. } => {
      for v in values {
        rewrite_expr(v, name, source);
      }
    }
    Stmt::If {
      cond,
      then_branch,
      else_branch,
    } => {
      rewrite_expr(cond, name, source);
      rewrite_stmts(then_branch, name, source);
      if let Some(b) = else_branch {
        rewrite_stmts(b, name, source);
      }
    }
    Stmt::While { cond, body } => {
      rewrite_expr(cond, name, source);
      rewrite_stmts(body, name, source);
    }
    Stmt::Return(Some(e)) => rewrite_expr(e, name, source),
    Stmt::Return(None) | Stmt::Break | Stmt::Next | Stmt::Retry => {}
    Stmt::Begin {
      body,
      rescues,
      ensure,
    } => {
      rewrite_stmts(body, name, source);
      for r in rescues {
        rewrite_stmts(&mut r.body, name, source);
      }
      if let Some(e) = ensure {
        rewrite_stmts(e, name, source);
      }
    }
    Stmt::Case {
      scrutinee,
      arms,
      else_body,
    } => {
      rewrite_expr(scrutinee, name, source);
      for (pattern, body) in arms {
        if let CasePattern::Values(values) = pattern {
          for v in values {
            rewrite_expr(v, name, source);
          }
        }
        rewrite_stmts(body, name, source);
      }
      if let Some(b) = else_body {
        rewrite_stmts(b, name, source);
      }
    }
    Stmt::For { elements, body, .. } => {
      for e in elements {
        rewrite_expr(e, name, source);
      }
      rewrite_stmts(body, name, source);
    }
    Stmt::Yield(args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Stmt::ForRange {
      start, end, body, ..
    } => {
      rewrite_expr(start, name, source);
      rewrite_expr(end, name, source);
      rewrite_stmts(body, name, source);
    }
    Stmt::MatchResult {
      scrutinee,
      ok_body,
      err_body,
      ..
    } => {
      rewrite_expr(scrutinee, name, source);
      rewrite_stmts(ok_body, name, source);
      rewrite_stmts(err_body, name, source);
    }
  }
}

fn rewrite_expr(expr: &mut Spanned<Expr>, name: &str, source: &str) {
  match &mut expr.node {
    Expr::Ident(_)
    | Expr::Int(_)
    | Expr::Float(_)
    | Expr::StringLit(_)
    | Expr::SymbolLit(_)
    | Expr::Bool(_)
    | Expr::Nil
    | Expr::InstanceVar(_) => {}
    Expr::Interpolate(parts) => {
      for p in parts {
        if let StringPart::Expr(e) = p {
          rewrite_expr(e, name, source);
        }
      }
    }
    Expr::Add(a, b)
    | Expr::Sub(a, b)
    | Expr::Mul(a, b)
    | Expr::Div(a, b)
    | Expr::Rem(a, b)
    | Expr::And(a, b)
    | Expr::Or(a, b)
    | Expr::BitAnd(a, b)
    | Expr::BitOr(a, b)
    | Expr::BitXor(a, b)
    | Expr::Shl(a, b)
    | Expr::Shr(a, b)
    | Expr::Index(a, b) => {
      rewrite_expr(a, name, source);
      rewrite_expr(b, name, source);
    }
    Expr::Neg(a) | Expr::Not(a) | Expr::BitNot(a) | Expr::ArrayNew(a) => {
      rewrite_expr(a, name, source)
    }
    Expr::Compare(a, _, b) => {
      rewrite_expr(a, name, source);
      rewrite_expr(b, name, source);
    }
    Expr::Call(fn_name, args) => {
      for a in args.iter_mut() {
        rewrite_expr(a, name, source);
      }
      let expected_arity = match fn_name.as_str() {
        "assert" => Some(2),
        "assert_eq" => Some(3),
        _ => None,
      };
      if expected_arity == Some(args.len()) {
        let offset = match args.last().map(|s| &s.node) {
          Some(Expr::Int(offset)) => Some(*offset),
          _ => None,
        };
        if let Some(offset) = offset {
          let line = line_at(source, offset as usize);
          args.last_mut().expect("checked non-empty above").node =
            Expr::StringLit(format!("{name}:{line}"));
        }
      }
    }
    Expr::CallKw(_, kwargs) => {
      for (_, e) in kwargs {
        rewrite_expr(e, name, source);
      }
    }
    Expr::New(_, args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Expr::MethodCall(recv, _, args) | Expr::SafeCall(recv, _, args) => {
      rewrite_expr(recv, name, source);
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
    Expr::ArrayLit(elems) => {
      for e in elems {
        rewrite_expr(e, name, source);
      }
    }
    Expr::HashLit(pairs) => {
      for (k, v) in pairs {
        rewrite_expr(k, name, source);
        rewrite_expr(v, name, source);
      }
    }
    Expr::Lambda { body, .. } => rewrite_stmts(body, name, source),
    Expr::TupleLit(elems) => {
      for e in elems {
        rewrite_expr(e, name, source);
      }
    }
    Expr::Ok(e) | Expr::Err(e) | Expr::Try(e) => rewrite_expr(e, name, source),
    Expr::Spawn(_, args) => {
      for a in args {
        rewrite_expr(a, name, source);
      }
    }
  }
}

pub fn parse_named(src: &str, name: &str) -> Result<Program, Vec<ParseError>> {
  let mut recovered = Vec::new();
  let result = grammar::grammar::ProgramParser::new().parse(&mut recovered, src);
  let mut errors: Vec<ParseError> = recovered
    .into_iter()
    .map(|e| to_parse_error(e.error, name, src))
    .collect();
  match result {
    Ok(mut program) if errors.is_empty() => {
      rewrite_assert_locations(&mut program.items, name, src);
      Ok(program)
    }
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

  /// Test-only shorthand for `Spanned::synthetic` — a hand-written
  /// expected-value literal has no real source text to derive a span
  /// from, so every nested `Expr`/`Stmt` position in one needs this.
  fn s<T>(node: T) -> Spanned<T> {
    Spanned::synthetic(node)
  }

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
          ty: "Int64".into(),
          default: None
        },
        Param {
          name: "b".into(),
          ty: "Int64".into(),
          default: None
        }
      ]
    );
    assert_eq!(f.return_type, "Int64");
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("a".into()))),
        Box::new(s(Expr::Ident("b".into())))
      ))))]
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

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be the puts call, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Call(
          "add".into(),
          vec![s(Expr::Int(20)), s(Expr::Int(22))]
        ))]
      )
    );
  }

  const MILESTONE2: &str = "x: Int64 = 10\n\nif x > 5\n  puts x\nend\n";

  #[test]
  fn parses_inception_milestone2_end_to_end() {
    let program = parse(MILESTONE2).expect("milestone-2 example should parse");
    assert_eq!(program.items.len(), 2);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected item 0 to be `x: Int64 = 10`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "x");
    assert_eq!(ty, "Int64");
    assert_eq!(*value, Expr::Int(10));

    let Item::Stmt(Spanned {
      node: Stmt::If {
        cond,
        then_branch,
        else_branch,
      },
      ..
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
        Box::new(s(Expr::Ident("x".into()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(5)))
      )
    );
    assert_eq!(
      then_branch,
      &vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::Ident("x".into()))]
      ))))]
    );
    assert_eq!(else_branch, &None);
  }

  #[test]
  fn parses_while_break_next() {
    let src = "while x < 3\n  next\n  break\nend\n";
    let program = parse(src).expect("while/break/next should parse");
    let Item::Stmt(Spanned {
      node: Stmt::While { body, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a while statement, got {:?}", program.items[0]);
    };
    assert_eq!(body, &vec![Stmt::Next, Stmt::Break]);
  }

  #[test]
  fn parses_if_else() {
    let src = "if x > 5\n  puts x\nelse\n  puts x\nend\n";
    let program = parse(src).expect("if/else should parse");
    let Item::Stmt(Spanned {
      node: Stmt::If { else_branch, .. },
      ..
    }) = &program.items[0]
    else {
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
    assert_eq!(
      f.body,
      vec![s(Stmt::Return(Some(s(Expr::Ident("a".into())))))]
    );
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
          ty: "Float64".into(),
          default: None
        },
        Param {
          name: "y".into(),
          ty: "Float64".into(),
          default: None
        }
      ]
    );
    assert_eq!(class.methods.len(), 2);
    assert_eq!(class.methods[0].name, "initialize");
    assert_eq!(
      class.methods[0].body,
      vec![
        s(Stmt::SetField {
          name: "x".into(),
          value: s(Expr::Ident("x".into()))
        }),
        s(Stmt::SetField {
          name: "y".into(),
          value: s(Expr::Ident("y".into()))
        }),
      ]
    );
    assert_eq!(class.methods[1].name, "sum");
    assert_eq!(
      class.methods[1].body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::InstanceVar("x".into()))),
        Box::new(s(Expr::InstanceVar("y".into())))
      ))))]
    );

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be `p: Point = Point.new(...)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(name, "p");
    assert_eq!(ty, "Point");
    assert_eq!(
      *value,
      Expr::New(
        "Point".into(),
        vec![s(Expr::Float(2.0)), s(Expr::Float(3.0))]
      )
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts p.sum`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("p".into()))),
          "sum".into(),
          vec![]
        ))]
      )
    );
  }

  const ARRAY_EXAMPLE: &str =
    "arr: Array[Int64] = [10, 20, 30]\narr[1] = 99\nputs arr[1]\nputs arr[0] + arr[2]\n";

  #[test]
  fn parses_array_literal_index_read_and_write() {
    let program = parse(ARRAY_EXAMPLE).expect("array example should parse");
    assert_eq!(program.items.len(), 4);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected item 0 to be `arr: Array[Int64] = [...]`, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(name, "arr");
    assert_eq!(ty, "Array[Int64]");
    assert_eq!(
      *value,
      Expr::ArrayLit(vec![s(Expr::Int(10)), s(Expr::Int(20)), s(Expr::Int(30))])
    );

    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::SetIndex {
        array: s(Expr::Ident("arr".into())),
        index: s(Expr::Int(1)),
        value: s(Expr::Int(99)),
      }))
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts arr[1]`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Index(
          Box::new(s(Expr::Ident("arr".into()))),
          Box::new(s(Expr::Int(1)))
        ))]
      )
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[3]
    else {
      panic!(
        "expected item 3 to be `puts arr[0] + arr[2]`, got {:?}",
        program.items[3]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::Add(
          Box::new(s(Expr::Index(
            Box::new(s(Expr::Ident("arr".into()))),
            Box::new(s(Expr::Int(0)))
          ))),
          Box::new(s(Expr::Index(
            Box::new(s(Expr::Ident("arr".into()))),
            Box::new(s(Expr::Int(2)))
          )))
        ))]
      )
    );
  }

  const LAMBDA_EXAMPLE: &str =
    "x: Int64 = 10\nadd_x: Proc = ->(y: Int64) -> Int64 { y + x }\nputs add_x.call(5)\n";

  #[test]
  fn parses_lambda_capture_and_call() {
    let program = parse(LAMBDA_EXAMPLE).expect("lambda example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[1]
    else {
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
          ty: "Int64".into(),
          default: None
        }],
        return_type: "Int64".into(),
        body: vec![s(Stmt::Expr(s(Expr::Add(
          Box::new(s(Expr::Ident("y".into()))),
          Box::new(s(Expr::Ident("x".into())))
        ))))],
      }
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be `puts add_x.call(5)`, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("add_x".into()))),
          "call".into(),
          vec![s(Expr::Int(5))]
        ))]
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
    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected a method-call statement, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(
      *call,
      Expr::MethodCall(
        Box::new(s(Expr::Ident("p".into()))),
        "move".into(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )
    );
  }

  #[test]
  fn parses_raise() {
    let src = "raise MyError.new(99)\n";
    let program = parse(src).expect("raise should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Raise(s(Expr::New(
        "MyError".into(),
        vec![s(Expr::Int(99))]
      )))))
    );
  }

  const EXCEPTION_EXAMPLE: &str = "class MyError\n  code: Int64\n\n  def initialize(code: Int64) -> Void\n    @code = code\n  end\n\n  def code -> Int64\n    @code\n  end\nend\n\ndef risky(x: Int64) -> Int64\n  if x > 100\n    raise MyError.new(99)\n  end\n  return x\nend\n\nbegin\n  puts risky(999)\nrescue MyError => e\n  puts e.code\nend\n";

  #[test]
  fn parses_begin_rescue() {
    let program = parse(EXCEPTION_EXAMPLE).expect("exception example should parse");
    assert_eq!(program.items.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        body,
        rescues,
        ensure,
      },
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected item 2 to be a begin/rescue statement, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(rescues.len(), 1);
    assert_eq!(rescues[0].class_name, Some("MyError".to_string()));
    assert_eq!(rescues[0].var, "e");
    assert_eq!(ensure, &None);
    assert_eq!(
      body,
      &vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::Call("risky".into(), vec![s(Expr::Int(999))]))]
      ))))]
    );
    assert_eq!(
      rescues[0].body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("e".into()))),
          "code".into(),
          vec![]
        ))]
      ))))]
    );
  }

  // Plan 38 (full exception model).

  #[test]
  fn begin_with_multiple_rescues_and_ensure_parses_in_source_order() {
    let src = "begin\n  puts risky(999)\nrescue NotFoundError => e\n  puts e.code\nrescue TimeoutError => e2\n  puts e2.code\nensure\n  puts \"cleanup\"\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        rescues, ensure, ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues.len(), 2);
    assert_eq!(rescues[0].class_name, Some("NotFoundError".to_string()));
    assert_eq!(rescues[1].class_name, Some("TimeoutError".to_string()));
    assert_eq!(
      ensure,
      &Some(vec![s(Stmt::Expr(s(Expr::Call(
        "puts".into(),
        vec![s(Expr::StringLit("cleanup".into()))]
      ))))])
    );
  }

  #[test]
  fn begin_with_bare_rescue_and_no_ensure_parses() {
    let src = "begin\n  puts 1\nrescue => e\n  puts 2\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin {
        rescues, ensure, ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues.len(), 1);
    assert_eq!(rescues[0].class_name, None);
    assert_eq!(rescues[0].var, "e");
    assert_eq!(ensure, &None);
  }

  #[test]
  fn retry_parses_to_stmt_retry_wherever_a_stmt_is_legal() {
    let src = "begin\n  puts 1\nrescue => e\n  retry\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Begin { rescues, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a begin statement");
    };
    assert_eq!(rescues[0].body, vec![Stmt::Retry]);
  }

  // Plan 39 (function signature completeness).

  #[test]
  fn default_param_value_parses_into_param_default() {
    let src = "def inc(n: Int64, step: Int64 = 1) -> Int64\n  n + step\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params[0].default, None);
    assert_eq!(f.params[1].default, Some(s(Expr::Int(1))));
  }

  #[test]
  fn default_referencing_another_parameter_is_a_parse_error() {
    let src = "def bad(n: Int64, step: Int64 = n) -> Int64\n  n + step\nend\n";
    assert!(
      parse(src).is_err(),
      "DefaultLit admits only literal tokens, never an Expr::Ident"
    );
  }

  #[test]
  fn splat_param_parses_into_function_splat_param() {
    let src = "def sum_all(*xs: Int64) -> Int64\n  0\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert!(f.params.is_empty());
    let splat = f.splat_param.as_ref().expect("splat_param should be Some");
    assert_eq!(splat.name, "xs");
    assert_eq!(splat.ty, "Int64");
  }

  #[test]
  fn ordinary_param_then_splat_param_parses() {
    let src = "def f(a: Int64, *xs: Int64) -> Int64\n  0\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params.len(), 1);
    assert_eq!(f.params[0].name, "a");
    assert_eq!(f.splat_param.as_ref().unwrap().name, "xs");
  }

  #[test]
  fn keyword_call_parses_to_expr_call_kw() {
    let src = "greet(name: \"yo\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::CallKw(
        "greet".to_string(),
        vec![("name".to_string(), s(Expr::StringLit("yo".to_string())))]
      )))))
    );
  }

  #[test]
  fn positional_call_still_parses_to_expr_call_unchanged() {
    let src = "greet(\"yo\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "greet".to_string(),
        vec![s(Expr::StringLit("yo".to_string()))]
      )))))
    );
  }

  #[test]
  fn return_tuple_parses_into_a_tuple_lit() {
    let src = "def divmod(a: Int64, b: Int64) -> (Int64, Int64)\n  return a / b, a % b\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.return_type, "(Int64, Int64)");
    let Stmt::Return(Some(Spanned {
      node: Expr::TupleLit(elems),
      ..
    })) = &f.body[0].node
    else {
      panic!("expected a tuple-literal return, got {:?}", f.body[0]);
    };
    assert_eq!(elems.len(), 2);
  }

  #[test]
  fn multi_assign_from_a_single_call_value_parses() {
    let src = "q: Int64 = 0\nr: Int64 = 0\nq, r = f(1, 2)\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::MultiAssign { names, values },
      ..
    }) = &program.items[2]
    else {
      panic!(
        "expected a MultiAssign statement, got {:?}",
        program.items[2]
      );
    };
    assert_eq!(names, &vec!["q".to_string(), "r".to_string()]);
    assert_eq!(values.len(), 1);
  }

  // Plan 40 (operator overloading).

  fn operator_method_name(src: &str) -> String {
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    c.methods[0].name.clone()
  }

  #[test]
  fn plus_operator_method_parses_to_a_function_named_plus() {
    let src = "class Vector2\n  def +(other: Vector2) -> Vector2\n    self\n  end\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a class");
    };
    assert_eq!(
      c.methods[0],
      Function {
        name: "+".to_string(),
        params: vec![Param {
          name: "other".to_string(),
          ty: "Vector2".to_string(),
          default: None,
        }],
        return_type: "Vector2".to_string(),
        body: vec![s(Stmt::Expr(s(Expr::Ident("self".to_string()))))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
      }
    );
  }

  #[test]
  fn all_eight_operator_tokens_parse_as_method_names() {
    assert_eq!(
      operator_method_name("class C\n  def -(o: C) -> C\n    self\n  end\nend\n"),
      "-"
    );
    assert_eq!(
      operator_method_name("class C\n  def *(o: C) -> C\n    self\n  end\nend\n"),
      "*"
    );
    assert_eq!(
      operator_method_name("class C\n  def /(o: C) -> C\n    self\n  end\nend\n"),
      "/"
    );
    assert_eq!(
      operator_method_name("class C\n  def ==(o: C) -> Boolean\n    true\nend\nend\n"),
      "=="
    );
    assert_eq!(
      operator_method_name("class C\n  def <=>(o: C) -> Int64\n    0\n  end\nend\n"),
      "<=>"
    );
    assert_eq!(
      operator_method_name("class C\n  def [](i: Int64) -> Float64\n    1.0\n  end\nend\n"),
      "[]"
    );
    assert_eq!(
      operator_method_name(
        "class C\n  def []=(i: Int64, v: Float64) -> Void\n    puts 1\n  end\nend\n"
      ),
      "[]="
    );
  }

  #[test]
  fn top_level_operator_named_def_is_a_parse_error() {
    let src = "def +(a: Int64, b: Int64) -> Int64\n  a + b\nend\n";
    assert!(
      parse(src).is_err(),
      "operator-named methods stay class-body-only"
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
        ty: "Int64".into(),
        default: None
      }]
    );
    assert_eq!(m.methods[0].return_type, "Int64");
    assert_eq!(
      m.methods[0].body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("x".into()))),
        Box::new(s(Expr::Ident("x".into())))
      ))))]
    );

    let Item::Stmt(Spanned {
      node: Stmt::Expr(call),
      ..
    }) = &program.items[1]
    else {
      panic!(
        "expected item 1 to be `puts MathUtils.double(21)`, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(
      *call,
      Expr::Call(
        "puts".into(),
        vec![s(Expr::MethodCall(
          Box::new(s(Expr::Ident("MathUtils".into()))),
          "double".into(),
          vec![s(Expr::Int(21))]
        ))]
      )
    );
  }

  // Plan 18 (arithmetic & logical operators) — precedence is real, not
  // just parseable (AC1).

  #[test]
  fn mul_binds_tighter_than_add() {
    let program = parse("puts 2 + 3 * 4\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(s(Expr::Int(2))),
        Box::new(s(Expr::Mul(
          Box::new(s(Expr::Int(3))),
          Box::new(s(Expr::Int(4)))
        )))
      )
    );
  }

  #[test]
  fn unary_minus_binds_tighter_than_add() {
    let program = parse("puts -3 + 10\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Add(
        Box::new(s(Expr::Neg(Box::new(s(Expr::Int(3)))))),
        Box::new(s(Expr::Int(10)))
      )
    );
  }

  #[test]
  fn and_binds_tighter_than_or() {
    let program = parse("puts a > 0 && b > 0 || c > 0\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    let gt = |name: &str, n: i64| {
      Expr::Compare(
        Box::new(s(Expr::Ident(name.into()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(n))),
      )
    };
    assert_eq!(
      args[0],
      Expr::Or(
        Box::new(s(Expr::And(
          Box::new(s(gt("a", 0))),
          Box::new(s(gt("b", 0)))
        ))),
        Box::new(s(gt("c", 0)))
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
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(
      args[0],
      Expr::Compare(
        Box::new(s(Expr::Not(Box::new(s(Expr::Ident("x".into())))))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(0)))
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
        cond: s(Expr::Compare(
          Box::new(s(Expr::Ident("n".into()))),
          CompareOp::Le,
          Box::new(s(Expr::Int(1)))
        )),
        then_branch: vec![s(Stmt::Return(Some(s(Expr::Int(1)))))],
        else_branch: None,
      }
    );
    assert_eq!(
      f.body[1],
      Stmt::Return(Some(s(Expr::Mul(
        Box::new(s(Expr::Ident("n".into()))),
        Box::new(s(Expr::Call(
          "factorial".into(),
          vec![s(Expr::Sub(
            Box::new(s(Expr::Ident("n".into()))),
            Box::new(s(Expr::Int(1)))
          ))]
        )))
      ))))
    );
  }

  const SHORT_CIRCUIT_EXAMPLE: &str = "def noisy(n: Int64) -> Boolean\n  puts n\n  return n > 0\nend\n\nx: Int64 = -5\nif x > 0 && noisy(1)\n  puts 100\nend\nif x > -10 && noisy(3)\n  puts 300\nend\nif x < 0 || noisy(2)\n  puts 200\nend\nif x > 0 || noisy(4)\n  puts 400\nend\n";

  #[test]
  fn parses_short_circuit_example() {
    let program = parse(SHORT_CIRCUIT_EXAMPLE).expect("short-circuit example should parse");
    // 1 function + 1 let + 4 ifs.
    assert_eq!(program.items.len(), 6);
    let Item::Stmt(Spanned {
      node: Stmt::If { cond, .. },
      ..
    }) = &program.items[2]
    else {
      panic!("expected the first if, got {:?}", program.items[2]);
    };
    assert_eq!(
      *cond,
      Expr::And(
        Box::new(s(Expr::Compare(
          Box::new(s(Expr::Ident("x".into()))),
          CompareOp::Gt,
          Box::new(s(Expr::Int(0)))
        ))),
        Box::new(s(Expr::Call("noisy".into(), vec![s(Expr::Int(1))])))
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
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("hello".to_string()));
  }

  #[test]
  fn decodes_escaped_quote() {
    let program = parse("puts \"a\\\"b\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a\"b".to_string()));
  }

  #[test]
  fn decodes_escaped_newline() {
    let program = parse("puts \"line1\\nline2\"\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
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
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    }) = &program.items[0]
    else {
      panic!("expected a puts call, got {:?}", program.items[0]);
    };
    assert_eq!(args[0], Expr::StringLit("a#b".to_string()));
  }

  const CASE_EXAMPLE: &str = "# classify an integer by a fixed set of buckets\nn: Int64 = 2\nlabel: Int64 = 0\ncase n\nwhen 1\n  label: Int64 = 10\nwhen 2, 3\n  label: Int64 = 20\nelse\n  label: Int64 = 99\nend\nputs label\n";

  #[test]
  fn parses_case_when_example() {
    let program = parse(CASE_EXAMPLE).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case {
        scrutinee,
        arms,
        else_body,
      },
      ..
    }) = &program.items[2]
    else {
      panic!("expected a case statement, got {:?}", program.items[2]);
    };
    assert_eq!(*scrutinee, Expr::Ident("n".into()));
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, CasePattern::Values(vec![s(Expr::Int(1))]));
    assert_eq!(
      arms[0].1,
      vec![s(Stmt::Let {
        name: "label".into(),
        ty: "Int64".into(),
        value: s(Expr::Int(10)),
      })]
    );
    assert_eq!(
      arms[1].0,
      CasePattern::Values(vec![s(Expr::Int(2)), s(Expr::Int(3))])
    );
    assert!(else_body.is_some());
  }

  // Plan 25 (stdlib expansion).

  #[test]
  fn parses_bool_and_nil_literals() {
    let program = parse("puts true\nputs false\nputs nil\n")
      .expect("should parse")
      .items;
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a1),
        ..
      }),
      ..
    }) = &program[0]
    else {
      panic!("expected a puts call, got {:?}", program[0]);
    };
    assert_eq!(a1[0], Expr::Bool(true));
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a2),
        ..
      }),
      ..
    }) = &program[1]
    else {
      panic!("expected a puts call, got {:?}", program[1]);
    };
    assert_eq!(a2[0], Expr::Bool(false));
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, a3),
        ..
      }),
      ..
    }) = &program[2]
    else {
      panic!("expected a puts call, got {:?}", program[2]);
    };
    assert_eq!(a3[0], Expr::Nil);
  }

  #[test]
  fn parses_hash_literal_and_indexing() {
    let src = "h: Hash[Int64, Int64] = {1 => 10, 2 => 20, 3 => 30}\nputs h[2]\nh[2] = 99\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { name, ty, value },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(name, "h");
    assert_eq!(ty, "Hash[Int64, Int64]");
    assert_eq!(
      *value,
      Expr::HashLit(vec![
        (s(Expr::Int(1)), s(Expr::Int(10))),
        (s(Expr::Int(2)), s(Expr::Int(20))),
        (s(Expr::Int(3)), s(Expr::Int(30))),
      ])
    );
    assert_eq!(
      program.items[2],
      Item::Stmt(s(Stmt::SetIndex {
        array: s(Expr::Ident("h".into())),
        index: s(Expr::Int(2)),
        value: s(Expr::Int(99)),
      }))
    );
  }

  #[test]
  fn parses_array_new() {
    let program = parse("arr: Array[Int64] = Array.new(5)\n").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: s(Expr::ArrayNew(Box::new(s(Expr::Int(5))))),
      }))
    );
  }

  #[test]
  fn parses_array_new_with_runtime_size() {
    let program = parse("n: Int64 = 5\narr: Array[Int64] = Array.new(n)\n").expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::Let {
        name: "arr".into(),
        ty: "Array[Int64]".into(),
        value: s(Expr::ArrayNew(Box::new(s(Expr::Ident("n".into()))))),
      }))
    );
  }

  #[test]
  fn case_with_no_else_parses() {
    let src = "case n\nwhen 1\n  puts 1\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case { else_body, .. },
      ..
    }) = &program.items[0]
    else {
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
      Item::Stmt(s(Stmt::Assign {
        name: "x".to_string(),
        value: s(Expr::Int(2)),
      }))
    );
  }

  #[test]
  fn compound_plus_assign_desugars_to_assign_of_add() {
    let src = "total += i\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Assign {
        name: "total".to_string(),
        value: s(Expr::Add(
          Box::new(s(Expr::Ident("total".to_string()))),
          Box::new(s(Expr::Ident("i".to_string()))),
        )),
      }))
    );
  }

  fn assert_compound_assign_desugars_to(src: &str, expected_value: Expr) {
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Assign {
        name: "x".to_string(),
        value: s(expected_value),
      })),
      "mismatched desugaring for {src:?}"
    );
  }

  #[test]
  fn all_compound_assign_operators_desugar_to_the_matching_binop() {
    let x = || Box::new(s(Expr::Ident("x".to_string())));
    let one = || Box::new(s(Expr::Int(1)));
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
      Item::Stmt(s(Stmt::MultiAssign {
        names: vec!["a".to_string(), "b".to_string()],
        values: vec![
          s(Expr::Ident("b".to_string())),
          s(Expr::Ident("a".to_string()))
        ],
      }))
    );
  }

  #[test]
  fn plan_31_worked_example_parses() {
    let src = "total: Int64 = 0\ni: Int64 = 0\nwhile i < 5\n  total += i\n  i += 1\nend\nputs total\n\na: Int64 = 1\nb: Int64 = 2\na, b = b, a\nputs a\nputs b\n";
    parse(src).expect("plan 31's worked example must parse cleanly");
  }

  // Plan 34 (blocks and yield).

  #[test]
  fn block_param_marker_parses_separately_from_params() {
    let src = "def repeat(n: Int64, &blk) -> Void\n  yield n\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(
      f.params,
      vec![Param {
        name: "n".to_string(),
        ty: "Int64".to_string(),
        default: None
      }]
    );
    assert_eq!(f.block_param, Some("blk".to_string()));
  }

  #[test]
  fn zero_arg_block_param_parses() {
    let src = "def once(&blk) -> Void\n  yield 1\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.params, vec![]);
    assert_eq!(f.block_param, Some("blk".to_string()));
  }

  #[test]
  fn function_with_no_block_param_is_none() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend\n";
    let program = parse(src).unwrap();
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function");
    };
    assert_eq!(f.block_param, None);
  }

  #[test]
  fn trailing_block_literal_desugars_to_an_extra_call_argument() {
    let src = "repeat(3) { |i: Int64| puts i }\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "repeat".to_string(),
        vec![
          s(Expr::Int(3)),
          s(Expr::Lambda {
            params: vec![Param {
              name: "i".to_string(),
              ty: "Int64".to_string(),
              default: None
            }],
            return_type: "Void".to_string(),
            body: vec![s(Stmt::Expr(s(Expr::Call(
              "puts".to_string(),
              vec![s(Expr::Ident("i".to_string()))]
            ))))],
          }),
        ]
      )))))
    );
  }

  #[test]
  fn call_with_no_trailing_block_parses_unchanged() {
    let src = "add(1, 2)\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "add".to_string(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )))))
    );
  }

  #[test]
  fn yield_parses_to_stmt_yield() {
    let src = "yield i\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Yield(vec![s(Expr::Ident("i".to_string()))])))
    );
  }

  #[test]
  fn plan_34_worked_example_parses() {
    let src = "def repeat(n: Int64, &blk) -> Void\n  i: Int64 = 0\n  while i < n\n    yield i\n    i: Int64 = i + 1\n  end\nend\n\nrepeat(3) { |i: Int64| puts i }\n";
    parse(src).expect("plan 34's worked example must parse cleanly");
  }

  // Plan 36 (string interpolation and heredocs).

  #[test]
  fn interpolated_string_splits_into_parts() {
    let src = "puts \"Hello, #{name}!\"\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::Interpolate(vec![
          StringPart::Literal("Hello, ".to_string()),
          StringPart::Expr(Box::new(s(Expr::Ident("name".to_string())))),
          StringPart::Literal("!".to_string()),
        ]))]
      )))))
    );
  }

  #[test]
  fn plain_string_with_no_interpolation_still_parses_as_string_lit() {
    let src = "puts \"hello\"\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::StringLit("hello".to_string()))]
      )))))
    );
  }

  #[test]
  fn nested_hash_literal_inside_interpolation_parses_via_brace_depth_scan() {
    let src = "puts \"#{ {1 => 2}[1] }\"\n";
    parse(src).expect("brace-depth scan must not cut the span short at the hash literal's own `}`");
  }

  #[test]
  fn unterminated_interpolation_is_a_real_parse_error() {
    let src = "puts \"unterminated #{name\"\n";
    assert!(
      parse(src).is_err(),
      "an unterminated `#{{` must be a real parse error, not a silent truncation"
    );
  }

  // Plan 23 (multi-file compilation).

  #[test]
  fn require_single_segment_parses() {
    let src = "require helpers\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items[0], Item::Require("helpers".to_string()));
  }

  #[test]
  fn require_multi_segment_path_parses() {
    let src = "require utils/math\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items[0], Item::Require("utils/math".to_string()));
  }

  #[test]
  fn require_inside_a_function_body_is_a_parse_error() {
    let src = "def f -> Void\n  require helpers\nend\n";
    assert!(
      parse(src).is_err(),
      "`require` must be rejected inside a function body"
    );
  }

  // Plan 33 (field-access sugar).

  #[test]
  fn read_field_synthesizes_a_zero_arg_accessor() {
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    // `fields` is unchanged: two plain, unmarked Params.
    assert_eq!(
      c.fields,
      vec![
        Param {
          name: "x".to_string(),
          ty: "Int64".to_string(),
          default: None
        },
        Param {
          name: "y".to_string(),
          ty: "Int64".to_string(),
          default: None
        },
      ]
    );
    // `methods` contains a synthesized zero-arg accessor for `x`,
    // alongside the hand-written `initialize` — but nothing for `y`.
    let x_accessor = c
      .methods
      .iter()
      .find(|m| m.name == "x")
      .expect("expected a synthesized `x` accessor method");
    assert_eq!(x_accessor.params, vec![]);
    assert_eq!(x_accessor.return_type, "Int64");
    assert_eq!(
      x_accessor.body,
      vec![s(Stmt::Expr(s(Expr::InstanceVar("x".to_string()))))]
    );
    assert!(c.methods.iter().any(|m| m.name == "initialize"));
    assert!(!c.methods.iter().any(|m| m.name == "y"));
  }

  #[test]
  fn plan_33_worked_example_parses() {
    let src = "class Point\n  read x: Int64\n  y: Int64\n\n  def initialize(x: Int64, y: Int64) -> Void\n    @x = x\n    @y = y\n  end\nend\n\np: Point = Point.new(3, 4)\nputs p.x\n";
    parse(src).expect("plan 33's worked example must parse cleanly");
  }

  // Plan 32 (class inheritance).

  #[test]
  fn class_with_no_superclass_parses_with_none() {
    let src = "class Counter\n  n: Int64\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    assert_eq!(c.superclass, None);
  }

  #[test]
  fn class_with_superclass_parses_the_name() {
    let src = "class Dog < Animal\n  breed_code: Int64\nend\n";
    let program = parse(src).unwrap();
    let Item::Class(c) = &program.items[0] else {
      panic!("expected a ClassDef");
    };
    assert_eq!(c.superclass, Some("Animal".to_string()));
  }

  #[test]
  fn plan_32_worked_example_parses() {
    let src = "class Animal\n  age: Int64\n\n  def initialize(age: Int64) -> Void\n    @age = age\n  end\n\n  def age -> Int64\n    @age\n  end\n\n  def describe -> Int64\n    @age\n  end\nend\n\nclass Dog < Animal\n  breed_code: Int64\n\n  def initialize(age: Int64, breed_code: Int64) -> Void\n    @age = age\n    @breed_code = breed_code\n  end\n\n  def describe -> Int64\n    @age + @breed_code\n  end\nend\n\na: Animal = Animal.new(5)\nd: Dog = Dog.new(3, 100)\nputs a.describe\nputs d.age\nputs d.describe\n";
    parse(src).expect("plan 32's worked example must parse cleanly");
  }

  // Plan 30 (for-in iteration).

  #[test]
  fn for_in_over_a_literal_array_parses() {
    let src = "for x in [10, 20, 30]\n  puts x\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::For {
        var: "x".to_string(),
        elements: vec![s(Expr::Int(10)), s(Expr::Int(20)), s(Expr::Int(30))],
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("x".to_string()))]
        ))))],
      }))
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

  // Plan 37 (ranges and range-based iteration).

  #[test]
  fn inclusive_range_for_in_parses_to_stmt_for_range() {
    let src = "for i in 1..5\n  puts i\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::ForRange {
        var: "i".to_string(),
        start: s(Expr::Int(1)),
        end: s(Expr::Int(5)),
        exclusive: false,
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("i".to_string()))]
        ))))],
      }))
    );
  }

  #[test]
  fn exclusive_range_for_in_parses_to_stmt_for_range() {
    let src = "for i in 1...5\n  puts i\nend\n";
    let program = parse(src).unwrap();
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::ForRange {
        var: "i".to_string(),
        start: s(Expr::Int(1)),
        end: s(Expr::Int(5)),
        exclusive: true,
        body: vec![s(Stmt::Expr(s(Expr::Call(
          "puts".to_string(),
          vec![s(Expr::Ident("i".to_string()))]
        ))))],
      }))
    );
  }

  #[test]
  fn bare_range_outside_a_for_in_scrutinee_is_a_parse_error() {
    let src = "r = 1..5\n";
    assert!(
      parse(src).is_err(),
      "a Range has no standalone AST shape — only legal directly after `for <var> in`"
    );
  }

  #[test]
  fn endless_range_for_in_is_a_parse_error() {
    let src = "for i in 5..\n  puts i\nend\n";
    assert!(
      parse(src).is_err(),
      "an endless range (no end operand) must be rejected at parse time"
    );
  }

  // Plan 29 (control-flow completeness).

  #[test]
  fn elsif_chain_desugars_to_nested_if_in_else_branch() {
    let src = "if a\n  1\nelsif b\n  2\nelsif c\n  3\nelse\n  4\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::If {
        then_branch,
        else_branch,
        ..
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a top-level If statement");
    };
    assert_eq!(*then_branch, vec![s(Stmt::Expr(s(Expr::Int(1))))]);

    // First elsif link.
    let Some(outer_else) = else_branch else {
      panic!("expected an else_branch from the first elsif");
    };
    assert_eq!(outer_else.len(), 1);
    let Spanned {
      node: Stmt::If {
        then_branch: b2,
        else_branch: e2,
        ..
      },
      ..
    } = &outer_else[0]
    else {
      panic!("expected a nested If for the first elsif");
    };
    assert_eq!(*b2, vec![s(Stmt::Expr(s(Expr::Int(2))))]);

    // Second elsif link.
    let Some(inner_else) = e2 else {
      panic!("expected an else_branch from the second elsif");
    };
    assert_eq!(inner_else.len(), 1);
    let Spanned {
      node: Stmt::If {
        then_branch: b3,
        else_branch: e3,
        ..
      },
      ..
    } = &inner_else[0]
    else {
      panic!("expected a nested If for the second elsif");
    };
    assert_eq!(*b3, vec![s(Stmt::Expr(s(Expr::Int(3))))]);

    // Trailing plain else.
    assert_eq!(*e3, Some(vec![s(Stmt::Expr(s(Expr::Int(4))))]));
  }

  #[test]
  fn unless_desugars_to_if_not() {
    let src = "unless x > 0\n  return 0\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::If {
        cond,
        then_branch,
        else_branch,
      },
      ..
    }) = &program.items[0]
    else {
      panic!("expected an If statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(s(Expr::Compare(
        Box::new(s(Expr::Ident("x".to_string()))),
        CompareOp::Gt,
        Box::new(s(Expr::Int(0))),
      ))))
    );
    assert_eq!(*then_branch, vec![s(Stmt::Return(Some(s(Expr::Int(0)))))]);
    assert_eq!(*else_branch, None);
  }

  #[test]
  fn until_desugars_to_while_not() {
    let src = "until i >= 3\n  puts i\nend\n";
    let program = parse(src).unwrap();
    let Item::Stmt(Spanned {
      node: Stmt::While { cond, body },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a While statement");
    };
    assert_eq!(
      *cond,
      Expr::Not(Box::new(s(Expr::Compare(
        Box::new(s(Expr::Ident("i".to_string()))),
        CompareOp::Ge,
        Box::new(s(Expr::Int(3))),
      ))))
    );
    assert_eq!(
      *body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "puts".to_string(),
        vec![s(Expr::Ident("i".to_string()))]
      ))))]
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
        Box::new(s(Expr::Int(1))),
        Box::new(s(Expr::BitAnd(
          Box::new(s(Expr::Int(2))),
          Box::new(s(Expr::Int(3)))
        ))),
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
        Box::new(s(Expr::Shl(
          Box::new(s(Expr::Int(1))),
          Box::new(s(Expr::Int(2)))
        ))),
        Box::new(s(Expr::Int(3))),
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
        Box::new(s(Expr::BitAnd(
          Box::new(s(Expr::Ident("flags".to_string()))),
          Box::new(s(Expr::Ident("flag".to_string()))),
        ))),
        CompareOp::Eq,
        Box::new(s(Expr::Ident("flag".to_string()))),
      )
    );
  }

  #[test]
  fn bit_not_binds_as_tight_as_neg() {
    let src = "x: Int64 = ~0\ny: Int64 = ~x + 1\n";
    let program = parse(src).unwrap();
    assert_eq!(program.items.len(), 2);
    let Item::Stmt(Spanned {
      node: Stmt::Let { value: v0, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(*v0, Expr::BitNot(Box::new(s(Expr::Int(0)))));
    let Item::Stmt(Spanned {
      node: Stmt::Let { value: v1, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      *v1,
      Expr::Add(
        Box::new(s(Expr::BitNot(Box::new(s(Expr::Ident("x".to_string())))))),
        Box::new(s(Expr::Int(1))),
      )
    );
  }

  fn as_let(program: &Program) -> &Stmt {
    match &program.items[0] {
      Item::Stmt(
        spanned @ Spanned {
          node: Stmt::Let { .. },
          ..
        },
      ) => &spanned.node,
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
      Item::Stmt(s(Stmt::Let {
        name: "s".into(),
        ty: "String".into(),
        value: s(Expr::StringLit("hello".into())),
      }))
    );
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::Let {
        name: "a".into(),
        ty: "String".into(),
        value: s(Expr::Add(
          Box::new(s(Expr::StringLit("foo".into()))),
          Box::new(s(Expr::StringLit("bar".into())))
        )),
      }))
    );
  }

  // Plan 41 (interfaces and generics).

  const INTERFACES_GENERICS_EXAMPLE: &str = "interface Comparable\n  def compare_to(other: Self) -> Int64\nend\n\nclass Money implements Comparable\n  read cents: Int64\n\n  def initialize(cents: Int64) -> Void\n    @cents = cents\n  end\n\n  def compare_to(other: Money) -> Int64\n    @cents - other.cents\n  end\nend\n\nclass Distance implements Comparable\n  read meters: Int64\n\n  def initialize(meters: Int64) -> Void\n    @meters = meters\n  end\n\n  def compare_to(other: Distance) -> Int64\n    @meters - other.meters\n  end\nend\n\ndef max[T: Comparable](a: T, b: T) -> T\n  if a.compare_to(b) >= 0\n    return a\n  end\n  return b\nend\n\nm1: Money = Money.new(500)\nm2: Money = Money.new(750)\nwinner_money: Money = max(m1, m2)\nputs winner_money.cents\n\nd1: Distance = Distance.new(100)\nd2: Distance = Distance.new(42)\nwinner_distance: Distance = max(d1, d2)\nputs winner_distance.meters\n";

  #[test]
  fn worked_example_parses_end_to_end_into_the_expected_ast_shapes() {
    let program = parse(INTERFACES_GENERICS_EXAMPLE).expect("should parse");

    let Item::Interface(iface) = &program.items[0] else {
      panic!(
        "expected item 0 to be the `Comparable` interface, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(iface.name, "Comparable");
    assert_eq!(iface.method_name, "compare_to");
    assert_eq!(
      iface.params,
      vec![Param {
        name: "other".to_string(),
        ty: "Self".to_string(),
        default: None,
      }]
    );
    assert_eq!(iface.return_type, "Int64");

    let Item::Class(money) = &program.items[1] else {
      panic!(
        "expected item 1 to be the `Money` class, got {:?}",
        program.items[1]
      );
    };
    assert_eq!(money.name, "Money");
    assert_eq!(money.implements, Some("Comparable".to_string()));

    let Item::Function(max_fn) = &program.items[3] else {
      panic!(
        "expected item 3 to be the `max` function, got {:?}",
        program.items[3]
      );
    };
    assert_eq!(max_fn.name, "max");
    assert_eq!(
      max_fn.type_params,
      vec![TypeParam {
        name: "T".to_string(),
        bound: "Comparable".to_string(),
      }]
    );
  }

  #[test]
  fn multiple_type_parameters_parse_the_grammar_does_not_restrict_the_count() {
    // Plan 41's Decision log: the grammar's `TypeParamList` is a general
    // comma list — the single-type-parameter restriction is sema's job.
    let src = "def bad[T: Comparable, U: Comparable](a: T, b: U) -> T\n  a\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a function, got {:?}", program.items[0]);
    };
    assert_eq!(f.type_params.len(), 2);
    assert_eq!(f.type_params[0].name, "T");
    assert_eq!(f.type_params[1].name, "U");
  }

  #[test]
  fn interface_with_a_second_method_before_end_is_a_parse_error_not_a_panic() {
    // The grammar structurally admits exactly one method — a second
    // `def` before the outer `end` cannot reduce as this same
    // production, so this is a real parse error.
    let src = "interface Comparable\n  def compare_to(other: Self) -> Int64\n  def other_method(x: Int64) -> Int64\nend\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  #[test]
  fn interface_missing_the_inner_arrow_return_type_is_a_parse_error_not_a_panic() {
    let src = "interface Comparable\n  def compare_to(other: Self)\nend\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  // Plan 43 (nullable types and safe navigation).

  #[test]
  fn nullable_suffix_parses_into_a_compound_type_string() {
    let src = "g: Greeter? = nil\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(ty, "Greeter?");
  }

  #[test]
  fn safe_call_parses_to_expr_safe_call() {
    let src = "message: String? = g&.shout\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::SafeCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "shout".to_string(),
        vec![]
      )
    );
  }

  #[test]
  fn safe_call_with_args_parses_to_expr_safe_call() {
    let src = "x: Int64? = g&.add(1, 2)\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::SafeCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "add".to_string(),
        vec![s(Expr::Int(1)), s(Expr::Int(2))]
      )
    );
  }

  #[test]
  fn ordinary_dot_method_call_still_parses_to_expr_method_call_unchanged() {
    let src = "x: Int64 = g.shout\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::MethodCall(
        Box::new(s(Expr::Ident("g".to_string()))),
        "shout".to_string(),
        vec![]
      )
    );
  }

  #[test]
  fn or_assign_parses_to_stmt_or_assign() {
    let src = "message: String? = nil\nmessage ||= \"nobody here\"\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::OrAssign {
        name: "message".to_string(),
        default: s(Expr::StringLit("nobody here".to_string())),
      }))
    );
  }

  #[test]
  fn and_assign_parses_to_stmt_and_assign() {
    let src = "g: Greeter? = nil\ng &&= Greeter.new(\"upgraded\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[1],
      Item::Stmt(s(Stmt::AndAssign {
        name: "g".to_string(),
        value: s(Expr::New(
          "Greeter".to_string(),
          vec![s(Expr::StringLit("upgraded".to_string()))]
        )),
      }))
    );
  }

  #[test]
  fn or_assign_on_a_non_ident_target_is_a_parse_error_not_a_panic() {
    let src = "arr[0] ||= 1\n";
    let errs = parse(src).unwrap_err();
    assert!(!errs.is_empty());
  }

  #[test]
  fn nullable_worked_example_parses_end_to_end() {
    let src = "class Greeter\n  name: String\n\n  def initialize(name: String) -> Void\n    @name = name\n  end\n\n  def shout -> String\n    @name + \"!\"\n  end\nend\n\ndef find_greeter(id: Int64) -> Greeter?\n  if id == 1\n    return Greeter.new(\"ada\")\n  end\n  return nil\nend\n\ndef greet(id: Int64) -> String\n  g: Greeter? = find_greeter(id)\n  message: String? = g&.shout\n  message ||= \"nobody here\"\n  return message\nend\n\nputs greet(1)\nputs greet(2)\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 5);
  }

  // Plan 44 (symbols).

  #[test]
  fn symbol_literal_parses_to_expr_symbol_lit() {
    let src = ":foo\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::SymbolLit("foo".to_string())))))
    );
  }

  #[test]
  fn symbol_typed_let_parses() {
    let src = "x: Symbol = :foo\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "x".to_string(),
        ty: "Symbol".to_string(),
        value: s(Expr::SymbolLit("foo".to_string())),
      }))
    );
  }

  #[test]
  fn ordinary_spaced_type_annotation_still_parses_identically() {
    let src = "x: Int64 = 1\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "x".to_string(),
        ty: "Int64".to_string(),
        value: s(Expr::Int(1)),
      }))
    );
  }

  #[test]
  fn symbol_keyed_hash_literal_parses() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82}\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let, got {:?}", program.items[0]);
    };
    assert_eq!(
      *value,
      Expr::HashLit(vec![
        (s(Expr::SymbolLit("alice".to_string())), s(Expr::Int(90))),
        (s(Expr::SymbolLit("bob".to_string())), s(Expr::Int(82))),
      ])
    );
  }

  #[test]
  fn symbols_worked_example_parses_end_to_end() {
    let src = "scores: Hash[Symbol, Int64] = {:alice => 90, :bob => 82, :carol => 95}\nputs scores[:bob]\nscores[:bob] = 100\nputs scores[:bob]\n\nif :foo == :foo\n  puts 1\nelse\n  puts 0\nend\n\nif :foo == :bar\n  puts 1\nelse\n  puts 0\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 6);
  }

  // Plan 45 (stdlib strings and I/O).

  #[test]
  fn dot_read_method_call_parses_despite_read_being_reserved_by_class_field_sugar() {
    // Plan 33's `read <name>: <Type>` class-field sugar already
    // reserves `read` as a distinct terminal, globally — `.read` (e.g.
    // `File.read(path)`) needs `CallMethodName` to widen the
    // method-name slot at every call site, or this fails to parse.
    let src = "content: String = File.read(\"x.txt\")\n";
    let program = parse(src).expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Let {
        name: "content".to_string(),
        ty: "String".to_string(),
        value: s(Expr::MethodCall(
          Box::new(s(Expr::Ident("File".to_string()))),
          "read".to_string(),
          vec![s(Expr::StringLit("x.txt".to_string()))],
        )),
      }))
    );
  }

  #[test]
  fn plan_45_worked_example_parses_end_to_end() {
    let src = "input: String = \"hello world foo\"\nupper: String = input.upcase\nFile.write(\"plan45_demo.txt\", upper)\nreadback: String = File.read(\"plan45_demo.txt\")\nputs readback\nn: Int64 = readback.split_count(\" \")\nputs n\nwords: Array[String] = readback.split(\" \")\ni: Int64 = 0\nwhile i < n\n  puts words[i]\n  i += 1\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 10);
  }

  // Plan 47 (REPL and test framework).

  #[test]
  fn test_block_parses_into_item_test() {
    let src = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 1);
    let Item::Test { description, body } = &program.items[0] else {
      panic!("expected an Item::Test, got {:?}", program.items[0]);
    };
    assert_eq!(description, "addition works");
    assert_eq!(body.len(), 1);
  }

  #[test]
  fn assert_call_rewrites_its_trailing_offset_into_a_file_line_string() {
    // `assert(...)` is on line 2 (1-based) of this named source.
    let src = "def f() -> Void\n  assert(true)\nend\n";
    let program = parse_named(src, "t.em").expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item");
    };
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Call(
        "assert".to_string(),
        vec![
          s(Expr::Bool(true)),
          s(Expr::StringLit("t.em:2".to_string()))
        ]
      ))))]
    );
  }

  #[test]
  fn assert_eq_call_rewrites_its_trailing_offset_into_a_file_line_string() {
    let src = "assert_eq(3, 1 + 1)\n";
    let program = parse_named(src, "math_test.em").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "assert_eq".to_string(),
        vec![
          s(Expr::Int(3)),
          s(Expr::Add(
            Box::new(s(Expr::Int(1))),
            Box::new(s(Expr::Int(1)))
          )),
          s(Expr::StringLit("math_test.em:1".to_string())),
        ]
      )))))
    );
  }

  #[test]
  fn assert_inside_a_nested_block_still_gets_its_own_correct_line() {
    // The rewrite pass must recurse into `if`/`while`/etc. bodies, not
    // just top-level statements — this asserts on line 3.
    let src = "x: Int64 = 1\nif x == 1\n  assert(x == 1)\nend\n";
    let program = parse_named(src, "nested.em").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::If { then_branch, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected an if statement");
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Call(_, args),
        ..
      }),
      ..
    } = &then_branch[0]
    else {
      panic!("expected an assert call");
    };
    assert_eq!(args[1], Expr::StringLit("nested.em:3".to_string()));
  }

  #[test]
  fn math_test_worked_example_parses_with_two_test_blocks() {
    let src = "test \"addition works\" do\n  assert_eq(2, 1 + 1)\nend\n\ntest \"addition is broken on purpose\" do\n  assert_eq(3, 1 + 1)\nend\n";
    let program = parse(src).expect("should parse");
    assert_eq!(program.items.len(), 2);
    assert!(matches!(program.items[0], Item::Test { .. }));
    assert!(matches!(program.items[1], Item::Test { .. }));
  }

  // Plan 22 (sema diagnostic spans) — leaf-ast-spans.

  #[test]
  fn add_rhs_span_points_at_the_real_second_b_not_the_parameter_or_the_whole_fn() {
    // This plan's own concrete-proof program (Decision log): the `rhs`
    // of `a + b` inside `add`'s body must carry the real byte offset of
    // the *second* `b` in the source — the one actually used in `a +
    // b`, not the first `b` (the `b: String` parameter declaration) and
    // not some placeholder/whole-document span.
    let src = "def add(a: Int64, b: String) -> Int64\n  a + b\nend";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Add(_, rhs),
        ..
      }),
      ..
    } = &f.body[0]
    else {
      panic!("expected `a + b` as add's only statement, got {:?}", f.body);
    };
    let second_b = src
      .match_indices('b')
      .nth(1)
      .expect("source has two occurrences of 'b'")
      .0;
    assert_eq!(rhs.span, (second_b, second_b + 1));
  }

  #[test]
  fn spans_are_non_degenerate_for_both_multi_and_single_character_tokens() {
    let src = "def add(a: Int64, b: Int64) -> Int64\n  a + b\nend";
    let program = parse(src).expect("should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected a Function item, got {:?}", program.items[0]);
    };
    let Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::Add(lhs, rhs),
        span: add_span,
      }),
      ..
    } = &f.body[0]
    else {
      panic!("expected `a + b` as add's only statement, got {:?}", f.body);
    };
    // The whole `a + b` expression spans "a + b" — 5 real bytes, a
    // genuinely multi-character span.
    assert!(
      add_span.1 > add_span.0,
      "multi-character span must be non-degenerate: {add_span:?}"
    );
    assert_eq!(
      *add_span,
      (src.find("a + b").unwrap(), src.find("a + b").unwrap() + 5)
    );
    // `a` and `b` are each a single-character token: span.1 ==
    // span.0 + 1, never wider and never degenerate (span.1 == span.0).
    assert_eq!(lhs.span.1, lhs.span.0 + 1);
    assert_eq!(rhs.span.1, rhs.span.0 + 1);
  }

  #[test]
  fn parses_to_an_ast_structurally_equal_before_and_after_this_plan_modulo_spans() {
    // Regression (AC3): every prior worked example in this file still
    // parses; this one specifically proves a real, non-`FUNC_ONLY`
    // program's `.node` shape survives completely unchanged — real
    // spans differ per-node (never asserted equal to each other here),
    // but `Spanned<T>`'s own `PartialEq` (node-only, see ast.rs) is
    // exactly what makes this comparison possible without hand-deriving
    // every nested byte offset.
    let program = parse(HELLO_EM).expect("hello.em should parse");
    let Item::Function(f) = &program.items[0] else {
      panic!("expected the add function");
    };
    assert_eq!(
      f.body,
      vec![s(Stmt::Expr(s(Expr::Add(
        Box::new(s(Expr::Ident("a".into()))),
        Box::new(s(Expr::Ident("b".into())))
      ))))]
    );
  }

  // Plan 52 (algebraic data types and exhaustive pattern matching).

  const SHAPE_ENUM_SRC: &str =
    "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n";

  #[test]
  fn enum_declaration_parses_into_item_enum() {
    let program = parse(SHAPE_ENUM_SRC).expect("should parse");
    let Item::Enum(def) = &program.items[0] else {
      panic!("expected an Item::Enum, got {:?}", program.items[0]);
    };
    assert_eq!(def.name, "Shape");
    assert_eq!(
      def.variants,
      vec![
        EnumVariant {
          name: "Circle".into(),
          fields: vec!["Float64".into()],
        },
        EnumVariant {
          name: "Square".into(),
          fields: vec!["Float64".into()],
        },
        EnumVariant {
          name: "Rectangle".into(),
          fields: vec!["Float64".into(), "Float64".into()],
        },
      ]
    );
  }

  #[test]
  fn case_over_variant_patterns_parses_arms_with_bindings_in_source_order() {
    let src = "case circle\nwhen Circle(r)\n  puts r\nwhen Square(s)\n  puts s\nwhen Rectangle(w, h)\n  puts w\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Case { arms, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a case statement, got {:?}", program.items[0]);
    };
    assert_eq!(arms.len(), 3);
    assert_eq!(
      arms[0].0,
      CasePattern::Variant {
        name: "Circle".into(),
        bindings: vec!["r".into()],
      }
    );
    assert_eq!(
      arms[1].0,
      CasePattern::Variant {
        name: "Square".into(),
        bindings: vec!["s".into()],
      }
    );
    assert_eq!(
      arms[2].0,
      CasePattern::Variant {
        name: "Rectangle".into(),
        bindings: vec!["w".into(), "h".into()],
      }
    );
  }

  #[test]
  fn variant_construction_parses_as_an_ordinary_call() {
    // Decision log: no new grammar production for construction — the
    // parser can't tell "call a function" from "construct a variant"
    // apart at all; that's entirely sema's job. `Circle(2.0)` parses
    // exactly like any other bare function call.
    let program = parse("Circle(2.0)\n").expect("should parse");
    assert_eq!(
      program.items[0],
      Item::Stmt(s(Stmt::Expr(s(Expr::Call(
        "Circle".to_string(),
        vec![s(Expr::Float(2.0))]
      )))))
    );
  }

  #[test]
  fn plan_52_worked_example_parses_end_to_end() {
    let src = "enum Shape = Circle(Float64) | Square(Float64) | Rectangle(Float64, Float64)\n\ncircle: Shape = Circle(2.0)\n\ncase circle\nwhen Circle(r)\n  puts r\nwhen Square(s)\n  puts s\nwhen Rectangle(w, h)\n  puts w\nend\n";
    let program = parse(src).expect("worked example should parse");
    assert!(matches!(program.items[0], Item::Enum(_)));
    assert!(matches!(program.items[1], Item::Stmt(_)));
    assert!(matches!(program.items[2], Item::Stmt(_)));
  }

  // Plan 53 (Result type and error propagation).

  #[test]
  fn ok_and_err_construction_parse_to_their_own_expr_variants() {
    let program = parse("x: Result[Int64, String] = Ok(42)\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { ty, value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(ty, "Result[Int64, String]");
    assert_eq!(value.node, Expr::Ok(Box::new(s(Expr::Int(42)))));

    let program = parse("y: Result[Int64, String] = Err(\"bad\")\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      value.node,
      Expr::Err(Box::new(s(Expr::StringLit("bad".to_string()))))
    );
  }

  #[test]
  fn postfix_try_parses_to_expr_try() {
    let program = parse("n: Int64 = parse_int(s)?\n").expect("should parse");
    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[0]
    else {
      panic!("expected a Let statement");
    };
    assert_eq!(
      value.node,
      Expr::Try(Box::new(s(Expr::Call(
        "parse_int".to_string(),
        vec![s(Expr::Ident("s".to_string()))]
      ))))
    );
  }

  #[test]
  fn ok_err_match_form_parses_to_stmt_match_result() {
    let src = "case result\nwhen Ok(v)\n  puts v\nwhen Err(e)\n  puts e\nend\n";
    let program = parse(src).expect("should parse");
    let Item::Stmt(Spanned {
      node:
        Stmt::MatchResult {
          ok_var,
          ok_body,
          err_var,
          err_body,
          ..
        },
      ..
    }) = &program.items[0]
    else {
      panic!(
        "expected a MatchResult statement, got {:?}",
        program.items[0]
      );
    };
    assert_eq!(ok_var, "v");
    assert_eq!(err_var, "e");
    assert_eq!(ok_body.len(), 1);
    assert_eq!(err_body.len(), 1);
  }

  // Plan 54 (actor declarations and isolated heaps).

  // Plan 55's Decision log tightened this worked example (originally
  // authored under plan 54, before that rule existed): an actor
  // method other than `initialize` may no longer declare a return
  // type — `value` now prints `@count` itself (`puts @count`) rather
  // than returning it for a top-level `puts a.value` to print.
  const COUNTER_ACTOR_EXAMPLE: &str = "actor Counter\n  count: Int64\n\n  def initialize(start: Int64) -> Void\n    @count = start\n  end\n\n  def increment -> Void\n    @count = @count + 1\n  end\n\n  def value -> Void\n    puts @count\n  end\nend\n\na: Counter = Counter.spawn(0)\nb: Counter = Counter.spawn(100)\n\na.increment\na.increment\nb.increment\n\na.value\nb.value\n";

  #[test]
  fn actor_worked_example_parses_into_expected_shapes() {
    let program = parse(COUNTER_ACTOR_EXAMPLE).expect("worked example should parse");
    let Item::Actor(a) = &program.items[0] else {
      panic!("expected Item::Actor, got {:?}", program.items[0]);
    };
    assert_eq!(a.name, "Counter");
    assert_eq!(a.fields.len(), 1);
    assert_eq!(a.fields[0].name, "count");
    assert_eq!(a.methods.len(), 3);

    let Item::Stmt(Spanned {
      node: Stmt::Let { value, .. },
      ..
    }) = &program.items[1]
    else {
      panic!("expected a Let statement, got {:?}", program.items[1]);
    };
    assert_eq!(
      value.node,
      Expr::Spawn("Counter".to_string(), vec![s(Expr::Int(0))])
    );

    // `a.increment`/`a.value` parse as the *existing* Expr::MethodCall/
    // Stmt::Expr shapes — no new AST node for the call sites themselves.
    let Item::Stmt(Spanned {
      node: Stmt::Expr(Spanned {
        node: Expr::MethodCall(..),
        ..
      }),
      ..
    }) = &program.items[3]
    else {
      panic!(
        "expected a MethodCall statement, got {:?}",
        program.items[3]
      );
    };
  }

  #[test]
  fn actor_inheritance_is_a_real_parse_error() {
    let src = "actor Dog < Animal\nend\n";
    assert!(
      parse(src).is_err(),
      "actor grammar has no superclass clause at all"
    );
  }

  #[test]
  fn a_class_spawn_called_and_an_actor_new_called_both_parse_at_this_leaf() {
    // Grammar-only leaf: rejecting these two shapes is leaf-sema-actor's
    // job, not this one's — both must parse successfully here.
    let src1 = "class Foo\nend\n\nf: Foo = Foo.spawn()\n";
    assert!(parse(src1).is_ok());
    let src2 = "actor Bar\nend\n\nb: Bar = Bar.new()\n";
    assert!(parse(src2).is_ok());
  }

  #[test]
  fn plan_53_worked_example_parses_end_to_end() {
    let src = "def parse_int(s: String) -> Result[Int64, String]\n  if is_valid_int(s)\n    return Ok(parse_digits(s))\n  end\n  return Err(\"not a number\")\nend\n\ndef try_parse(s: String) -> Result[Int64, String]\n  n: Int64 = parse_int(s)?\n  return Ok(n * 2)\nend\n\nresult: Result[Int64, String] = try_parse(\"21\")\ncase result\nwhen Ok(v)\n  puts v\nwhen Err(e)\n  puts e\nend\n";
    let program = parse(src).expect("worked example should parse");
    assert!(matches!(program.items[0], Item::Function(_)));
    assert!(matches!(program.items[1], Item::Function(_)));
    assert!(matches!(program.items[2], Item::Stmt(_)));
    assert!(matches!(
      program.items[3],
      Item::Stmt(Spanned {
        node: Stmt::MatchResult { .. },
        ..
      })
    ));
  }
}
