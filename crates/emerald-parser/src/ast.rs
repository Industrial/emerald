/// Plan 22's Decision log: wraps every `Expr`/`Stmt` *position* with the
/// real byte-offset span LALRPOP's own `@L`/`@R` markers compute at
/// parse time — an outer wrapper around each node rather than a `span`
/// field bolted onto every enum variant, so `infer_expr_type`/
/// `check_stmt`/`build_expr`/`build_stmt` change their *signatures*
/// (taking `&Spanned<Expr>`/`&Spanned<Stmt>`) but not their match arms'
/// binding patterns. `Deref`/`DerefMut` to `T` keep `.node`-holding
/// field access ergonomic at the (many) call sites that only ever
/// wanted the node, never the span.
#[derive(Debug, Clone)]
pub struct Spanned<T> {
  pub span: (usize, usize),
  pub node: T,
}

/// Deliberately `node`-only, not `#[derive(PartialEq)]` — a real gap
/// found implementing this plan, not the Decision log's original
/// assumption: comments are skipped as lexer whitespace (`match {}`'s
/// `r"#[^\n]*" => {}`), so `@L`/`@R` positions still shift by exactly a
/// leading comment's length even though the parsed `Expr`/`Stmt` shape
/// is identical either way. `trailing_and_leading_comments_dont_change_
/// the_ast` (this crate's own existing regression test, predating this
/// plan) asserts exactly that: two parses of comment-differing source
/// produce an equal AST — comparing spans too would make that
/// assertion fail on the equality it's specifically testing for.
/// Comparing `node` only keeps `assert_eq!` a real structural check
/// (this AST shape vs. that one) without conflating it with a
/// byte-offset check no test in this codebase actually wants from
/// `==` — a real span is still verified directly via `.span` wherever
/// a test's whole point *is* the position (see `leaf-ast-spans`' own
/// concrete-proof test).
impl<T: PartialEq> PartialEq for Spanned<T> {
  fn eq(&self, other: &Self) -> bool {
    self.node == other.node
  }
}

impl<T> std::ops::Deref for Spanned<T> {
  type Target = T;
  fn deref(&self) -> &T {
    &self.node
  }
}

impl<T> std::ops::DerefMut for Spanned<T> {
  fn deref_mut(&mut self) -> &mut T {
    &mut self.node
  }
}

impl<T> Spanned<T> {
  /// A synthetic node with no real source position — used only where
  /// codegen/rewrite passes construct a brand-new `Expr`/`Stmt` that
  /// was never actually written in source (e.g. the assert/assert_eq
  /// desugaring pass's synthesized `if`/`raise`).
  pub fn synthetic(node: T) -> Self {
    Spanned { span: (0, 0), node }
  }
}

/// Lets a test compare a real, parsed `Spanned<T>` directly against a
/// bare `T` literal (`assert_eq!(*value, Expr::Int(10))`) without a
/// `.node` accessor at every flat (non-nested) comparison site — there
/// is no span on the bare-literal side to compare, so this compares
/// `node` only. `std`'s own blanket impls (`&A: PartialEq<&B>`,
/// `Vec<A>: PartialEq<Vec<B>>`, `Option<A>: PartialEq<Option<B>>`) then
/// make `&Spanned<T>`/`Vec<Spanned<T>>`/`Option<Spanned<T>>` comparable
/// to `&T`/`Vec<T>`/`Option<T>` the same way, for free.
impl<T: PartialEq> PartialEq<T> for Spanned<T> {
  fn eq(&self, other: &T) -> bool {
    self.node == *other
  }
}

/// Plan 88's Decision log: a real, recursive type-expression AST,
/// replacing the old `TypeName: String` grammar production's flat,
/// pre-formatted-string convention (`"Array[Int64]"`, `"Hash[K, V]"`)
/// — that convention could only ever represent one level of nesting
/// (the grammar's own `Array`/`Hash`/`Pair`/`Result` alternatives each
/// accepted a bare `Ident` per bracket slot, never another `TypeName`),
/// so `Proc[Array[Int64], Int64]` or a generic method returning
/// `Hash[K, Array[V]]` simply couldn't be written at all. Four cases
/// are enough for every type-annotation position this compiler has:
///
/// - `Named` — a plain class/interface/primitive/type-parameter name
///   (`Int64`, `MyClass`, `T`), and also a BARE `Proc`/`Array`/... with
///   no bracket clause (`Proc`'s own bare form still exists, so the
///   pre-existing lambda-literal-inference path — a `Let`'s declared
///   `Proc` type recovering its real signature from a co-located
///   `Expr::Lambda` — keeps working completely unchanged).
/// - `Generic` — a name plus a bracketed, comma-separated argument
///   list, each itself a full `TypeExpr` — subsumes `Array[T]`,
///   `Hash[K, V]`, `Pair[K, V]`, `Result[T, E]`, `Option[T]`, and any
///   user-declared generic class/enum instantiation (`Stack[Int64]`,
///   `Box[Box[Int64]]`) as one general case, rather than each needing
///   its own hand-rolled grammar alternative and string format.
/// - `Tuple` — `(T1, T2, ...)`, legal only as a function's declared
///   return type (`resolve_return_type`'s own restriction, unchanged).
/// - `Func` — `Proc`'s real, written, parameterized form (see
///   `TypeExpr`'s own grammar production for the chosen `Proc[Args...,
///   Ret]` spelling and why) — a parameter list plus a return type,
///   both full `TypeExpr`s, so `Proc[Array[Int64], Int64]` round-trips
///   through this AST exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeExpr {
  Named(String),
  Generic(String, Vec<TypeExpr>),
  Tuple(Vec<TypeExpr>),
  Func(Vec<TypeExpr>, Box<TypeExpr>),
}

impl TypeExpr {
  /// `Some(name)` only for a bare `Named` — used at the handful of call
  /// sites (`Self` substitution, `"Proc"`/type-parameter-name
  /// comparisons) that only ever cared about a plain identifier, never
  /// a compound shape.
  pub fn as_named(&self) -> Option<&str> {
    match self {
      TypeExpr::Named(n) => Some(n.as_str()),
      _ => None,
    }
  }
}

/// A real, disclosed test-ergonomics convenience: this whole crate's
/// pre-existing test suite (predating this plan) asserts a parsed
/// type's shape against a plain string literal (`assert_eq!(p.ty,
/// "Array[Int64]")`) — a reasonable idiom this plan doesn't want to
/// force hundreds of mechanical rewrites onto for zero behavioral
/// gain. Comparing via `Display`'s own canonical rendering (which
/// reproduces the exact old flat-string convention for every
/// pre-existing shape, and correctly extends it to genuinely nested
/// ones) keeps every such assertion meaningful and source-compatible.
/// Never used by any non-test production code path in this workspace
/// — `resolve_type` and friends match on `TypeExpr`'s real structure
/// directly, never through this string bridge.
#[allow(clippy::cmp_owned)]
impl PartialEq<str> for TypeExpr {
  fn eq(&self, other: &str) -> bool {
    self.to_string() == other
  }
}

