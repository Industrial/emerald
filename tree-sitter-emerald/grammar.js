// Emerald tree-sitter grammar (plan 24: `leaf-grammar-js`).
//
// Grounded directly in `crates/emerald-parser/src/grammar.lalrpop`'s real
// terminals/productions, re-verified this session — NOT mechanically
// converted from it (no such tool exists; see the plan's Decision log).
// Where LALRPOP's own grammar needed an elaborate `Stmt`/`StmtExpr` split
// purely to dodge LALR(1) shift/reduce conflicts (bare array/hash
// literals at statement-initial position, a trailing block on a call),
// this grammar doesn't need the same split: tree-sitter's GLR parser
// disambiguates those cases from ordinary lookahead/precedence without
// it, so the node shapes here follow the language's real structure
// rather than LALRPOP's own implementation-driven restrictions.

function sep1(rule, separator) {
	return seq(rule, repeat(seq(separator, rule)));
}

module.exports = grammar({
	name: "emerald",

	extras: ($) => [/\s/, $.comment],

	word: ($) => $.identifier,

	conflicts: ($) => [
		[$.call_expression],
		[$.method_call_expression],
		// `for x in [...]` (dedicated bracketed list) vs. `for x in
		// [...] .. y` (a `for_range_statement` whose `start` expression
		// happens to itself be an array literal) share the identical
		// `'[' Args ']'` prefix — genuinely disambiguated only by the
		// token *after* `]` (`end`/a fresh statement vs. `..`/`...`),
		// same as `grammar.lalrpop`'s own LALR(1) table resolves this
		// via lookahead rather than a grammar restriction.
		[$.for_statement, $.array_literal],
	],

	rules: {
		source_file: ($) => repeat($._item),

		comment: (_) => token(seq("#", /[^\n]*/)),

		_item: ($) =>
			choice(
				$.function_definition,
				$.class_definition,
				$.module_definition,
				$.interface_definition,
				$.require_statement,
				$._statement,
			),

		require_statement: ($) => seq("require", $.require_path),
		require_path: ($) => sep1($.identifier, "/"),

		// ---------------------------------------------------------------
		// Declarations
		// ---------------------------------------------------------------

		class_definition: ($) =>
			seq(
				"class",
				field("name", $.identifier),
				optional($.superclass_clause),
				optional($.implements_clause),
				repeat($.field_declaration),
				repeat($.method_definition),
				"end",
			),

		superclass_clause: ($) => seq("<", field("superclass", $.identifier)),
		implements_clause: ($) =>
			seq("implements", field("interface", $.identifier)),

		field_declaration: ($) =>
			seq(
				optional("read"),
				field("name", $.identifier),
				":",
				field("type", $._type),
			),

		module_definition: ($) =>
			seq(
				"module",
				field("name", $.identifier),
				repeat($.method_definition),
				"end",
			),

		interface_definition: ($) =>
			seq(
				"interface",
				field("name", $.identifier),
				"fn",
				field("method_name", $.identifier),
				"(",
				optional($.parameter_list),
				")",
				":",
				field("return_type", $._type),
				"end",
			),

		function_definition: ($) =>
			seq(
				"fn",
				field("name", $.identifier),
				optional($.type_param_clause),
				optional($.param_clause),
				":",
				field("return_type", $._type),
				"do",
				field("body", repeat($._statement)),
				"end",
			),

		method_definition: ($) =>
			seq(
				"fn",
				field("name", $._method_name),
				optional($.type_param_clause),
				optional($.param_clause),
				":",
				field("return_type", $._type),
				"do",
				field("body", repeat($._statement)),
				"end",
			),

		_method_name: ($) =>
			choice(
				$.identifier,
				"+",
				"-",
				"*",
				"/",
				"==",
				"<=>",
				seq("[", "]"),
				seq("[", "]", "="),
			),

		type_param_clause: ($) => seq("[", sep1($.type_param, ","), "]"),
		type_param: ($) =>
			seq(field("name", $.identifier), ":", field("bound", $.identifier)),

		param_clause: ($) =>
			seq(
				"(",
				optional(
					choice(
						seq(
							sep1($._param, ","),
							optional(seq(",", "&", field("block_param", $.identifier))),
						),
						seq("&", field("block_param", $.identifier)),
					),
				),
				")",
			),

		parameter_list: ($) => sep1($.parameter, ","),

		_param: ($) => choice($.parameter, $.splat_parameter),

		parameter: ($) =>
			seq(
				field("name", $.identifier),
				":",
				field("type", $._type),
				optional(seq("=", field("default", $._default_literal))),
			),

		splat_parameter: ($) =>
			seq("*", field("name", $.identifier), ":", field("type", $._type)),

		_default_literal: ($) =>
			choice($.integer, $.float, $.string, "true", "false", "nil"),

		// ---------------------------------------------------------------
		// Types
		// ---------------------------------------------------------------

		_type: ($) =>
			choice($.identifier, $.array_type, $.hash_type, "Proc", $.nullable_type),

		array_type: ($) => seq("Array", "[", field("element", $.identifier), "]"),
		hash_type: ($) =>
			seq(
				"Hash",
				"[",
				field("key", $.identifier),
				",",
				field("value", $.identifier),
				"]",
			),
		nullable_type: ($) => prec.left(seq(field("base", $._type), "?")),

		// ---------------------------------------------------------------
		// Statements
		// ---------------------------------------------------------------

		_statement: ($) =>
			choice(
				$.let_statement,
				$.if_statement,
				$.unless_statement,
				$.while_statement,
				$.until_statement,
				$.for_statement,
				$.for_range_statement,
				$.return_statement,
				$.break_statement,
				$.next_statement,
				$.puts_statement,
				$.raise_statement,
				$.yield_statement,
				$.begin_statement,
				$.retry_statement,
				$.match_statement,
				$.match_result_statement,
				$.multiple_assignment_statement,
				$.compound_assignment_statement,
				$.or_assign_statement,
				$.and_assign_statement,
				$.assignment_statement,
				$.expression_statement,
			),

		let_statement: ($) =>
			seq(
				field("name", $.identifier),
				":",
				field("type", $._type),
				"=",
				field("value", $._expression),
			),

		if_statement: ($) =>
			seq(
				"if",
				field("condition", $._expression),
				"do",
				field("consequence", repeat($._statement)),
				optional($._else_clause),
				"end",
			),

		unless_statement: ($) =>
			seq(
				"unless",
				field("condition", $._expression),
				"do",
				field("consequence", repeat($._statement)),
				optional($._else_clause),
				"end",
			),

		while_statement: ($) =>
			seq(
				"while",
				field("condition", $._expression),
				"do",
				field("body", repeat($._statement)),
				"end",
			),

		until_statement: ($) =>
			seq(
				"until",
				field("condition", $._expression),
				"do",
				field("body", repeat($._statement)),
				"end",
			),

		_else_clause: ($) => choice($.else_clause, $.elsif_clause),

		else_clause: ($) => seq("else", field("body", repeat($._statement))),

		elsif_clause: ($) =>
			seq(
				"elsif",
				field("condition", $._expression),
				"do",
				field("consequence", repeat($._statement)),
				optional($._else_clause),
			),

		// `elements` is a dedicated bracketed list, NOT `$.array_literal` —
		// sharing that node here would make it ambiguous whether a
		// trailing binary operator (`-`, `+`, ...) continues the literal
		// as a larger expression or ends the `for` clause; a separate,
		// unreachable-from-`_expression` production sidesteps the
		// conflict entirely, mirroring `grammar.lalrpop`'s own
		// `"[" <elements:Args> "]"` restriction here (never a general
		// `Expr` scrutinee).
		for_statement: ($) =>
			seq(
				"for",
				field("variable", $.identifier),
				"in",
				"[",
				field("elements", optional($.argument_list)),
				"]",
				field("body", repeat($._statement)),
				"end",
			),

		for_range_statement: ($) =>
			seq(
				"for",
				field("variable", $.identifier),
				"in",
				field("start", $._expression),
				field("operator", choice("..", "...")),
				field("end", $._expression),
				field("body", repeat($._statement)),
				"end",
			),

		return_statement: ($) => seq("return", field("value", $._expression)),
		break_statement: (_) => "break",
		next_statement: (_) => "next",
		puts_statement: ($) => seq("puts", field("argument", $._expression)),
		raise_statement: ($) => seq("raise", field("value", $._expression)),

		yield_statement: ($) => seq("yield", field("arguments", $._yield_args)),
		_yield_args: ($) => sep1($._expression, ","),

		begin_statement: ($) =>
			seq(
				"begin",
				field("body", repeat($._statement)),
				repeat1($.rescue_clause),
				optional($.ensure_clause),
				"end",
			),

		rescue_clause: ($) =>
			seq(
				"rescue",
				optional(field("class", $.identifier)),
				"=>",
				field("variable", $.identifier),
				field("body", repeat($._statement)),
			),

		ensure_clause: ($) => seq("ensure", field("body", repeat($._statement))),
		retry_statement: (_) => "retry",

		// Plan 71: `case`/`when` becomes `match`/`do` — the scrutinee is
		// followed by a mandatory "do", each arm supplies its own trailing
		// "do ... end" instead of running until the next "when"/"else"/
		// "end", and the default arm spells `_ do ... end` (match_wildcard)
		// rather than reusing `_else_clause` (still owned exclusively by
		// if/unless/elsif now). A separate, textually-disjoint
		// match_result_statement covers the dedicated `Ok(...)`/`Err(...)`
		// destructuring form — disambiguated from an ordinary match by the
		// very next token after "do" ("Ok" is a reserved-keyword terminal,
		// categorically distinct from the integer/identifier that head
		// every other match_arm), mirroring grammar.lalrpop's own
		// disambiguation exactly.
		match_statement: ($) =>
			seq(
				"match",
				field("scrutinee", $._expression),
				"do",
				repeat1($._match_arm),
				optional($.match_wildcard),
				"end",
			),

		_match_arm: ($) => choice($.match_values_arm, $.match_variant_arm),

		match_values_arm: ($) =>
			seq(
				field("values", $._case_values),
				"do",
				field("body", repeat($._statement)),
				"end",
			),
		_case_values: ($) => sep1($.integer, ","),

		match_variant_arm: ($) =>
			seq(
				field("name", $.identifier),
				"(",
				optional(field("bindings", $.binding_list)),
				")",
				"do",
				field("body", repeat($._statement)),
				"end",
			),
		binding_list: ($) => sep1($.identifier, ","),

		match_wildcard: ($) =>
			seq("_", "do", field("body", repeat($._statement)), "end"),

		match_result_statement: ($) =>
			seq(
				"match",
				field("scrutinee", $._expression),
				"do",
				"Ok",
				"(",
				field("ok_variable", $.identifier),
				")",
				"do",
				field("ok_body", repeat($._statement)),
				"end",
				"Err",
				"(",
				field("err_variable", $.identifier),
				")",
				"do",
				field("err_body", repeat($._statement)),
				"end",
				"end",
			),

		multiple_assignment_statement: ($) =>
			seq(
				field("names", $._multi_names),
				"=",
				field("values", $._multi_values),
			),
		_multi_names: ($) => seq($.identifier, ",", sep1($.identifier, ",")),
		_multi_values: ($) => seq($._expression, ",", sep1($._expression, ",")),

		compound_assignment_statement: ($) =>
			seq(
				field("target", $.identifier),
				field("operator", choice("+=", "-=", "*=", "/=", "%=")),
				field("value", $._expression),
			),

		or_assign_statement: ($) =>
			seq(
				field("target", $.identifier),
				"||=",
				field("default", $._expression),
			),
		and_assign_statement: ($) =>
			seq(field("target", $.identifier), "&&=", field("value", $._expression)),

		assignment_statement: ($) =>
			seq(
				field("target", $._assignment_target),
				"=",
				field("value", $._expression),
			),
		_assignment_target: ($) =>
			choice($.instance_variable, $.index_expression, $.identifier),

		expression_statement: ($) => $._expression,

		// ---------------------------------------------------------------
		// Expressions
		// ---------------------------------------------------------------

		_expression: ($) =>
			choice($.binary_expression, $.unary_expression, $._primary_expression),

		binary_expression: ($) =>
			choice(
				prec.left(
					1,
					seq(
						field("left", $._expression),
						field("operator", "||"),
						field("right", $._expression),
					),
				),
				prec.left(
					2,
					seq(
						field("left", $._expression),
						field("operator", "&&"),
						field("right", $._expression),
					),
				),
				prec.left(
					3,
					seq(
						field("left", $._expression),
						field("operator", choice("==", "!=", "<", ">", "<=", ">=")),
						field("right", $._expression),
					),
				),
				prec.left(
					4,
					seq(
						field("left", $._expression),
						field("operator", choice("|", "^")),
						field("right", $._expression),
					),
				),
				prec.left(
					5,
					seq(
						field("left", $._expression),
						field("operator", "&"),
						field("right", $._expression),
					),
				),
				prec.left(
					6,
					seq(
						field("left", $._expression),
						field("operator", choice("<<", ">>")),
						field("right", $._expression),
					),
				),
				prec.left(
					7,
					seq(
						field("left", $._expression),
						field("operator", choice("+", "-")),
						field("right", $._expression),
					),
				),
				prec.left(
					8,
					seq(
						field("left", $._expression),
						field("operator", choice("*", "/", "%")),
						field("right", $._expression),
					),
				),
			),

		unary_expression: ($) =>
			prec(
				9,
				seq(
					field("operator", choice("-", "!", "~")),
					field("operand", $._expression),
				),
			),

		_primary_expression: ($) =>
			choice(
				$.call_expression,
				$.call_kw_expression,
				$.new_expression,
				$.array_new_expression,
				$.method_call_expression,
				$.safe_call_expression,
				$.lambda_expression,
				$.index_expression,
				$.array_literal,
				$.hash_literal,
				$.integer,
				$.float,
				$.string,
				$.symbol,
				"true",
				"false",
				"nil",
				$.instance_variable,
				$.identifier,
			),

		index_expression: ($) =>
			prec.left(
				11,
				seq(
					field("array", $._expression),
					"[",
					field("index", $._expression),
					"]",
				),
			),

		call_expression: ($) =>
			prec(
				10,
				seq(
					field("function", $.identifier),
					"(",
					optional($.argument_list),
					")",
					optional(field("block", $.block)),
				),
			),

		call_kw_expression: ($) =>
			prec(
				10,
				seq(field("function", $.identifier), "(", $.keyword_argument_list, ")"),
			),

		new_expression: ($) =>
			prec(
				10,
				seq(
					field("class", $.identifier),
					".",
					"new",
					"(",
					optional($.argument_list),
					")",
				),
			),

		array_new_expression: ($) =>
			prec(
				10,
				seq("Array", ".", "new", "(", field("size", $._expression), ")"),
			),

		method_call_expression: ($) =>
			prec(
				10,
				seq(
					field("receiver", $.identifier),
					".",
					field("method", $._call_method_name),
					optional(
						seq(
							"(",
							optional($.argument_list),
							")",
							optional(field("block", $.block)),
						),
					),
				),
			),

		safe_call_expression: ($) =>
			prec(
				10,
				seq(
					field("receiver", $.identifier),
					"&.",
					field("method", $._call_method_name),
					optional(seq("(", optional($.argument_list), ")")),
				),
			),

		_call_method_name: ($) => choice($.identifier, "read"),

		// Plan 71: plan 10's `->(params) -> ReturnType { body }` lambda
		// literal is deleted outright, not arrow-shortened — Sable has no
		// separate lambda syntax at all. A function value is a bare
		// `do |params: T| ... end` block used directly as an ordinary
		// expression, typed entirely by its surrounding context (e.g. a
		// `Proc` annotation on the binding it's assigned to) — never by its
		// own literal syntax, which is why this production carries no
		// return-type field of its own any more. Reachable from the general
		// `_expression`/`_primary_expression` chain (not split into a
		// separate Stmt-initial-exclusion the way grammar.lalrpop's own
		// LALR(1) table needs) — this file's header comment explains why
		// tree-sitter's GLR parser doesn't need that split.
		lambda_expression: ($) =>
			prec(
				10,
				seq(
					"do",
					"|",
					optional($.parameter_list),
					"|",
					field("body", repeat($._statement)),
					"end",
				),
			),

		block: ($) =>
			seq("{", "|", optional($.parameter_list), "|", repeat($._statement), "}"),

		array_literal: ($) => seq("[", optional($.argument_list), "]"),
		argument_list: ($) => sep1($._expression, ","),
		keyword_argument_list: ($) =>
			sep1(
				seq(field("name", $.identifier), ":", field("value", $._expression)),
				",",
			),

		hash_literal: ($) => seq("{", optional($._hash_pairs), "}"),
		_hash_pairs: ($) => sep1($.hash_pair, ","),
		hash_pair: ($) =>
			seq(field("key", $._expression), "=>", field("value", $._expression)),

		// ---------------------------------------------------------------
		// Tokens
		// ---------------------------------------------------------------

		integer: (_) => /[0-9]+/,
		float: (_) => /[0-9]+\.[0-9]+/,
		string: (_) => /"([^"\\]|\\[n"])*"/,
		symbol: (_) => /:[A-Za-z_][A-Za-z0-9_]*/,
		identifier: (_) => /[A-Za-z_][A-Za-z0-9_]*/,
		instance_variable: (_) => /@[A-Za-z_][A-Za-z0-9_]*/,
	},
});
