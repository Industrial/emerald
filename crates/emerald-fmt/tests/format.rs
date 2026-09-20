//! Regression tests for `emerald-fmt`'s canonical pretty-printer.
//!
//! Covers exactly the three things plan 78's own task brief calls for:
//! (1) formatting a real `examples/*.em` file reparses to a
//!     structurally-equivalent AST (span-blind, via `Spanned<T>`'s own
//!     `PartialEq` — the same idiom `emerald_parser`'s own
//!     `trailing_and_leading_comments_dont_change_the_ast` test uses);
//! (2) formatting is idempotent (format(format(src)) == format(src));
//! (3) a deliberately messily-formatted snippet comes out canonically
//!     formatted (an exact expected-string assertion, not just "it
//!     parses").

fn parse(src: &str, name: &str) -> emerald_parser::Program {
  emerald_parser::parse_named(src, name)
    .unwrap_or_else(|errs| panic!("{name} failed to parse: {errs:?}"))
}

fn format_ok(src: &str, name: &str) -> String {
  emerald_fmt::format_source(src, name)
    .unwrap_or_else(|errs| panic!("{name} failed to format: {errs:?}"))
}

/// Every real example this repository ships, reformatted, must still
/// parse, and must reparse to the identical AST (modulo whitespace/
/// comments/spans) the original source parsed to.
#[test]
fn every_example_file_formats_and_reparses_to_an_equivalent_ast() {
  let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
  let files = emerald_fmt::collect_em_files(&examples_dir).expect("collect examples/*.em");
  assert!(
    !files.is_empty(),
    "expected to find real .em files under {}",
    examples_dir.display()
  );

  for file in files {
    let name = file.display().to_string();
    let source = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {name}: {e}"));
    let original_ast = parse(&source, &name);

    let formatted = format_ok(&source, &name);
    let reparsed_ast = parse(&formatted, &name);

    assert_eq!(
      original_ast, reparsed_ast,
      "{name}: formatting changed the parsed AST (modulo spans) — \
       formatted output was:\n{formatted}"
    );
  }
}

/// Formatting twice in a row must be a no-op: `format` is a pure
/// function of the parsed `Program`, and reformatting
/// already-canonical source reparses to the exact same `Program` it
/// started from, so it must re-print identically.
#[test]
fn every_example_file_formatting_is_idempotent() {
  let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
  let files = emerald_fmt::collect_em_files(&examples_dir).expect("collect examples/*.em");
  assert!(!files.is_empty());

  for file in files {
    let name = file.display().to_string();
    let source = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {name}: {e}"));

    let once = format_ok(&source, &name);
    let twice = format_ok(&once, &name);

    assert_eq!(
      once, twice,
      "{name}: formatting its own output changed it — not idempotent"
    );
  }
}

/// A single, deliberately messy snippet (inconsistent indentation, no
/// spacing around operators/colons/commas, `+=`/`unless`/`elsif`
/// sugar, a `read` field, extra blank lines) must come out exactly
/// canonically formatted — a real, exact-string assertion, not just
/// "it still parses".
#[test]
fn messy_snippet_formats_to_the_exact_canonical_form() {
  // NOTE: a `:` immediately followed by an identifier with NO space
  // (`x:Float64`) lexes as a `SymbolLitTok` (`:Float64`), not a type
  // annotation's `":"` — a real, documented lexer rule (`grammar.
  // lalrpop`'s own `SymbolLitTok` comment), not a formatter concern —
  // so every colon below keeps its mandatory following space even
  // while everything else about this snippet is deliberately messy.
  let messy = "\
class   Point
        read x: Float64
    read y: Float64


      fn initialize(x: Float64,y: Float64): Void do
   @x=x
        @y   =   y
end

  fn sum: Float64 do
@x+@y
   end
end



fn classify(n: Int64): Int64 do
if n==0 do
      return 0
    end
unless n>0 do
return 2
end
    return 1
end

total: Int64=0
i: Int64=0
while i<5 do
i+=1
end
puts total
";

  // `read x`/`read y` each expand to the plain field PLUS a
  // synthesized zero-arg accessor method (see `emerald_parser::
  // grammar.lalrpop`'s own `ClassField` production) — both accessors
  // land ahead of the hand-written `initialize`/`sum` methods, in
  // field-declaration order, exactly mirroring the AST shape
  // `class_fields:ClassField*`'s own left-to-right processing builds.
  let expected = "\
class Point
  x: Float64
  y: Float64

  fn x: Float64 do
    @x
  end

  fn y: Float64 do
    @y
  end

  fn initialize(x: Float64, y: Float64): Void do
    @x = x
    @y = y
  end

  fn sum: Float64 do
    @x + @y
  end
end

fn classify(n: Int64): Int64 do
  if n == 0 do
    return 0
  end

  unless n > 0 do
    return 2
  end

  return 1
end

total: Int64 = 0
i: Int64 = 0

while i < 5 do
  i += 1
end

puts total
";

  let formatted = format_ok(messy, "messy.em");
  assert_eq!(formatted, expected);

  // The exact-string assertion above is itself only meaningful if the
  // messy input and the canonical output really do parse to the same
  // AST — confirm that explicitly too.
  let original_ast = parse(messy, "messy.em");
  let reparsed_ast = parse(&formatted, "messy.em");
  assert_eq!(original_ast, reparsed_ast);

  // And confirm the exact-string canonical form is itself a fixed
  // point.
  let reformatted = format_ok(&formatted, "messy.em");
  assert_eq!(formatted, reformatted);
}