#[allow(clippy::cmp_owned)]
impl PartialEq<&str> for TypeExpr {
  fn eq(&self, other: &&str) -> bool {
    self.to_string() == *other
  }
}

#[allow(clippy::cmp_owned)]
impl PartialEq<TypeExpr> for str {
  fn eq(&self, other: &TypeExpr) -> bool {
    other.to_string() == self
  }
}

#[allow(clippy::cmp_owned)]
impl PartialEq<String> for TypeExpr {
  fn eq(&self, other: &String) -> bool {
    &self.to_string() == other
  }
}

#[allow(clippy::cmp_owned)]
impl PartialEq<TypeExpr> for String {
  fn eq(&self, other: &TypeExpr) -> bool {
    other.to_string() == *self
  }
}

/// The construction-side half of the same test-ergonomics convenience
/// `PartialEq<str>` above documents — lets this crate's pre-existing
/// test fixtures keep building a `Param`/`Function` literal's plain
/// (non-compound) type via `"Int64".into()`/`"Int64".to_string().
/// into()` rather than `TypeExpr::Named("Int64".to_string())` at every
/// call site. Never used by any production parsing/resolution path —
/// the grammar always constructs a `TypeExpr` directly via its own
/// real variants, never through this conversion.
impl From<&str> for TypeExpr {
  fn from(name: &str) -> Self {
    TypeExpr::Named(name.to_string())
  }
}

impl From<String> for TypeExpr {
  fn from(name: String) -> Self {
    TypeExpr::Named(name)
  }
}

/// Renders a `TypeExpr` back into this compiler's existing flat,
/// human-readable type-name convention (`"Array[Int64]"`, `"Hash[K,
/// V]"`, `"(A, B)"`) — genuinely recursive, unlike the old grammar-level
/// string-concatenation it replaces, so a nested shape like `"Proc[
/// Array[Int64], Int64]"` now formats correctly too. Used only for
/// diagnostics and for the handful of legacy string-keyed registries
/// (mangled generic-instantiation names) that predate this plan and are
/// out of this plan's own scope to redesign — never as an intermediate
/// re-parsed representation (`resolve_type` consumes `TypeExpr`
/// directly and recursively, never this string).
impl std::fmt::Display for TypeExpr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      TypeExpr::Named(name) => write!(f, "{name}"),
      TypeExpr::Generic(name, args) => {
        write!(f, "{name}[")?;
        for (i, a) in args.iter().enumerate() {
          if i > 0 {
            write!(f, ", ")?;
          }
          write!(f, "{a}")?;
        }
        write!(f, "]")
      }
      TypeExpr::Tuple(parts) => {
        write!(f, "(")?;
        for (i, p) in parts.iter().enumerate() {
          if i > 0 {
            write!(f, ", ")?;
          }
          write!(f, "{p}")?;
        }
        write!(f, ")")
      }
      TypeExpr::Func(params, ret) => {
        write!(f, "Proc[")?;
        for p in params {
          write!(f, "{p}, ")?;
        }
        write!(f, "{ret}]")
      }
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
  pub name: String,
  pub ty: TypeExpr,
  /// Plan 39's Decision log: `Some(_)` only for a `def` function
  /// parameter declared with a trailing `= <literal>` — always `None`
  /// for a class field, a lambda parameter, or a block parameter (the
  /// shared `Param`/`Params` grammar productions those reuse can never
  /// produce `Some`; only the function-only `FuncParam` production
  /// can). Restricted to compile-time-evaluable literal tokens
  /// (`Int`/`Float`/`StringLit`/`Bool`/`Nil`) — never an arbitrary
  /// expression, and never a reference to another parameter.
  pub default: Option<Spanned<Expr>>,
}

/// Decodes a raw `"..."` token's `\"`/`\n` escapes into the literal's
/// real `String` value (plan 19's Decision log — only these two
/// escapes are recognized; `grammar.lalrpop`'s `StringLitTok` regex
/// admits no other backslash sequence, so an unrecognized escape fails
/// to lex as a string token at all — a real parse error, not a silent
/// pass-through). Strips the surrounding quotes. Lives here rather than
/// inline in `grammar.lalrpop` — LALRPOP's grammar-file format doesn't
/// support a bare top-level `fn` definition the way a plain Rust module
/// does.
pub fn decode_string_lit(raw: &str) -> String {
  let inner = &raw[1..raw.len() - 1];
  let mut out = String::with_capacity(inner.len());
  let mut chars = inner.chars();
  while let Some(c) = chars.next() {
    if c == '\\' {
      match chars.next() {
        Some('"') => out.push('"'),
        Some('n') => out.push('\n'),
        _ => unreachable!("the lexer's regex only admits \\\" and \\n escapes"),
      }
    } else {
      out.push(c);
    }
  }
  out
}

