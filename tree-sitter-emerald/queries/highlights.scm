; Emerald tree-sitter highlight query (plan 24: `leaf-highlight-queries`).
;
; Standard capture-name vocabulary both Zed's and `nvim-treesitter`'s
; default themes already recognize — no project-specific capture names
; invented (see the plan's Decision log). No `locals.scm`, no
; injections — basic highlighting only, the same scope cut plan 17's
; LSP made for its own diagnostics-only leaf.

; ---------------------------------------------------------------------
; Keywords
; ---------------------------------------------------------------------

[
  "class"
  "module"
  "interface"
  "def"
  "end"
  "read"
  "implements"
  "require"
] @keyword

[
  "if"
  "elsif"
  "else"
  "unless"
  "while"
  "until"
  "for"
  "in"
  "case"
  "when"
] @keyword.control.conditional

(break_statement) @keyword.control.return
(next_statement) @keyword.control.return
["return" "yield"] @keyword.control.return

(retry_statement) @keyword.control.exception
["begin" "rescue" "ensure" "raise"] @keyword.control.exception

["puts" "new"] @keyword.function

; ---------------------------------------------------------------------
; Literals
; ---------------------------------------------------------------------

(integer) @number
(float) @number
(string) @string
(symbol) @string.special.symbol
["true" "false"] @boolean
"nil" @constant.builtin

(comment) @comment

; ---------------------------------------------------------------------
; Types
; ---------------------------------------------------------------------

["Array" "Hash" "Proc"] @type.builtin

(array_type element: (identifier) @type)
(hash_type key: (identifier) @type)
(hash_type value: (identifier) @type)
(field_declaration type: (identifier) @type)
(parameter type: (identifier) @type)
(splat_parameter type: (identifier) @type)
(function_definition return_type: (identifier) @type)
(method_definition return_type: (identifier) @type)
(interface_definition return_type: (identifier) @type)
(lambda_expression return_type: (identifier) @type)
(let_statement type: (identifier) @type)
(nullable_type base: (identifier) @type)
(superclass_clause superclass: (identifier) @type)
(implements_clause interface: (identifier) @type)
(rescue_clause class: (identifier) @type)
(type_param bound: (identifier) @type)

; ---------------------------------------------------------------------
; Functions, methods, classes
; ---------------------------------------------------------------------

(function_definition name: (identifier) @function)
(method_definition name: (identifier) @function.method)
(class_definition name: (identifier) @type)
(module_definition name: (identifier) @type)
(interface_definition name: (identifier) @type)

(call_expression function: (identifier) @function.call)
(call_kw_expression function: (identifier) @function.call)
(method_call_expression method: (identifier) @function.method.call)
(safe_call_expression method: (identifier) @function.method.call)
(new_expression class: (identifier) @type)

; ---------------------------------------------------------------------
; Variables, parameters, properties
; ---------------------------------------------------------------------

(instance_variable) @property

(parameter name: (identifier) @variable.parameter)
(splat_parameter name: (identifier) @variable.parameter)
(type_param name: (identifier) @variable.parameter)

(let_statement name: (identifier) @variable)
(assignment_statement target: (identifier) @variable)
(compound_assignment_statement target: (identifier) @variable)
(or_assign_statement target: (identifier) @variable)
(and_assign_statement target: (identifier) @variable)
(for_statement variable: (identifier) @variable)
(for_range_statement variable: (identifier) @variable)
(rescue_clause variable: (identifier) @variable)
(field_declaration name: (identifier) @property)
(hash_pair key: (identifier) @variable)

(method_call_expression receiver: (identifier) @variable)
(safe_call_expression receiver: (identifier) @variable)

(identifier) @variable

; ---------------------------------------------------------------------
; Operators and punctuation
; ---------------------------------------------------------------------

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "<=>"
  "&&"
  "||"
  "!"
  "&"
  "|"
  "^"
  "~"
  "<<"
  ">>"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "||="
  "&&="
  "->"
  "=>"
  "&."
  ".."
  "..."
] @operator

["." "," ":" "?"] @punctuation.delimiter

["(" ")"] @punctuation.bracket
["[" "]"] @punctuation.bracket
["{" "}"] @punctuation.bracket