/// `read x: Type` field sugar expands to the plain field plus its
/// synthesized accessor — a targeted, direct check of that
/// canonicalization (already exercised indirectly above via
/// `examples/interfaces_generics.em`/`operator_overloading.em`, both
/// of which use `read`).
#[test]
fn read_field_sugar_expands_to_field_plus_accessor() {
  let src = "class Money\n  read cents: Int64\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\nend\n";
  let formatted = format_ok(src, "money.em");
  assert_eq!(
    formatted,
    "class Money\n  cents: Int64\n\n  fn cents: Int64 do\n    @cents\n  end\n\n  fn initialize(cents: Int64): Void do\n    @cents = cents\n  end\nend\n"
  );
}

/// `puts`, `assert`, and `assert_eq` each have exactly one legal
/// surface spelling this crate must reproduce precisely (see
/// `emerald_fmt`'s own doc comments on `write_stmt_expr`/`write_call`
/// for why printing them any other way would fail to reparse).
#[test]
fn puts_assert_and_assert_eq_round_trip_through_their_one_legal_spelling() {
  let src =
    "test \"arithmetic\" do\n  assert(1 + 1 == 2)\n  assert_eq(4, 2 + 2)\n  puts 1 + 1\nend\n";
  let formatted = format_ok(src, "t.em");
  assert_eq!(formatted, src);
}

/// Regression test for a real bug found while building this crate:
/// `total += a + b` desugars into a RIGHT-nested `Add(Ident(total),
/// Add(a, b))`, not the left-nested shape an ordinary `total = total +
/// a + b` parse would produce. Printing this by expanding `+=` (this
/// crate's first, wrong design) into `total = total + a + b` reparses
/// into the WRONG, left-nested tree — a real AST-equivalence failure,
/// not just a style regression. See `write_assign`'s own doc comment.
#[test]
fn compound_assign_with_a_looser_rhs_round_trips_to_the_same_ast() {
  let src = "total: Int64 = 0\na: Int64 = 1\nb: Int64 = 2\ntotal += a + b\n";
  let original_ast = parse(src, "compound.em");
  let formatted = format_ok(src, "compound.em");
  assert_eq!(
    formatted,
    "total: Int64 = 0\na: Int64 = 1\nb: Int64 = 2\ntotal += a + b\n"
  );
  let reparsed_ast = parse(&formatted, "compound.em");
  assert_eq!(original_ast, reparsed_ast);
}

/// Regression test for a real bug found while building this crate:
/// `unless a > b && c do ... end` desugars into `Expr::Not` wrapping a
/// full comparison/logical expression, not just a bare identifier.
/// Printing this by expanding `unless` (this crate's first, wrong
/// design) into `if !a > b && c do` reparses into a COMPLETELY
/// different meaning (`!` binds at the tightest precedence tier, so
/// that text means `(!a) > b && c`), since this grammar has no
/// parentheses at all to disambiguate. See `write_cond_head`'s own doc
/// comment.
#[test]
fn unless_with_a_looser_condition_round_trips_to_the_same_ast() {
  let src = "a: Boolean = true\nb: Int64 = 1\nc: Int64 = 2\nunless b > c && a do\n  puts 1\nend\n";
  let original_ast = parse(src, "unless.em");
  let formatted = format_ok(src, "unless.em");
  assert_eq!(
    formatted,
    "a: Boolean = true\nb: Int64 = 1\nc: Int64 = 2\n\nunless b > c && a do\n  puts 1\nend\n"
  );
  let reparsed_ast = parse(&formatted, "unless.em");
  assert_eq!(original_ast, reparsed_ast);
}