/// `class Point derive Comparable ... end` (plan 61's Decision log) —
/// runs once, between `require`-splicing and `emerald_sema::
/// check_program` (a real, new pipeline stage `emerald-cli`'s own
/// `main.rs` wires in — `check_program` takes `&Program`, not `&mut
/// Program`, so no sema-internal pass could inject a synthesized method
/// into the AST even if this ran later). A strict no-op for any
/// `ClassDef` with `derive: None` — every prior plan's example parses,
/// type-checks, and compiles identically either way.
///
/// Colocated with the AST it rewrites, the same place `decode_string_
/// lit` above already lives as an AST-adjacent free function, rather
/// than living in `lib.rs` alongside the LALRPOP-driven parse entry
/// points.
pub fn expand_derives(program: &mut Program) -> Result<(), String> {
  use std::collections::HashMap;

  let indices: Vec<usize> = program
    .items
    .iter()
    .enumerate()
    .filter_map(|(i, item)| match item {
      Item::Class(c) if c.derive.is_some() => Some(i),
      _ => None,
    })
    .collect();

  for i in indices {
    let (class_name, derive_name, superclass) = match &program.items[i] {
      Item::Class(c) => (
        c.name.clone(),
        c.derive.clone().unwrap(),
        c.superclass.clone(),
      ),
      _ => unreachable!("indices were collected from Item::Class matches above"),
    };
    if derive_name != "Comparable" {
      return Err(format!(
        "class `{class_name}` declares an unsupported derive target `{derive_name}` — only `Comparable` is supported"
      ));
    }
    let already_has_eq = match &program.items[i] {
      Item::Class(c) => c.methods.iter().any(|m| m.name == "=="),
      _ => unreachable!(),
    };
    if already_has_eq {
      return Err(format!(
        "class `{class_name}` already defines `==`; remove it or drop `derive Comparable`"
      ));
    }

    // Walk `superclass` across the already-`require`-merged `Program.
    // items`, collecting each ancestor's own `ClassDef`, nearest-parent
    // first — reversed below to root-ancestor-first, so a subclass's own
    // (possibly shadowing) field declaration is inserted into `fields`
    // last, the same "self wins over an inherited name" precedent
    // `emerald-sema`'s own `ClassInfo.fields` flattening already uses.
    let mut ancestors: Vec<ClassDef> = Vec::new();
    let mut cur = superclass;
    while let Some(name) = cur {
      let parent = program.items.iter().find_map(|item| match item {
        Item::Class(c) if c.name == name => Some(c.clone()),
        _ => None,
      });
      let Some(parent) = parent else {
        return Err(format!(
          "class `{class_name}` (transitively) extends unknown class `{name}`"
        ));
      };
      cur = parent.superclass.clone();
      ancestors.push(parent);
    }
    ancestors.reverse();

    let mut fields: HashMap<String, TypeExpr> = HashMap::new();
    for anc in &ancestors {
      for f in &anc.fields {
        fields.insert(f.name.clone(), f.ty.clone());
      }
    }
    if let Item::Class(c) = &program.items[i] {
      for f in &c.fields {
        fields.insert(f.name.clone(), f.ty.clone());
      }
    }

    // Determinism pitfall (Decision log): both `ClassInfo.fields` and
    // `ClassLayout.fields` are `HashMap`s with no guaranteed iteration
    // order; this flattened map inherits the same property. Sorting
    // alphabetically here keeps the synthesized `&&`-chain's own IR
    // deterministic across compiler runs.
    let mut field_names: Vec<String> = fields.keys().cloned().collect();
    field_names.sort();

    let has_accessor = |field: &str| -> bool {
      let self_has = match &program.items[i] {
        Item::Class(c) => c
          .methods
          .iter()
          .any(|m| m.name == field && m.params.is_empty()),
        _ => unreachable!(),
      };
      self_has
        || ancestors.iter().any(|a| {
          a.methods
            .iter()
            .any(|m| m.name == field && m.params.is_empty())
        })
    };
    let missing_accessors: Vec<(String, TypeExpr)> = field_names
      .iter()
      .filter(|f| !has_accessor(f))
      .map(|f| (f.clone(), fields[f].clone()))
      .collect();

    let cmp_chain = field_names.iter().fold(None, |acc, name| {
      let cmp = Spanned::synthetic(Expr::Compare(
        Box::new(Spanned::synthetic(Expr::InstanceVar(name.clone()))),
        CompareOp::Eq,
        Box::new(Spanned::synthetic(Expr::MethodCall(
          Box::new(Spanned::synthetic(Expr::Ident("other".to_string()))),
          name.clone(),
          vec![],
        ))),
      ));
      match acc {
        None => Some(cmp),
        Some(prev) => Some(Spanned::synthetic(Expr::And(Box::new(prev), Box::new(cmp)))),
      }
    });
    let body_expr = cmp_chain.unwrap_or_else(|| Spanned::synthetic(Expr::Bool(true)));
    let eq_fn = Function {
      name: "==".to_string(),
      params: vec![Param {
        name: "other".to_string(),
        ty: TypeExpr::Named(class_name.clone()),
        default: None,
      }],
      return_type: TypeExpr::Named("Boolean".to_string()),
      body: vec![Spanned::synthetic(Stmt::Return(Some(body_expr)))],
      block_param: None,
      splat_param: None,
      type_params: Vec::new(),
      is_comptime: false,
      requires: Vec::new(),
      ensures: Vec::new(),
      is_pure: false,
    };

    let Item::Class(c) = &mut program.items[i] else {
      unreachable!("indices were collected from Item::Class matches above")
    };
    for (fname, fty) in missing_accessors {
      c.methods.push(Function {
        name: fname.clone(),
        params: vec![],
        return_type: fty,
        body: vec![Spanned::synthetic(Stmt::Expr(Spanned::synthetic(
          Expr::InstanceVar(fname),
        )))],
        block_param: None,
        splat_param: None,
        type_params: Vec::new(),
        is_comptime: false,
        requires: Vec::new(),
        ensures: Vec::new(),
        is_pure: false,
      });
    }
    c.methods.push(eq_fn);
  }

  Ok(())
}

/// One piece of an interpolated string (plan 36's Decision log) — a
/// run of literal text, or a `#{...}` span's already-parsed `Expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
  Literal(String),
  Expr(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
  Lt,
  Gt,
  Le,
  Ge,
  Eq,
  Ne,
}

// NOTE: `Eq` (not just `PartialEq`) is intentionally not derived below —
// `Expr::Float(f64)` can't implement `Eq` (NaN != NaN violates Eq's
// reflexivity requirement), and that's transitive through every type that
// contains an `Expr`. Nothing in this workspace needs `Expr`/`Stmt`/etc. as
// a `HashMap`/`HashSet` key; `assert_eq!` in tests only needs `PartialEq`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
  Ident(String),
  Int(i64),
  Float(f64),
  /// `"..."` — plan 19's Decision log: a real, decoded `String` value
  /// (only `\"`/`\n` escapes are ever recognized by the lexer that
  /// produces this — see `grammar.lalrpop`'s `StringLitTok`).
  StringLit(String),
  /// `"...#{expr}..."` (plan 36's Decision log) — a *compile-time*
  /// parse, never a runtime `eval`: each `#{...}` span is re-parsed by
  /// the same static expression grammar that parses the rest of the
  /// program. Only ever constructed when the literal actually contains
  /// at least one `#{...}` span — a plain string with none still
  /// produces `Expr::StringLit`, unchanged, for zero codegen
  /// regression on the overwhelmingly common case.
  Interpolate(Vec<StringPart>),
  /// `:foo` (plan 44's Decision log) — carries the spelling with the
  /// leading `:` already stripped, mirroring `decode_string_lit`'s own
  /// "AST already holds the real value" convention. A genuinely
  /// separate type/runtime representation from `String`, not an alias —
  /// see `emerald-sema`'s `Type::Symbol` and `emerald-codegen`'s
  /// `ValKind::Symbol`.
  SymbolLit(String),
  Add(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Sub(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Mul(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Div(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Rem(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Neg(Box<Spanned<Expr>>),
  Not(Box<Spanned<Expr>>),
  And(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Or(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  Compare(Box<Spanned<Expr>>, CompareOp, Box<Spanned<Expr>>),
  Call(String, Vec<Spanned<Expr>>),
  /// `f(name: value, ...)` (plan 39's Decision log) — a plain function
  /// call whose arguments are resolved entirely at compile time by
  /// name-to-position matching against the callee's declared parameter
  /// names, never a runtime hash/dispatch. Scoped to bare function
  /// calls only (never `.method(...)`/`.new(...)`) and to all-keyword
  /// call sites (mixing positional and keyword arguments in one call is
  /// out of scope) — a call with any `name:` argument parses as this
  /// node instead of `Expr::Call`.
  CallKw(String, Vec<(String, Spanned<Expr>)>),
  /// `ClassName.new(args)`.
  New(String, Vec<Spanned<Expr>>),
  /// `receiver.method(args)` — `args` is empty for a bare `receiver.method`
  /// call (plan `08`'s Point example, `sum`/`initialize` never take
  /// arguments); plan `10` is the first to actually parse a non-empty
  /// argument list here.
  MethodCall(Box<Spanned<Expr>>, String, Vec<Spanned<Expr>>),
  /// `receiver?.method(args)` (plan 73's Decision log — replaces plan
  /// 43's `&.` outright, same AST shape, new `Option[T]` semantics): kept
  /// distinct from `MethodCall`, not a reuse — sema's dispatch (an
  /// `Option[T]` receiver only) and codegen (a real match-on-tag branch +
  /// PHI, producing an `Option[U]` result: `None` if the receiver was
  /// `None`, `Some(v.method(args))` if it was `Some(v)`) are both
  /// genuinely different, not just an evaluation-order variant of an
  /// ordinary call.
  SafeCall(Box<Spanned<Expr>>, String, Vec<Spanned<Expr>>),
  /// `lhs ?? rhs` (plan 73's Decision log) — `Option[T]` coalesce: `rhs`
  /// (a plain `T`) only if `lhs` (an `Option[T]`) is `None`; `lhs`'s own
  /// unwrapped `Some` payload otherwise. Purely a desugaring onto the
  /// same match machinery `Option[T]` construction/pattern-matching
  /// already uses — no new runtime representation.
  Coalesce(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `@name` — instance-variable read, valid only inside a method body.
  InstanceVar(String),
  /// `[e1, e2, ...]` — an array literal.
  ArrayLit(Vec<Spanned<Expr>>),
  /// `array[index]` — an indexed read.
  Index(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `->(params) -> ReturnType { body }` — a lambda literal (plan `10`'s
  /// Decision log: by-value capture, top-level-`Let`-only, statically
  /// dispatched `.call`).
  Lambda {
    params: Vec<Param>,
    return_type: TypeExpr,
    body: Vec<Spanned<Stmt>>,
  },
  /// `true`/`false` (plan 25's Decision log) — a real `Boolean` value,
  /// not just `Compare`'s byproduct.
  Bool(bool),
  /// `{k1 => v1, k2 => v2, ...}` (plan 25's Decision log) — `Int64`
  /// keys only, no `Symbol`-keyed `{a: 1}` shorthand.
  HashLit(Vec<(Spanned<Expr>, Spanned<Expr>)>),
  /// `Array.new(size)` (plan 25's Decision log) — a dedicated node, not
  /// a reuse of `Expr::New`, since `Array` is a reserved keyword, not a
  /// class name in the class registry.
  ArrayNew(Box<Spanned<Expr>>),
  /// `a & b` (plan 28's Decision log) — `Int64`-only bitwise AND.
  BitAnd(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a | b` — `Int64`-only bitwise OR.
  BitOr(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a ^ b` — `Int64`-only bitwise XOR.
  BitXor(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `~a` — `Int64`-only bitwise NOT.
  BitNot(Box<Spanned<Expr>>),
  /// `a << b` — `Int64`-only left shift.
  Shl(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `a >> b` — `Int64`-only arithmetic (signed) right shift.
  Shr(Box<Spanned<Expr>>, Box<Spanned<Expr>>),
  /// `return a, b` (plan 39's Decision log) — legal ONLY as `Stmt::
  /// Return`'s direct argument, never a first-class value that can be
  /// stored in a variable, passed as an argument, or nested inside
  /// another tuple. Sema accepts this only when the enclosing
  /// function's declared return type is `Type::Tuple` of matching
  /// arity and per-position types.
  TupleLit(Vec<Spanned<Expr>>),
  /// `Ok(value)` (plan 53's Decision log) — a `Result[T, E]` constructor,
  /// checked only in the three expected-type-providing positions
  /// (`Let`'s declared type, `Assign`'s recorded type, `Return`'s
  /// threaded return type) since `infer_expr_type` carries no
  /// expected-type parameter anywhere in this compiler.
  Ok(Box<Spanned<Expr>>),
  /// `Err(value)` — `Result[T, E]`'s other constructor, same
  /// expected-type-position restriction as `Ok` above.
  Err(Box<Spanned<Expr>>),
  /// `expr?` (plan 53's Decision log) — legal ONLY as a `Stmt::Let`'s or
  /// `Stmt::Assign`'s direct value, inside a function whose own declared
  /// return type is `Result[_, E]` for an exactly-matching `E`. Unwraps
  /// `Ok`'s payload in place; on `Err`, immediately returns the same
  /// `Result` value from the enclosing function, unchanged.
  Try(Box<Spanned<Expr>>),
  /// `ActorName.spawn(args)` (plan 54's Decision log) — deliberately
  /// distinct from `Expr::New`, not a reuse disambiguated later in sema:
  /// keeps `Counter.new(0)`/`AnyClass.spawn(0)` both real, symmetric,
  /// grammar-level-textually-distinct call forms. Allocates the
  /// instance from its own arena (plan 51) instead of the shared heap;
  /// every method call on the result still compiles as an ordinary
  /// synchronous call in this plan — only isolation is proven here.
  Spawn(String, Vec<Spanned<Expr>>),
  /// `supervise do <name> = <Class>.spawn(<args>) ... end` (plan 57's
  /// Decision log) — a block, reusing plan 34's own established
  /// mechanism, not a declarative list (`.spawn`'s own argument lists
  /// are ordinary Emerald expressions, and a block body is the only
  /// existing shape able to hold a statement sequence with ordinary
  /// expression arguments). Deliberately NOT a new restricted AST
  /// shape either — an ordinary `Vec<Spanned<Stmt>>`, the exact same
  /// list an `if`/`while`/method body already carries; sema (not the
  /// grammar) restricts it to a flat list of `<name> = <Class>.spawn
  /// (<args>)` (or a bare, unnamed `<Class>.spawn(<args>)`) statements,
  /// the same "grammar stays general, sema narrows" discipline plan
  /// 31 already established for its own restricted-shape leaves.
  Supervise(Vec<Spanned<Stmt>>),
  /// `ActorName.remote(addr, name)` (plan 60's Decision log) —
  /// `.spawn`'s distributed counterpart: connects to `addr` (a
  /// `"host:port"` `String`) and resolves `name`, raising a real,
  /// catchable `RemoteActorError` on any failure rather than returning
  /// a null the caller could dereference. The returned value is usable
  /// at every call site `.spawn`'s own already is (Design decision 1 —
  /// both are the identical tagged `EmeraldActorRef*` shape).
  Remote {
    class: String,
    addr: Box<Spanned<Expr>>,
    name: Box<Spanned<Expr>>,
  },
  /// `ActorName.locate(key, args...)` (plan 65's Decision log,
  /// `leaf-virtual-actor-placement`) — Orleans-style on-demand virtual
  /// actor placement: `key` (a `String`) consistent-hashes over the
  /// discovered `EMERALD_PEERS` set to decide the owning node; `args`
  /// are the class's real `initialize` arguments, used only the first
  /// time this key is ever activated (a lazily-spawned instance is
  /// cached thereafter, on whichever node owns it). Deliberately a
  /// THIRD, textually-disjoint reserved call form alongside `.spawn`/
  /// `.remote` (never a reuse of either) — sema decides which
  /// receivers are valid, same as both siblings.
  Locate {
    class: String,
    key: Box<Spanned<Expr>>,
    args: Vec<Spanned<Expr>>,
  },
  /// `comptime <expr>` (plan 61's Decision log) — the operand binds at
  /// `UnaryExpr`'s tight precedence tier, the exact same real, build-
  /// verified-necessary placement `-`/`!`/`~` already use (an
  /// un-delimited prefix keyword whose operand could itself extend via a
  /// trailing binary operator or postfix `[...]`/`?` is genuinely
  /// ambiguous at any looser tier — see `UnaryExpr`'s own grammar
  /// comment for the two concrete counter-derivations LALRPOP's build
  /// actually reported), a real deviation from the task brief's own
  /// literal `<e:Expr>` snippet, not the brief's own worked examples,
  /// which are both plain calls. Legal grammatically wherever a
  /// `UnaryExpr` is reachable, narrowed further to exactly two positions
  /// (a top-level constant's
  /// initializer, `Array.new`'s size argument) by `emerald-sema`, the
  /// same "grammar stays general, sema narrows" discipline plan 31
  /// already established. Evaluated by
  /// `emerald-codegen`'s tree-walking `ComptimeInterpreter`, never by a
  /// JIT and never at sema time (sema only checks the inner expression's
  /// *shape* against the legal-node allow-list, which always terminates;
  /// only codegen actually runs it, which is why the step ceiling lives
  /// there).
  Comptime(Box<Spanned<Expr>>),
}

/// One statement in a block (a function body or the program's top level).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
  Let {
    name: String,
    ty: TypeExpr,
    value: Spanned<Expr>,
    /// Plan 72's Decision log: `true` only when this declaration spelled
    /// the `var` keyword (`var name: Type = expr`) — a binding declared
    /// without it is immutable by default (Sable §5), and `emerald-sema`
    /// rejects any later `Stmt::Assign`/`Stmt::MultiAssign`/`OrAssign`/
    /// `AndAssign` targeting a `false` binding with a real diagnostic
    /// rather than silently accepting the reassignment. Never read by
    /// `emerald-codegen` — mutability is a purely static, sema-time
    /// property with no runtime representation.
    is_var: bool,
  },
  /// `@name = value` — instance-variable write, valid only inside a
  /// method body. No type annotation (the field's type is already
  /// declared on the class), unlike `Let`.
  SetField {
    name: String,
    value: Spanned<Expr>,
  },
  /// `array[index] = value` — an indexed write.
  SetIndex {
    array: Spanned<Expr>,
    index: Spanned<Expr>,
    value: Spanned<Expr>,
  },
  /// `name = value` — reassignment of an already-declared plain local,
  /// no type annotation restated (plan 31's Decision log: the
  /// prerequisite shape neither compound nor multiple assignment can
  /// exist without). `+=`/`-=`/`*=`/`/=`/`%=` desugar into this at
  /// parse time (`value` becomes the corresponding `Expr::Add`/`Sub`/
  /// `Mul`/`Div`/`Rem` over the current value). Sema requires `name`
  /// already present in scope — this is never a fresh declaration.
  Assign {
    name: String,
    value: Spanned<Expr>,
  },
  /// `n1, n2, ... = v1, v2, ...` — fixed-arity multiple assignment over
  /// already-declared plain locals (plan 31's Decision log: no fresh
  /// multi-declaration, no mixed instance-var/index targets, no
  /// splat). `values` are evaluated in full before any `names` target
  /// is written — codegen's proof this actually matters is a real
  /// swap (`a, b = b, a`).
  MultiAssign {
    names: Vec<String>,
    values: Vec<Spanned<Expr>>,
  },
  If {
    cond: Spanned<Expr>,
    then_branch: Vec<Spanned<Stmt>>,
    else_branch: Option<Vec<Spanned<Stmt>>>,
  },
  While {
    cond: Spanned<Expr>,
    body: Vec<Spanned<Stmt>>,
  },
  Return(Option<Spanned<Expr>>),
  Break,
  Next,
  Expr(Spanned<Expr>),
  /// `raise <expr>` — `expr` must evaluate to a class instance (plan
  /// `11`'s Decision log: in practice always a direct `ClassName.new(args)`
  /// call, the only shape codegen supports).
  Raise(Spanned<Expr>),
  /// `begin body rescue Type => e ... [rescue => e2 ...] [ensure ...]
  /// end` (plan 38's Decision log) — one or more `rescue` clauses tried
  /// in source order (subtype-aware: a clause naming a superclass
  /// matches any raised subclass), an optional trailing `ensure` that
  /// always runs, on every exit path (normal fallthrough, a matched
  /// `rescue` clause's own fallthrough, and the mismatch-exhausted
  /// re-raise path alike). Generalizes plan 11's original single-typed-
  /// clause, no-`ensure` shape (`rescues.len() == 1`, `ensure: None`).
  Begin {
    body: Vec<Spanned<Stmt>>,
    rescues: Vec<RescueClause>,
    ensure: Option<Vec<Spanned<Stmt>>>,
  },
  /// `case scrutinee when v1, v2 ... when v3 ... else ... end` (plan
  /// 20's Decision log): value-match via the same `CompareOp::Eq`
  /// `Expr::Compare` already performs, over an `Int64`-only scrutinee —
  /// not `spec/GRAMMAR.md`'s eventual method-dispatched `===` (no
  /// operator-overload dispatch mechanism exists in this compiler).
  /// Each arm's `Vec<Expr>` holds one or more `when` values; the arm
  /// matches if the scrutinee equals *any* of them. First matching arm
  /// wins, source order, matching Ruby's own semantics.
  /// Plan 52's Decision log: `arms` now carries a `CasePattern` per arm
  /// instead of a bare `Vec<Spanned<Expr>>` — `Values` is this
  /// statement's original Int64-value-match shape, unchanged behavior,
  /// just wrapped; `Variant` is this plan's addition (a closed-enum
  /// tag match with per-arm-scoped bindings).
  Case {
    scrutinee: Spanned<Expr>,
    arms: Vec<CaseArm>,
    else_body: Option<Vec<Spanned<Stmt>>>,
  },
  /// `for var in [e1, e2, ...] body end` (plan 30's Decision log) —
  /// `elements` is restricted to a literal array at the grammar level
  /// (an arbitrary `Array[T]`-typed scrutinee is a parse error, not a
  /// sema error), since `Array[T]`'s own representation carries no
  /// runtime length to iterate against. Codegen desugars this straight
  /// into the same index-based `while` shape plan 09's array traversal
  /// already proved.
  For {
    var: String,
    elements: Vec<Spanned<Expr>>,
    body: Vec<Spanned<Stmt>>,
  },
  /// `yield <args>` (plan 34's Decision log) — legal only inside a
  /// function that declares `Function.block_param`. Codegen lowers this
  /// via call-site specialization: a `yield`-using function is compiled
  /// fresh per call site that attaches a literal block, and each
  /// `Stmt::Yield` becomes a direct call to that block's synthesized
  /// function — never an indirect/first-class call.
  Yield(Vec<Spanned<Expr>>),
  /// `for var in start..end body end` (`exclusive: false`) or
  /// `start...end` (`exclusive: true`) — plan 37's Decision log: no
  /// first-class `Range` value exists anywhere (this shape only ever
  /// appears directly as a `for...in` scrutinee, grammar-restricted the
  /// same way `Stmt::For`'s own literal-array scrutinee is); `start`/
  /// `end` are arbitrary `Int64`-typed expressions, evaluated once each
  /// before the loop begins, not just literals.
  ForRange {
    var: String,
    start: Spanned<Expr>,
    end: Spanned<Expr>,
    exclusive: bool,
    body: Vec<Spanned<Stmt>>,
  },
  /// `retry` (plan 38's Decision log) — legal only inside a `rescue`
  /// clause's own body (not the `begin`'s try body, not `ensure`),
  /// re-attempts the enclosing `begin` construct from scratch. Does
  /// not run the enclosing `ensure` on its way back — it doesn't exit
  /// the `begin` construct at all, unlike `Return`/`Break`/`Next`/
  /// `Raise`.
  Retry,
  /// `case scrutinee when Ok(ok_var) ok_body when Err(err_var) err_body
  /// end` (plan 53's Decision log) — a small, dedicated destructuring
  /// form for `Result[T, E]`, deliberately independent of plan 52's
  /// `Stmt::Case`/`CasePattern` mechanism (see that plan's own Decision
  /// log for why). Both arms are mandatory, fixed order (`Ok` then
  /// `Err`), no `else`. `ok_var`/`err_var` are bound only inside their
  /// own arm's body, at the scrutinee's real, statically-known `T`/`E`.
  MatchResult {
    scrutinee: Spanned<Expr>,
    ok_var: String,
    ok_body: Vec<Spanned<Stmt>>,
    err_var: String,
    err_body: Vec<Spanned<Stmt>>,
  },
}

/// One `when ... body` arm of a `Stmt::Case` (plan 22's Decision log —
/// named to keep `arms`' field type out of clippy's `type_complexity`
/// lint, not a new AST concept).
pub type CaseArm = (CasePattern, Vec<Spanned<Stmt>>);

/// Plan 52's Decision log: `Values` is plan 20's original `when v1, v2,
/// ...` shape (Int64-only value-match), wrapped unchanged; `Variant` is
/// this plan's `when VariantName(b1, b2, ...)` closed-enum tag match —
/// `bindings` are plain identifiers only (no nested pattern), scoped to
/// this arm's own body alone, never the surrounding flat `env`.
#[derive(Debug, Clone, PartialEq)]
pub enum CasePattern {
  Values(Vec<Spanned<Expr>>),
  Variant { name: String, bindings: Vec<String> },
}

/// One `rescue` clause of a `Stmt::Begin` (plan 38's Decision log).
/// `class_name: None` is a bare `rescue => e` catch-all — matches
/// unconditionally, and `var` is never bound in `env` (no universal
/// root class exists to type it at).
#[derive(Debug, Clone, PartialEq)]
pub struct RescueClause {
  pub class_name: Option<String>,
  pub var: String,
  pub body: Vec<Spanned<Stmt>>,
}

/// `requires <expr>` / `ensures <expr>` on a function signature (plan
/// 62's Decision log) — a small, dedicated struct rather than a bare
/// `Spanned<Expr>`, because a runtime contract-violation message needs
/// the clause's own real source text and this codebase has no
/// unparser. `text` is filled in by a small, targeted post-parse pass
/// over the freshly-built `Program` (`emerald_parser::lib::rs`'s own
/// `fill_contract_text`, mirroring plan 47's identical `loc`-capture
/// technique) — a fresh `Contract` straight out of the grammar action
/// always has an empty `text`/`line: 0` placeholder.
#[derive(Debug, Clone, PartialEq)]
pub struct Contract {
  pub expr: Spanned<Expr>,
  pub text: String,
  pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: TypeExpr,
  pub body: Vec<Spanned<Stmt>>,
  /// `&blk` in the parameter list (plan 34's Decision log) — a bare
  /// name, not a `Param`: unlike an ordinary parameter, its type can't
  /// be pinned at the function's own declaration site (no first-class
  /// `Proc` value exists to check a caller's argument against; a
  /// `yield`-using function is instead call-site-specialized fresh
  /// against whichever block literal a given call attaches). `None` for
  /// every function that doesn't declare one — unchanged from before
  /// this plan.
  pub block_param: Option<String>,
  /// `*xs: Int64` (plan 39's Decision log) — the trailing splat
  /// parameter's *element* type only (e.g. `Int64`, not `Array[Int64]`).
  /// Ordered after ordinary/defaulted parameters and before an optional
  /// `&blk`; packed into a real `Array[Elem]` at each call site (plan
  /// 09's actual representation), never a variadic LLVM signature — the
  /// compiled function itself stays fixed-arity. `None` for every
  /// function that doesn't declare one.
  pub splat_param: Option<Param>,
  /// `[T: Bound]` (plan 41's Decision log) — legal only on a top-level
  /// function; sema rejects a non-empty `type_params` found on a class
  /// method or module function with a real diagnostic, and rejects more
  /// than one entry with a real diagnostic (the grammar's `TypeParamList`
  /// is a general comma list — the single-type-parameter, bound-only
  /// restriction is entirely sema's job, not the grammar's). Empty
  /// `Vec` for every function that doesn't declare one — additive,
  /// source-compatible with every prior plan.
  pub type_params: Vec<TypeParam>,
  /// `comptime def <name>(...) -> T ... end` (plan 61's Decision log) —
  /// `false` for every pre-existing declaration, additive and
  /// source-compatible. Legal only on a top-level function; `emerald-
  /// sema` rejects `true` found on a class or module method with a real
  /// diagnostic, mirroring `type_params`'s own top-level-only precedent
  /// immediately above. A function with `is_comptime: true` is checked,
  /// once at registration, against `check_comptime_legal`'s allow-list
  /// (recursively re-checked for every `comptime` function it itself
  /// calls) — never lowered to LLVM IR unless some ordinary, non-
  /// `comptime` call site also reaches it (see `leaf-comptime-const-
  /// context-integration`'s Decision log).
  pub is_comptime: bool,
  /// `requires <expr>` clauses (plan 62's Decision log) — empty `Vec`
  /// for every function that doesn't declare one, additive and
  /// source-compatible, mirroring `type_params`'/`is_comptime`'s own
  /// precedent. Grammatically reachable only on a top-level `FuncDef`,
  /// never a `MethodDef` — a real, disclosed narrower cut than
  /// `type_params`'s own "grammatically reachable on methods, sema-
  /// rejected" precedent (see this plan's own Decision log for why a
  /// stricter grammar-level fence is the simpler choice here).
  pub requires: Vec<Contract>,
  /// `ensures <expr>` clauses — same shape/precedent as `requires`
  /// immediately above. `expr` may reference the pseudo-identifier
  /// `result` (the function's own about-to-be-returned value) — this
  /// needs no grammar support at all: `result` parses as an ordinary
  /// `Expr::Ident("result")`, given meaning only by `emerald-sema`'s
  /// own `env` construction while checking an `ensures` clause (the
  /// same "grammar stays general, sema narrows" discipline plan 31
  /// already established, applied here in reverse — adding meaning to
  /// an existing shape rather than restricting one).
  pub ensures: Vec<Contract>,
  /// `pure def ...`/`pure def ... end` inside a class/module (plan 63's
  /// Decision log) — `false` for every pre-existing declaration,
  /// additive and source-compatible. Grammatically reachable on a
  /// top-level function, a module function, AND a class/actor method
  /// (`FuncDef`/`MethodDef` both gain the identical optional `"pure"?`
  /// prefix) — deliberately left general at the grammar level; `emerald-
  /// sema`'s own whole-call-graph purity checker is what rejects it on
  /// an actor method specifically (the same "grammar stays general,
  /// sema narrows" discipline `type_params`/`is_comptime`/`requires`/
  /// `ensures` already established).
  pub is_pure: bool,
}

/// `T: Comparable` inside a generic function's or generic class's `[...]`
/// clause. Plan 58's Decision log: `bound` widens from a mandatory
/// `String` (plan 41's original, function-only design — no `def
/// identity[T](x: T)` unbounded form) to `Option<String>`, since a
/// generic *class*'s own headline case (`class Box[T] ... end`,
/// `class Stack[T] ... end`) has no bound at all. A real, disclosed,
/// source-compatible widening of the one `TypeParam` shape plan 41
/// already shipped, not a second, class-only type-parameter struct —
/// every FUNCTION-side reader keeps requiring a bound (now enforced by
/// sema's own diagnostic instead of structurally by the grammar).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
  pub name: String,
  /// `[T: Bound1 + Bound2 + ...]` (plan 88's Decision log) — widened
  /// from a single `Option<String>` to a real bound SET: an empty
  /// `Vec` is the bound-less case (`class Box[T] ... end`'s own
  /// headline case, unchanged), and every entry must be satisfied
  /// (conjunction, never disjunction) for a concrete type argument to
  /// be accepted. A top-level generic FUNCTION's own registration still
  /// requires at least one bound (`emerald-sema`'s own diagnostic,
  /// unchanged) — only the COUNT of bounds a single type parameter may
  /// carry widens here, not whether one is required at all.
  pub bounds: Vec<String>,
}

/// One `fn <name>(<params>): <ReturnType>` required-method declaration
/// inside an `interface ... end` body (plan 89's Decision log widens
/// `InterfaceDef` from exactly one such method to a `Vec` of these).
/// `type_params` is the method's OWN `[U]`/`[U: Bound]` clause (plan
/// 89's worked example, `fn map[U](f: Proc[T, U]): Array[U]`) — empty
/// for an ordinary, non-generic required method, mirroring `Function.
/// type_params`'s identical "empty means not generic" convention.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethod {
  pub method_name: String,
  pub type_params: Vec<TypeParam>,
  pub params: Vec<Param>,
  pub return_type: TypeExpr,
}

/// `interface Comparable fn compare_to(other: Self): Int64 end` (plan
/// 41's Decision log) — originally structurally exactly one required
/// method with no type-parameter clause at all. Plan 89's Decision log
/// widens this two ways, both additive and source-compatible: `type_
/// params` is the interface's OWN `[T]`/`[T: Bound]` clause (`interface
/// Iterable[T] ... end`), empty for a non-generic interface exactly like
/// every other `type_params` field in this AST; `methods` widens from a
/// single required method to a `Vec` (still non-empty — the grammar
/// requires at least one `InterfaceMethod`, mirroring the original
/// "at least one" cardinality exactly, just no longer capped at one).
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDef {
  pub name: String,
  pub type_params: Vec<TypeParam>,
  pub methods: Vec<InterfaceMethod>,
}

/// A class declaration: fields (reusing `Param`'s `{name, ty}` shape —
/// structurally identical to a parameter declaration) and methods
/// (ordinary `Function`s, including `initialize`). `superclass` is
/// `class Dog < Animal`'s `Animal` (plan 32's Decision log) — single
/// inheritance only, no `include`/`extend` mixin composition, no real
/// dynamic dispatch (every call still resolves by the receiver's
/// *declared* class, same as before this plan).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
  pub name: String,
  pub superclass: Option<String>,
  /// `implements Comparable` (plan 41's Decision log) — a single
  /// interface name, checked structurally (conformance, not
  /// assignability) at class-declaration time. `None` for every class
  /// that doesn't declare one. Plan 89's Decision log widens the second
  /// tuple element from nothing to `Vec<TypeExpr>` — `implements
  /// Iterable[Int64]`'s `[Int64]` — empty for a bare `implements
  /// Comparable` naming a non-generic interface, source-compatible with
  /// every pre-plan-89 `implements` clause.
  pub implements: Option<(String, Vec<TypeExpr>)>,
  pub fields: Vec<Param>,
  pub methods: Vec<Function>,
  /// `class Point derive Comparable ... end` (plan 61's Decision log) —
  /// `None` for every class that doesn't declare one. The only accepted
  /// value is `Some("Comparable".to_string())`; any other name is a real
  /// `emerald_parser::expand_derives` diagnostic, not a grammar-level
  /// restriction (the grammar accepts an arbitrary `Ident` here, the
  /// same "grammar stays general" split `implements`'s own class-only
  /// reachability already uses).
  pub derive: Option<String>,
  /// `class Stack[T]`/`class Box[T: Comparable]` (plan 58's Decision
  /// log) — empty `Vec` for every class that doesn't declare one,
  /// additive and source-compatible, mirroring `Function.type_params`'s
  /// own precedent above. A non-empty `type_params` marks this `ClassDef`
  /// as a raw, unresolved TEMPLATE — `emerald-sema` registers it into a
  /// separate `generic_classes` registry instead of the ordinary
  /// `classes` table, and it is never monomorphized without a real
  /// concrete instantiation actually written somewhere in the program.
  pub type_params: Vec<TypeParam>,
}

/// One `fn <name>(<params>): <ReturnType>` declaration inside an
/// `unsafe extern "C" { ... }` block (plan 59's Decision log). Reuses
/// `Param`'s `{name, ty}` shape verbatim for params, the same reuse
/// `ClassDef`/`InterfaceDef` already make — but deliberately `:` rather
/// than `def`'s `->` (grammar-level, not just the mandatory `unsafe`
/// prefix) so an extern declaration is visually distinct from an
/// ordinary function signature at a glance.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternFn {
  pub name: String,
  pub params: Vec<Param>,
  pub return_type: TypeExpr,
}

/// `unsafe extern "C" { fn ... ... }` (plan 59's Decision log) — `abi`
/// is the decoded string literal (`"C"`), checked against exactly that
/// value by `emerald-sema` (the explicit C++ decline), not restricted
/// at the grammar level (a bare string literal, not a reserved keyword
/// set, so a bad ABI is a real sema diagnostic naming it, not a parse
/// error).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
  pub abi: String,
  pub fns: Vec<ExternFn>,
}

/// A namespace-only module (plan `12`'s Decision log — `SEMANTICS.md`
/// §10 explicitly removes `include`/`extend` mixin composition): a flat
/// collection of methods, called as `ModuleName.method(args)`. No
/// fields — there's no instance for them to live on.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
  pub name: String,
  pub methods: Vec<Function>,
}

/// `Circle(Float64)` inside `enum Shape = Circle(Float64) | ...` (plan
/// 52's Decision log) — `fields` are raw `TypeName` strings, the same
/// string-based type-annotation convention `Param.ty` already uses,
/// deferring resolution to sema like every other type annotation in
/// this AST. At least one field is required — no bare, nullary tag
/// variants in this plan.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
  pub name: String,
  pub fields: Vec<TypeExpr>,
}

/// `enum Shape = Circle(Float64) | Square(Float64) | ...` (plan 52's
/// Decision log) — a CLOSED set of variants, fixed at declaration: no
/// grammar path reopens an already-declared enum from another file,
/// the deliberate opposite of `ClassDef`'s open, extensible hierarchy.
/// Pure data — no method bodies, no `implements`, no superclass.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
  pub name: String,
  pub variants: Vec<EnumVariant>,
  /// `enum Option[T] = Some(T) | None` (plan 73's Decision log) — mirrors
  /// `ClassDef.type_params` exactly: empty for every ordinary, non-generic
  /// enum (unchanged from plan 52), non-empty marks this `EnumDef` as a
  /// raw, unresolved TEMPLATE — `emerald-sema`/`emerald-codegen` route it
  /// into a separate `generic_enums` registry instead of the ordinary
  /// enum table, monomorphized on demand per concrete type argument the
  /// same way a generic class already is (plan 41/58's strategy, reused
  /// here for the first time on an enum). `Option[T]` itself is a
  /// compiler-synthesized `EnumDef` built directly in Rust (never parsed
  /// from source), so it can declare `None`'s zero-field variant despite
  /// `EnumVariant`'s own grammar-level "at least one field" restriction,
  /// which still applies unchanged to every user-written `enum`.
  pub type_params: Vec<TypeParam>,
}

/// `actor Counter ... end` (plan 54's Decision log) — a flat,
/// non-inheriting top-level declaration reusing `ClassDef`'s own field/
/// method syntax verbatim, its own struct rather than `ClassDef` plus a
/// marker flag: unlike `ClassDef`, `superclass` doesn't exist as a
/// field at all — `actor Foo < Bar` is unrepresentable in this AST, a
/// real grammar-level decline, not merely a semantic rejection of an
/// otherwise-parseable shape. Instances live in their own arena (plan
/// 51's mechanism) instead of the shared never-freed heap; this plan
/// proves isolation only — every method call still compiles as an
/// ordinary, synchronous call (no scheduler, no mailbox exist yet).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorDef {
  pub name: String,
  pub fields: Vec<Param>,
  pub methods: Vec<Function>,
}

/// A single top-level construct: a function definition, a class
/// definition, a module definition, or a top-level statement (e.g.
/// `x: Int64 = 10`, `if x > 5 ... end`, `puts x`).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  Function(Function),
  Class(ClassDef),
  Module(ModuleDef),
  /// `actor Counter ... end` (plan 54's Decision log).
  Actor(ActorDef),
  /// `enum Shape = Circle(Float64) | ...` (plan 52's Decision log).
  Enum(EnumDef),
  /// `interface Comparable ... end` (plan 41's Decision log) — a
  /// general, user-declarable grammar production, not a fourth
  /// hardcoded builtin the way `Array`/`Hash`/`Proc` are.
  Interface(InterfaceDef),
  /// `unsafe extern "C" { ... }` (plan 59's Decision log) — the one
  /// deliberate, explicitly-fenced escape from this compiler's
  /// otherwise-total static safety.
  Extern(ExternBlock),
  Stmt(Spanned<Stmt>),
  /// `require <path>` (plan 23's Decision log) — a bare, unquoted,
  /// `/`-separated path (no string-literal syntax dependency), always
  /// relative to the *containing* file, `.em` implied. Reachable only
  /// from `Program`'s top-level `Item*` rule, never from `Stmt*` — a
  /// `require` inside a function body is a real parse error, not
  /// silently accepted. Plan 17's `emerald-driver` doesn't splice
  /// multi-file `require`s itself (that's `emerald-cli`'s own
  /// `require.rs`, plan 46) — one reaching `emerald-sema`/
  /// `emerald-codegen` directly is a real, disclosed gap: sema treats
  /// it as a no-op, codegen returns a descriptive `Err` rather than
  /// silently ignoring it.
  Require(String),
  /// `test "description" do ... end` (plan 47's Decision log) — a
  /// narrow, self-contained grammar addition (its own `"do" Stmt*
  /// "end"` production, not plan 34's general `do |params| ... end`
  /// block syntax, which explicitly declined a `do...end` form).
  /// `body` is compiled as its own synthesized zero-parameter,
  /// `Void`-returning function, wrapped in `begin ... rescue
  /// AssertionError => e ... end` by `emerald_codegen::
  /// compile_test_harness` (`leaf-test-runner`) — never reachable via
  /// the ordinary `emerald <file>` compile path, which rejects a
  /// `Program` containing one.
  Test {
    description: String,
    body: Vec<Spanned<Stmt>>,
  },
  /// A top-level construct LALRPOP's `!` error-recovery mechanism
  /// resynchronized past (plan 26's Decision log) — a real parse error
  /// was recorded for it. Never appears in a `Program` `parse`/
  /// `parse_named` actually returns `Ok(_)` for: the moment any
  /// `Item::Error` (or any other recovered error) exists, the whole
  /// call returns `Err(Vec<ParseError>)` instead. `emerald-sema`/
  /// `emerald-codegen` never see this variant — their match arms for
  /// it are `unreachable!()`, not a defensive `Err`.
  Error,
}

/// A full Emerald source file — inception §17's milestone unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
  pub items: Vec<Item>,
}
